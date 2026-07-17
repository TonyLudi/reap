use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use futures_util::stream::{FuturesUnordered, StreamExt};
use reap_core::AccountUpdate;
use reap_order::{
    CancelOutcome, GatewayError, OkxOrderGateway, RegularSubmitCompletion, SubmitOutcome,
    SubmitPreparation, okx_order_dispatch_key,
};
use reap_telemetry::{AlertDeliveryFailure, AlertRuntime, AlertSink, AlertStats};
use reap_venue::{RemoteFill, RemoteOrder};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use super::{
    CancelAction, ConnectionStatus, LiveRuntimeError, OkxOrderWsStatus, OperatorEnvelope,
    OperatorService, SubmitAction, elapsed_us, unix_time_ms,
};

pub(super) struct DispatchState {
    pub(super) control_rx: mpsc::Receiver<RuntimeEvent>,
    pub(super) order_senders: HashMap<String, mpsc::Sender<OrderTaskCommand>>,
    pub(super) order_tasks: Vec<JoinHandle<()>>,
    pub(super) operator_service: Option<OperatorService>,
    pub(super) operator_rx: Option<mpsc::Receiver<OperatorEnvelope>>,
    pub(super) operator_shutdown_reason: Option<String>,
    pub(super) alert_runtime: Option<AlertRuntime>,
    pub(super) alert_sink: Option<AlertSink>,
    pub(super) alert_failures: Option<mpsc::Receiver<AlertDeliveryFailure>>,
    pub(super) alert_shutdown_timeout_ms: u64,
    pub(super) alert_delivery_failure_is_fatal: bool,
    pub(super) observed_alert_delivery_failures: u64,
    pub(super) alert_stats: AlertStats,
}

pub(super) enum RuntimeEvent {
    Raw {
        source_id: usize,
        envelope: reap_core::RawEnvelope,
    },
    Connection {
        source_id: usize,
        status: ConnectionStatus,
    },
    OrderTransport(OkxOrderWsStatus),
    SubmitComplete {
        account_id: String,
        symbol: String,
        outcome: SubmitOutcome,
        ts_ms: u64,
        latency_us: Option<u64>,
    },
    SubmitFailed {
        account_id: String,
        client_order_id: String,
        symbol: String,
        ts_ms: u64,
        ambiguous: bool,
        reason: String,
    },
    CancelComplete {
        account_id: String,
        symbol: String,
        outcome: CancelOutcome,
        ts_ms: u64,
        latency_us: u64,
    },
    CancelFailed {
        account_id: String,
        client_order_id: String,
        symbol: String,
        ts_ms: u64,
        ambiguous: bool,
        reason: String,
    },
    RemoteState {
        account_id: String,
        remote_orders: Vec<RemoteOrder>,
        remote_fills: Vec<RemoteFill>,
        remote_account: AccountUpdate,
        ts_ms: u64,
    },
    ReconcileFailed {
        account_id: String,
        ts_ms: u64,
        reason: String,
    },
    Fatal(RuntimeTaskFailure),
}

pub(super) enum RuntimeTaskFailure {
    Gateway(String),
    DeadmanHeartbeat(String),
    ExchangeClockSkew(String),
    ExchangeClockCheck(String),
    ExchangeStatus(String),
    ExchangeStatusCheck(String),
    ExchangeFeeDrift(String),
    ExchangeFeeCheck(String),
    ExchangeInstrumentDrift(String),
    ExchangeInstrumentCheck(String),
    AccountConfigDrift(String),
    AccountConfigCheck(String),
}

impl From<RuntimeTaskFailure> for LiveRuntimeError {
    fn from(value: RuntimeTaskFailure) -> Self {
        match value {
            RuntimeTaskFailure::Gateway(message) => Self::GatewayTask(message),
            RuntimeTaskFailure::DeadmanHeartbeat(message) => Self::DeadmanHeartbeat(message),
            RuntimeTaskFailure::ExchangeClockSkew(message) => Self::ExchangeClockSkew(message),
            RuntimeTaskFailure::ExchangeClockCheck(message) => Self::ExchangeClockCheck(message),
            RuntimeTaskFailure::ExchangeStatus(message) => Self::ExchangeStatus(message),
            RuntimeTaskFailure::ExchangeStatusCheck(message) => Self::ExchangeStatusCheck(message),
            RuntimeTaskFailure::ExchangeFeeDrift(message) => Self::ExchangeFeeDrift(message),
            RuntimeTaskFailure::ExchangeFeeCheck(message) => Self::ExchangeFeeCheck(message),
            RuntimeTaskFailure::ExchangeInstrumentDrift(message) => {
                Self::ExchangeInstrumentDrift(message)
            }
            RuntimeTaskFailure::ExchangeInstrumentCheck(message) => {
                Self::ExchangeInstrumentCheck(message)
            }
            RuntimeTaskFailure::AccountConfigDrift(message) => Self::AccountConfigDrift(message),
            RuntimeTaskFailure::AccountConfigCheck(message) => Self::AccountConfigCheck(message),
        }
    }
}

