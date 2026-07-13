use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reap_core::{
    Channel, ConnId, FeedPriority, NormalizedEvent, OrderStatus, Subscription, SystemEvent,
    SystemEventKind, TimerEvent, Venue,
};
use reap_feed::{
    ConnectionStatus, ConnectionStatusKind, FeedOutput, FeedProcessor, ReconnectPolicy, SocketPlan,
    SupervisedFeed, okx_login_bootstrap, partition_subscriptions, spawn_supervised_feed,
};
use reap_order::{
    CancelOutcome, OkxOrderGateway, ReconcileReport, SubmitOutcome, SubmitPreparation, reconcile,
};
use reap_storage::{
    BootstrapRecord, OrderOperation, OrderRequestRecord, StorageConfig, StorageError,
    StorageRecord, StorageRuntime, StorageSink, recover_jsonl, start_jsonl_storage,
};
use reap_venue::okx::{OkxAdapter, OkxRestClient, OkxSigner, ReqwestTransport};
use reap_venue::{PrivateOrderState, PrivateOrderUpdate, RemoteFill, RemoteOrder, VenueAdapter};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::{
    AccountBootstrapSnapshot, CancelAction, CoordinatorError, CoordinatorOutput, LiveAction,
    LiveConfig, LiveConfigError, LiveCoordinator, OperatorCommand, OperatorEnvelope, OperatorError,
    OperatorResponse, OperatorService, OperatorStatus, ReadinessSnapshot, ReconcileAction,
    ReconciliationResult, StartupGate, SubmitAction, TradingEnvironment, VerifiedBootstrap,
    okx_instrument_type, start_operator_service, verify_bootstrap,
};

type LiveGateway = OkxOrderGateway<ReqwestTransport>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveMode {
    Validate,
    Observe,
    Demo,
}

