use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reap_core::{
    AccountUpdate, Channel, ConnId, FeedPriority, NormalizedEvent, OrderStatus, Subscription,
    SystemEvent, SystemEventKind, TimeMs, TimerEvent, Venue,
};
use reap_feed::{
    ConnectionStatus, ConnectionStatusKind, FeedOutput, FeedProcessor, ReconnectPolicy, SocketPlan,
    SupervisedFeed, okx_login_bootstrap, partition_subscriptions, spawn_supervised_feed,
};
use reap_order::{
    CancelOutcome, OkxOrderGateway, ReconcileReport, SubmitOutcome, SubmitPreparation,
    reconcile_full_state,
};
use reap_storage::{
    BootstrapRecord, OrderOperation, OrderRequestRecord, RecoveredStorage, SafetyLatchRecord,
    SafetyLatchScope, SafetyLatchSource, StorageConfig, StorageError, StorageRecord,
    StorageRuntime, StorageSink, acquire_storage_lease, recover_jsonl,
    start_jsonl_storage_with_lease,
};
use reap_telemetry::{
    AlertDeliveryFailure, AlertError, AlertEvent, AlertRuntime, AlertSeverity, AlertSink,
    AlertStats, start_webhook_alerts,
};
use reap_venue::okx::{
    HttpTransport, OkxAdapter, OkxRestClient, OkxSigner, ReqwestTransport, RestError,
};
use reap_venue::{PrivateOrderState, PrivateOrderUpdate, RemoteFill, RemoteOrder, VenueAdapter};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::convergence::FillConvergenceGuard;
use crate::{
    AccountBootstrapSnapshot, CancelAction, CoordinatorError, CoordinatorOutput, HostGuardRuntime,
    HostHealthError, HostHealthSnapshot, LiveAction, LiveConfig, LiveConfigError, LiveCoordinator,
    OperatorCommand, OperatorEnvelope, OperatorError, OperatorResponse, OperatorService,
    OperatorStatus, ReadinessSnapshot, ReconcileAction, ReconciliationResult, StartupGate,
    SubmitAction, TradingEnvironment, VerifiedBootstrap, check_host_health, okx_instrument_type,
    start_host_guard, start_operator_service, verify_bootstrap,
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
    pub alerts_delivered: u64,
    pub alert_delivery_failures: u64,
    pub alert_failure_notifications_dropped: u64,
    pub max_alert_queue_depth: usize,
    pub host_preflight: Option<HostHealthSnapshot>,
    pub host_checks: u64,
    pub host_last_snapshot: Option<HostHealthSnapshot>,
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
    #[error(transparent)]
    Alert(#[from] AlertError),
    #[error(transparent)]
    Host(#[from] HostHealthError),
    #[error("alert delivery failed for {code} after {attempts} attempts: {reason}")]
    AlertDelivery {
        code: String,
        attempts: usize,
        reason: String,
    },
    #[error("alert failure monitor closed unexpectedly")]
    AlertMonitorClosed,
    #[error("host health monitor closed unexpectedly")]
    HostGuardClosed,
    #[error("alert shutdown timed out after {0}ms")]
    AlertShutdownTimeout(u64),
    #[error("{0} external alert deliveries failed during runtime teardown")]
    AlertFailuresDuringShutdown(u64),
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
    #[error("durable safety latch sync timed out after {0}ms")]
    SafetyLatchSyncTimeout(u64),
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
    alert_delivery_failures: u64,
) -> bool {
    outcome.stop_reason == LiveStopReason::DurationElapsed
        && outcome.reached_ready
        && outcome.readiness_at_stop.is_ready()
        && evidence.reconciliation_drift_events == 0
        && evidence.operator_mutations == 0
        && dropped_storage_records == 0
        && active_orders_after_shutdown == 0
        && alert_delivery_failures == 0
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
            alerts_delivered: 0,
            alert_delivery_failures: 0,
            alert_failure_notifications_dropped: 0,
            max_alert_queue_depth: 0,
            host_preflight: None,
            host_checks: 0,
            host_last_snapshot: None,
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
    safety_client: OkxRestClient<ReqwestTransport>,
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
        let client = OkxRestClient::new(transport, signer.clone()).with_order_request_expiry(
            Duration::from_millis(config.runtime.order_request_expiry_ms),
        );
        let clock_skew_ms = rest_clock_skew_ms(&client)
            .await
            .map_err(|error| bootstrap_error(&account.id, "exchange clock", error.to_string()))?;
        if clock_skew_ms > config.runtime.max_exchange_clock_skew_ms {
            return Err(bootstrap_error(
                &account.id,
                "exchange clock",
                format!(
                    "clock skew {clock_skew_ms}ms exceeds configured maximum {}ms",
                    config.runtime.max_exchange_clock_skew_ms
                ),
            ));
        }
        let safety_client = client.clone();
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
            safety_client,
        });
    }
    let verified = verify_bootstrap(config, &snapshots)
        .map_err(|error| LiveRuntimeError::BootstrapVerification(error.to_string()))?;
    Ok((verified, seeds, snapshots))
}