pub(super) enum OrderTaskCommand {
    Submit {
        action: SubmitAction,
        enqueued_at: Instant,
    },
    Cancel {
        action: CancelAction,
        enqueued_at: Instant,
    },
    Flush(oneshot::Sender<()>),
    Shutdown,
}

pub(super) enum ReconcileTaskCommand {
    Reconcile {
        restored_orders: Vec<ReconcileOrderRef>,
        command_flush: Option<oneshot::Receiver<()>>,
    },
    Shutdown,
}

pub(super) enum SafetyTaskCommand {
    DisableDeadMan {
        result: oneshot::Sender<Result<(), String>>,
    },
    Shutdown,
}

#[derive(Debug, Clone)]
pub(super) struct ReconcileOrderRef {
    pub(super) order_id: String,
    pub(super) symbol: String,
    pub(super) side: reap_core::Side,
    pub(super) price: f64,
    pub(super) qty: f64,
    pub(super) filled_qty: f64,
    pub(super) average_fill_price: f64,
    pub(super) last_update_ms: u64,
}

enum OrderTaskCompletion {
    Submit {
        dispatch_key: String,
        symbol: String,
        client_order_id: String,
        completion: RegularSubmitCompletion,
        enqueued_at: Instant,
    },
    Cancel {
        dispatch_key: String,
        symbol: String,
        client_order_id: String,
        result: Result<CancelOutcome, GatewayError>,
        enqueued_at: Instant,
    },
}

impl OrderTaskCompletion {
    fn dispatch_key(&self) -> &str {
        match self {
            Self::Submit { dispatch_key, .. } | Self::Cancel { dispatch_key, .. } => dispatch_key,
        }
    }
}