#[derive(Debug, Clone)]
pub struct LiveRunOptions {
    pub mode: LiveMode,
    pub demo_confirmed: bool,
    pub run_duration: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveStopReason {
    Validation,
    OperatorSignal,
    OperatorCommand,
    DurationElapsed,
    ReadinessTimeout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveRunReport {
    pub mode: LiveMode,
    pub stop_reason: LiveStopReason,
    pub elapsed_ms: u64,
    pub reached_ready: bool,
    pub time_to_ready_ms: Option<u64>,
    pub readiness_loss_count: u64,
    pub max_readiness_outage_ms: u64,
    pub reconciliation_drift_events: u64,
    pub book_recovery_events: u64,
    pub stream_stale_events: u64,
    pub connection_disconnect_events: u64,
    pub operator_commands: u64,
    pub operator_mutations: u64,
    pub max_storage_queue_depth: usize,
    pub readiness_at_stop: ReadinessSnapshot,
    pub readiness: ReadinessSnapshot,
    pub dropped_storage_records: u64,
    pub active_orders_after_shutdown: usize,
    pub clean_soak: bool,
}

#[derive(Debug, Error)]
pub enum LiveRuntimeError {
    #[error(transparent)]
    Config(#[from] LiveConfigError),
    #[error("demo order entry requires explicit confirmation")]
    DemoConfirmationRequired,
    #[error("demo mode refuses production exchange configuration")]
    DemoRequiresSimulatedTrading,
    #[error("live run duration must be greater than zero")]
    InvalidRunDuration,
    #[error("account {account_id} bootstrap failed: {message}")]
    Bootstrap { account_id: String, message: String },
    #[error("bootstrap verification failed: {0}")]
    BootstrapVerification(String),
    #[error("checkpoint identity mismatch for account {account_id}: {message}")]
    CheckpointIdentity { account_id: String, message: String },
    #[error("feed subscription planning failed: {0}")]
    Subscription(String),
    #[error("order gateway setup failed for account {account_id}: {message}")]
    GatewaySetup { account_id: String, message: String },
    #[error(transparent)]
    Coordinator(#[from] CoordinatorError),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Operator(#[from] OperatorError),
    #[error("storage remained unavailable during graceful shutdown: {0}")]
    ShutdownStorage(String),
    #[error("runtime event channel closed")]
    EventChannelClosed,
    #[error("operator command channel closed")]
    OperatorChannelClosed,
    #[error("order command queue for account {0} is unavailable or full")]
    OrderQueueUnavailable(String),
    #[error("graceful shutdown generated an unsafe new-order action")]
    UnsafeShutdownSubmit,
    #[error("book recovery request had no matching public socket for {0}")]
    MissingRecoveryRoute(String),
    #[error("live readiness timed out after {0}ms")]
    ReadinessTimeout(u64),
    #[error("gateway task failed: {0}")]
    GatewayTask(String),
    #[error(
        "shutdown timed out with {active_orders} active local orders and {unreconciled_accounts} unreconciled accounts"
    )]
    ShutdownUnresolved {
        active_orders: usize,
        unreconciled_accounts: usize,
    },
    #[error("feed adapter failed: {0}")]
    FeedAdapter(String),
    #[error("runtime task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("shutdown signal failed: {0}")]
    Signal(#[from] std::io::Error),
    #[error("{primary}; additional lifecycle failures: {secondary}")]
    LifecycleFailure {
        #[source]
        primary: Box<LiveRuntimeError>,
        secondary: String,
    },
}

#[derive(Debug, Clone, Copy, Default)]
struct RuntimeEvidence {
    reconciliation_drift_events: u64,
    book_recovery_events: u64,
    stream_stale_events: u64,
    connection_disconnect_events: u64,
    operator_commands: u64,
    operator_mutations: u64,
    max_storage_queue_depth: usize,
}

impl RuntimeEvidence {
    fn observe_record(&mut self, record: &StorageRecord) {
        let StorageRecord::System(event) = record else {
            return;
        };
        match event.kind {
            SystemEventKind::ReconcileDrift => self.reconciliation_drift_events += 1,
            SystemEventKind::BookRecoveryStarted => self.book_recovery_events += 1,
            SystemEventKind::FeedStale | SystemEventKind::PrivateStreamStale => {
                self.stream_stale_events += 1;
            }
            _ => {}
        }
    }
}

#[derive(Debug)]
struct RunLoopOutcome {
    stop_reason: LiveStopReason,
    elapsed_ms: u64,
    reached_ready: bool,
    time_to_ready_ms: Option<u64>,
    readiness_loss_count: u64,
    max_readiness_outage_ms: u64,
    readiness_at_stop: ReadinessSnapshot,
}

#[derive(Debug, Default)]
struct ReadinessTracker {
    reached_ready: bool,
    time_to_ready_ms: Option<u64>,
    readiness_loss_count: u64,
    outage_started_ms: Option<u64>,
    max_readiness_outage_ms: u64,
}

impl ReadinessTracker {
    fn observe(&mut self, elapsed_ms: u64, readiness: &ReadinessSnapshot) {
        if readiness.is_ready() {
            if !self.reached_ready {
                self.reached_ready = true;
                self.time_to_ready_ms = Some(elapsed_ms);
            }
            if let Some(started_ms) = self.outage_started_ms.take() {
                self.max_readiness_outage_ms = self
                    .max_readiness_outage_ms
                    .max(elapsed_ms.saturating_sub(started_ms));
            }
        } else if self.reached_ready && self.outage_started_ms.is_none() {
            self.readiness_loss_count += 1;
            self.outage_started_ms = Some(elapsed_ms);
        }
    }

    fn finish(
        mut self,
        stop_reason: LiveStopReason,
        elapsed_ms: u64,
        readiness: ReadinessSnapshot,
    ) -> RunLoopOutcome {
        self.observe(elapsed_ms, &readiness);
        if let Some(started_ms) = self.outage_started_ms {
            self.max_readiness_outage_ms = self
                .max_readiness_outage_ms
                .max(elapsed_ms.saturating_sub(started_ms));
        }
        RunLoopOutcome {
            stop_reason,
            elapsed_ms,
            reached_ready: self.reached_ready,
            time_to_ready_ms: self.time_to_ready_ms,
            readiness_loss_count: self.readiness_loss_count,
            max_readiness_outage_ms: self.max_readiness_outage_ms,
            readiness_at_stop: readiness,
        }
    }
}

fn qualifies_as_clean_soak(
    outcome: &RunLoopOutcome,
    evidence: RuntimeEvidence,
    dropped_storage_records: u64,
    active_orders_after_shutdown: usize,
) -> bool {
    outcome.stop_reason == LiveStopReason::DurationElapsed
        && outcome.reached_ready
        && outcome.readiness_at_stop.is_ready()
        && evidence.reconciliation_drift_events == 0
        && evidence.operator_mutations == 0
        && dropped_storage_records == 0
        && active_orders_after_shutdown == 0
}

fn is_zero_order_reconciliation(report: &ReconcileReport) -> bool {
    report.is_clean() && report.local_live_orders == 0 && report.remote_live_orders == 0
}

fn combine_lifecycle_errors(
    primary: LiveRuntimeError,
    additional: Vec<(&'static str, LiveRuntimeError)>,
) -> LiveRuntimeError {
    if additional.is_empty() {
        return primary;
    }
    let secondary = additional
        .into_iter()
        .map(|(stage, error)| format!("{stage}: {error}"))
        .collect::<Vec<_>>()
        .join("; ");
    LiveRuntimeError::LifecycleFailure {
        primary: Box::new(primary),
        secondary,
    }
}

pub async fn run_live_path(
    path: impl AsRef<Path>,
    options: LiveRunOptions,
) -> Result<LiveRunReport, LiveRuntimeError> {
    let config = LiveConfig::load(path)?;
    run_live(config, options).await
}

pub async fn run_live(
    config: LiveConfig,
    options: LiveRunOptions,
) -> Result<LiveRunReport, LiveRuntimeError> {
    config.ensure_valid()?;
    if options
        .run_duration
        .is_some_and(|duration| duration.is_zero())
    {
        return Err(LiveRuntimeError::InvalidRunDuration);
    }
    if options.mode == LiveMode::Validate {
        let readiness = StartupGate::new(&config).snapshot();
        return Ok(LiveRunReport {
            mode: options.mode,
            stop_reason: LiveStopReason::Validation,
            elapsed_ms: 0,
            reached_ready: false,
            time_to_ready_ms: None,
            readiness_loss_count: 0,
            max_readiness_outage_ms: 0,
            reconciliation_drift_events: 0,
            book_recovery_events: 0,
            stream_stale_events: 0,
            connection_disconnect_events: 0,
            operator_commands: 0,
            operator_mutations: 0,
            max_storage_queue_depth: 0,
            readiness_at_stop: readiness.clone(),
            readiness,
            dropped_storage_records: 0,
            active_orders_after_shutdown: 0,
            clean_soak: false,
        });
    }
    if options.mode == LiveMode::Demo {
        if !options.demo_confirmed {
            return Err(LiveRuntimeError::DemoConfirmationRequired);
        }
        if config.venue.environment != TradingEnvironment::Demo {
            return Err(LiveRuntimeError::DemoRequiresSimulatedTrading);
        }
    }
    let runtime = LiveRuntime::build(config, options.mode, options.run_duration).await?;
    runtime.run().await
}

struct AccountSeed {
    account_id: String,
    signer: OkxSigner,
    gateway: LiveGateway,
}

async fn bootstrap_accounts(
    config: &LiveConfig,
    restored_orders: &HashMap<String, Vec<reap_core::OrderUpdate>>,
) -> Result<
    (
        VerifiedBootstrap,
        Vec<AccountSeed>,
        HashMap<String, AccountBootstrapSnapshot>,
    ),
    LiveRuntimeError,
> {
    let mut snapshots = HashMap::new();
    let mut seeds = Vec::new();
    for account in &config.accounts {
        let credentials = account.credentials_from_env()?;
        let signer = OkxSigner::new(credentials, config.venue.environment.is_demo());
        let transport = ReqwestTransport::with_timeouts(
            &config.venue.rest_url,
            Duration::from_millis(config.runtime.rest_connect_timeout_ms),
            Duration::from_millis(config.runtime.rest_request_timeout_ms),
        )
        .map_err(|error| LiveRuntimeError::Bootstrap {
            account_id: account.id.clone(),
            message: error.to_string(),
        })?;
        let client = OkxRestClient::new(transport, signer.clone());
        let account_config = client
            .account_config()
            .await
            .map_err(|error| bootstrap_error(&account.id, "account config", error.to_string()))?;
        let balance = client
            .account_balance()
            .await
            .map_err(|error| bootstrap_error(&account.id, "account balance", error.to_string()))?;
        let positions = client
            .account_positions(None, None)
            .await
            .map_err(|error| {
                bootstrap_error(&account.id, "account positions", error.to_string())
            })?;
        let mut open_orders = client
            .open_orders(None, None)
            .await
            .map_err(|error| bootstrap_error(&account.id, "open orders", error.to_string()))?;
        let recent_fills = client
            .fills(None, None)
            .await
            .map_err(|error| bootstrap_error(&account.id, "recent fills", error.to_string()))?;
        let mut remote_ids = open_orders
            .iter()
            .map(remote_order_id)
            .collect::<HashSet<_>>();
        for restored in restored_orders.get(&account.id).into_iter().flatten() {
            if remote_ids.contains(&restored.order_id) {
                continue;
            }
            let details = match client
                .order_details(&restored.symbol, None, Some(&restored.order_id))
                .await
            {
                Ok(details) => details,
                Err(error)
                    if error.is_order_not_found()
                        && unix_time_ms().saturating_sub(restored.ts_ms)
                            < config.runtime.ambiguous_submit_grace_ms =>
                {
                    continue;
                }
                Err(error) if error.is_order_not_found() => RemoteOrder {
                    exchange_order_id: String::new(),
                    client_order_id: restored.order_id.clone(),
                    symbol: restored.symbol.clone(),
                    side: restored.side,
                    state: PrivateOrderState::Rejected,
                    price: restored.price,
                    qty: restored.qty,
                    cumulative_filled_qty: restored.filled_qty,
                    average_fill_price: restored.avg_fill_price,
                    update_time_ms: unix_time_ms(),
                },
                Err(error) => {
                    return Err(bootstrap_error(
                        &account.id,
                        &format!("order details {}", restored.order_id),
                        error.to_string(),
                    ));
                }
            };
            remote_ids.insert(remote_order_id(&details));
            open_orders.push(details);
        }
        let mut instruments = HashMap::new();
        for instrument in config.instruments_for_account(&account.id) {
            let metadata = client
                .account_instruments(
                    okx_instrument_type(instrument.kind),
                    Some(&instrument.symbol),
                )
                .await
                .map_err(|error| {
                    bootstrap_error(
                        &account.id,
                        &format!("instrument {}", instrument.symbol),
                        error.to_string(),
                    )
                })?
                .into_iter()
                .find(|metadata| metadata.symbol == instrument.symbol)
                .ok_or_else(|| LiveRuntimeError::Bootstrap {
                    account_id: account.id.clone(),
                    message: format!("exchange returned no metadata for {}", instrument.symbol),
                })?;
            instruments.insert(instrument.symbol.clone(), metadata);
        }
        snapshots.insert(
            account.id.clone(),
            AccountBootstrapSnapshot {
                account_config,
                instruments,
                balance,
                positions,
                open_orders,
                recent_fills,
            },
        );
        let trade_modes = account
            .trade_modes
            .iter()
            .map(|(symbol, mode)| (symbol.clone(), (*mode).into()))
            .collect();
        let gateway = OkxOrderGateway::new(
            client,
            account.id_prefix.clone(),
            account.node_id,
            trade_modes,
            config.runtime.pacing_policy(),
        )
        .map_err(|error| LiveRuntimeError::GatewaySetup {
            account_id: account.id.clone(),
            message: error.to_string(),
        })?;
        seeds.push(AccountSeed {
            account_id: account.id.clone(),
            signer,
            gateway,
        });
    }
    let verified = verify_bootstrap(config, &snapshots)
        .map_err(|error| LiveRuntimeError::BootstrapVerification(error.to_string()))?;
    Ok((verified, seeds, snapshots))
}

fn bootstrap_error(account_id: &str, operation: &str, message: String) -> LiveRuntimeError {
    LiveRuntimeError::Bootstrap {
        account_id: account_id.to_string(),
        message: format!("{operation}: {message}"),
    }
}

fn remote_order_id(order: &RemoteOrder) -> String {
    if order.client_order_id.is_empty() {
        order.exchange_order_id.clone()
    } else {
        order.client_order_id.clone()
    }
}

fn private_update_from_remote(order: RemoteOrder) -> PrivateOrderUpdate {
    PrivateOrderUpdate {
        ts_ms: if order.update_time_ms == 0 {
            unix_time_ms()
        } else {
            order.update_time_ms
        },
        exchange_order_id: order.exchange_order_id,
        client_order_id: order.client_order_id,
        symbol: order.symbol,
        side: order.side,
        state: order.state,
        price: order.price,
        qty: order.qty,
        cumulative_filled_qty: order.cumulative_filled_qty,
        average_fill_price: order.average_fill_price,
        last_fill_qty: 0.0,
        last_fill_price: 0.0,
        liquidity: None,
        fill_id: None,
        reject_reason: if order.state == PrivateOrderState::Rejected {
            "order not present during restart reconciliation".to_string()
        } else {
            String::new()
        },
    }
}

struct LiveRuntime {
    mode: LiveMode,
    run_duration: Option<Duration>,
    coordinator: LiveCoordinator,
    processor: FeedProcessor,
    storage: Option<StorageRuntime>,
    storage_sink: StorageSink,
    control_rx: mpsc::Receiver<RuntimeEvent>,
    feed_rx: mpsc::Receiver<RuntimeEvent>,
    order_senders: HashMap<String, mpsc::Sender<OrderTaskCommand>>,
    order_tasks: Vec<JoinHandle<()>>,
    feeds: Vec<SupervisedFeed>,
    feed_tasks: Vec<JoinHandle<()>>,
    sources: Vec<FeedSourceState>,
    public_feed_index: usize,
    reconcile_inflight: HashSet<String>,
    cancel_inflight: HashSet<(String, String)>,
    last_reconcile_attempt: HashMap<String, Instant>,
    readiness_timeout_ms: u64,
    timer_interval_ms: u64,
    max_feed_age_ms: u64,
    shutdown_timeout_ms: u64,
    evidence: RuntimeEvidence,
    shutdown_in_progress: bool,
    shutdown_storage_error: Option<String>,
    shutdown_reconciliation_requested: HashSet<String>,
    shutdown_reconciled_accounts: HashSet<String>,
    operator_service: Option<OperatorService>,
    operator_rx: Option<mpsc::Receiver<OperatorEnvelope>>,
    operator_shutdown_reason: Option<String>,
}

impl LiveRuntime {
    async fn build(
        config: LiveConfig,
        mode: LiveMode,
        run_duration: Option<Duration>,
    ) -> Result<Self, LiveRuntimeError> {
        let operator_config = config.operator.clone();
        let operator_secret = operator_config.secret_from_env()?;
        let config_fingerprint = config.fingerprint()?;
        let recovered = recover_jsonl(&config.storage.path)?;
        for (account_id, (strategy_name, fingerprint)) in &recovered.bootstrap_identities {
            if config.account(account_id).is_none() {
                return Err(LiveRuntimeError::CheckpointIdentity {
                    account_id: account_id.clone(),
                    message: "checkpoint account is not present in the live config".to_string(),
                });
            }
            if strategy_name != &config.strategy.strategy_name || fingerprint != &config_fingerprint
            {
                return Err(LiveRuntimeError::CheckpointIdentity {
                    account_id: account_id.clone(),
                    message: "strategy name or live config fingerprint changed; rotate the storage path after reconciling all orders".to_string(),
                });
            }
        }
        let recovered_orders = recovered
            .latest_orders
            .values()
            .filter(|update| {
                matches!(
                    update.status,
                    OrderStatus::PendingNew | OrderStatus::Live | OrderStatus::PartiallyFilled
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut restored_by_account: HashMap<String, Vec<reap_core::OrderUpdate>> = HashMap::new();
        for update in &recovered_orders {
            let account_id = config
                .account_for_symbol(&update.symbol)
                .map(|account| account.id.clone())
                .ok_or_else(|| {
                    LiveRuntimeError::BootstrapVerification(format!(
                        "recovered order {} has unmapped symbol {}",
                        update.order_id, update.symbol
                    ))
                })?;
            restored_by_account
                .entry(account_id)
                .or_default()
                .push(update.clone());
        }
        let (mut verified, seeds, snapshots) =
            bootstrap_accounts(&config, &restored_by_account).await?;
        let mut bootstrap_records = Vec::new();
        for account in &config.accounts {
            let exchange_baseline = verified
                .baseline_fill_ids
                .get(&account.id)
                .cloned()
                .unwrap_or_default();
            let mut fill_ids = recovered
                .baseline_fill_ids
                .get(&account.id)
                .cloned()
                .unwrap_or_else(|| exchange_baseline.clone());
            for fill in &recovered.fills {
                let fill_account_id = fill.account_id.as_deref().or_else(|| {
                    config
                        .account_for_symbol(&fill.symbol)
                        .map(|owner| owner.id.as_str())
                });
                if fill_account_id == Some(account.id.as_str()) {
                    fill_ids.insert(fill.fill_id.clone());
                }
            }
            verified
                .baseline_fill_ids
                .insert(account.id.clone(), fill_ids);
            if !recovered.baseline_fill_ids.contains_key(&account.id) {
                let mut baseline_fill_ids = exchange_baseline.into_iter().collect::<Vec<_>>();
                baseline_fill_ids.sort();
                bootstrap_records.push(StorageRecord::Bootstrap(BootstrapRecord {
                    ts_ms: unix_time_ms(),
                    account_id: account.id.clone(),
                    strategy_name: config.strategy.strategy_name.clone(),
                    config_fingerprint: config_fingerprint.clone(),
                    baseline_fill_ids,
                }));
            }
        }
        let session_id = format!("{:x}", unix_time_ns());
        let mut coordinator =
            LiveCoordinator::new(config.clone(), verified, mode == LiveMode::Demo, session_id)?;
        let mut initial_outputs = vec![CoordinatorOutput {
            actions: Vec::new(),
            records: bootstrap_records,
        }];
        for update in recovered_orders {
            let account_id = config
                .account_for_symbol(&update.symbol)
                .map(|account| account.id.clone())
                .ok_or_else(|| {
                    LiveRuntimeError::BootstrapVerification(format!(
                        "recovered order {} has unmapped symbol {}",
                        update.order_id, update.symbol
                    ))
                })?;
            initial_outputs.push(coordinator.restore_order(&account_id, update)?);
        }
        for account in &config.accounts {
            let snapshot = snapshots.get(&account.id).ok_or_else(|| {
                LiveRuntimeError::BootstrapVerification(format!(
                    "missing reconciliation snapshot for {}",
                    account.id
                ))
            })?;
            for fill in &snapshot.recent_fills {
                let order_id = if fill.client_order_id.is_empty() {
                    &fill.exchange_order_id
                } else {
                    &fill.client_order_id
                };
                let should_apply = coordinator.private_state(&account.id).is_some_and(|state| {
                    state.order_reducer().contains_order(order_id)
                        && !state.seen_fill_ids().contains(&fill.fill_id)
                });
                if should_apply {
                    initial_outputs.push(coordinator.process_feed(FeedOutput::PrivateFill {
                        account_id: Some(account.id.clone()),
                        fill: fill.clone(),
                    })?);
                }
            }
            for remote in &snapshot.open_orders {
                let order_id = remote_order_id(remote);
                let known = coordinator
                    .private_state(&account.id)
                    .is_some_and(|state| state.order_reducer().contains_order(&order_id));
                if known {
                    initial_outputs.push(coordinator.process_feed(FeedOutput::PrivateOrder {
                        account_id: Some(account.id.clone()),
                        update: private_update_from_remote(remote.clone()),
                    })?);
                }
            }
            initial_outputs.push(coordinator.process_feed(FeedOutput::PrivateAccount {
                account_id: Some(account.id.clone()),
                update: snapshot.scoped_account_update(&account.id),
            })?);
            let state = coordinator
                .private_state(&account.id)
                .ok_or_else(|| CoordinatorError::UnknownAccount(account.id.clone()))?;
            let report = reconcile(
                state.order_reducer(),
                state.seen_fill_ids(),
                &snapshot.open_orders,
                &snapshot.recent_fills,
            );
            initial_outputs.push(coordinator.on_reconciliation(ReconciliationResult {
                account_id: account.id.clone(),
                ts_ms: unix_time_ms(),
                clean: report.is_clean(),
                local_live_orders: report.local_live_orders,
                remote_live_orders: report.remote_live_orders,
                remote_recent_fills: report.remote_fills,
                reason: if report.is_clean() {
                    "startup REST reconciliation is clean".to_string()
                } else {
                    format!("startup reconciliation drift: {:?}", report.issues)
                },
            })?);
        }
        let public_subscriptions = public_subscriptions(&config);
        let public_plans = partition_subscriptions(
            &public_subscriptions,
            config.runtime.max_subscriptions_per_socket,
        )
        .map_err(|error| LiveRuntimeError::Subscription(error.to_string()))?;
        let private_subscriptions = private_subscriptions(config.venue.enable_vip_fills_channel);
        let private_plans = partition_subscriptions(
            &private_subscriptions,
            config.runtime.max_subscriptions_per_socket,
        )
        .map_err(|error| LiveRuntimeError::Subscription(error.to_string()))?;
        let storage = start_jsonl_storage(StorageConfig {
            path: config.storage.path.clone(),
            channel_capacity: config.storage.channel_capacity,
            flush_every_records: config.storage.flush_every_records,
        })
        .await?;
        let storage_sink = storage.sink();
        coordinator.mark_storage_ready(true, "storage file opened");

        let (control_tx, control_rx) = mpsc::channel(config.runtime.event_channel_capacity);
        let (feed_tx, feed_rx) = mpsc::channel(config.runtime.event_channel_capacity);
        let mut feeds = Vec::new();
        let mut feed_tasks = Vec::new();
        let mut sources = Vec::new();

        let public_adapter: Arc<dyn VenueAdapter> = Arc::new(OkxAdapter::new(
            &config.venue.public_ws_url,
            &config.venue.private_ws_url,
        ));
        let mut public_feed = spawn_supervised_feed(
            Arc::clone(&public_adapter),
            public_plans.clone(),
            reap_feed::no_bootstrap(),
            config.runtime.feed_channel_capacity,
            ReconnectPolicy::default(),
        );
        let public_source_id = sources.len();
        sources.push(FeedSourceState::public(public_adapter, &public_plans));
        spawn_feed_forwarders(
            public_source_id,
            &mut public_feed,
            &feed_tx,
            &mut feed_tasks,
        );
        feeds.push(public_feed);
        let public_feed_index = 0;

        let mut order_senders = HashMap::new();
        let mut order_tasks = Vec::new();
        for seed in seeds {
            let private_adapter: Arc<dyn VenueAdapter> = Arc::new(
                OkxAdapter::new(&config.venue.public_ws_url, &config.venue.private_ws_url)
                    .with_account_id(&seed.account_id),
            );
            let mut private_feed = spawn_supervised_feed(
                Arc::clone(&private_adapter),
                private_plans.clone(),
                okx_login_bootstrap(seed.signer),
                config.runtime.feed_channel_capacity,
                ReconnectPolicy::default(),
            );
            let source_id = sources.len();
            sources.push(FeedSourceState::private(
                private_adapter,
                seed.account_id.clone(),
                &private_plans,
            ));
            spawn_feed_forwarders(source_id, &mut private_feed, &feed_tx, &mut feed_tasks);
            feeds.push(private_feed);

            let (order_tx, order_rx) = mpsc::channel(config.runtime.order_channel_capacity);
            order_senders.insert(seed.account_id.clone(), order_tx);
            order_tasks.push(tokio::spawn(run_order_task(
                seed.account_id,
                seed.gateway,
                order_rx,
                control_tx.clone(),
                config.runtime.ambiguous_submit_grace_ms,
            )));
        }

        let mut runtime = Self {
            mode,
            run_duration,
            coordinator,
            processor: FeedProcessor::new(
                config.runtime.dedup_capacity_per_stream,
                config.runtime.max_sequence_buffer,
            ),
            storage: Some(storage),
            storage_sink,
            control_rx,
            feed_rx,
            order_senders,
            order_tasks,
            feeds,
            feed_tasks,
            sources,
            public_feed_index,
            reconcile_inflight: HashSet::new(),
            cancel_inflight: HashSet::new(),
            last_reconcile_attempt: HashMap::new(),
            readiness_timeout_ms: config.runtime.readiness_timeout_ms,
            timer_interval_ms: config.runtime.timer_interval_ms,
            max_feed_age_ms: config.risk.max_feed_age_ms,
            shutdown_timeout_ms: config.runtime.shutdown_timeout_ms,
            evidence: RuntimeEvidence::default(),
            shutdown_in_progress: false,
            shutdown_storage_error: None,
            shutdown_reconciliation_requested: HashSet::new(),
            shutdown_reconciled_accounts: HashSet::new(),
            operator_service: None,
            operator_rx: None,
            operator_shutdown_reason: None,
        };
        for output in initial_outputs {
            if let Err(primary) = runtime.commit_output(output) {
                let context = format!("runtime initialization failure: {primary}");
                return Err(runtime.close_after_error(primary, &context).await);
            }
        }
        if let Some(secret) = operator_secret {
            let (operator_tx, operator_rx) =
                mpsc::channel(operator_config.command_channel_capacity);
            match start_operator_service(&operator_config, secret, operator_tx).await {
                Ok(service) => {
                    runtime.operator_service = Some(service);
                    runtime.operator_rx = Some(operator_rx);
                }
                Err(error) => {
                    let primary = LiveRuntimeError::Operator(error);
                    let context = format!("operator service startup failure: {primary}");
                    return Err(runtime.close_after_error(primary, &context).await);
                }
            }
        }
        Ok(runtime)
    }

    async fn close_after_error(
        &mut self,
        primary: LiveRuntimeError,
        context: &str,
    ) -> LiveRuntimeError {
        let stop_result = self.graceful_stop(context).await;
        let shutdown_result = self.shutdown().await;
        let mut additional = Vec::new();
        if let Err(error) = stop_result {
            additional.push(("fail-closed cleanup", error));
        }
        if let Err(error) = shutdown_result {
            additional.push(("runtime teardown", error));
        }
        combine_lifecycle_errors(primary, additional)
    }

    async fn run(mut self) -> Result<LiveRunReport, LiveRuntimeError> {
        let loop_result = self.run_loop().await;
        let stop_context = match &loop_result {
            Ok(outcome) => match outcome.stop_reason {
                LiveStopReason::OperatorSignal => "operator signal".to_string(),
                LiveStopReason::OperatorCommand => self
                    .operator_shutdown_reason
                    .clone()
                    .unwrap_or_else(|| "authenticated operator command".to_string()),
                LiveStopReason::DurationElapsed => "bounded duration elapsed".to_string(),
                LiveStopReason::ReadinessTimeout => "bounded readiness timeout".to_string(),
                LiveStopReason::Validation => "validation".to_string(),
            },
            Err(error) => format!("runtime failure: {error}"),
        };
        let stop_result = self.graceful_stop(&stop_context).await;
        let shutdown_result = self.shutdown().await;
        let outcome = match loop_result {
            Ok(outcome) => match stop_result {
                Ok(()) => {
                    shutdown_result?;
                    outcome
                }
                Err(primary) => {
                    let additional = shutdown_result
                        .err()
                        .map(|error| vec![("runtime teardown", error)])
                        .unwrap_or_default();
                    return Err(combine_lifecycle_errors(primary, additional));
                }
            },
            Err(primary) => {
                let mut additional = Vec::new();
                if let Err(error) = stop_result {
                    additional.push(("fail-closed cleanup", error));
                }
                if let Err(error) = shutdown_result {
                    additional.push(("runtime teardown", error));
                }
                return Err(combine_lifecycle_errors(primary, additional));
            }
        };
        let readiness = self.coordinator.readiness();
        let dropped_storage_records = self.storage_sink.dropped_records();
        let active_orders_after_shutdown = self.coordinator.active_order_count();
        let evidence = self.evidence;
        let clean_soak = qualifies_as_clean_soak(
            &outcome,
            evidence,
            dropped_storage_records,
            active_orders_after_shutdown,
        );
        Ok(LiveRunReport {
            mode: self.mode,
            stop_reason: outcome.stop_reason,
            elapsed_ms: outcome.elapsed_ms,
            reached_ready: outcome.reached_ready,
            time_to_ready_ms: outcome.time_to_ready_ms,
            readiness_loss_count: outcome.readiness_loss_count,
            max_readiness_outage_ms: outcome.max_readiness_outage_ms,
            reconciliation_drift_events: evidence.reconciliation_drift_events,
            book_recovery_events: evidence.book_recovery_events,
            stream_stale_events: evidence.stream_stale_events,
            connection_disconnect_events: evidence.connection_disconnect_events,
            operator_commands: evidence.operator_commands,
            operator_mutations: evidence.operator_mutations,
            max_storage_queue_depth: evidence.max_storage_queue_depth,
            readiness_at_stop: outcome.readiness_at_stop,
            readiness,
            dropped_storage_records,
            active_orders_after_shutdown,
            clean_soak,
        })
    }

    async fn run_loop(&mut self) -> Result<RunLoopOutcome, LiveRuntimeError> {
        let started = Instant::now();
        let mut readiness_tracker = ReadinessTracker::default();
        let initial_readiness = self.coordinator.readiness();
        readiness_tracker.observe(0, &initial_readiness);
        let mut last_phase = initial_readiness.phase;
        let mut timer = tokio::time::interval(Duration::from_millis(self.timer_interval_ms));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let shutdown = shutdown_signal();
        tokio::pin!(shutdown);
        let run_duration = self.run_duration;
        let duration_elapsed = async move {
            match run_duration {
                Some(duration) => tokio::time::sleep(duration).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::pin!(duration_elapsed);

        loop {
            tokio::select! {
                biased;
                signal = &mut shutdown => {
                    signal?;
                    self.drain_queued_events()?;
                    let elapsed_ms = elapsed_ms(&started);
                    let outcome = readiness_tracker.finish(
                        LiveStopReason::OperatorSignal,
                        elapsed_ms,
                        self.coordinator.readiness(),
                    );
                    return Ok(outcome);
                }
                _ = &mut duration_elapsed => {
                    self.drain_queued_events()?;
                    let elapsed_ms = elapsed_ms(&started);
                    let outcome = readiness_tracker.finish(
                        LiveStopReason::DurationElapsed,
                        elapsed_ms,
                        self.coordinator.readiness(),
                    );
                    return Ok(outcome);
                }
                event = self.control_rx.recv() => {
                    let event = event.ok_or(LiveRuntimeError::EventChannelClosed)?;
                    self.handle_runtime_event(event)?;
                }
                operator = receive_operator(&mut self.operator_rx) => {
                    let operator = operator.ok_or(LiveRuntimeError::OperatorChannelClosed)?;
                    self.handle_operator_envelope(operator)?;
                }
                _ = timer.tick() => {
                    let now_ms = unix_time_ms();
                    for event in self.processor.mark_stale(
                        now_ms,
                        self.coordinator_risk_max_feed_age(),
                    ) {
                        let output = self.coordinator.process_event(NormalizedEvent::System(event));
                        self.commit_output(output)?;
                    }
                    let output = self.coordinator.process_event(NormalizedEvent::Timer(TimerEvent {
                        ts_ms: now_ms,
                        name: "live_tick".to_string(),
                    }));
                    self.commit_output(output)?;
                    self.retry_reconciliation(now_ms)?;
                }
                event = self.feed_rx.recv() => {
                    let event = event.ok_or(LiveRuntimeError::EventChannelClosed)?;
                    self.handle_runtime_event(event)?;
                }
            }

            let readiness = self.coordinator.readiness();
            let elapsed_ms = elapsed_ms(&started);
            readiness_tracker.observe(elapsed_ms, &readiness);
            if readiness.phase != last_phase {
                tracing::info!(from = ?last_phase, to = ?readiness.phase, ?readiness, "live readiness changed");
                last_phase = readiness.phase;
            }
            if self.operator_shutdown_reason.is_some() {
                return Ok(readiness_tracker.finish(
                    LiveStopReason::OperatorCommand,
                    elapsed_ms,
                    readiness,
                ));
            }
            if !readiness_tracker.reached_ready
                && started.elapsed() > Duration::from_millis(self.readiness_timeout_ms)
            {
                if self.run_duration.is_some() {
                    let outcome = readiness_tracker.finish(
                        LiveStopReason::ReadinessTimeout,
                        elapsed_ms,
                        readiness,
                    );
                    return Ok(outcome);
                }
                return Err(LiveRuntimeError::ReadinessTimeout(
                    self.readiness_timeout_ms,
                ));
            }
        }
    }

    fn coordinator_risk_max_feed_age(&self) -> u64 {
        self.max_feed_age_ms
    }

    async fn graceful_stop(&mut self, reason: &str) -> Result<(), LiveRuntimeError> {
        if self.mode != LiveMode::Demo {
            return Ok(());
        }
        self.shutdown_in_progress = true;
        let result = match tokio::time::timeout(
            Duration::from_millis(self.shutdown_timeout_ms),
            self.graceful_stop_inner(reason),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(self.shutdown_unresolved_error()),
        };
        match (result, self.shutdown_storage_error.take()) {
            (Ok(()), None) => Ok(()),
            (Ok(()), Some(error)) => Err(LiveRuntimeError::ShutdownStorage(error)),
            (Err(primary), None) => Err(primary),
            (Err(primary), Some(error)) => Err(combine_lifecycle_errors(
                primary,
                vec![("shutdown storage", LiveRuntimeError::ShutdownStorage(error))],
            )),
        }
    }

    async fn graceful_stop_inner(&mut self, reason: &str) -> Result<(), LiveRuntimeError> {
        self.coordinator.set_order_entry_enabled(false);
        let now_ms = unix_time_ms();
        let output = self
            .coordinator
            .process_event(NormalizedEvent::System(SystemEvent {
                ts_ms: now_ms,
                kind: SystemEventKind::KillSwitchActivated,
                venue: None,
                account_id: None,
                symbol: None,
                reason: reason.to_string(),
            }));
        self.commit_shutdown_output(output).await?;
        self.request_shutdown_reconciliation(now_ms, true).await?;

        let mut retry = tokio::time::interval(Duration::from_millis(100));
        retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            self.drain_shutdown_events()?;
            let readiness = self.coordinator.readiness();
            if self.coordinator.active_order_count() == 0
                && readiness.missing_reconciliation.is_empty()
                && self.reconcile_inflight.is_empty()
                && self.shutdown_reconciled_accounts.len() == self.order_senders.len()
            {
                return Ok(());
            }
            tokio::select! {
                biased;
                event = self.control_rx.recv(), if !self.control_rx.is_closed() => {
                    if let Some(event) = event {
                        self.handle_runtime_event(event)?;
                    }
                }
                event = self.feed_rx.recv(), if !self.feed_rx.is_closed() => {
                    if let Some(event) = event {
                        self.handle_runtime_event(event)?;
                    }
                }
                _ = retry.tick() => {
                    self.request_shutdown_reconciliation(unix_time_ms(), false).await?;
                }
            }
        }
    }

    fn shutdown_unresolved_error(&self) -> LiveRuntimeError {
        LiveRuntimeError::ShutdownUnresolved {
            active_orders: self.coordinator.active_order_count(),
            unreconciled_accounts: self
                .coordinator
                .readiness()
                .missing_reconciliation
                .into_iter()
                .chain(self.reconcile_inflight.iter().cloned())
                .chain(
                    self.order_senders
                        .keys()
                        .filter(|account_id| {
                            !self.shutdown_reconciled_accounts.contains(*account_id)
                        })
                        .cloned(),
                )
                .collect::<HashSet<_>>()
                .len(),
        }
    }

    fn drain_shutdown_events(&mut self) -> Result<(), LiveRuntimeError> {
        let pending_control = self.control_rx.len();
        for _ in 0..pending_control {
            match self.control_rx.try_recv() {
                Ok(event) => self.handle_runtime_event(event)?,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        let pending_feed = self.feed_rx.len();
        for _ in 0..pending_feed {
            match self.feed_rx.try_recv() {
                Ok(event) => self.handle_runtime_event(event)?,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        Ok(())
    }

    fn drain_queued_events(&mut self) -> Result<(), LiveRuntimeError> {
        let pending_control = self.control_rx.len();
        for _ in 0..pending_control {
            match self.control_rx.try_recv() {
                Ok(event) => self.handle_runtime_event(event)?,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return Err(LiveRuntimeError::EventChannelClosed);
                }
            }
        }
        let pending_feed = self.feed_rx.len();
        for _ in 0..pending_feed {
            match self.feed_rx.try_recv() {
                Ok(event) => self.handle_runtime_event(event)?,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return Err(LiveRuntimeError::EventChannelClosed);
                }
            }
        }
        Ok(())
    }

    fn handle_operator_envelope(
        &mut self,
        envelope: OperatorEnvelope,
    ) -> Result<(), LiveRuntimeError> {
        let OperatorEnvelope {
            request_id,
            command,
            response,
        } = envelope;
        self.evidence.operator_commands = self.evidence.operator_commands.saturating_add(1);
        let result = self.execute_operator_command(&request_id, command);
        match result {
            Ok(operator_response) => {
                let _ = response.send(operator_response);
                Ok(())
            }
            Err(error) => {
                let _ = response.send(OperatorResponse::rejected(
                    request_id,
                    format!("operator command failed: {error}"),
                ));
                Err(error)
            }
        }
    }

    fn execute_operator_command(
        &mut self,
        request_id: &str,
        command: OperatorCommand,
    ) -> Result<OperatorResponse, LiveRuntimeError> {
        match command {
            OperatorCommand::Status => Ok(OperatorResponse::accepted(
                request_id,
                "runtime status",
                Some(self.operator_status()),
            )),
            OperatorCommand::KillSwitch { reason } => {
                self.coordinator.set_order_entry_enabled(false);
                self.commit_operator_system_event(
                    request_id,
                    SystemEventKind::KillSwitchActivated,
                    None,
                    reason,
                )?;
                self.evidence.operator_mutations =
                    self.evidence.operator_mutations.saturating_add(1);
                Ok(OperatorResponse::accepted(
                    request_id,
                    "kill switch activated",
                    Some(self.operator_status()),
                ))
            }
            OperatorCommand::KillAccount { account_id, reason } => {
                if !self.coordinator.manages_account(&account_id) {
                    return Ok(OperatorResponse::rejected(
                        request_id,
                        format!("account {account_id} is not managed by this runtime"),
                    ));
                }
                let output = self.coordinator.halt_account(
                    unix_time_ms(),
                    &account_id,
                    format!("authenticated operator request {request_id}: {reason}"),
                )?;
                self.commit_output(output)?;
                self.evidence.operator_mutations =
                    self.evidence.operator_mutations.saturating_add(1);
                Ok(OperatorResponse::accepted(
                    request_id,
                    format!("account {account_id} halted"),
                    Some(self.operator_status()),
                ))
            }
            OperatorCommand::HaltSymbol { symbol, reason } => {
                if !self.coordinator.manages_symbol(&symbol) {
                    return Ok(OperatorResponse::rejected(
                        request_id,
                        format!("symbol {symbol} is not managed by this runtime"),
                    ));
                }
                self.commit_operator_system_event(
                    request_id,
                    SystemEventKind::SymbolHalted,
                    Some(symbol),
                    reason,
                )?;
                self.evidence.operator_mutations =
                    self.evidence.operator_mutations.saturating_add(1);
                Ok(OperatorResponse::accepted(
                    request_id,
                    "symbol halted",
                    Some(self.operator_status()),
                ))
            }
            OperatorCommand::ResumeSymbol { symbol, reason } => {
                if !self.coordinator.manages_symbol(&symbol) {
                    return Ok(OperatorResponse::rejected(
                        request_id,
                        format!("symbol {symbol} is not managed by this runtime"),
                    ));
                }
                if let Some(account_id) = self.coordinator.halted_account_for_symbol(&symbol) {
                    return Ok(OperatorResponse::rejected(
                        request_id,
                        format!(
                            "symbol {symbol} belongs to halted account {account_id}; account kills cannot be reset live"
                        ),
                    ));
                }
                self.commit_operator_system_event(
                    request_id,
                    SystemEventKind::SymbolResumed,
                    Some(symbol),
                    reason,
                )?;
                self.evidence.operator_mutations =
                    self.evidence.operator_mutations.saturating_add(1);
                Ok(OperatorResponse::accepted(
                    request_id,
                    "symbol resumed",
                    Some(self.operator_status()),
                ))
            }
            OperatorCommand::Shutdown { reason } => {
                self.coordinator.set_order_entry_enabled(false);
                self.evidence.operator_mutations =
                    self.evidence.operator_mutations.saturating_add(1);
                self.operator_shutdown_reason = Some(format!(
                    "authenticated operator shutdown {request_id}: {reason}"
                ));
                Ok(OperatorResponse::accepted(
                    request_id,
                    "graceful shutdown accepted",
                    Some(self.operator_status()),
                ))
            }
        }
    }

    fn commit_operator_system_event(
        &mut self,
        request_id: &str,
        kind: SystemEventKind,
        symbol: Option<String>,
        reason: String,
    ) -> Result<(), LiveRuntimeError> {
        let output = self
            .coordinator
            .process_event(NormalizedEvent::System(SystemEvent {
                ts_ms: unix_time_ms(),
                kind,
                venue: None,
                account_id: None,
                symbol,
                reason: format!("authenticated operator request {request_id}: {reason}"),
            }));
        self.commit_output(output)
    }

    fn operator_status(&self) -> OperatorStatus {
        OperatorStatus {
            readiness: self.coordinator.readiness(),
            active_orders: self.coordinator.active_order_count(),
            kill_switch_active: self.coordinator.kill_switch_active(),
            halted_accounts: self.coordinator.halted_accounts().clone(),
            shutdown_in_progress: self.shutdown_in_progress
                || self.operator_shutdown_reason.is_some(),
        }
    }

    fn handle_runtime_event(&mut self, event: RuntimeEvent) -> Result<(), LiveRuntimeError> {
        match event {
            RuntimeEvent::Raw {
                source_id,
                envelope,
            } => {
                let (account_id, adapter) = {
                    let source = self.sources.get(source_id).ok_or_else(|| {
                        LiveRuntimeError::FeedAdapter("unknown feed source".to_string())
                    })?;
                    (source.account_id.clone(), Arc::clone(&source.adapter))
                };
                self.record_storage(StorageRecord::Raw {
                    account_id,
                    envelope: envelope.clone(),
                })?;
                let parsed = adapter
                    .parse(&envelope)
                    .map_err(|error| LiveRuntimeError::FeedAdapter(error.to_string()))?;
                for event in parsed {
                    for output in self.processor.process(event) {
                        self.observe_feed_output(&output);
                        let output = self.coordinator.process_feed(output)?;
                        self.commit_output(output)?;
                    }
                }
            }
            RuntimeEvent::Connection { source_id, status } => {
                if status.kind == ConnectionStatusKind::Disconnected {
                    self.evidence.connection_disconnect_events += 1;
                }
                let (events, public_connectivity) = {
                    let source = self.sources.get_mut(source_id).ok_or_else(|| {
                        LiveRuntimeError::FeedAdapter("unknown feed source".to_string())
                    })?;
                    let events = source.on_status(status);
                    (events, source.public_connectivity_ready())
                };
                if let Some(ready) = public_connectivity {
                    self.coordinator.mark_public_connectivity(
                        ready,
                        if ready {
                            "every public subscription has an acknowledged connection"
                        } else {
                            "one or more public subscriptions has no acknowledged connection"
                        },
                    );
                }
                for event in events {
                    let output = self
                        .coordinator
                        .process_event(NormalizedEvent::System(event));
                    self.commit_output(output)?;
                }
            }
            RuntimeEvent::SubmitComplete {
                account_id,
                outcome,
                ts_ms,
            } => {
                let output = self
                    .coordinator
                    .on_submit_outcome(&account_id, outcome, ts_ms)?;
                self.commit_output(output)?;
            }
            RuntimeEvent::SubmitFailed {
                account_id,
                client_order_id,
                ts_ms,
                ambiguous,
                reason,
            } => {
                let output = self.coordinator.on_submit_error(
                    &account_id,
                    &client_order_id,
                    ts_ms,
                    ambiguous,
                    reason,
                )?;
                self.commit_output(output)?;
            }
            RuntimeEvent::CancelComplete {
                account_id,
                outcome,
                ts_ms,
            } => {
                let output = self
                    .coordinator
                    .on_cancel_outcome(&account_id, outcome, ts_ms)?;
                self.commit_output(output)?;
            }
            RuntimeEvent::CancelFailed {
                account_id,
                client_order_id,
                ts_ms,
                ambiguous,
                reason,
            } => {
                let output = self.coordinator.on_cancel_error(
                    &account_id,
                    &client_order_id,
                    ts_ms,
                    ambiguous,
                    reason,
                )?;
                self.commit_output(output)?;
            }
            RuntimeEvent::RemoteState {
                account_id,
                remote_orders,
                remote_fills,
                ts_ms,
            } => {
                self.reconcile_inflight.remove(&account_id);
                self.cancel_inflight
                    .retain(|(cancel_account, _)| cancel_account != &account_id);
                self.apply_remote_recovery(&account_id, &remote_orders, &remote_fills)?;
                let state = self
                    .coordinator
                    .private_state(&account_id)
                    .ok_or_else(|| CoordinatorError::UnknownAccount(account_id.clone()))?;
                let report = reconcile(
                    state.order_reducer(),
                    state.seen_fill_ids(),
                    &remote_orders,
                    &remote_fills,
                );
                let clean = report.is_clean();
                if self.shutdown_in_progress
                    && self.shutdown_reconciliation_requested.contains(&account_id)
                {
                    if is_zero_order_reconciliation(&report) {
                        self.shutdown_reconciled_accounts.insert(account_id.clone());
                    } else {
                        self.shutdown_reconciled_accounts.remove(&account_id);
                    }
                }
                let reason = if clean {
                    "REST and canonical private state agree".to_string()
                } else {
                    format!("{:?}", report.issues)
                };
                let output = self.coordinator.on_reconciliation(ReconciliationResult {
                    account_id,
                    ts_ms,
                    clean,
                    local_live_orders: report.local_live_orders,
                    remote_live_orders: report.remote_live_orders,
                    remote_recent_fills: report.remote_fills,
                    reason,
                })?;
                self.commit_output(output)?;
            }
            RuntimeEvent::ReconcileFailed {
                account_id,
                ts_ms,
                reason,
            } => {
                self.reconcile_inflight.remove(&account_id);
                self.shutdown_reconciled_accounts.remove(&account_id);
                let output = self.coordinator.on_reconciliation(ReconciliationResult {
                    account_id,
                    ts_ms,
                    clean: false,
                    local_live_orders: 0,
                    remote_live_orders: 0,
                    remote_recent_fills: 0,
                    reason: format!("REST reconciliation request failed: {reason}"),
                })?;
                self.commit_output(output)?;
            }
            RuntimeEvent::Fatal(message) => return Err(LiveRuntimeError::GatewayTask(message)),
        }
        Ok(())
    }

    fn observe_feed_output(&mut self, output: &FeedOutput) {
        if let FeedOutput::PrivateOrder {
            account_id: Some(account_id),
            update,
        } = output
            && matches!(
                update.state,
                PrivateOrderState::Filled
                    | PrivateOrderState::Cancelled
                    | PrivateOrderState::Rejected
            )
        {
            let order_id = if update.client_order_id.is_empty() {
                &update.exchange_order_id
            } else {
                &update.client_order_id
            };
            self.cancel_inflight
                .remove(&(account_id.clone(), order_id.clone()));
        }
    }

    fn apply_remote_recovery(
        &mut self,
        account_id: &str,
        remote_orders: &[RemoteOrder],
        remote_fills: &[RemoteFill],
    ) -> Result<(), LiveRuntimeError> {
        for fill in remote_fills {
            let order_id = if fill.client_order_id.is_empty() {
                &fill.exchange_order_id
            } else {
                &fill.client_order_id
            };
            let should_apply = self
                .coordinator
                .private_state(account_id)
                .is_some_and(|state| {
                    state.order_reducer().contains_order(order_id)
                        && !state.seen_fill_ids().contains(&fill.fill_id)
                });
            if should_apply {
                let output = self.coordinator.process_feed(FeedOutput::PrivateFill {
                    account_id: Some(account_id.to_string()),
                    fill: fill.clone(),
                })?;
                self.commit_output(output)?;
            }
        }
        for remote in remote_orders {
            let order_id = remote_order_id(remote);
            let known = self
                .coordinator
                .private_state(account_id)
                .is_some_and(|state| state.order_reducer().contains_order(&order_id));
            if known {
                let output = self.coordinator.process_feed(FeedOutput::PrivateOrder {
                    account_id: Some(account_id.to_string()),
                    update: private_update_from_remote(remote.clone()),
                })?;
                self.commit_output(output)?;
            }
        }
        Ok(())
    }

    fn commit_output(&mut self, output: CoordinatorOutput) -> Result<(), LiveRuntimeError> {
        for record in output.records {
            self.record_storage(record)?;
        }
        for action in output.actions {
            self.dispatch_action(action)?;
        }
        Ok(())
    }

    async fn commit_shutdown_output(
        &mut self,
        output: CoordinatorOutput,
    ) -> Result<(), LiveRuntimeError> {
        for record in output.records {
            self.record_storage(record)?;
        }
        for action in output.actions {
            match action {
                LiveAction::Submit(_) => return Err(LiveRuntimeError::UnsafeShutdownSubmit),
                LiveAction::Cancel(action) => self.dispatch_shutdown_cancel(action).await?,
                LiveAction::RecoverBook(_) => {}
                LiveAction::Reconcile(action) => {
                    self.dispatch_shutdown_reconcile(action).await?;
                }
            }
        }
        Ok(())
    }

    async fn dispatch_shutdown_cancel(
        &mut self,
        action: CancelAction,
    ) -> Result<(), LiveRuntimeError> {
        let cancel_key = (action.account_id.clone(), action.client_order_id.clone());
        if !self.cancel_inflight.insert(cancel_key.clone()) {
            return Ok(());
        }
        if let Err(error) = self.record_storage(StorageRecord::OrderRequest(OrderRequestRecord {
            ts_ms: action.ts_ms,
            account_id: action.account_id.clone(),
            operation: OrderOperation::Cancel,
            idempotency_key: None,
            client_order_id: Some(action.client_order_id.clone()),
            exchange_order_id: None,
            symbol: action.symbol.clone(),
        })) {
            self.cancel_inflight.remove(&cancel_key);
            return Err(error);
        }
        let sender = match self.order_sender(&action.account_id) {
            Ok(sender) => sender.clone(),
            Err(error) => {
                self.cancel_inflight.remove(&cancel_key);
                return Err(error);
            }
        };
        if sender.send(OrderTaskCommand::Cancel(action)).await.is_err() {
            self.cancel_inflight.remove(&cancel_key);
            return Err(LiveRuntimeError::OrderQueueUnavailable(cancel_key.0));
        }
        Ok(())
    }

    fn record_storage(&mut self, record: StorageRecord) -> Result<(), LiveRuntimeError> {
        self.evidence.observe_record(&record);
        if let Err(error) = self.storage_sink.try_record(record) {
            if !self.shutdown_in_progress {
                return Err(error.into());
            }
            tracing::error!(%error, "storage unavailable during fail-closed shutdown");
            self.shutdown_storage_error
                .get_or_insert_with(|| error.to_string());
            return Ok(());
        }
        self.evidence.max_storage_queue_depth = self
            .evidence
            .max_storage_queue_depth
            .max(self.storage_sink.queue_depth());
        Ok(())
    }

    fn dispatch_action(&mut self, action: LiveAction) -> Result<(), LiveRuntimeError> {
        match action {
            LiveAction::Submit(action) => {
                self.record_storage(StorageRecord::OrderRequest(OrderRequestRecord {
                    ts_ms: action.ts_ms,
                    account_id: action.account_id.clone(),
                    operation: OrderOperation::Submit,
                    idempotency_key: Some(action.idempotency_key.clone()),
                    client_order_id: Some(action.client_order_id.clone()),
                    exchange_order_id: None,
                    symbol: action.order.symbol.clone(),
                }))?;
                self.order_sender(&action.account_id)?
                    .try_send(OrderTaskCommand::Submit(action))
                    .map_err(|_| {
                        LiveRuntimeError::OrderQueueUnavailable("submit account queue".to_string())
                    })?;
            }
            LiveAction::Cancel(action) => {
                let cancel_key = (action.account_id.clone(), action.client_order_id.clone());
                if !self.cancel_inflight.insert(cancel_key.clone()) {
                    return Ok(());
                }
                if let Err(error) =
                    self.record_storage(StorageRecord::OrderRequest(OrderRequestRecord {
                        ts_ms: action.ts_ms,
                        account_id: action.account_id.clone(),
                        operation: OrderOperation::Cancel,
                        idempotency_key: None,
                        client_order_id: Some(action.client_order_id.clone()),
                        exchange_order_id: None,
                        symbol: action.symbol.clone(),
                    }))
                {
                    self.cancel_inflight.remove(&cancel_key);
                    return Err(error);
                }
                let sender = match self.order_sender(&action.account_id) {
                    Ok(sender) => sender.clone(),
                    Err(error) => {
                        self.cancel_inflight.remove(&cancel_key);
                        return Err(error);
                    }
                };
                if sender.try_send(OrderTaskCommand::Cancel(action)).is_err() {
                    self.cancel_inflight.remove(&cancel_key);
                    return Err(LiveRuntimeError::OrderQueueUnavailable(cancel_key.0));
                }
            }
            LiveAction::RecoverBook(request) => {
                let routes = self.feeds[self.public_feed_index].request_recovery(&request);
                if routes == 0 {
                    return Err(LiveRuntimeError::MissingRecoveryRoute(
                        request.stream.symbol,
                    ));
                }
            }
            LiveAction::Reconcile(action) => self.dispatch_reconcile(action)?,
        }
        Ok(())
    }

    fn dispatch_reconcile(&mut self, action: ReconcileAction) -> Result<(), LiveRuntimeError> {
        if !self.reconcile_inflight.insert(action.account_id.clone()) {
            return Ok(());
        }
        self.last_reconcile_attempt
            .insert(action.account_id.clone(), Instant::now());
        let orders = match self.reconciliation_order_refs(&action.account_id) {
            Ok(orders) => orders,
            Err(error) => {
                self.reconcile_inflight.remove(&action.account_id);
                return Err(error);
            }
        };
        let sender = match self.order_sender(&action.account_id) {
            Ok(sender) => sender.clone(),
            Err(error) => {
                self.reconcile_inflight.remove(&action.account_id);
                return Err(error);
            }
        };
        sender
            .try_send(OrderTaskCommand::Reconcile(orders))
            .map_err(|_| {
                self.reconcile_inflight.remove(&action.account_id);
                LiveRuntimeError::OrderQueueUnavailable(action.account_id)
            })
    }

    async fn dispatch_shutdown_reconcile(
        &mut self,
        action: ReconcileAction,
    ) -> Result<(), LiveRuntimeError> {
        if !self.reconcile_inflight.insert(action.account_id.clone()) {
            return Ok(());
        }
        self.last_reconcile_attempt
            .insert(action.account_id.clone(), Instant::now());
        let orders = match self.reconciliation_order_refs(&action.account_id) {
            Ok(orders) => orders,
            Err(error) => {
                self.reconcile_inflight.remove(&action.account_id);
                return Err(error);
            }
        };
        let sender = match self.order_sender(&action.account_id) {
            Ok(sender) => sender.clone(),
            Err(error) => {
                self.reconcile_inflight.remove(&action.account_id);
                return Err(error);
            }
        };
        if sender
            .send(OrderTaskCommand::Reconcile(orders))
            .await
            .is_err()
        {
            self.reconcile_inflight.remove(&action.account_id);
            return Err(LiveRuntimeError::OrderQueueUnavailable(action.account_id));
        }
        Ok(())
    }

    fn reconciliation_order_refs(
        &self,
        account_id: &str,
    ) -> Result<Vec<ReconcileOrderRef>, LiveRuntimeError> {
        let state = self
            .coordinator
            .private_state(account_id)
            .ok_or_else(|| CoordinatorError::UnknownAccount(account_id.to_string()))?;
        Ok(state
            .order_reducer()
            .orders()
            .filter(|(_, order)| {
                matches!(
                    order.status,
                    OrderStatus::PendingNew | OrderStatus::Live | OrderStatus::PartiallyFilled
                )
            })
            .map(|(order_id, order)| ReconcileOrderRef {
                order_id: order_id.to_string(),
                symbol: order.symbol.clone(),
                side: order.side,
                price: order.price,
                qty: order.qty,
                filled_qty: order.filled_qty,
                average_fill_price: order.avg_fill_price,
                last_update_ms: state.last_order_update_ms(order_id).unwrap_or(0),
            })
            .collect())
    }

    async fn request_shutdown_reconciliation(
        &mut self,
        ts_ms: u64,
        force: bool,
    ) -> Result<(), LiveRuntimeError> {
        let accounts = self
            .order_senders
            .keys()
            .filter(|account_id| !self.shutdown_reconciled_accounts.contains(*account_id))
            .filter(|account_id| !self.reconcile_inflight.contains(*account_id))
            .filter(|account_id| {
                !self.shutdown_reconciliation_requested.contains(*account_id)
                    || force
                    || self
                        .last_reconcile_attempt
                        .get(*account_id)
                        .is_none_or(|last| last.elapsed() >= Duration::from_secs(2))
            })
            .cloned()
            .collect::<Vec<_>>();
        for account_id in accounts {
            self.dispatch_shutdown_reconcile(ReconcileAction {
                ts_ms,
                account_id: account_id.clone(),
                reason: "verify zero exchange orders during graceful shutdown".to_string(),
            })
            .await?;
            self.shutdown_reconciliation_requested.insert(account_id);
        }
        Ok(())
    }

    fn retry_reconciliation(&mut self, ts_ms: u64) -> Result<(), LiveRuntimeError> {
        let readiness = self.coordinator.readiness();
        let accounts = readiness
            .missing_reconciliation
            .into_iter()
            .filter(|account_id| !self.reconcile_inflight.contains(account_id))
            .filter(|account_id| {
                self.last_reconcile_attempt
                    .get(account_id)
                    .is_none_or(|last| last.elapsed() >= Duration::from_secs(2))
            })
            .collect::<Vec<_>>();
        for account_id in accounts {
            self.dispatch_reconcile(ReconcileAction {
                ts_ms,
                account_id,
                reason: "retry degraded reconciliation".to_string(),
            })?;
        }
        Ok(())
    }

    fn order_sender(
        &self,
        account_id: &str,
    ) -> Result<&mpsc::Sender<OrderTaskCommand>, LiveRuntimeError> {
        self.order_senders
            .get(account_id)
            .ok_or_else(|| LiveRuntimeError::OrderQueueUnavailable(account_id.to_string()))
    }

    async fn shutdown(&mut self) -> Result<(), LiveRuntimeError> {
        let operator_result = match self.operator_service.take() {
            Some(service) => service.shutdown().await.map_err(LiveRuntimeError::from),
            None => Ok(()),
        };
        self.operator_rx.take();
        for sender in self.order_senders.values() {
            let _ = sender.try_send(OrderTaskCommand::Shutdown);
        }
        self.order_senders.clear();
        self.control_rx.close();
        self.feed_rx.close();
        for feed in self.feeds.drain(..) {
            feed.shutdown().await;
        }
        for task in self.feed_tasks.drain(..) {
            task.await?;
        }
        for task in self.order_tasks.drain(..) {
            task.await?;
        }
        if let Some(storage) = self.storage.take() {
            storage.shutdown().await?;
        }
        operator_result?;
        Ok(())
    }
}

enum RuntimeEvent {
    Raw {
        source_id: usize,
        envelope: reap_core::RawEnvelope,
    },
    Connection {
        source_id: usize,
        status: ConnectionStatus,
    },
    SubmitComplete {
        account_id: String,
        outcome: SubmitOutcome,
        ts_ms: u64,
    },
    SubmitFailed {
        account_id: String,
        client_order_id: String,
        ts_ms: u64,
        ambiguous: bool,
        reason: String,
    },
    CancelComplete {
        account_id: String,
        outcome: CancelOutcome,
        ts_ms: u64,
    },
    CancelFailed {
        account_id: String,
        client_order_id: String,
        ts_ms: u64,
        ambiguous: bool,
        reason: String,
    },
    RemoteState {
        account_id: String,
        remote_orders: Vec<RemoteOrder>,
        remote_fills: Vec<RemoteFill>,
        ts_ms: u64,
    },
    ReconcileFailed {
        account_id: String,
        ts_ms: u64,
        reason: String,
    },
    Fatal(String),
}

enum OrderTaskCommand {
    Submit(SubmitAction),
    Cancel(CancelAction),
    Reconcile(Vec<ReconcileOrderRef>),
    Shutdown,
}

#[derive(Debug, Clone)]
struct ReconcileOrderRef {
    order_id: String,
    symbol: String,
    side: reap_core::Side,
    price: f64,
    qty: f64,
    filled_qty: f64,
    average_fill_price: f64,
    last_update_ms: u64,
}

async fn run_order_task(
    account_id: String,
    mut gateway: LiveGateway,
    mut commands: mpsc::Receiver<OrderTaskCommand>,
    events: mpsc::Sender<RuntimeEvent>,
    ambiguous_submit_grace_ms: u64,
) {
    while let Some(command) = commands.recv().await {
        match command {
            OrderTaskCommand::Submit(action) => {
                let client_order_id = action.client_order_id;
                let preparation = match gateway.prepare_registered_submit(
                    action.idempotency_key,
                    action.order,
                    client_order_id.clone(),
                ) {
                    Ok(preparation) => preparation,
                    Err(error) => {
                        if events
                            .send(RuntimeEvent::Fatal(format!(
                                "account {account_id} submit preparation failed: {error}"
                            )))
                            .await
                            .is_err()
                        {
                            return;
                        }
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
                            outcome,
                            ts_ms: unix_time_ms(),
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                    continue;
                };
                match gateway.execute_submit(prepared).await {
                    Ok(outcome) => {
                        if events
                            .send(RuntimeEvent::SubmitComplete {
                                account_id: account_id.clone(),
                                outcome,
                                ts_ms: unix_time_ms(),
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(error) => {
                        if events
                            .send(RuntimeEvent::SubmitFailed {
                                account_id: account_id.clone(),
                                client_order_id,
                                ts_ms: unix_time_ms(),
                                ambiguous: error.is_ambiguous(),
                                reason: error.to_string(),
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
            OrderTaskCommand::Cancel(action) => {
                match gateway
                    .cancel(&action.symbol, None, Some(action.client_order_id.clone()))
                    .await
                {
                    Ok(outcome) => {
                        if events
                            .send(RuntimeEvent::CancelComplete {
                                account_id: account_id.clone(),
                                outcome,
                                ts_ms: unix_time_ms(),
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(error) => {
                        if events
                            .send(RuntimeEvent::CancelFailed {
                                account_id: account_id.clone(),
                                client_order_id: action.client_order_id,
                                ts_ms: unix_time_ms(),
                                ambiguous: error.is_ambiguous(),
                                reason: error.to_string(),
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
            OrderTaskCommand::Reconcile(restored_orders) => {
                match gateway.fetch_remote_state(None, None).await {
                    Ok((mut remote_orders, remote_fills)) => {
                        let mut remote_ids = remote_orders
                            .iter()
                            .map(remote_order_id)
                            .collect::<HashSet<_>>();
                        let mut failed = None;
                        for restored in restored_orders {
                            if remote_ids.contains(&restored.order_id) {
                                continue;
                            }
                            let details = match gateway
                                .fetch_order_details(&restored.symbol, &restored.order_id)
                                .await
                            {
                                Ok(details) => details,
                                Err(error)
                                    if error.is_order_not_found()
                                        && unix_time_ms()
                                            .saturating_sub(restored.last_update_ms)
                                            < ambiguous_submit_grace_ms =>
                                {
                                    failed = Some(format!(
                                        "order {} is not visible within the ambiguous-submit grace period",
                                        restored.order_id
                                    ));
                                    break;
                                }
                                Err(error) if error.is_order_not_found() => RemoteOrder {
                                    exchange_order_id: String::new(),
                                    client_order_id: restored.order_id.clone(),
                                    symbol: restored.symbol,
                                    side: restored.side,
                                    state: PrivateOrderState::Rejected,
                                    price: restored.price,
                                    qty: restored.qty,
                                    cumulative_filled_qty: restored.filled_qty,
                                    average_fill_price: restored.average_fill_price,
                                    update_time_ms: unix_time_ms(),
                                },
                                Err(error) => {
                                    failed = Some(error.to_string());
                                    break;
                                }
                            };
                            remote_ids.insert(remote_order_id(&details));
                            remote_orders.push(details);
                        }
                        if let Some(reason) = failed {
                            if events
                                .send(RuntimeEvent::ReconcileFailed {
                                    account_id: account_id.clone(),
                                    ts_ms: unix_time_ms(),
                                    reason,
                                })
                                .await
                                .is_err()
                            {
                                return;
                            }
                            continue;
                        }
                        if events
                            .send(RuntimeEvent::RemoteState {
                                account_id: account_id.clone(),
                                remote_orders,
                                remote_fills,
                                ts_ms: unix_time_ms(),
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(error) => {
                        if events
                            .send(RuntimeEvent::ReconcileFailed {
                                account_id: account_id.clone(),
                                ts_ms: unix_time_ms(),
                                reason: error.to_string(),
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
            OrderTaskCommand::Shutdown => return,
        }
    }
}

struct FeedSourceState {
    adapter: Arc<dyn VenueAdapter>,
    account_id: Option<String>,
    expected_connections: HashSet<ConnId>,
    ready_connections: HashSet<ConnId>,
    public_subscriptions: Vec<PublicSubscriptionRoute>,
    private_ready: bool,
}

struct PublicSubscriptionRoute {
    channel: Channel,
    symbol: Option<String>,
    connections: HashSet<ConnId>,
}

impl FeedSourceState {
    fn public(adapter: Arc<dyn VenueAdapter>, plans: &[SocketPlan]) -> Self {
        let mut public_subscriptions: HashMap<(Channel, Option<String>), HashSet<ConnId>> =
            HashMap::new();
        for plan in plans {
            for subscription in &plan.subscriptions {
                public_subscriptions
                    .entry((subscription.channel.clone(), subscription.symbol.clone()))
                    .or_default()
                    .insert(plan.conn_id.clone());
            }
        }
        Self {
            adapter,
            account_id: None,
            expected_connections: plans.iter().map(|plan| plan.conn_id.clone()).collect(),
            ready_connections: HashSet::new(),
            public_subscriptions: public_subscriptions
                .into_iter()
                .map(|((channel, symbol), connections)| PublicSubscriptionRoute {
                    channel,
                    symbol,
                    connections,
                })
                .collect(),
            private_ready: false,
        }
    }

    fn private(adapter: Arc<dyn VenueAdapter>, account_id: String, plans: &[SocketPlan]) -> Self {
        Self {
            adapter,
            account_id: Some(account_id),
            expected_connections: plans.iter().map(|plan| plan.conn_id.clone()).collect(),
            ready_connections: HashSet::new(),
            public_subscriptions: Vec::new(),
            private_ready: false,
        }
    }

    fn public_connectivity_ready(&self) -> Option<bool> {
        self.account_id.is_none().then(|| {
            !self.public_subscriptions.is_empty()
                && self
                    .public_subscriptions
                    .iter()
                    .all(|route| !route.connections.is_disjoint(&self.ready_connections))
        })
    }

    fn on_status(&mut self, status: ConnectionStatus) -> Vec<SystemEvent> {
        match status.kind {
            ConnectionStatusKind::Ready | ConnectionStatusKind::Heartbeat => {
                self.ready_connections.insert(status.conn_id.clone());
            }
            ConnectionStatusKind::Disconnected => {
                self.ready_connections.remove(&status.conn_id);
            }
        }
        if let Some(account_id) = &self.account_id {
            let ready = self.expected_connections.is_subset(&self.ready_connections);
            let transition = ready != self.private_ready;
            self.private_ready = ready;
            if ready {
                return vec![SystemEvent {
                    ts_ms: status.ts_ms,
                    kind: if transition {
                        SystemEventKind::PrivateStreamRecovered
                    } else {
                        SystemEventKind::PrivateStreamHeartbeat
                    },
                    venue: Some(status.venue),
                    account_id: Some(account_id.clone()),
                    symbol: None,
                    reason: if transition {
                        "all required private websocket channels are connected".to_string()
                    } else {
                        status.reason
                    },
                }];
            }
            if transition || status.kind == ConnectionStatusKind::Disconnected {
                return vec![SystemEvent {
                    ts_ms: status.ts_ms,
                    kind: SystemEventKind::PrivateStreamStale,
                    venue: Some(status.venue),
                    account_id: Some(account_id.clone()),
                    symbol: None,
                    reason: format!(
                        "private websocket set is incomplete ({}/{})",
                        self.ready_connections.len(),
                        self.expected_connections.len()
                    ),
                }];
            }
            return Vec::new();
        }

        if status.kind != ConnectionStatusKind::Disconnected {
            return Vec::new();
        }
        self.public_subscriptions
            .iter()
            .filter(|route| route.connections.is_disjoint(&self.ready_connections))
            .map(|route| SystemEvent {
                ts_ms: status.ts_ms,
                kind: SystemEventKind::FeedStale,
                venue: Some(status.venue),
                account_id: None,
                symbol: route.symbol.clone(),
                reason: format!(
                    "all redundant {:?} websocket connections are down",
                    route.channel
                ),
            })
            .collect()
    }
}

fn spawn_feed_forwarders(
    source_id: usize,
    feed: &mut SupervisedFeed,
    events: &mpsc::Sender<RuntimeEvent>,
    tasks: &mut Vec<JoinHandle<()>>,
) {
    let mut raw = feed.take_raw();
    let raw_events = events.clone();
    tasks.push(tokio::spawn(async move {
        while let Some(envelope) = raw.recv().await {
            if raw_events
                .send(RuntimeEvent::Raw {
                    source_id,
                    envelope,
                })
                .await
                .is_err()
            {
                return;
            }
        }
    }));
    let mut status = feed.take_status();
    let status_events = events.clone();
    tasks.push(tokio::spawn(async move {
        while let Some(status) = status.recv().await {
            if status_events
                .send(RuntimeEvent::Connection { source_id, status })
                .await
                .is_err()
            {
                return;
            }
        }
    }));
}

fn public_subscriptions(config: &LiveConfig) -> Vec<Subscription> {
    let mut subscriptions = Vec::new();
    let mut seen = HashSet::new();
    for instrument in &config.strategy.instruments {
        push_public_subscription(
            &mut subscriptions,
            &mut seen,
            Channel::Books,
            &instrument.symbol,
            FeedPriority::Critical,
            config.runtime.public_connections_per_subscription,
        );
        push_public_subscription(
            &mut subscriptions,
            &mut seen,
            Channel::Trades,
            &instrument.symbol,
            FeedPriority::High,
            config.runtime.public_connections_per_subscription,
        );
        if instrument.kind.is_derivative() {
            for channel in ["funding-rate", "mark-price", "price-limit"] {
                push_public_subscription(
                    &mut subscriptions,
                    &mut seen,
                    Channel::Custom(channel.to_string()),
                    &instrument.symbol,
                    FeedPriority::High,
                    config.runtime.public_connections_per_subscription,
                );
            }
        }
        if let Some(index_symbol) = &instrument.index_symbol {
            push_public_subscription(
                &mut subscriptions,
                &mut seen,
                Channel::Custom("index-tickers".to_string()),
                index_symbol,
                FeedPriority::High,
                config.runtime.public_connections_per_subscription,
            );
        }
    }
    subscriptions
}

fn push_public_subscription(
    subscriptions: &mut Vec<Subscription>,
    seen: &mut HashSet<(Channel, String)>,
    channel: Channel,
    symbol: &str,
    priority: FeedPriority,
    connections: usize,
) {
    if !seen.insert((channel.clone(), symbol.to_string())) {
        return;
    }
    let mut subscription = Subscription::public(Venue::Okx, channel, symbol, priority);
    subscription.connections = connections;
    subscriptions.push(subscription);
}

fn private_subscriptions(enable_vip_fills_channel: bool) -> Vec<Subscription> {
    let mut channels = vec![Channel::Orders, Channel::Account, Channel::Positions];
    if enable_vip_fills_channel {
        channels.push(Channel::Fills);
    }
    channels
        .into_iter()
        .map(|channel| Subscription::private(Venue::Okx, channel, FeedPriority::Critical))
        .collect()
}

fn unix_time_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(u64::MAX as u128) as u64
}

fn unix_time_ms() -> u64 {
    unix_time_ns() / 1_000_000
}

fn elapsed_ms(started: &Instant) -> u64 {
    started.elapsed().as_millis().min(u64::MAX as u128) as u64
}

async fn receive_operator(
    receiver: &mut Option<mpsc::Receiver<OperatorEnvelope>>,
) -> Option<OperatorEnvelope> {
    match receiver {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

#[cfg(unix)]
async fn shutdown_signal() -> Result<(), std::io::Error> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() -> Result<(), std::io::Error> {
    tokio::signal::ctrl_c().await
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::sync::atomic::{AtomicBool, Ordering};

    use reap_core::{AccountUpdate, Balance, OrderEvent, OrderUpdate, Side};
    use reap_risk::{InstrumentRiskModel, RiskLimits};
    use reap_strategy::{ChaosConfig, InstrumentKindConfig};
    use reap_venue::okx::{OkxAccountLevel, OkxInstrumentType, OkxPositionMode};

    use crate::{
        LiveAccountConfig, LiveStorageConfig, OkxTradeModeConfig, OkxVenueConfig, RuntimeConfig,
        VerifiedInstrument,
    };

    use super::*;

    fn status(conn_id: &str, kind: ConnectionStatusKind) -> ConnectionStatus {
        ConnectionStatus {
            conn_id: ConnId::new(conn_id),
            venue: Venue::Okx,
            private: true,
            ts_ms: 1,
            kind,
            reason: "test".to_string(),
        }
    }

    fn plan(conn_id: &str, private: bool, channel: Channel, symbol: Option<&str>) -> SocketPlan {
        SocketPlan {
            conn_id: ConnId::new(conn_id),
            venue: Venue::Okx,
            private,
            subscriptions: vec![Subscription {
                venue: Venue::Okx,
                channel,
                symbol: symbol.map(str::to_string),
                priority: FeedPriority::Critical,
                connections: 1,
            }],
        }
    }

    fn config() -> LiveConfig {
        let mut strategy: ChaosConfig =
            toml::from_str(include_str!("../../../examples/iarb2-basic.toml")).unwrap();
        strategy.risk_groups[0].account_id = Some("main".to_string());
        LiveConfig {
            strategy,
            risk: RiskLimits::default(),
            venue: OkxVenueConfig::default(),
            runtime: RuntimeConfig::default(),
            storage: LiveStorageConfig::default(),
            operator: crate::OperatorConfig::default(),
            accounts: vec![LiveAccountConfig {
                id: "main".to_string(),
                api_key_env: "KEY".to_string(),
                secret_key_env: "SECRET".to_string(),
                passphrase_env: "PASS".to_string(),
                expected_account_level: OkxAccountLevel::SingleCurrencyMargin,
                expected_position_mode: OkxPositionMode::NetMode,
                id_prefix: "reap".to_string(),
                node_id: 1,
                trade_modes: HashMap::from([
                    ("BTC-USDT".to_string(), OkxTradeModeConfig::Cash),
                    ("BTC-PERP".to_string(), OkxTradeModeConfig::Cross),
                ]),
            }],
        }
    }

    fn readiness(phase: crate::LivePhase) -> ReadinessSnapshot {
        ReadinessSnapshot {
            phase,
            metadata_verified: phase == crate::LivePhase::Ready,
            storage_ready: phase == crate::LivePhase::Ready,
            public_connectivity_ready: phase == crate::LivePhase::Ready,
            missing_reconciliation: Vec::new(),
            missing_account_snapshots: Vec::new(),
            missing_books: Vec::new(),
            missing_private_streams: Vec::new(),
            faults: BTreeMap::new(),
        }
    }

    fn account_update(ts_ms: u64) -> AccountUpdate {
        AccountUpdate {
            ts_ms,
            balances: vec![Balance {
                account_id: Some("main".to_string()),
                currency: "USDT".to_string(),
                total: 10_000.0,
                available: 10_000.0,
                equity: 10_000.0,
                liability: 0.0,
                max_loan: 0.0,
            }],
            positions: Vec::new(),
            margins: Vec::new(),
        }
    }

    fn verified(config: &LiveConfig, update: AccountUpdate) -> VerifiedBootstrap {
        let instruments = config
            .strategy
            .instruments
            .iter()
            .map(|instrument| {
                let account = config.account_for_symbol(&instrument.symbol).unwrap();
                let instrument_type = match instrument.kind {
                    InstrumentKindConfig::Spot => OkxInstrumentType::Spot,
                    InstrumentKindConfig::LinearSwap | InstrumentKindConfig::InverseSwap => {
                        OkxInstrumentType::Swap
                    }
                    InstrumentKindConfig::Future
                    | InstrumentKindConfig::LinearFuture
                    | InstrumentKindConfig::InverseFuture => OkxInstrumentType::Futures,
                };
                let risk_model = if instrument.kind.is_spot() {
                    InstrumentRiskModel::Spot
                } else if instrument.kind.is_inverse() {
                    InstrumentRiskModel::InverseDerivative {
                        contract_value: instrument.contract_value,
                    }
                } else {
                    InstrumentRiskModel::LinearDerivative {
                        contract_value: instrument.contract_value,
                    }
                };
                (
                    instrument.symbol.clone(),
                    VerifiedInstrument {
                        account_id: account.id.clone(),
                        symbol: instrument.symbol.clone(),
                        instrument_type,
                        trade_mode: account.trade_modes[&instrument.symbol],
                        risk_model,
                        tick_size: instrument.tick_size,
                        lot_size: instrument.lot_size,
                        min_size: instrument.min_trade_size,
                        contract_value: instrument
                            .kind
                            .is_derivative()
                            .then_some(instrument.contract_value),
                    },
                )
            })
            .collect();
        VerifiedBootstrap {
            instruments,
            account_updates: HashMap::from([("main".to_string(), update)]),
            baseline_fill_ids: HashMap::from([("main".to_string(), HashSet::new())]),
        }
    }

    fn ready_coordinator(
        config: &LiveConfig,
        now_ms: u64,
        gateway_actions_enabled: bool,
    ) -> LiveCoordinator {
        let update = account_update(now_ms);
        let mut coordinator = LiveCoordinator::new(
            config.clone(),
            verified(config, update.clone()),
            gateway_actions_enabled,
            "bounded-test",
        )
        .unwrap();
        coordinator.mark_storage_ready(true, "test storage");
        coordinator.mark_public_connectivity(true, "test public sockets");
        coordinator
            .process_feed(FeedOutput::PrivateAccount {
                account_id: Some("main".to_string()),
                update,
            })
            .unwrap();
        coordinator
            .on_reconciliation(ReconciliationResult {
                account_id: "main".to_string(),
                ts_ms: now_ms,
                clean: true,
                local_live_orders: 0,
                remote_live_orders: 0,
                remote_recent_fills: 0,
                reason: "test reconciliation".to_string(),
            })
            .unwrap();
        for symbol in config.required_symbols() {
            coordinator.process_event(NormalizedEvent::System(SystemEvent {
                ts_ms: now_ms,
                kind: SystemEventKind::FeedRecovered,
                venue: Some(Venue::Okx),
                account_id: None,
                symbol: Some(symbol),
                reason: "test book".to_string(),
            }));
        }
        coordinator.process_event(NormalizedEvent::System(SystemEvent {
            ts_ms: now_ms,
            kind: SystemEventKind::PrivateStreamRecovered,
            venue: Some(Venue::Okx),
            account_id: Some("main".to_string()),
            symbol: None,
            reason: "test private sockets".to_string(),
        }));
        assert!(coordinator.readiness().is_ready());
        coordinator
    }

    #[test]
    fn readiness_tracker_records_recovered_outages() {
        let ready = readiness(crate::LivePhase::Ready);
        let degraded = readiness(crate::LivePhase::Degraded);
        let mut tracker = ReadinessTracker::default();

        tracker.observe(100, &ready);
        tracker.observe(250, &degraded);
        tracker.observe(425, &ready);
        let outcome = tracker.finish(LiveStopReason::DurationElapsed, 1_000, ready);

        assert!(outcome.reached_ready);
        assert_eq!(outcome.time_to_ready_ms, Some(100));
        assert_eq!(outcome.readiness_loss_count, 1);
        assert_eq!(outcome.max_readiness_outage_ms, 175);
    }

    #[test]
    fn clean_soak_requires_duration_readiness_no_drift_and_no_open_orders() {
        let mut outcome = RunLoopOutcome {
            stop_reason: LiveStopReason::DurationElapsed,
            elapsed_ms: 1_000,
            reached_ready: true,
            time_to_ready_ms: Some(10),
            readiness_loss_count: 1,
            max_readiness_outage_ms: 25,
            readiness_at_stop: readiness(crate::LivePhase::Ready),
        };
        assert!(qualifies_as_clean_soak(
            &outcome,
            RuntimeEvidence::default(),
            0,
            0,
        ));

        outcome.stop_reason = LiveStopReason::OperatorSignal;
        assert!(!qualifies_as_clean_soak(
            &outcome,
            RuntimeEvidence::default(),
            0,
            0,
        ));
        outcome.stop_reason = LiveStopReason::DurationElapsed;
        outcome.reached_ready = false;
        assert!(!qualifies_as_clean_soak(
            &outcome,
            RuntimeEvidence::default(),
            0,
            0,
        ));
        outcome.reached_ready = true;
        outcome.readiness_at_stop = readiness(crate::LivePhase::Degraded);
        assert!(!qualifies_as_clean_soak(
            &outcome,
            RuntimeEvidence::default(),
            0,
            0,
        ));
        outcome.readiness_at_stop = readiness(crate::LivePhase::Ready);

        let evidence = RuntimeEvidence {
            reconciliation_drift_events: 1,
            ..RuntimeEvidence::default()
        };
        assert!(!qualifies_as_clean_soak(&outcome, evidence, 0, 0));
        assert!(!qualifies_as_clean_soak(
            &outcome,
            RuntimeEvidence::default(),
            1,
            0,
        ));
        assert!(!qualifies_as_clean_soak(
            &outcome,
            RuntimeEvidence::default(),
            0,
            1,
        ));
    }

    #[test]
    fn shutdown_reconciliation_requires_clean_zero_order_state() {
        let mut report = ReconcileReport::default();
        assert!(is_zero_order_reconciliation(&report));

        report.local_live_orders = 1;
        report.remote_live_orders = 1;
        assert!(report.is_clean());
        assert!(!is_zero_order_reconciliation(&report));
    }

    #[test]
    fn runtime_evidence_classifies_persisted_system_events() {
        let system = |kind| {
            StorageRecord::System(SystemEvent {
                ts_ms: 1,
                kind,
                venue: Some(Venue::Okx),
                account_id: None,
                symbol: Some("BTC-USDT".to_string()),
                reason: "test".to_string(),
            })
        };
        let mut evidence = RuntimeEvidence::default();

        evidence.observe_record(&system(SystemEventKind::ReconcileDrift));
        evidence.observe_record(&system(SystemEventKind::BookRecoveryStarted));
        evidence.observe_record(&system(SystemEventKind::FeedStale));
        evidence.observe_record(&system(SystemEventKind::PrivateStreamStale));

        assert_eq!(evidence.reconciliation_drift_events, 1);
        assert_eq!(evidence.book_recovery_events, 1);
        assert_eq!(evidence.stream_stale_events, 2);
    }

    #[tokio::test]
    async fn bounded_run_rejects_zero_duration_before_credentials() {
        let error = run_live(
            config(),
            LiveRunOptions {
                mode: LiveMode::Observe,
                demo_confirmed: false,
                run_duration: Some(Duration::ZERO),
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(error, LiveRuntimeError::InvalidRunDuration));
    }

    #[tokio::test]
    async fn validation_report_is_not_a_soak_pass() {
        let report = run_live(
            config(),
            LiveRunOptions {
                mode: LiveMode::Validate,
                demo_confirmed: false,
                run_duration: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(report.stop_reason, LiveStopReason::Validation);
        assert!(!report.reached_ready);
        assert!(!report.clean_soak);
    }

    #[tokio::test]
    async fn bounded_ready_runtime_completes_with_clean_soak_report() {
        let config = config();
        let now_ms = unix_time_ms();
        let coordinator = ready_coordinator(&config, now_ms, false);
        let path = std::env::temp_dir().join(format!(
            "reap-bounded-soak-{}-{}.jsonl",
            std::process::id(),
            unix_time_ns()
        ));
        let storage = start_jsonl_storage(StorageConfig {
            path: path.clone(),
            channel_capacity: 1_024,
            flush_every_records: 1,
        })
        .await
        .unwrap();
        let storage_sink = storage.sink();
        let (control_tx, control_rx) = mpsc::channel(16);
        let (feed_tx, feed_rx) = mpsc::channel(16);
        let runtime = LiveRuntime {
            mode: LiveMode::Observe,
            run_duration: Some(Duration::from_millis(25)),
            coordinator,
            processor: FeedProcessor::new(16, 16),
            storage: Some(storage),
            storage_sink,
            control_rx,
            feed_rx,
            order_senders: HashMap::new(),
            order_tasks: Vec::new(),
            feeds: Vec::new(),
            feed_tasks: Vec::new(),
            sources: Vec::new(),
            public_feed_index: 0,
            reconcile_inflight: HashSet::new(),
            cancel_inflight: HashSet::new(),
            last_reconcile_attempt: HashMap::new(),
            readiness_timeout_ms: 1_000,
            timer_interval_ms: 100,
            max_feed_age_ms: 60_000,
            shutdown_timeout_ms: 100,
            evidence: RuntimeEvidence::default(),
            shutdown_in_progress: false,
            shutdown_storage_error: None,
            shutdown_reconciliation_requested: HashSet::new(),
            shutdown_reconciled_accounts: HashSet::new(),
            operator_service: None,
            operator_rx: None,
            operator_shutdown_reason: None,
        };

        let report = runtime.run().await.unwrap();
        drop(control_tx);
        drop(feed_tx);
        let _ = std::fs::remove_file(path);

        assert_eq!(report.stop_reason, LiveStopReason::DurationElapsed);
        assert!(report.elapsed_ms >= 20);
        assert!(report.reached_ready);
        assert_eq!(report.time_to_ready_ms, Some(0));
        assert!(report.readiness_at_stop.is_ready());
        assert_eq!(report.reconciliation_drift_events, 0);
        assert_eq!(report.dropped_storage_records, 0);
        assert_eq!(report.active_orders_after_shutdown, 0);
        assert!(report.clean_soak);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn authenticated_operator_commands_run_on_event_loop_and_shutdown_cleanly() {
        use crate::operator::send_operator_command_with_secret;

        const SECRET: &[u8] = b"0123456789abcdef0123456789abcdef";

        let config = config();
        let now_ms = unix_time_ms();
        let coordinator = ready_coordinator(&config, now_ms, false);
        let storage_path = std::env::temp_dir().join(format!(
            "reap-operator-runtime-{}-{}.jsonl",
            std::process::id(),
            unix_time_ns()
        ));
        let socket_path = std::env::temp_dir().join(format!(
            "reap-operator-runtime-{}-{}.sock",
            std::process::id(),
            unix_time_ns()
        ));
        let operator_config = crate::OperatorConfig {
            enabled: true,
            socket_path: socket_path.clone(),
            request_timeout_ms: 1_000,
            ..crate::OperatorConfig::default()
        };
        let storage = start_jsonl_storage(StorageConfig {
            path: storage_path.clone(),
            channel_capacity: 1_024,
            flush_every_records: 1,
        })
        .await
        .unwrap();
        let storage_sink = storage.sink();
        let (control_tx, control_rx) = mpsc::channel(16);
        let (feed_tx, feed_rx) = mpsc::channel(16);
        let (operator_tx, operator_rx) = mpsc::channel(16);
        let operator_service =
            start_operator_service(&operator_config, SECRET.to_vec(), operator_tx)
                .await
                .unwrap();
        let runtime = LiveRuntime {
            mode: LiveMode::Observe,
            run_duration: None,
            coordinator,
            processor: FeedProcessor::new(16, 16),
            storage: Some(storage),
            storage_sink,
            control_rx,
            feed_rx,
            order_senders: HashMap::new(),
            order_tasks: Vec::new(),
            feeds: Vec::new(),
            feed_tasks: Vec::new(),
            sources: Vec::new(),
            public_feed_index: 0,
            reconcile_inflight: HashSet::new(),
            cancel_inflight: HashSet::new(),
            last_reconcile_attempt: HashMap::new(),
            readiness_timeout_ms: 1_000,
            timer_interval_ms: 100,
            max_feed_age_ms: 60_000,
            shutdown_timeout_ms: 1_000,
            evidence: RuntimeEvidence::default(),
            shutdown_in_progress: false,
            shutdown_storage_error: None,
            shutdown_reconciliation_requested: HashSet::new(),
            shutdown_reconciled_accounts: HashSet::new(),
            operator_service: Some(operator_service),
            operator_rx: Some(operator_rx),
            operator_shutdown_reason: None,
        };
        let runtime_task = tokio::spawn(runtime.run());

        let status =
            send_operator_command_with_secret(&operator_config, SECRET, OperatorCommand::Status)
                .await
                .unwrap();
        assert!(status.ok);
        let status = status.status.unwrap();
        assert!(status.readiness.is_ready());
        assert!(!status.kill_switch_active);
        assert!(status.halted_accounts.is_empty());

        let halt = send_operator_command_with_secret(
            &operator_config,
            SECRET,
            OperatorCommand::HaltSymbol {
                symbol: "BTC-USDT".to_string(),
                reason: "integration test".to_string(),
            },
        )
        .await
        .unwrap();
        assert!(halt.ok);
        let resume = send_operator_command_with_secret(
            &operator_config,
            SECRET,
            OperatorCommand::ResumeSymbol {
                symbol: "BTC-USDT".to_string(),
                reason: "integration test".to_string(),
            },
        )
        .await
        .unwrap();
        assert!(resume.ok);
        let account_kill = send_operator_command_with_secret(
            &operator_config,
            SECRET,
            OperatorCommand::KillAccount {
                account_id: "main".to_string(),
                reason: "integration account isolation".to_string(),
            },
        )
        .await
        .unwrap();
        assert!(account_kill.ok);
        assert!(
            account_kill
                .status
                .unwrap()
                .halted_accounts
                .contains_key("main")
        );
        let blocked_resume = send_operator_command_with_secret(
            &operator_config,
            SECRET,
            OperatorCommand::ResumeSymbol {
                symbol: "BTC-USDT".to_string(),
                reason: "must remain blocked".to_string(),
            },
        )
        .await
        .unwrap();
        assert!(!blocked_resume.ok);
        assert!(
            blocked_resume
                .message
                .contains("account kills cannot be reset live")
        );
        let kill = send_operator_command_with_secret(
            &operator_config,
            SECRET,
            OperatorCommand::KillSwitch {
                reason: "integration global stop".to_string(),
            },
        )
        .await
        .unwrap();
        assert!(kill.ok);
        assert!(kill.status.unwrap().kill_switch_active);
        let shutdown = send_operator_command_with_secret(
            &operator_config,
            SECRET,
            OperatorCommand::Shutdown {
                reason: "integration test complete".to_string(),
            },
        )
        .await
        .unwrap();
        assert!(shutdown.ok);
        assert!(shutdown.status.unwrap().shutdown_in_progress);

        let report = runtime_task.await.unwrap().unwrap();
        drop(control_tx);
        drop(feed_tx);
        let _ = std::fs::remove_file(storage_path);

        assert_eq!(report.stop_reason, LiveStopReason::OperatorCommand);
        assert_eq!(report.operator_commands, 7);
        assert_eq!(report.operator_mutations, 5);
        assert_eq!(report.active_orders_after_shutdown, 0);
        assert!(!report.clean_soak);
        assert!(!socket_path.exists());
    }

    #[tokio::test]
    async fn fatal_runtime_error_with_closed_storage_still_resolves_live_orders() {
        let config = config();
        let now_ms = unix_time_ms();
        let mut coordinator = ready_coordinator(&config, now_ms, true);
        coordinator
            .restore_order(
                "main",
                OrderUpdate {
                    ts_ms: now_ms,
                    order_id: "client-live".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    side: Side::Buy,
                    event: OrderEvent::New,
                    status: OrderStatus::Live,
                    price: 100.0,
                    qty: 1.0,
                    open_qty: 1.0,
                    filled_qty: 0.0,
                    avg_fill_price: 0.0,
                    last_fill_qty: 0.0,
                    last_fill_price: 0.0,
                    last_fill_liquidity: None,
                    reason: "test live order".to_string(),
                },
            )
            .unwrap();
        coordinator.set_order_entry_enabled(false);
        assert_eq!(coordinator.active_order_count(), 1);

        let path = std::env::temp_dir().join(format!(
            "reap-fail-closed-{}-{}.jsonl",
            std::process::id(),
            unix_time_ns()
        ));
        let storage = start_jsonl_storage(StorageConfig {
            path: path.clone(),
            channel_capacity: 1_024,
            flush_every_records: 1,
        })
        .await
        .unwrap();
        let storage_sink = storage.sink();
        storage.shutdown().await.unwrap();
        let (control_tx, control_rx) = mpsc::channel(16);
        let (feed_tx, feed_rx) = mpsc::channel(16);
        let (order_tx, mut order_rx) = mpsc::channel(16);
        let cancel_observed = Arc::new(AtomicBool::new(false));
        let task_cancel_observed = Arc::clone(&cancel_observed);
        let task_events = control_tx.clone();
        let order_task = tokio::spawn(async move {
            while let Some(command) = order_rx.recv().await {
                match command {
                    OrderTaskCommand::Cancel(action) => {
                        assert_eq!(action.client_order_id, "client-live");
                        task_cancel_observed.store(true, Ordering::SeqCst);
                    }
                    OrderTaskCommand::Reconcile(orders) => {
                        assert!(task_cancel_observed.load(Ordering::SeqCst));
                        assert_eq!(orders.len(), 1);
                        task_events
                            .send(RuntimeEvent::RemoteState {
                                account_id: "main".to_string(),
                                remote_orders: vec![RemoteOrder {
                                    exchange_order_id: "exchange-live".to_string(),
                                    client_order_id: "client-live".to_string(),
                                    symbol: "BTC-USDT".to_string(),
                                    side: Side::Buy,
                                    state: PrivateOrderState::Cancelled,
                                    price: 100.0,
                                    qty: 1.0,
                                    cumulative_filled_qty: 0.0,
                                    average_fill_price: 0.0,
                                    update_time_ms: unix_time_ms(),
                                }],
                                remote_fills: Vec::new(),
                                ts_ms: unix_time_ms(),
                            })
                            .await
                            .unwrap();
                    }
                    OrderTaskCommand::Submit(_) => panic!("shutdown dispatched a submit"),
                    OrderTaskCommand::Shutdown => return,
                }
            }
        });
        control_tx
            .send(RuntimeEvent::Fatal("injected runtime failure".to_string()))
            .await
            .unwrap();
        let mut runtime = LiveRuntime {
            mode: LiveMode::Demo,
            run_duration: None,
            coordinator,
            processor: FeedProcessor::new(16, 16),
            storage: None,
            storage_sink,
            control_rx,
            feed_rx,
            order_senders: HashMap::from([("main".to_string(), order_tx)]),
            order_tasks: vec![order_task],
            feeds: Vec::new(),
            feed_tasks: Vec::new(),
            sources: Vec::new(),
            public_feed_index: 0,
            reconcile_inflight: HashSet::new(),
            cancel_inflight: HashSet::new(),
            last_reconcile_attempt: HashMap::new(),
            readiness_timeout_ms: 1_000,
            timer_interval_ms: 100,
            max_feed_age_ms: 60_000,
            shutdown_timeout_ms: 1_000,
            evidence: RuntimeEvidence::default(),
            shutdown_in_progress: false,
            shutdown_storage_error: None,
            shutdown_reconciliation_requested: HashSet::new(),
            shutdown_reconciled_accounts: HashSet::new(),
            operator_service: None,
            operator_rx: None,
            operator_shutdown_reason: None,
        };

        assert!(matches!(
            runtime.dispatch_action(LiveAction::Cancel(CancelAction {
                ts_ms: unix_time_ms(),
                account_id: "main".to_string(),
                symbol: "BTC-USDT".to_string(),
                client_order_id: "client-live".to_string(),
                reason: "injected pre-shutdown storage failure".to_string(),
            })),
            Err(LiveRuntimeError::Storage(StorageError::Closed))
        ));
        assert!(runtime.cancel_inflight.is_empty());

        let error = runtime.run().await.unwrap_err();
        drop(control_tx);
        drop(feed_tx);
        let _ = std::fs::remove_file(path);

        let LiveRuntimeError::LifecycleFailure { primary, secondary } = error else {
            panic!("expected combined runtime and shutdown-storage failure");
        };
        assert!(matches!(
            *primary,
            LiveRuntimeError::GatewayTask(message) if message == "injected runtime failure"
        ));
        assert!(secondary.contains("fail-closed cleanup"));
        assert!(secondary.contains("storage remained unavailable"));
        assert!(cancel_observed.load(Ordering::SeqCst));
    }

    #[test]
    fn private_account_is_ready_only_when_every_channel_is_connected() {
        let plans = vec![
            plan("orders", true, Channel::Orders, None),
            plan("fills", true, Channel::Fills, None),
            plan("account", true, Channel::Account, None),
            plan("positions", true, Channel::Positions, None),
        ];
        let adapter: Arc<dyn VenueAdapter> = Arc::new(OkxAdapter::default());
        let mut source = FeedSourceState::private(adapter, "main".to_string(), &plans);

        assert!(
            source
                .on_status(status("orders", ConnectionStatusKind::Ready))
                .is_empty()
        );
        assert!(
            source
                .on_status(status("fills", ConnectionStatusKind::Ready))
                .is_empty()
        );
        assert!(
            source
                .on_status(status("account", ConnectionStatusKind::Ready))
                .is_empty()
        );
        let ready = source.on_status(status("positions", ConnectionStatusKind::Ready));
        assert_eq!(ready[0].kind, SystemEventKind::PrivateStreamRecovered);

        let stale = source.on_status(status("fills", ConnectionStatusKind::Disconnected));
        assert_eq!(stale[0].kind, SystemEventKind::PrivateStreamStale);
    }

    #[test]
    fn one_redundant_book_disconnect_does_not_mark_feed_stale() {
        let plans = vec![
            plan("book-1", false, Channel::Books, Some("BTC-USDT")),
            plan("book-2", false, Channel::Books, Some("BTC-USDT")),
        ];
        let adapter: Arc<dyn VenueAdapter> = Arc::new(OkxAdapter::default());
        let mut source = FeedSourceState::public(adapter, &plans);
        source.on_status(status("book-1", ConnectionStatusKind::Ready));
        source.on_status(status("book-2", ConnectionStatusKind::Ready));
        assert_eq!(source.public_connectivity_ready(), Some(true));

        assert!(
            source
                .on_status(status("book-1", ConnectionStatusKind::Disconnected))
                .is_empty()
        );
        assert_eq!(source.public_connectivity_ready(), Some(true));
        let stale = source.on_status(status("book-2", ConnectionStatusKind::Disconnected));
        assert_eq!(source.public_connectivity_ready(), Some(false));
        assert_eq!(stale[0].kind, SystemEventKind::FeedStale);
        assert_eq!(stale[0].symbol.as_deref(), Some("BTC-USDT"));
    }

    #[test]
    fn public_plan_replicates_every_required_subscription() {
        let config = config();
        let subscriptions = public_subscriptions(&config);

        assert!(subscriptions.iter().all(|subscription| {
            subscription.connections == config.runtime.public_connections_per_subscription
        }));
        assert_eq!(private_subscriptions(false).len(), 3);
        assert_eq!(private_subscriptions(true).len(), 4);
    }

    #[tokio::test]
    async fn demo_mode_requires_confirmation_and_simulated_environment() {
        let error = run_live(
            config(),
            LiveRunOptions {
                mode: LiveMode::Demo,
                demo_confirmed: false,
                run_duration: None,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(error, LiveRuntimeError::DemoConfirmationRequired));

        let mut production = config();
        production.venue.environment = TradingEnvironment::Production;
        let error = run_live(
            production,
            LiveRunOptions {
                mode: LiveMode::Demo,
                demo_confirmed: true,
                run_duration: None,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            LiveRuntimeError::DemoRequiresSimulatedTrading
        ));
    }
}