async fn rest_clock_skew_ms<T>(client: &OkxRestClient<T>) -> Result<u64, RestError>
where
    T: HttpTransport,
{
    let before_ms = unix_time_ms();
    let exchange_ms = client.server_time_ms().await?;
    let after_ms = unix_time_ms();
    let midpoint_ms = before_ms.saturating_add(after_ms.saturating_sub(before_ms) / 2);
    Ok(midpoint_ms.abs_diff(exchange_ms))
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

fn validate_recovered_safety_latches(
    config: &LiveConfig,
    recovered: &RecoveredStorage,
) -> Result<(), LiveRuntimeError> {
    for account_id in recovered.account_safety_latches.keys() {
        if config.account(account_id).is_none() {
            return Err(LiveRuntimeError::BootstrapVerification(format!(
                "persistent safety latch references unknown account {account_id}; retain the journal and reconcile before changing account identity"
            )));
        }
    }
    for symbol in recovered.symbol_safety_latches.keys() {
        if !config.required_symbols().contains(symbol) {
            return Err(LiveRuntimeError::BootstrapVerification(format!(
                "persistent safety latch references unmanaged symbol {symbol}; retain the journal and reconcile before changing instrument identity"
            )));
        }
    }
    Ok(())
}

fn restored_latch_reason(latch: &SafetyLatchRecord) -> String {
    let source = match latch.source {
        SafetyLatchSource::Operator => "operator",
        SafetyLatchSource::Risk => "risk",
        SafetyLatchSource::LegacySystemEvent => "legacy system-event",
    };
    format!(
        "restored persistent {source} safety latch: {}",
        latch.reason
    )
}

fn restore_safety_latches(
    coordinator: &mut LiveCoordinator,
    recovered: &RecoveredStorage,
) -> Result<Vec<CoordinatorOutput>, CoordinatorError> {
    let mut outputs = Vec::new();
    let now_ms = unix_time_ms();
    if let Some(latch) = &recovered.global_safety_latch {
        coordinator.set_order_entry_enabled(false);
        outputs.push(
            coordinator.process_event(NormalizedEvent::System(SystemEvent {
                ts_ms: now_ms,
                kind: SystemEventKind::KillSwitchActivated,
                venue: None,
                account_id: None,
                symbol: None,
                reason: restored_latch_reason(latch),
            })),
        );
    }
    for (account_id, latch) in &recovered.account_safety_latches {
        outputs.push(coordinator.halt_account(now_ms, account_id, restored_latch_reason(latch))?);
    }
    for (symbol, latch) in &recovered.symbol_safety_latches {
        outputs.push(
            coordinator.process_event(NormalizedEvent::System(SystemEvent {
                ts_ms: now_ms,
                kind: SystemEventKind::SymbolHalted,
                venue: None,
                account_id: None,
                symbol: Some(symbol.clone()),
                reason: restored_latch_reason(latch),
            })),
        );
    }
    Ok(outputs)
}

fn alert_for_storage_record(record: &StorageRecord) -> Option<AlertEvent> {
    let StorageRecord::System(event) = record else {
        return None;
    };
    let (severity, component, code) = match event.kind {
        SystemEventKind::FeedStale => (AlertSeverity::Warning, "market_data", "feed_stale"),
        SystemEventKind::FeedGap => (AlertSeverity::Warning, "market_data", "feed_gap"),
        SystemEventKind::BookRecoveryStarted => (
            AlertSeverity::Warning,
            "market_data",
            "book_recovery_started",
        ),
        SystemEventKind::PrivateStreamStale => (
            AlertSeverity::Warning,
            "account_data",
            "private_stream_stale",
        ),
        SystemEventKind::BookRecoveryFailed => (
            AlertSeverity::Critical,
            "market_data",
            "book_recovery_failed",
        ),
        SystemEventKind::ReconcileDrift => {
            (AlertSeverity::Critical, "order_state", "reconcile_drift")
        }
        SystemEventKind::RiskBreach => (AlertSeverity::Critical, "risk", "risk_breach"),
        SystemEventKind::KillSwitchActivated => {
            (AlertSeverity::Critical, "risk", "kill_switch_activated")
        }
        SystemEventKind::AccountHalted => (AlertSeverity::Critical, "risk", "account_halted"),
        SystemEventKind::SymbolHalted => (AlertSeverity::Critical, "risk", "symbol_halted"),
        SystemEventKind::FeedHeartbeat
        | SystemEventKind::FeedRecovered
        | SystemEventKind::PrivateStreamHeartbeat
        | SystemEventKind::PrivateStreamRecovered
        | SystemEventKind::KillSwitchReset
        | SystemEventKind::SymbolResumed => return None,
    };
    let mut alert = AlertEvent::new(severity, component, code, event.reason.clone());
    alert.ts_ms = event.ts_ms;
    if let Some(venue) = &event.venue {
        alert = alert.with_attribute("venue", format!("{venue:?}").to_ascii_lowercase());
    }
    if let Some(account_id) = &event.account_id {
        alert = alert.with_attribute("account_id", account_id);
    }
    if let Some(symbol) = &event.symbol {
        alert = alert.with_attribute("symbol", symbol);
    }
    Some(alert)
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
    safety_senders: HashMap<String, mpsc::Sender<SafetyTaskCommand>>,
    safety_tasks: Vec<JoinHandle<()>>,
    feeds: Vec<SupervisedFeed>,
    feed_tasks: Vec<JoinHandle<()>>,
    sources: Vec<FeedSourceState>,
    public_feed_index: usize,
    reconcile_inflight: HashSet<String>,
    cancel_inflight: HashSet<(String, String)>,
    last_reconcile_attempt: HashMap<String, Instant>,
    fill_convergence: FillConvergenceGuard,
    readiness_timeout_ms: u64,
    timer_interval_ms: u64,
    max_feed_age_ms: u64,
    shutdown_timeout_ms: u64,
    safety_latch_sync_timeout_ms: u64,
    evidence: RuntimeEvidence,
    shutdown_in_progress: bool,
    shutdown_storage_error: Option<String>,
    preserve_deadman_on_shutdown: bool,
    shutdown_reconciliation_requested: HashSet<String>,
    shutdown_reconciled_accounts: HashSet<String>,
    operator_service: Option<OperatorService>,
    operator_rx: Option<mpsc::Receiver<OperatorEnvelope>>,
    operator_shutdown_reason: Option<String>,
    alert_runtime: Option<AlertRuntime>,
    alert_sink: Option<AlertSink>,
    alert_failures: Option<mpsc::Receiver<AlertDeliveryFailure>>,
    alert_shutdown_timeout_ms: u64,
    alert_delivery_failure_is_fatal: bool,
    observed_alert_delivery_failures: u64,
    alert_stats: AlertStats,
    host_guard: Option<HostGuardRuntime>,
    host_failures: Option<mpsc::Receiver<HostHealthError>>,
    host_preflight: Option<HostHealthSnapshot>,
    host_checks: u64,
    host_last_snapshot: Option<HostHealthSnapshot>,
}

impl LiveRuntime {
    async fn build(
        config: LiveConfig,
        mode: LiveMode,
        run_duration: Option<Duration>,
    ) -> Result<Self, LiveRuntimeError> {
        let storage_lease = acquire_storage_lease(&config.storage.path)?;
        let journal_path = storage_lease.journal_path().to_path_buf();
        let host_preflight = if config.host_guard.enabled {
            Some(check_host_health(&config.host_guard, &journal_path)?)
        } else {
            None
        };
        let mut alert_runtime = config
            .alerts
            .webhook_from_env()?
            .map(start_webhook_alerts)
            .transpose()?;
        let alert_sink = alert_runtime.as_ref().map(AlertRuntime::sink);
        let alert_failures = alert_runtime.as_mut().map(AlertRuntime::take_failures);
        let operator_config = config.operator.clone();
        let operator_secret = operator_config.secret_from_env()?;
        let config_fingerprint = config.fingerprint()?;
        let fill_convergence = FillConvergenceGuard::new(&config);
        let recovered = recover_jsonl(storage_lease.journal_path())?;
        validate_recovered_safety_latches(&config, &recovered)?;
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
        // Apply recovered halt state before replaying anything that can produce an intent.
        // Reapplying it after reconciliation below generates cancels for restored live orders.
        let _ = restore_safety_latches(&mut coordinator, &recovered)?;
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
            let account_snapshot = snapshot.scoped_account_update(&account.id);
            initial_outputs.push(
                coordinator
                    .apply_authoritative_account_snapshot(&account.id, account_snapshot.clone())?,
            );
            let state = coordinator
                .private_state(&account.id)
                .ok_or_else(|| CoordinatorError::UnknownAccount(account.id.clone()))?;
            let report = reconcile_full_state(
                state,
                &snapshot.open_orders,
                &snapshot.recent_fills,
                &account_snapshot,
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
        initial_outputs.extend(restore_safety_latches(&mut coordinator, &recovered)?);
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
        let storage = start_jsonl_storage_with_lease(
            StorageConfig {
                path: config.storage.path.clone(),
                channel_capacity: config.storage.channel_capacity,
                flush_every_records: config.storage.flush_every_records,
            },
            storage_lease,
        )
        .await?;
        let storage_sink = storage.sink();
        coordinator.mark_storage_ready(true, "storage file opened");

        let (control_tx, control_rx) = mpsc::channel(config.runtime.event_channel_capacity);
        let (feed_tx, feed_rx) = mpsc::channel(config.runtime.event_channel_capacity);
        let mut host_guard = config
            .host_guard
            .enabled
            .then(|| start_host_guard(config.host_guard.clone(), journal_path));
        let host_failures = host_guard.as_mut().map(HostGuardRuntime::take_failures);
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
        let mut safety_senders = HashMap::new();
        let mut safety_tasks = Vec::new();
        for seed in seeds {
            let AccountSeed {
                account_id,
                signer,
                gateway,
                safety_client,
            } = seed;
            let deadman_timeout_secs =
                (mode == LiveMode::Demo).then_some(config.runtime.cancel_all_after_timeout_secs);
            if let Some(timeout_secs) = deadman_timeout_secs {
                safety_client
                    .cancel_all_after(timeout_secs)
                    .await
                    .map_err(|error| LiveRuntimeError::GatewaySetup {
                        account_id: account_id.clone(),
                        message: format!("failed to arm Cancel All After: {error}"),
                    })?;
            }
            let (safety_tx, safety_rx) = mpsc::channel(8);
            safety_senders.insert(account_id.clone(), safety_tx);
            safety_tasks.push(tokio::spawn(run_account_safety_task(
                account_id.clone(),
                safety_client,
                safety_rx,
                control_tx.clone(),
                deadman_timeout_secs,
                config.runtime.cancel_all_after_heartbeat_ms,
                config.runtime.exchange_clock_check_interval_ms,
                config.runtime.max_exchange_clock_skew_ms,
            )));
            let private_adapter: Arc<dyn VenueAdapter> = Arc::new(
                OkxAdapter::new(&config.venue.public_ws_url, &config.venue.private_ws_url)
                    .with_account_id(&account_id),
            );
            let mut private_feed = spawn_supervised_feed(
                Arc::clone(&private_adapter),
                private_plans.clone(),
                okx_login_bootstrap(signer),
                config.runtime.feed_channel_capacity,
                ReconnectPolicy::default(),
            );
            let source_id = sources.len();
            sources.push(FeedSourceState::private(
                private_adapter,
                account_id.clone(),
                &private_plans,
            ));
            spawn_feed_forwarders(source_id, &mut private_feed, &feed_tx, &mut feed_tasks);
            feeds.push(private_feed);

            let (order_tx, order_rx) = mpsc::channel(config.runtime.order_channel_capacity);
            order_senders.insert(account_id.clone(), order_tx);
            order_tasks.push(tokio::spawn(run_order_task(
                account_id,
                gateway,
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
            safety_senders,
            safety_tasks,
            feeds,
            feed_tasks,
            sources,
            public_feed_index,
            reconcile_inflight: HashSet::new(),
            cancel_inflight: HashSet::new(),
            last_reconcile_attempt: HashMap::new(),
            fill_convergence: FillConvergenceGuard::default(),
            readiness_timeout_ms: config.runtime.readiness_timeout_ms,
            timer_interval_ms: config.runtime.timer_interval_ms,
            max_feed_age_ms: config.risk.max_feed_age_ms,
            shutdown_timeout_ms: config.runtime.shutdown_timeout_ms,
            safety_latch_sync_timeout_ms: config.runtime.safety_latch_sync_timeout_ms,
            evidence: RuntimeEvidence::default(),
            shutdown_in_progress: false,
            shutdown_storage_error: None,
            preserve_deadman_on_shutdown: false,
            shutdown_reconciliation_requested: HashSet::new(),
            shutdown_reconciled_accounts: HashSet::new(),
            operator_service: None,
            operator_rx: None,
            operator_shutdown_reason: None,
            alert_runtime,
            alert_sink,
            alert_failures,
            alert_shutdown_timeout_ms: config.alerts.shutdown_timeout_ms,
            alert_delivery_failure_is_fatal: config.alerts.delivery_failure_is_fatal,
            observed_alert_delivery_failures: 0,
            alert_stats: AlertStats::default(),
            host_guard,
            host_failures,
            host_checks: u64::from(host_preflight.is_some()),
            host_last_snapshot: host_preflight.clone(),
            host_preflight,
        };
        for output in initial_outputs {
            if let Err(primary) = runtime.commit_output(output).await {
                let context = format!("runtime initialization failure: {primary}");
                return Err(runtime.close_after_error(primary, &context).await);
            }
        }
        runtime.fill_convergence = fill_convergence;
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
        let runtime_alert_result = match &loop_result {
            Ok(_) => Ok(()),
            Err(error) => self.emit_runtime_failure_alert(error),
        };
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
                if let Err(error) = runtime_alert_result {
                    additional.push(("runtime alert enqueue", error));
                }
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
            self.alert_stats.failed,
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
            alerts_delivered: self.alert_stats.delivered,
            alert_delivery_failures: self.alert_stats.failed,
            alert_failure_notifications_dropped: self.alert_stats.failure_notifications_dropped,
            max_alert_queue_depth: self.alert_stats.max_queue_depth,
            host_preflight: self.host_preflight.clone(),
            host_checks: self.host_checks,
            host_last_snapshot: self.host_last_snapshot.clone(),
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
                    self.drain_queued_events().await?;
                    let elapsed_ms = elapsed_ms(&started);
                    let outcome = readiness_tracker.finish(
                        LiveStopReason::OperatorSignal,
                        elapsed_ms,
                        self.coordinator.readiness(),
                    );
                    return Ok(outcome);
                }
                _ = &mut duration_elapsed => {
                    self.drain_queued_events().await?;
                    let elapsed_ms = elapsed_ms(&started);
                    let outcome = readiness_tracker.finish(
                        LiveStopReason::DurationElapsed,
                        elapsed_ms,
                        self.coordinator.readiness(),
                    );
                    return Ok(outcome);
                }
                failure = receive_alert_failure(&mut self.alert_failures) => {
                    let failure = failure.ok_or(LiveRuntimeError::AlertMonitorClosed)?;
                    self.observed_alert_delivery_failures = self
                        .observed_alert_delivery_failures
                        .saturating_add(1);
                    tracing::error!(
                        event_id = %failure.event_id,
                        code = %failure.code,
                        attempts = failure.attempts,
                        reason = %failure.reason,
                        "external alert delivery failed"
                    );
                    if self.alert_delivery_failure_is_fatal {
                        return Err(LiveRuntimeError::AlertDelivery {
                            code: failure.code,
                            attempts: failure.attempts,
                            reason: failure.reason,
                        });
                    }
                }
                failure = receive_host_failure(&mut self.host_failures) => {
                    let failure = failure.ok_or(LiveRuntimeError::HostGuardClosed)?;
                    return Err(failure.into());
                }
                event = self.control_rx.recv() => {
                    let event = event.ok_or(LiveRuntimeError::EventChannelClosed)?;
                    self.handle_runtime_event(event).await?;
                }
                operator = receive_operator(&mut self.operator_rx) => {
                    let operator = operator.ok_or(LiveRuntimeError::OperatorChannelClosed)?;
                    self.handle_operator_envelope(operator).await?;
                }
                _ = timer.tick() => {
                    let now_ms = unix_time_ms();
                    for event in self.processor.mark_stale(
                        now_ms,
                        self.coordinator_risk_max_feed_age(),
                    ) {
                        let output = self.coordinator.process_event(NormalizedEvent::System(event));
                        self.commit_output(output).await?;
                    }
                    for breach in self.fill_convergence.expire(now_ms) {
                        let output = self.coordinator.reconciliation_fault(
                            &breach.account_id,
                            now_ms,
                            breach.symbol,
                            breach.reason,
                        )?;
                        self.commit_output(output).await?;
                    }
                    let output = self.coordinator.process_event(NormalizedEvent::Timer(TimerEvent {
                        ts_ms: now_ms,
                        name: "live_tick".to_string(),
                    }));
                    self.commit_output(output).await?;
                    self.retry_reconciliation(now_ms)?;
                }
                event = self.feed_rx.recv() => {
                    let event = event.ok_or(LiveRuntimeError::EventChannelClosed)?;
                    self.handle_runtime_event(event).await?;
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
            self.drain_shutdown_events().await?;
            let readiness = self.coordinator.readiness();
            if self.coordinator.active_order_count() == 0
                && readiness.missing_reconciliation.is_empty()
                && self.reconcile_inflight.is_empty()
                && self.shutdown_reconciled_accounts.len() == self.order_senders.len()
            {
                if !self.preserve_deadman_on_shutdown {
                    self.disable_deadman_all().await?;
                }
                return Ok(());
            }
            tokio::select! {
                biased;
                event = self.control_rx.recv(), if !self.control_rx.is_closed() => {
                    if let Some(event) = event {
                        self.handle_runtime_event(event).await?;
                    }
                }
                event = self.feed_rx.recv(), if !self.feed_rx.is_closed() => {
                    if let Some(event) = event {
                        self.handle_runtime_event(event).await?;
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

    async fn drain_shutdown_events(&mut self) -> Result<(), LiveRuntimeError> {
        let pending_control = self.control_rx.len();
        for _ in 0..pending_control {
            match self.control_rx.try_recv() {
                Ok(event) => self.handle_runtime_event(event).await?,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        let pending_feed = self.feed_rx.len();
        for _ in 0..pending_feed {
            match self.feed_rx.try_recv() {
                Ok(event) => self.handle_runtime_event(event).await?,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        Ok(())
    }

    async fn drain_queued_events(&mut self) -> Result<(), LiveRuntimeError> {
        let pending_control = self.control_rx.len();
        for _ in 0..pending_control {
            match self.control_rx.try_recv() {
                Ok(event) => self.handle_runtime_event(event).await?,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return Err(LiveRuntimeError::EventChannelClosed);
                }
            }
        }
        let pending_feed = self.feed_rx.len();
        for _ in 0..pending_feed {
            match self.feed_rx.try_recv() {
                Ok(event) => self.handle_runtime_event(event).await?,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return Err(LiveRuntimeError::EventChannelClosed);
                }
            }
        }
        Ok(())
    }

    async fn handle_operator_envelope(
        &mut self,
        envelope: OperatorEnvelope,
    ) -> Result<(), LiveRuntimeError> {
        let OperatorEnvelope {
            request_id,
            command,
            response,
        } = envelope;
        self.evidence.operator_commands = self.evidence.operator_commands.saturating_add(1);
        let result = self.execute_operator_command(&request_id, command).await;
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

    async fn execute_operator_command(
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
                    SafetyLatchScope::Global,
                    true,
                    reason,
                )
                .await?;
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
                let now_ms = unix_time_ms();
                let reason = format!("authenticated operator request {request_id}: {reason}");
                let mut output =
                    self.coordinator
                        .halt_account(now_ms, &account_id, reason.clone())?;
                output.records.insert(
                    0,
                    StorageRecord::SafetyLatch(SafetyLatchRecord {
                        ts_ms: now_ms,
                        scope: SafetyLatchScope::Account {
                            account_id: account_id.clone(),
                        },
                        active: true,
                        source: SafetyLatchSource::Operator,
                        request_id: Some(request_id.to_string()),
                        reason,
                    }),
                );
                self.commit_output(output).await?;
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
                    Some(symbol.clone()),
                    SafetyLatchScope::Symbol { symbol },
                    true,
                    reason,
                )
                .await?;
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
                    Some(symbol.clone()),
                    SafetyLatchScope::Symbol { symbol },
                    false,
                    reason,
                )
                .await?;
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

    async fn commit_operator_system_event(
        &mut self,
        request_id: &str,
        kind: SystemEventKind,
        symbol: Option<String>,
        scope: SafetyLatchScope,
        active: bool,
        reason: String,
    ) -> Result<(), LiveRuntimeError> {
        let now_ms = unix_time_ms();
        let reason = format!("authenticated operator request {request_id}: {reason}");
        let mut output = self
            .coordinator
            .process_event(NormalizedEvent::System(SystemEvent {
                ts_ms: now_ms,
                kind,
                venue: None,
                account_id: None,
                symbol,
                reason: reason.clone(),
            }));
        output.records.insert(
            0,
            StorageRecord::SafetyLatch(SafetyLatchRecord {
                ts_ms: now_ms,
                scope,
                active,
                source: SafetyLatchSource::Operator,
                request_id: Some(request_id.to_string()),
                reason,
            }),
        );
        self.commit_output(output).await
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

    async fn handle_runtime_event(&mut self, event: RuntimeEvent) -> Result<(), LiveRuntimeError> {
        match event {
            RuntimeEvent::Raw {
                source_id,
                envelope,
            } => {
                let (account_id, adapter, private_source) = {
                    let source = self.sources.get(source_id).ok_or_else(|| {
                        LiveRuntimeError::FeedAdapter("unknown feed source".to_string())
                    })?;
                    (
                        source.account_id.clone(),
                        Arc::clone(&source.adapter),
                        source.account_id.is_some(),
                    )
                };
                self.record_storage(StorageRecord::Raw {
                    account_id,
                    envelope: envelope.clone(),
                })?;
                let parsed = adapter
                    .parse(&envelope)
                    .map_err(|error| LiveRuntimeError::FeedAdapter(error.to_string()))?;
                let private_state_frame = private_source
                    && matches!(envelope.channel, Channel::Account | Channel::Positions)
                    && (!parsed.is_empty()
                        || adapter
                            .is_data_frame(&envelope)
                            .map_err(|error| LiveRuntimeError::FeedAdapter(error.to_string()))?);
                for event in parsed {
                    for output in self.processor.process(event) {
                        self.observe_feed_output(&output);
                        let private_account_id = match &output {
                            FeedOutput::PrivateAccount {
                                account_id: Some(account_id),
                                ..
                            } => Some(account_id.clone()),
                            _ => None,
                        };
                        let private_order_event = matches!(
                            &output,
                            FeedOutput::PrivateOrder {
                                account_id: Some(_),
                                ..
                            } | FeedOutput::PrivateFill {
                                account_id: Some(_),
                                ..
                            }
                        );
                        let output = self.coordinator.process_feed(output)?;
                        if let Some(account_id) = private_account_id {
                            self.observe_account_convergence(
                                &account_id,
                                &output,
                                envelope.recv_ts_ns / 1_000_000,
                            );
                        }
                        if private_order_event {
                            self.observe_fill_convergence(&output, envelope.recv_ts_ns / 1_000_000);
                        }
                        self.commit_output(output).await?;
                    }
                }
                if private_state_frame {
                    let events = self
                        .sources
                        .get_mut(source_id)
                        .ok_or_else(|| {
                            LiveRuntimeError::FeedAdapter("unknown feed source".to_string())
                        })?
                        .on_private_data(envelope.channel.clone(), envelope.recv_ts_ns / 1_000_000);
                    self.handle_feed_source_events(events).await?;
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
                self.handle_feed_source_events(events).await?;
            }
            RuntimeEvent::SubmitComplete {
                account_id,
                outcome,
                ts_ms,
            } => {
                let output = self
                    .coordinator
                    .on_submit_outcome(&account_id, outcome, ts_ms)?;
                self.commit_output(output).await?;
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
                self.commit_output(output).await?;
            }
            RuntimeEvent::CancelComplete {
                account_id,
                outcome,
                ts_ms,
            } => {
                let output = self
                    .coordinator
                    .on_cancel_outcome(&account_id, outcome, ts_ms)?;
                self.commit_output(output).await?;
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
                self.commit_output(output).await?;
            }
            RuntimeEvent::RemoteState {
                account_id,
                remote_orders,
                remote_fills,
                remote_account,
                ts_ms,
            } => {
                self.reconcile_inflight.remove(&account_id);
                self.cancel_inflight
                    .retain(|(cancel_account, _)| cancel_account != &account_id);
                self.apply_remote_recovery(&account_id, &remote_orders, &remote_fills)
                    .await?;
                let report = {
                    let state = self
                        .coordinator
                        .private_state(&account_id)
                        .ok_or_else(|| CoordinatorError::UnknownAccount(account_id.clone()))?;
                    reconcile_full_state(state, &remote_orders, &remote_fills, &remote_account)
                };
                let remote_account_ts_ms = remote_account.ts_ms;
                let account_output = self
                    .coordinator
                    .apply_authoritative_account_snapshot(&account_id, remote_account)?;
                self.fill_convergence
                    .observe_authoritative(&account_id, remote_account_ts_ms);
                self.commit_output(account_output).await?;
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
                    "REST orders, fills, balances, positions, and canonical private state agree"
                        .to_string()
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
                self.commit_output(output).await?;
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
                self.commit_output(output).await?;
            }
            RuntimeEvent::Fatal(message) => return Err(LiveRuntimeError::GatewayTask(message)),
        }
        Ok(())
    }

    async fn handle_feed_source_events(
        &mut self,
        events: Vec<SystemEvent>,
    ) -> Result<(), LiveRuntimeError> {
        for event in events {
            if event.kind == SystemEventKind::PrivateStreamRecovered {
                let account_id = event.account_id.as_deref().ok_or_else(|| {
                    LiveRuntimeError::FeedAdapter(
                        "private recovery event has no account identity".to_string(),
                    )
                })?;
                let output = self.coordinator.require_reconciliation(
                    account_id,
                    event.ts_ms,
                    "verify REST state after private websocket state recovery",
                )?;
                self.commit_output(output).await?;
            }
            let output = self
                .coordinator
                .process_event(NormalizedEvent::System(event));
            self.commit_output(output).await?;
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

    fn observe_account_convergence(
        &mut self,
        account_id: &str,
        output: &CoordinatorOutput,
        observed_ms: u64,
    ) {
        for record in &output.records {
            if let StorageRecord::Normalized(NormalizedEvent::Account(update)) = record {
                self.fill_convergence
                    .observe_account(account_id, update, observed_ms);
            }
        }
    }

    async fn apply_remote_recovery(
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
                self.observe_fill_convergence(&output, unix_time_ms());
                self.commit_output(output).await?;
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
                self.observe_fill_convergence(&output, unix_time_ms());
                self.commit_output(output).await?;
            }
        }
        Ok(())
    }

    async fn commit_output(&mut self, output: CoordinatorOutput) -> Result<(), LiveRuntimeError> {
        let alerts = if self.alert_sink.is_some() {
            output
                .records
                .iter()
                .filter_map(alert_for_storage_record)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        for record in output.records {
            if matches!(record, StorageRecord::SafetyLatch(_)) {
                self.record_durable_storage(record).await?;
            } else {
                self.record_storage(record)?;
            }
        }
        for alert in alerts {
            self.emit_alert(alert)?;
        }
        for action in output.actions {
            self.dispatch_action(action)?;
        }
        Ok(())
    }

    fn observe_fill_convergence(&mut self, output: &CoordinatorOutput, observed_ms: u64) {
        for record in &output.records {
            if let StorageRecord::Order {
                account_id: Some(account_id),
                update,
            } = record
            {
                self.fill_convergence
                    .observe_fill(account_id, update, observed_ms);
            }
        }
    }

    fn emit_alert(&self, alert: AlertEvent) -> Result<(), LiveRuntimeError> {
        if let Some(sink) = &self.alert_sink {
            sink.try_emit(alert)?;
        }
        Ok(())
    }

    fn emit_runtime_failure_alert(&self, error: &LiveRuntimeError) -> Result<(), LiveRuntimeError> {
        if matches!(
            error,
            LiveRuntimeError::Alert(_)
                | LiveRuntimeError::AlertDelivery { .. }
                | LiveRuntimeError::AlertMonitorClosed
                | LiveRuntimeError::AlertShutdownTimeout(_)
                | LiveRuntimeError::AlertFailuresDuringShutdown(_)
        ) {
            return Ok(());
        }
        let mut alert = AlertEvent::new(
            AlertSeverity::Critical,
            "runtime",
            "runtime_failure",
            error.to_string(),
        );
        if let LiveRuntimeError::Host(host_error) = error {
            alert.code = host_error.code().to_string();
            if let Some(snapshot) = host_error.snapshot() {
                alert = alert
                    .with_attribute(
                        "disk_available_bytes",
                        snapshot.disk_available_bytes.to_string(),
                    )
                    .with_attribute(
                        "memory_available_bytes",
                        snapshot.memory_available_bytes.to_string(),
                    )
                    .with_attribute(
                        "clock_synchronized",
                        snapshot.clock_synchronized.to_string(),
                    );
            }
        }
        self.emit_alert(alert)
    }

    async fn commit_shutdown_output(
        &mut self,
        output: CoordinatorOutput,
    ) -> Result<(), LiveRuntimeError> {
        for record in output.records {
            if matches!(record, StorageRecord::SafetyLatch(_)) {
                self.record_durable_storage(record).await?;
            } else {
                self.record_storage(record)?;
            }
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

    async fn record_durable_storage(
        &mut self,
        record: StorageRecord,
    ) -> Result<(), LiveRuntimeError> {
        self.evidence.observe_record(&record);
        self.evidence.max_storage_queue_depth = self
            .evidence
            .max_storage_queue_depth
            .max(self.storage_sink.queue_depth().saturating_add(1));
        let result = tokio::time::timeout(
            Duration::from_millis(self.safety_latch_sync_timeout_ms),
            self.storage_sink.record_durable(record),
        )
        .await;
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                self.preserve_deadman_on_shutdown = true;
                Err(error.into())
            }
            Err(_) => {
                self.preserve_deadman_on_shutdown = true;
                Err(LiveRuntimeError::SafetyLatchSyncTimeout(
                    self.safety_latch_sync_timeout_ms,
                ))
            }
        }
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

    async fn disable_deadman_all(&mut self) -> Result<(), LiveRuntimeError> {
        let mut acknowledgements = Vec::new();
        for (account_id, sender) in &self.safety_senders {
            let (result_tx, result_rx) = oneshot::channel();
            sender
                .send(SafetyTaskCommand::DisableDeadMan { result: result_tx })
                .await
                .map_err(|_| {
                    LiveRuntimeError::GatewayTask(format!(
                        "account {account_id} safety task is unavailable"
                    ))
                })?;
            acknowledgements.push((account_id.clone(), result_rx));
        }
        for (account_id, result) in acknowledgements {
            let result = result.await.map_err(|_| {
                LiveRuntimeError::GatewayTask(format!(
                    "account {account_id} safety task dropped disable acknowledgement"
                ))
            })?;
            result.map_err(|message| {
                LiveRuntimeError::GatewayTask(format!(
                    "account {account_id} failed to disable Cancel All After: {message}"
                ))
            })?;
        }
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), LiveRuntimeError> {
        let mut errors = Vec::new();
        if let Some(host_guard) = self.host_guard.take() {
            match host_guard.shutdown().await {
                Ok(stats) => {
                    self.host_checks = self.host_checks.saturating_add(stats.checks);
                    if let Some(snapshot) = stats.last_snapshot {
                        self.host_last_snapshot = Some(snapshot);
                    }
                }
                Err(error) => errors.push(("host guard", LiveRuntimeError::Join(error))),
            }
        }
        if let Some(failures) = &mut self.host_failures
            && let Ok(error) = failures.try_recv()
        {
            errors.push(("host health", error.into()));
        }
        self.host_failures.take();
        if let Some(service) = self.operator_service.take()
            && let Err(error) = service.shutdown().await
        {
            errors.push(("operator service", LiveRuntimeError::Operator(error)));
        }
        self.operator_rx.take();
        for sender in self.order_senders.values() {
            let _ = sender.try_send(OrderTaskCommand::Shutdown);
        }
        self.order_senders.clear();
        for sender in self.safety_senders.values() {
            let _ = sender.try_send(SafetyTaskCommand::Shutdown);
        }
        self.safety_senders.clear();
        self.control_rx.close();
        self.feed_rx.close();
        for feed in self.feeds.drain(..) {
            feed.shutdown().await;
        }
        for task in self.feed_tasks.drain(..) {
            if let Err(error) = task.await {
                errors.push(("feed task", LiveRuntimeError::Join(error)));
            }
        }
        for task in self.order_tasks.drain(..) {
            if let Err(error) = task.await {
                errors.push(("order task", LiveRuntimeError::Join(error)));
            }
        }
        for task in self.safety_tasks.drain(..) {
            if let Err(error) = task.await {
                errors.push(("safety task", LiveRuntimeError::Join(error)));
            }
        }
        if let Some(storage) = self.storage.as_mut()
            && let Err(error) = storage.stop_writer().await
        {
            errors.push(("storage", LiveRuntimeError::Storage(error)));
        }
        if !errors.is_empty() {
            let message = errors
                .iter()
                .map(|(stage, error)| format!("{stage}: {error}"))
                .collect::<Vec<_>>()
                .join("; ");
            if let Err(error) = self.emit_alert(AlertEvent::new(
                AlertSeverity::Critical,
                "runtime",
                "teardown_failure",
                message,
            )) {
                errors.push(("teardown alert enqueue", error));
            }
        }
        self.alert_sink.take();
        if let Some(alert_runtime) = self.alert_runtime.take() {
            match tokio::time::timeout(
                Duration::from_millis(self.alert_shutdown_timeout_ms),
                alert_runtime.shutdown(),
            )
            .await
            {
                Ok(Ok(stats)) => {
                    self.alert_stats = stats;
                    let unobserved_failures = stats
                        .failed
                        .saturating_sub(self.observed_alert_delivery_failures);
                    if self.alert_delivery_failure_is_fatal && unobserved_failures > 0 {
                        errors.push((
                            "alert delivery",
                            LiveRuntimeError::AlertFailuresDuringShutdown(unobserved_failures),
                        ));
                    }
                }
                Ok(Err(error)) => errors.push(("alert service", error.into())),
                Err(_) => errors.push((
                    "alert service",
                    LiveRuntimeError::AlertShutdownTimeout(self.alert_shutdown_timeout_ms),
                )),
            }
        }
        self.alert_failures.take();
        if errors.is_empty() {
            return Ok(());
        }
        let (_, first) = errors.remove(0);
        let additional = errors;
        Err(combine_lifecycle_errors(first, additional))
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
        remote_account: AccountUpdate,
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

enum SafetyTaskCommand {
    DisableDeadMan {
        result: oneshot::Sender<Result<(), String>>,
    },
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
                        let remote_account = match gateway.fetch_remote_account_state().await {
                            Ok(account) => account,
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
                                continue;
                            }
                        };
                        if events
                            .send(RuntimeEvent::RemoteState {
                                account_id: account_id.clone(),
                                remote_orders,
                                remote_fills,
                                remote_account,
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

#[allow(clippy::too_many_arguments)]
async fn run_account_safety_task<T>(
    account_id: String,
    client: OkxRestClient<T>,
    mut commands: mpsc::Receiver<SafetyTaskCommand>,
    events: mpsc::Sender<RuntimeEvent>,
    mut deadman_timeout_secs: Option<u64>,
    deadman_heartbeat_ms: u64,
    clock_check_interval_ms: u64,
    max_clock_skew_ms: u64,
) where
    T: HttpTransport + 'static,
{
    let mut deadman = tokio::time::interval(Duration::from_millis(deadman_heartbeat_ms));
    deadman.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    deadman.tick().await;
    let mut clock = tokio::time::interval(Duration::from_millis(clock_check_interval_ms));
    clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    clock.tick().await;

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { return; };
                match command {
                    SafetyTaskCommand::DisableDeadMan { result } => {
                        let disabled = match deadman_timeout_secs {
                            Some(_) => client.cancel_all_after(0).await.map_err(|error| error.to_string()),
                            None => Ok(()),
                        };
                        if disabled.is_ok() {
                            deadman_timeout_secs = None;
                        }
                        let _ = result.send(disabled);
                    }
                    SafetyTaskCommand::Shutdown => return,
                }
            }
            _ = deadman.tick(), if deadman_timeout_secs.is_some() => {
                let timeout_secs = deadman_timeout_secs.expect("guarded dead-man timeout");
                if let Err(error) = client.cancel_all_after(timeout_secs).await {
                    let _ = events.send(RuntimeEvent::Fatal(format!(
                        "account {account_id} Cancel All After heartbeat failed: {error}"
                    ))).await;
                    return;
                }
            }
            _ = clock.tick() => {
                match rest_clock_skew_ms(&client).await {
                    Ok(skew_ms) if skew_ms <= max_clock_skew_ms => {}
                    Ok(skew_ms) => {
                        let _ = events.send(RuntimeEvent::Fatal(format!(
                            "account {account_id} exchange clock skew {skew_ms}ms exceeds {max_clock_skew_ms}ms"
                        ))).await;
                        return;
                    }
                    Err(error) => {
                        let _ = events.send(RuntimeEvent::Fatal(format!(
                            "account {account_id} exchange clock check failed: {error}"
                        ))).await;
                        return;
                    }
                }
            }
        }
    }
}

struct FeedSourceState {
    adapter: Arc<dyn VenueAdapter>,
    account_id: Option<String>,
    expected_connections: HashSet<ConnId>,
    ready_connections: HashSet<ConnId>,
    public_subscriptions: Vec<PublicSubscriptionRoute>,
    required_private_data_channels: HashSet<Channel>,
    private_data_round: HashSet<Channel>,
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
            required_private_data_channels: HashSet::new(),
            private_data_round: HashSet::new(),
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
            required_private_data_channels: HashSet::from([Channel::Account, Channel::Positions]),
            private_data_round: HashSet::new(),
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
                self.private_ready = false;
                self.private_data_round.clear();
            }
        }
        if let Some(account_id) = &self.account_id {
            if status.kind == ConnectionStatusKind::Disconnected {
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
            if let Some(event) = self.private_health_event(
                status.ts_ms,
                status.venue,
                "all private transports and state-data channels are healthy",
            ) {
                return vec![event];
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

    fn on_private_data(&mut self, channel: Channel, ts_ms: TimeMs) -> Vec<SystemEvent> {
        if self.account_id.is_none() || !self.required_private_data_channels.contains(&channel) {
            return Vec::new();
        }
        self.private_data_round.insert(channel);
        self.private_health_event(
            ts_ms,
            self.adapter.venue(),
            "fresh account and positions websocket payloads received",
        )
        .into_iter()
        .collect()
    }

    fn private_health_event(
        &mut self,
        ts_ms: TimeMs,
        venue: Venue,
        reason: &str,
    ) -> Option<SystemEvent> {
        if !self.expected_connections.is_subset(&self.ready_connections)
            || !self
                .required_private_data_channels
                .is_subset(&self.private_data_round)
        {
            return None;
        }
        let kind = if self.private_ready {
            SystemEventKind::PrivateStreamHeartbeat
        } else {
            SystemEventKind::PrivateStreamRecovered
        };
        self.private_ready = true;
        self.private_data_round.clear();
        Some(SystemEvent {
            ts_ms,
            kind,
            venue: Some(venue),
            account_id: self.account_id.clone(),
            symbol: None,
            reason: reason.to_string(),
        })
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
    for guard in &config.risk.stablecoin_guards {
        push_public_subscription(
            &mut subscriptions,
            &mut seen,
            Channel::Custom("index-tickers".to_string()),
            &guard.symbol,
            FeedPriority::Critical,
            config.runtime.public_connections_per_subscription,
        );
    }
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

async fn receive_alert_failure(
    receiver: &mut Option<mpsc::Receiver<AlertDeliveryFailure>>,
) -> Option<AlertDeliveryFailure> {
    match receiver {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

async fn receive_host_failure(
    receiver: &mut Option<mpsc::Receiver<HostHealthError>>,
) -> Option<HostHealthError> {
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
    use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use reap_core::{AccountUpdate, Balance, OrderEvent, OrderUpdate, Side};
    use reap_risk::{InstrumentRiskModel, RiskLimits, StablecoinGuardConfig};
    use reap_storage::start_jsonl_storage;
    use reap_strategy::{ChaosConfig, InstrumentKindConfig};
    use reap_venue::okx::{
        HttpResponse, OkxAccountLevel, OkxCredentials, OkxInstrumentType, OkxPositionMode,
        SignedRequest,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use crate::{
        LiveAccountConfig, LiveStorageConfig, OkxTradeModeConfig, OkxVenueConfig, RuntimeConfig,
        VerifiedInstrument,
    };

    use super::*;

    #[derive(Clone)]
    struct SafetyMockTransport {
        responses: Arc<Mutex<VecDeque<Result<String, RestError>>>>,
        requests: Arc<Mutex<Vec<SignedRequest>>>,
    }

    #[async_trait]
    impl HttpTransport for SafetyMockTransport {
        async fn execute(&self, request: SignedRequest) -> Result<HttpResponse, RestError> {
            self.requests.lock().unwrap().push(request);
            let body = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("mock response");
            Ok(HttpResponse {
                status: 200,
                body: body?,
            })
        }
    }

    fn safety_client(
        responses: Vec<Result<&str, RestError>>,
    ) -> (
        OkxRestClient<SafetyMockTransport>,
        Arc<Mutex<Vec<SignedRequest>>>,
    ) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let transport = SafetyMockTransport {
            responses: Arc::new(Mutex::new(
                responses
                    .into_iter()
                    .map(|response| response.map(str::to_string))
                    .collect(),
            )),
            requests: Arc::clone(&requests),
        };
        let signer = OkxSigner::new(OkxCredentials::new("key", "secret", "pass"), true);
        (OkxRestClient::new(transport, signer), requests)
    }

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

    async fn serve_one_alert(listener: tokio::net::TcpListener) {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4_096];
        loop {
            let read = socket.read(&mut buffer).await.unwrap();
            assert!(read > 0, "alert client closed before sending a request");
            request.extend_from_slice(&buffer[..read]);
            let text = String::from_utf8_lossy(&request);
            let Some(header_end) = text.find("\r\n\r\n") else {
                continue;
            };
            let content_length = text[..header_end]
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                break;
            }
        }
        socket
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
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
            alerts: crate::AlertConfig::default(),
            host_guard: crate::HostGuardConfig::default(),
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
            missing_stablecoin_rates: Vec::new(),
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

    fn latch(scope: SafetyLatchScope, source: SafetyLatchSource) -> SafetyLatchRecord {
        SafetyLatchRecord {
            ts_ms: 1,
            scope,
            active: true,
            source,
            request_id: None,
            reason: "test latch".to_string(),
        }
    }

    #[test]
    fn recovered_latches_are_validated_and_applied_fail_closed() {
        let config = config();
        let mut recovered = RecoveredStorage {
            global_safety_latch: Some(latch(SafetyLatchScope::Global, SafetyLatchSource::Risk)),
            ..RecoveredStorage::default()
        };
        recovered.account_safety_latches.insert(
            "main".to_string(),
            latch(
                SafetyLatchScope::Account {
                    account_id: "main".to_string(),
                },
                SafetyLatchSource::Operator,
            ),
        );
        recovered.symbol_safety_latches.insert(
            "BTC-USDT".to_string(),
            latch(
                SafetyLatchScope::Symbol {
                    symbol: "BTC-USDT".to_string(),
                },
                SafetyLatchSource::Operator,
            ),
        );
        validate_recovered_safety_latches(&config, &recovered).unwrap();

        let mut coordinator = ready_coordinator(&config, unix_time_ms(), true);
        let outputs = restore_safety_latches(&mut coordinator, &recovered).unwrap();

        assert_eq!(outputs.len(), 3);
        assert!(coordinator.kill_switch_active());
        assert!(coordinator.halted_accounts().contains_key("main"));
        assert!(coordinator.is_symbol_halted("BTC-USDT"));
        assert!(!coordinator.readiness().is_ready());
        assert!(outputs.iter().all(|output| {
            output
                .records
                .iter()
                .all(|record| !matches!(record, StorageRecord::SafetyLatch(_)))
        }));
    }

    #[test]
    fn recovered_latch_identity_must_match_live_config() {
        let config = config();
        let mut recovered = RecoveredStorage::default();
        recovered.account_safety_latches.insert(
            "removed".to_string(),
            latch(
                SafetyLatchScope::Account {
                    account_id: "removed".to_string(),
                },
                SafetyLatchSource::Operator,
            ),
        );
        let error = validate_recovered_safety_latches(&config, &recovered).unwrap_err();
        assert!(error.to_string().contains("unknown account removed"));
    }

    #[test]
    fn recovered_account_latch_blocks_replay_and_cancels_restored_orders() {
        let config = config();
        let mut recovered = RecoveredStorage::default();
        recovered.account_safety_latches.insert(
            "main".to_string(),
            latch(
                SafetyLatchScope::Account {
                    account_id: "main".to_string(),
                },
                SafetyLatchSource::Operator,
            ),
        );
        let mut coordinator = ready_coordinator(&config, unix_time_ms(), true);
        let _ = restore_safety_latches(&mut coordinator, &recovered).unwrap();
        let replay = coordinator
            .restore_order(
                "main",
                OrderUpdate {
                    ts_ms: 2,
                    order_id: "restored-live".to_string(),
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
                    reason: "restored quote".to_string(),
                },
            )
            .unwrap();

        assert!(
            replay
                .actions
                .iter()
                .all(|action| !matches!(action, LiveAction::Submit(_)))
        );
        let reapplied = restore_safety_latches(&mut coordinator, &recovered).unwrap();
        assert!(
            reapplied
                .iter()
                .flat_map(|output| &output.actions)
                .any(|action| matches!(
                    action,
                    LiveAction::Cancel(cancel) if cancel.client_order_id == "restored-live"
                ))
        );
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
            0,
        ));

        outcome.stop_reason = LiveStopReason::OperatorSignal;
        assert!(!qualifies_as_clean_soak(
            &outcome,
            RuntimeEvidence::default(),
            0,
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
            0,
        ));
        outcome.reached_ready = true;
        outcome.readiness_at_stop = readiness(crate::LivePhase::Degraded);
        assert!(!qualifies_as_clean_soak(
            &outcome,
            RuntimeEvidence::default(),
            0,
            0,
            0,
        ));
        outcome.readiness_at_stop = readiness(crate::LivePhase::Ready);

        let evidence = RuntimeEvidence {
            reconciliation_drift_events: 1,
            ..RuntimeEvidence::default()
        };
        assert!(!qualifies_as_clean_soak(&outcome, evidence, 0, 0, 0));
        assert!(!qualifies_as_clean_soak(
            &outcome,
            RuntimeEvidence::default(),
            1,
            0,
            0,
        ));
        assert!(!qualifies_as_clean_soak(
            &outcome,
            RuntimeEvidence::default(),
            0,
            1,
            0,
        ));
        assert!(!qualifies_as_clean_soak(
            &outcome,
            RuntimeEvidence::default(),
            0,
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

    #[test]
    fn persisted_safety_events_map_to_stable_external_alerts() {
        let record = |kind| {
            StorageRecord::System(SystemEvent {
                ts_ms: 42,
                kind,
                venue: Some(Venue::Okx),
                account_id: Some("main".to_string()),
                symbol: Some("BTC-USDT".to_string()),
                reason: "test condition".to_string(),
            })
        };

        let warning = alert_for_storage_record(&record(SystemEventKind::FeedGap)).unwrap();
        assert_eq!(warning.ts_ms, 42);
        assert_eq!(warning.severity, AlertSeverity::Warning);
        assert_eq!(warning.code, "feed_gap");
        assert_eq!(warning.attributes.get("venue").unwrap(), "okx");
        assert_eq!(warning.attributes.get("account_id").unwrap(), "main");
        assert_eq!(warning.attributes.get("symbol").unwrap(), "BTC-USDT");

        let critical = alert_for_storage_record(&record(SystemEventKind::RiskBreach)).unwrap();
        assert_eq!(critical.severity, AlertSeverity::Critical);
        assert_eq!(critical.code, "risk_breach");
        assert!(alert_for_storage_record(&record(SystemEventKind::FeedHeartbeat)).is_none());
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
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let alert_endpoint = format!("http://{}/alerts", listener.local_addr().unwrap());
        let alert_server = tokio::spawn(serve_one_alert(listener));
        let mut alert_runtime =
            start_webhook_alerts(reap_telemetry::WebhookAlertConfig::new(alert_endpoint)).unwrap();
        let alert_sink = alert_runtime.sink();
        let alert_failures = alert_runtime.take_failures();
        alert_sink
            .try_emit(AlertEvent::new(
                AlertSeverity::Warning,
                "test",
                "lifecycle_test",
                "test alert",
            ))
            .unwrap();
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
            safety_senders: HashMap::new(),
            safety_tasks: Vec::new(),
            feeds: Vec::new(),
            feed_tasks: Vec::new(),
            sources: Vec::new(),
            public_feed_index: 0,
            reconcile_inflight: HashSet::new(),
            cancel_inflight: HashSet::new(),
            last_reconcile_attempt: HashMap::new(),
            fill_convergence: FillConvergenceGuard::default(),
            readiness_timeout_ms: 1_000,
            timer_interval_ms: 100,
            max_feed_age_ms: 60_000,
            shutdown_timeout_ms: 100,
            safety_latch_sync_timeout_ms: 1_000,
            evidence: RuntimeEvidence::default(),
            shutdown_in_progress: false,
            shutdown_storage_error: None,
            preserve_deadman_on_shutdown: false,
            shutdown_reconciliation_requested: HashSet::new(),
            shutdown_reconciled_accounts: HashSet::new(),
            operator_service: None,
            operator_rx: None,
            operator_shutdown_reason: None,
            alert_runtime: Some(alert_runtime),
            alert_sink: Some(alert_sink),
            alert_failures: Some(alert_failures),
            alert_shutdown_timeout_ms: 1_000,
            alert_delivery_failure_is_fatal: true,
            observed_alert_delivery_failures: 0,
            alert_stats: AlertStats::default(),
            host_guard: None,
            host_failures: None,
            host_preflight: None,
            host_checks: 0,
            host_last_snapshot: None,
        };

        let report = runtime.run().await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), alert_server)
            .await
            .unwrap()
            .unwrap();
        drop(control_tx);
        drop(feed_tx);
        let _ = std::fs::remove_file(path);

        assert_eq!(report.stop_reason, LiveStopReason::DurationElapsed);
        assert!(report.elapsed_ms >= 20);
        assert!(report.reached_ready);
        assert_eq!(report.time_to_ready_ms, Some(0));
        assert!(report.readiness_at_stop.is_ready());
        assert_eq!(report.alerts_delivered, 1);
        assert_eq!(report.alert_delivery_failures, 0);
        assert_eq!(report.max_alert_queue_depth, 1);
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
            safety_senders: HashMap::new(),
            safety_tasks: Vec::new(),
            feeds: Vec::new(),
            feed_tasks: Vec::new(),
            sources: Vec::new(),
            public_feed_index: 0,
            reconcile_inflight: HashSet::new(),
            cancel_inflight: HashSet::new(),
            last_reconcile_attempt: HashMap::new(),
            fill_convergence: FillConvergenceGuard::default(),
            readiness_timeout_ms: 1_000,
            timer_interval_ms: 100,
            max_feed_age_ms: 60_000,
            shutdown_timeout_ms: 1_000,
            safety_latch_sync_timeout_ms: 1_000,
            evidence: RuntimeEvidence::default(),
            shutdown_in_progress: false,
            shutdown_storage_error: None,
            preserve_deadman_on_shutdown: false,
            shutdown_reconciliation_requested: HashSet::new(),
            shutdown_reconciled_accounts: HashSet::new(),
            operator_service: Some(operator_service),
            operator_rx: Some(operator_rx),
            operator_shutdown_reason: None,
            alert_runtime: None,
            alert_sink: None,
            alert_failures: None,
            alert_shutdown_timeout_ms: 100,
            alert_delivery_failure_is_fatal: true,
            observed_alert_delivery_failures: 0,
            alert_stats: AlertStats::default(),
            host_guard: None,
            host_failures: None,
            host_preflight: None,
            host_checks: 0,
            host_last_snapshot: None,
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
        let recovered = recover_jsonl(&storage_path).unwrap();
        assert!(recovered.global_safety_latch.is_some());
        assert!(recovered.account_safety_latches.contains_key("main"));
        assert!(recovered.symbol_safety_latches.is_empty());
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
                                remote_account: account_update(unix_time_ms()),
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
            safety_senders: HashMap::new(),
            safety_tasks: Vec::new(),
            feeds: Vec::new(),
            feed_tasks: Vec::new(),
            sources: Vec::new(),
            public_feed_index: 0,
            reconcile_inflight: HashSet::new(),
            cancel_inflight: HashSet::new(),
            last_reconcile_attempt: HashMap::new(),
            fill_convergence: FillConvergenceGuard::default(),
            readiness_timeout_ms: 1_000,
            timer_interval_ms: 100,
            max_feed_age_ms: 60_000,
            shutdown_timeout_ms: 1_000,
            safety_latch_sync_timeout_ms: 1_000,
            evidence: RuntimeEvidence::default(),
            shutdown_in_progress: false,
            shutdown_storage_error: None,
            preserve_deadman_on_shutdown: false,
            shutdown_reconciliation_requested: HashSet::new(),
            shutdown_reconciled_accounts: HashSet::new(),
            operator_service: None,
            operator_rx: None,
            operator_shutdown_reason: None,
            alert_runtime: None,
            alert_sink: None,
            alert_failures: None,
            alert_shutdown_timeout_ms: 100,
            alert_delivery_failure_is_fatal: true,
            observed_alert_delivery_failures: 0,
            alert_stats: AlertStats::default(),
            host_guard: None,
            host_failures: None,
            host_preflight: None,
            host_checks: 0,
            host_last_snapshot: None,
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

    #[tokio::test]
    async fn safety_task_disables_deadman_only_on_explicit_command() {
        let (client, requests) = safety_client(vec![Ok(
            r#"{"code":"0","msg":"","data":[{"triggerTime":"0","tag":"","ts":"1"}]}"#,
        )]);
        let (command_tx, command_rx) = mpsc::channel(2);
        let (event_tx, _event_rx) = mpsc::channel(2);
        let task = tokio::spawn(run_account_safety_task(
            "main".to_string(),
            client,
            command_rx,
            event_tx,
            Some(30),
            60_000,
            60_000,
            1_000,
        ));
        let (result_tx, result_rx) = oneshot::channel();
        command_tx
            .send(SafetyTaskCommand::DisableDeadMan { result: result_tx })
            .await
            .unwrap();
        result_rx.await.unwrap().unwrap();
        command_tx.send(SafetyTaskCommand::Shutdown).await.unwrap();
        task.await.unwrap();

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/api/v5/trade/cancel-all-after");
        assert_eq!(requests[0].body, r#"{"timeOut":"0"}"#);
    }

    #[tokio::test]
    async fn deadman_heartbeat_failure_is_fatal() {
        let (client, _) = safety_client(vec![Err(RestError::Transport(
            "injected heartbeat failure".to_string(),
        ))]);
        let (_command_tx, command_rx) = mpsc::channel(2);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let task = tokio::spawn(run_account_safety_task(
            "main".to_string(),
            client,
            command_rx,
            event_tx,
            Some(30),
            1,
            60_000,
            1_000,
        ));

        let event = tokio::time::timeout(Duration::from_millis(100), event_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            event,
            RuntimeEvent::Fatal(message)
                if message.contains("Cancel All After heartbeat failed")
                    && message.contains("injected heartbeat failure")
        ));
        task.await.unwrap();
    }

    #[test]
    fn private_account_requires_every_transport_and_state_data_round() {
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
        assert!(
            source
                .on_status(status("positions", ConnectionStatusKind::Ready))
                .is_empty()
        );
        assert!(
            source
                .on_status(status("orders", ConnectionStatusKind::Heartbeat))
                .is_empty()
        );
        assert!(source.on_private_data(Channel::Account, 2).is_empty());
        assert!(
            source
                .on_status(status("positions", ConnectionStatusKind::Heartbeat))
                .is_empty()
        );
        let ready = source.on_private_data(Channel::Positions, 3);
        assert_eq!(ready[0].kind, SystemEventKind::PrivateStreamRecovered);

        assert!(source.on_private_data(Channel::Account, 4).is_empty());
        assert!(source.on_private_data(Channel::Account, 5).is_empty());
        assert!(
            source
                .on_status(status("orders", ConnectionStatusKind::Heartbeat))
                .is_empty()
        );
        let heartbeat = source.on_private_data(Channel::Positions, 6);
        assert_eq!(heartbeat[0].kind, SystemEventKind::PrivateStreamHeartbeat);

        let stale = source.on_status(status("fills", ConnectionStatusKind::Disconnected));
        assert_eq!(stale[0].kind, SystemEventKind::PrivateStreamStale);
        assert!(
            source
                .on_status(status("fills", ConnectionStatusKind::Ready))
                .is_empty()
        );
        assert!(source.on_private_data(Channel::Positions, 7).is_empty());
        let recovered = source.on_private_data(Channel::Account, 8);
        assert_eq!(recovered[0].kind, SystemEventKind::PrivateStreamRecovered);
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
        assert_eq!(
            private_subscriptions(false)
                .into_iter()
                .map(|subscription| subscription.channel)
                .collect::<HashSet<_>>(),
            HashSet::from([Channel::Orders, Channel::Account, Channel::Positions])
        );
        assert_eq!(
            private_subscriptions(true)
                .into_iter()
                .map(|subscription| subscription.channel)
                .collect::<HashSet<_>>(),
            HashSet::from([
                Channel::Orders,
                Channel::Fills,
                Channel::Account,
                Channel::Positions,
            ])
        );
    }

    #[test]
    fn public_plan_replicates_stablecoin_guards_as_critical_feeds() {
        let mut config = config();
        config.risk.stablecoin_guards = vec![StablecoinGuardConfig {
            symbol: "USDT-USD".to_string(),
            max_downside_deviation: 0.01,
        }];
        config.strategy.instruments[0].index_symbol = Some("USDT-USD".to_string());

        let subscriptions = public_subscriptions(&config);
        let stablecoin = subscriptions
            .iter()
            .filter(|subscription| {
                subscription.channel == Channel::Custom("index-tickers".to_string())
                    && subscription.symbol.as_deref() == Some("USDT-USD")
            })
            .collect::<Vec<_>>();

        assert_eq!(stablecoin.len(), 1);
        assert_eq!(stablecoin[0].priority, FeedPriority::Critical);
        assert_eq!(
            stablecoin[0].connections,
            config.runtime.public_connections_per_subscription
        );
    }

    #[tokio::test]
    async fn journal_ownership_is_checked_before_credentials_or_network() {
        let path = std::env::temp_dir().join(format!(
            "reap-live-owner-{}-{}.jsonl",
            std::process::id(),
            unix_time_ns()
        ));
        let lease = acquire_storage_lease(&path).unwrap();
        let lock_path = lease.lock_path().to_path_buf();
        let mut config = config();
        config.storage.path = path;

        let error = run_live(
            config,
            LiveRunOptions {
                mode: LiveMode::Observe,
                demo_confirmed: false,
                run_duration: Some(Duration::from_millis(1)),
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(
            error,
            LiveRuntimeError::Storage(StorageError::AlreadyLocked { .. })
        ));
        drop(lease);
        let _ = std::fs::remove_file(lock_path);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn host_preflight_fails_before_credentials_or_network_and_releases_lease() {
        let path = std::env::temp_dir().join(format!(
            "reap-live-host-preflight-{}-{}.jsonl",
            std::process::id(),
            unix_time_ns()
        ));
        let mut config = config();
        config.storage.path = path.clone();
        config.host_guard.enabled = true;
        config.host_guard.min_disk_available_bytes = u64::MAX;
        config.host_guard.min_memory_available_bytes = 1;
        config.host_guard.require_clock_synchronized = false;

        let error = run_live(
            config,
            LiveRunOptions {
                mode: LiveMode::Observe,
                demo_confirmed: false,
                run_duration: Some(Duration::from_millis(1)),
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(
            error,
            LiveRuntimeError::Host(HostHealthError::Unhealthy { ref code, .. })
                if code == "disk_low"
        ));
        let lease = acquire_storage_lease(&path).unwrap();
        let lock_path = lease.lock_path().to_path_buf();
        drop(lease);
        let _ = std::fs::remove_file(lock_path);
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
        production.risk.stablecoin_guards = vec![StablecoinGuardConfig {
            symbol: "USDT-USD".to_string(),
            max_downside_deviation: 0.01,
        }];
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