pub(super) async fn run_order_task(
    account_id: String,
    mut gateway: OkxOrderGateway,
    mut commands: mpsc::Receiver<OrderTaskCommand>,
    events: mpsc::Sender<RuntimeEvent>,
    max_inflight: usize,
    max_pending: usize,
) {
    let io = match gateway.take_command_dispatcher() {
        Ok(io) => Arc::new(io),
        Err(error) => {
            let _ = events
                .send(RuntimeEvent::Fatal(RuntimeTaskFailure::Gateway(format!(
                    "account {account_id} command dispatcher setup failed: {error}"
                ))))
                .await;
            return;
        }
    };
    let max_inflight = max_inflight.max(1);
    let max_pending = max_pending.max(1);
    let mut pending = HashMap::<String, VecDeque<OrderTaskCommand>>::new();
    let mut pending_count = 0_usize;
    let mut ready_dispatch_keys = VecDeque::<String>::new();
    let mut busy_dispatch_keys = HashSet::<String>::new();
    let mut inflight = FuturesUnordered::<BoxFuture<'static, OrderTaskCompletion>>::new();
    let mut flush_waiter: Option<oneshot::Sender<()>> = None;
    let mut shutting_down = false;

    loop {
        while inflight.len() < max_inflight {
            let Some(dispatch_key) = ready_dispatch_keys.pop_front() else {
                break;
            };
            debug_assert!(!busy_dispatch_keys.contains(&dispatch_key));
            let (command, remove_queue) = {
                let queue = pending
                    .get_mut(&dispatch_key)
                    .expect("ready dispatch key must have a pending queue");
                let command = queue
                    .pop_front()
                    .expect("ready dispatch queue must have a command");
                (command, queue.is_empty())
            };
            if remove_queue {
                pending.remove(&dispatch_key);
            }
            pending_count -= 1;
            match command {
                OrderTaskCommand::Submit {
                    action,
                    enqueued_at,
                } => {
                    if action.account_id() != account_id {
                        if events
                            .send(RuntimeEvent::Fatal(RuntimeTaskFailure::Gateway(format!(
                                "account {account_id} received submit authority for account {}",
                                action.account_id()
                            ))))
                            .await
                            .is_err()
                        {
                            return;
                        }
                        make_dispatch_key_ready(
                            &dispatch_key,
                            &pending,
                            &busy_dispatch_keys,
                            &mut ready_dispatch_keys,
                        );
                        continue;
                    }
                    let symbol = action.order().symbol.clone();
                    let client_order_id = action.client_order_id().to_string();
                    let (idempotency_key, reserved) = action.into_parts();
                    let preparation = match gateway.prepare_submit(idempotency_key, reserved) {
                        Ok(preparation) => preparation,
                        Err(error) => {
                            if events
                                .send(RuntimeEvent::Fatal(RuntimeTaskFailure::Gateway(format!(
                                    "account {account_id} submit preparation failed: {error}"
                                ))))
                                .await
                                .is_err()
                            {
                                return;
                            }
                            make_dispatch_key_ready(
                                &dispatch_key,
                                &pending,
                                &busy_dispatch_keys,
                                &mut ready_dispatch_keys,
                            );
                            continue;
                        }
                    };
                    let SubmitPreparation::Ready(prepared) = preparation else {
                        let SubmitPreparation::Complete(outcome) = preparation else {
                            unreachable!()
                        };
                        if events
                            .send(RuntimeEvent::SubmitComplete {
                                account_id: account_id.clone(),
                                symbol,
                                outcome,
                                ts_ms: unix_time_ms(),
                                latency_us: None,
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                        make_dispatch_key_ready(
                            &dispatch_key,
                            &pending,
                            &busy_dispatch_keys,
                            &mut ready_dispatch_keys,
                        );
                        continue;
                    };
                    busy_dispatch_keys.insert(dispatch_key.clone());
                    let io = io.clone();
                    inflight.push(
                        async move {
                            let completion = io.place_prepared(prepared).await;
                            OrderTaskCompletion::Submit {
                                dispatch_key,
                                symbol,
                                client_order_id,
                                completion,
                                enqueued_at,
                            }
                        }
                        .boxed(),
                    );
                }
                OrderTaskCommand::Cancel {
                    action,
                    enqueued_at,
                } => {
                    if action.account_id() != account_id {
                        if events
                            .send(RuntimeEvent::Fatal(RuntimeTaskFailure::Gateway(format!(
                                "account {account_id} received cancel authority for account {}",
                                action.account_id()
                            ))))
                            .await
                            .is_err()
                        {
                            return;
                        }
                        make_dispatch_key_ready(
                            &dispatch_key,
                            &pending,
                            &busy_dispatch_keys,
                            &mut ready_dispatch_keys,
                        );
                        continue;
                    }
                    let symbol = action.symbol().to_string();
                    let client_order_id = action.client_order_id().to_string();
                    let prepared = match gateway.prepare_cancel(action.into_approved()) {
                        Ok(prepared) => prepared,
                        Err(error) => {
                            if events
                                .send(RuntimeEvent::Fatal(RuntimeTaskFailure::Gateway(format!(
                                    "account {account_id} cancel preparation failed: {error}"
                                ))))
                                .await
                                .is_err()
                            {
                                return;
                            }
                            make_dispatch_key_ready(
                                &dispatch_key,
                                &pending,
                                &busy_dispatch_keys,
                                &mut ready_dispatch_keys,
                            );
                            continue;
                        }
                    };
                    busy_dispatch_keys.insert(dispatch_key.clone());
                    let io = io.clone();
                    inflight.push(
                        async move {
                            let result = io.cancel_prepared(prepared).await;
                            OrderTaskCompletion::Cancel {
                                dispatch_key,
                                symbol,
                                client_order_id,
                                result,
                                enqueued_at,
                            }
                        }
                        .boxed(),
                    );
                }
                OrderTaskCommand::Flush(_) | OrderTaskCommand::Shutdown => {
                    unreachable!("control commands are not queued for execution")
                }
            }
        }

        if flush_waiter.is_some() && pending_count == 0 && inflight.is_empty() {
            if let Some(waiter) = flush_waiter.take() {
                let _ = waiter.send(());
            }
            continue;
        }
        if shutting_down && pending_count == 0 && inflight.is_empty() {
            return;
        }

        tokio::select! {
            completion = inflight.next(), if !inflight.is_empty() => {
                let completion = completion.expect("non-empty command future set");
                let dispatch_key = completion.dispatch_key().to_string();
                busy_dispatch_keys.remove(&dispatch_key);
                if !emit_order_task_completion(
                    &account_id,
                    &mut gateway,
                    &events,
                    completion,
                ).await {
                    return;
                }
                make_dispatch_key_ready(
                    &dispatch_key,
                    &pending,
                    &busy_dispatch_keys,
                    &mut ready_dispatch_keys,
                );
            }
            command = commands.recv(), if !shutting_down && flush_waiter.is_none() && pending_count < max_pending => {
                match command {
                    Some(command @ OrderTaskCommand::Submit { .. })
                    | Some(command @ OrderTaskCommand::Cancel { .. }) => {
                        let symbol = match &command {
                            OrderTaskCommand::Submit { action, .. } => &action.order().symbol,
                            OrderTaskCommand::Cancel { action, .. } => action.symbol(),
                            OrderTaskCommand::Flush(_) | OrderTaskCommand::Shutdown => unreachable!(),
                        };
                        let dispatch_key = okx_order_dispatch_key(symbol);
                        let queue = pending.entry(dispatch_key.clone()).or_default();
                        let queue_was_empty = queue.is_empty();
                        queue.push_back(command);
                        pending_count += 1;
                        if queue_was_empty && !busy_dispatch_keys.contains(&dispatch_key) {
                            ready_dispatch_keys.push_back(dispatch_key);
                        }
                    }
                    Some(OrderTaskCommand::Flush(waiter)) => flush_waiter = Some(waiter),
                    Some(OrderTaskCommand::Shutdown) | None => shutting_down = true,
                }
            }
        }
    }
}

fn make_dispatch_key_ready(
    dispatch_key: &str,
    pending: &HashMap<String, VecDeque<OrderTaskCommand>>,
    busy_dispatch_keys: &HashSet<String>,
    ready_dispatch_keys: &mut VecDeque<String>,
) {
    if !busy_dispatch_keys.contains(dispatch_key)
        && pending
            .get(dispatch_key)
            .is_some_and(|queue| !queue.is_empty())
    {
        ready_dispatch_keys.push_back(dispatch_key.to_string());
    }
}

async fn emit_order_task_completion(
    account_id: &str,
    gateway: &mut OkxOrderGateway,
    events: &mpsc::Sender<RuntimeEvent>,
    completion: OrderTaskCompletion,
) -> bool {
    let event = match completion {
        OrderTaskCompletion::Submit {
            symbol,
            client_order_id,
            completion,
            enqueued_at,
            ..
        } => match gateway.finish_submit(completion) {
            Ok(outcome) => RuntimeEvent::SubmitComplete {
                account_id: account_id.to_string(),
                symbol,
                outcome,
                ts_ms: unix_time_ms(),
                latency_us: Some(elapsed_us(enqueued_at)),
            },
            Err(error) => RuntimeEvent::SubmitFailed {
                account_id: account_id.to_string(),
                client_order_id,
                symbol,
                ts_ms: unix_time_ms(),
                ambiguous: error.is_ambiguous(),
                reason: error.to_string(),
            },
        },
        OrderTaskCompletion::Cancel {
            symbol,
            client_order_id,
            result,
            enqueued_at,
            ..
        } => match result {
            Ok(mut outcome) => {
                if outcome.client_order_id.is_empty() || outcome.client_order_id == "0" {
                    outcome.client_order_id = client_order_id;
                }
                RuntimeEvent::CancelComplete {
                    account_id: account_id.to_string(),
                    symbol,
                    outcome,
                    ts_ms: unix_time_ms(),
                    latency_us: elapsed_us(enqueued_at),
                }
            }
            Err(error) => RuntimeEvent::CancelFailed {
                account_id: account_id.to_string(),
                client_order_id,
                symbol,
                ts_ms: unix_time_ms(),
                ambiguous: error.is_ambiguous(),
                reason: error.to_string(),
            },
        },
    };
    events.send(event).await.is_ok()
}

pub(super) fn order_dispatch_lane(dispatch_family: &str, lane_count: usize) -> usize {
    debug_assert!(lane_count > 0);
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in dispatch_family.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    (hash as usize) % lane_count
}
