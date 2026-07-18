use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reap_core::{
    BacktestLatencyClass, Channel, FillKey, MarketEvent, NormalizedEvent, OrderStatus,
    PINNED_JAVA_REVISION, SystemEvent, SystemEventKind, TimerEvent, Venue,
};
use reap_feed::{
    ConnectionAttemptPacer, ConnectionStatus, ConnectionStatusKind, FeedOutput, FeedProcessor,
    ReconnectPolicy, partition_subscriptions, try_spawn_supervised_feed,
};
#[cfg(test)]
use reap_okx_live_adapter::OrderCommandWebsocketLifecycle;
use reap_okx_live_adapter::{
    ConnectionSettings, CredentialEnvNames, OrderCommandWebsocketConfig,
    OrderCommandWebsocketStatusKind, demo_from_env, observe_from_env,
};
#[cfg(test)]
use reap_order::OkxOrderGateway;
use reap_order::{OkxReconciliationClient, reconcile_full_state};
use reap_storage::{
    BootstrapRecord, OrderOperation, OrderRequestRecord, RecoveredStorage, SafetyLatchRecord,
    SafetyLatchScope, SafetyLatchSource, SessionStartRecord, StorageConfig, StorageError,
    StorageRecord, acquire_storage_lease, recover_leased_jsonl, start_jsonl_storage_with_lease,
};
use reap_telemetry::{
    AlertDeliveryFailure, AlertError, AlertEvent, AlertRuntime, AlertSeverity, AlertStats,
    start_webhook_alerts,
};
use reap_venue::okx::{
    OKX_MIN_ACCOUNT_INSTRUMENT_REQUEST_INTERVAL_MS, OkxAdapter, okx_capability_registration,
};
#[cfg(test)]
use reap_venue::okx::{
    OkxInstrument, OkxSystemEnvironment, OkxSystemServiceType, OkxSystemStatus,
    OkxSystemStatusState, OkxTradeFeeRate,
};
use reap_venue::{PrivateOrderState, PrivateOrderUpdate, RemoteFill, RemoteOrder, VenueAdapter};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

use crate::convergence::{FillConvergenceGuard, OrderStateConvergenceGuard};
use crate::coordinator::{CancelAction, LiveAction, ReconcileAction, SubmitAction};
use crate::forbidden_orders::{
    ForbiddenOrderEvent, ForbiddenOrderObserverPort, ForbiddenSentinelPolicy,
    run_forbidden_order_sentinel,
};
use crate::provenance::{
    current_executable_sha256 as hash_current_executable,
    host_identity_sha256 as hash_host_identity, okx_account_identity_sha256,
};
use crate::safety_contracts::LiveFaultFailureCode;
use crate::{
    AccountBootstrapSnapshot, ChaosConnectivityPlan, ChaosConnectivityPlanError, CoordinatorError,
    CoordinatorOutput, HostGuardRuntime, HostHealthError, HostHealthSnapshot, LiveConfig,
    LiveConfigError, LiveConfigFileEvidence, LiveCoordinator, LiveLatencyCollector,
    LiveLatencyEvidence, LiveLatencySemantics, LiveMode, MaintenanceRelevancePlan, OperatorCommand,
    OperatorEnvelope, OperatorError, OperatorResponse, OperatorService, OperatorStatus,
    ReadinessSnapshot, ReconciliationResult, StartupGate, TradingEnvironment, VerifiedBootstrap,
    alert_webhook_from_env, check_host_health, load_live_config_with_evidence, okx_instrument_type,
    operator_secret_from_env, start_host_guard, start_operator_service, verify_bootstrap,
};

mod composition;
mod connectivity;
mod dispatch;
mod planning;
mod readiness_safety;
mod reconciliation;
mod shutdown;

#[cfg(test)]
use composition::truncate_utf8;
use composition::{
    AccountSeed, CompositionState, ReadinessTracker, RunLoopFailure, RunLoopOutcome,
    RuntimeEvidence, combine_lifecycle_errors, live_failure_evidence,
    live_startup_failure_evidence, qualifies_as_clean_soak,
};
use connectivity::{ConnectivityState, FeedSourceState, spawn_feed_forwarders};
#[cfg(test)]
use dispatch::RuntimeTaskFailure;
use dispatch::{
    DispatchState, OrderTaskCommand, ReconcileOrderRef, ReconcileTaskCommand, RuntimeEvent,
    SafetyTaskCommand, run_order_task,
};
#[cfg(test)]
use planning::validate_private_state_socket_count;
use planning::{
    planned_order_session_counts, private_socket_plans_by_account, runtime_public_subscriptions,
    validate_public_socket_plans, validate_runtime_connectivity_plan,
};
#[cfg(test)]
use readiness_safety::{
    ExchangeInstrumentExpectation, exchange_fee_drift_reason, exchange_fee_request_interval_ms,
    exchange_instrument_drift_reason,
};
use readiness_safety::{
    ExchangeInstrumentGuard, ExchangeStatusGuard, ReadinessPort, ReadinessSafetyState, SafetyPort,
    exchange_instrument_expectations, exchange_status_block_reason, rest_clock_skew_ms,
    run_account_safety_task, verify_initial_exchange_fees, verify_initial_exchange_instruments,
};
use reconciliation::{ReconciliationState, run_reconcile_task};
use shutdown::{ShutdownState, StartupTaskGroup, is_zero_order_reconciliation, shutdown_signal};

pub const LIVE_RUN_REPORT_SCHEMA_VERSION: u32 = 8;
pub const MAX_LIVE_FAILURE_CODE_BYTES: usize = 64;
pub const MAX_LIVE_FAILURE_MESSAGE_BYTES: usize = 4_096;

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
    RuntimeFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveFailureEvidence {
    /// Stable machine-readable classification of the primary failure.
    pub code: String,
    /// Bounded diagnostic text. This is evidence, not a stable API contract.
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveRunReport {
    pub schema_version: u32,
    pub session_id: Option<String>,
    pub session_started_at_ms: u64,
    #[serde(default)]
    pub config_source: Option<LiveConfigFileEvidence>,
    pub config_fingerprint: String,
    pub evidence_config_fingerprint: String,
    pub java_reference_revision: String,
    pub reap_version: String,
    pub executable_sha256: String,
    pub host_identity_sha256: Option<String>,
    pub account_identity_sha256s: BTreeMap<String, String>,
    pub mode: LiveMode,
    pub stop_reason: LiveStopReason,
    pub failure: Option<LiveFailureEvidence>,
    pub elapsed_ms: u64,
    pub reached_ready: bool,
    pub time_to_ready_ms: Option<u64>,
    pub readiness_loss_count: u64,
    pub max_readiness_outage_ms: u64,
    pub reconciliation_drift_events: u64,
    pub book_recovery_events: u64,
    pub stream_stale_events: u64,
    pub connection_disconnect_events: u64,
    pub public_connection_disconnect_events: u64,
    pub private_connection_disconnect_events: u64,
    pub order_transport_disconnect_events: u64,
    pub order_transport_stale_events: u64,
    pub ambiguous_submit_events: u64,
    pub ambiguous_cancel_events: u64,
    pub partial_fill_events: u64,
    pub fill_convergence_timeout_events: u64,
    pub order_convergence_timeout_events: u64,
    pub restored_safety_latches: u64,
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
    pub latency_evidence: LiveLatencyEvidence,
    pub clean_soak: bool,
}

#[derive(Debug, Error)]
pub enum LiveRuntimeError {
    #[error(transparent)]
    Config(#[from] LiveConfigError),
    #[error(transparent)]
    ConnectivityPlan(#[from] ChaosConnectivityPlanError),
    #[error("demo order entry requires explicit confirmation")]
    DemoConfirmationRequired,
    #[error("demo mode refuses production exchange configuration")]
    DemoRequiresSimulatedTrading,
    #[error("live run duration must be greater than zero")]
    InvalidRunDuration,
    #[error("live evidence provenance failed: {0}")]
    Provenance(String),
    #[error("account {account_id} bootstrap failed: {message}")]
    Bootstrap { account_id: String, message: String },
    #[error("bootstrap verification failed: {0}")]
    BootstrapVerification(String),
    #[error("checkpoint identity mismatch for account {account_id}: {message}")]
    CheckpointIdentity { account_id: String, message: String },
    #[error("feed subscription planning failed: {0}")]
    Subscription(String),
    #[error("live connection pacer failed: {0}")]
    ConnectionPacer(#[from] reap_feed::ConnectionAttemptPacerError),
    #[error("live connection pacer failed during runtime: {0}")]
    ConnectionPacerRuntime(String),
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
    #[error("Cancel All After heartbeat failed: {0}")]
    DeadmanHeartbeat(String),
    #[error("exchange clock skew exceeded the configured bound: {0}")]
    ExchangeClockSkew(String),
    #[error("exchange clock check failed: {0}")]
    ExchangeClockCheck(String),
    #[error("OKX exchange status blocks strategy operation: {0}")]
    ExchangeStatus(String),
    #[error("OKX exchange status check failed: {0}")]
    ExchangeStatusCheck(String),
    #[error("configured exchange fee is unsafe: {0}")]
    ExchangeFeeDrift(String),
    #[error("authenticated exchange fee check failed: {0}")]
    ExchangeFeeCheck(String),
    #[error("authenticated exchange instrument metadata drifted: {0}")]
    ExchangeInstrumentDrift(String),
    #[error("authenticated exchange instrument check failed: {0}")]
    ExchangeInstrumentCheck(String),
    #[error("authenticated account configuration drifted: {0}")]
    AccountConfigDrift(String),
    #[error("authenticated account configuration check failed: {0}")]
    AccountConfigCheck(String),
    #[error("durable safety latch sync timed out after {0}ms")]
    SafetyLatchSyncTimeout(u64),
    #[error(
        "shutdown timed out with {active_orders} active local orders and {unreconciled_accounts} unreconciled accounts"
    )]
    ShutdownUnresolved {
        active_orders: usize,
        unreconciled_accounts: usize,
    },
    #[error("runtime task teardown timed out after {0}ms; remaining owners were aborted")]
    TeardownTimeout(u64),
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
    #[error("{source}")]
    ReportedFailure {
        #[source]
        source: Box<LiveRuntimeError>,
        report: Box<LiveRunReport>,
    },
}

impl LiveRuntimeError {
    fn stable_code(&self) -> &'static str {
        match self {
            Self::Config(_) => "config",
            Self::ConnectivityPlan(_) => "connectivity_plan",
            Self::DemoConfirmationRequired => "demo_confirmation_required",
            Self::DemoRequiresSimulatedTrading => "demo_requires_simulated_trading",
            Self::InvalidRunDuration => "invalid_run_duration",
            Self::Provenance(_) => "provenance",
            Self::Bootstrap { .. } => "bootstrap",
            Self::BootstrapVerification(_) => "bootstrap_verification",
            Self::CheckpointIdentity { .. } => "checkpoint_identity",
            Self::Subscription(_) => "subscription",
            Self::ConnectionPacer(_) | Self::ConnectionPacerRuntime(_) => "connection_pacer",
            Self::GatewaySetup { .. } => "gateway_setup",
            Self::Coordinator(_) => "coordinator",
            Self::Storage(_) => "storage",
            Self::Operator(_) => "operator",
            Self::Alert(_) => "alert",
            Self::Host(_) => "host_guard",
            Self::AlertDelivery { .. } => "alert_delivery",
            Self::AlertMonitorClosed => "alert_monitor_closed",
            Self::HostGuardClosed => "host_guard_closed",
            Self::AlertShutdownTimeout(_) => "alert_shutdown_timeout",
            Self::AlertFailuresDuringShutdown(_) => "alert_failures_during_shutdown",
            Self::ShutdownStorage(_) => "shutdown_storage",
            Self::EventChannelClosed => "event_channel_closed",
            Self::OperatorChannelClosed => "operator_channel_closed",
            Self::OrderQueueUnavailable(_) => "order_queue_unavailable",
            Self::UnsafeShutdownSubmit => "unsafe_shutdown_submit",
            Self::MissingRecoveryRoute(_) => "missing_recovery_route",
            Self::ReadinessTimeout(_) => "readiness_timeout",
            Self::GatewayTask(_) => "gateway_task",
            Self::DeadmanHeartbeat(_) => LiveFaultFailureCode::DeadmanHeartbeat.as_str(),
            Self::ExchangeClockSkew(_) => LiveFaultFailureCode::ExchangeClockSkew.as_str(),
            Self::ExchangeClockCheck(_) => LiveFaultFailureCode::ExchangeClockCheck.as_str(),
            Self::ExchangeStatus(_) => LiveFaultFailureCode::ExchangeStatus.as_str(),
            Self::ExchangeStatusCheck(_) => LiveFaultFailureCode::ExchangeStatusCheck.as_str(),
            Self::ExchangeFeeDrift(_) => LiveFaultFailureCode::ExchangeFeeDrift.as_str(),
            Self::ExchangeFeeCheck(_) => LiveFaultFailureCode::ExchangeFeeCheck.as_str(),
            Self::ExchangeInstrumentDrift(_) => {
                LiveFaultFailureCode::ExchangeInstrumentDrift.as_str()
            }
            Self::ExchangeInstrumentCheck(_) => {
                LiveFaultFailureCode::ExchangeInstrumentCheck.as_str()
            }
            Self::AccountConfigDrift(_) => LiveFaultFailureCode::AccountConfigDrift.as_str(),
            Self::AccountConfigCheck(_) => LiveFaultFailureCode::AccountConfigCheck.as_str(),
            Self::SafetyLatchSyncTimeout(_) => "safety_latch_sync_timeout",
            Self::ShutdownUnresolved { .. } => "shutdown_unresolved",
            Self::TeardownTimeout(_) => "teardown_timeout",
            Self::FeedAdapter(_) => "feed_adapter",
            Self::Join(_) => "task_join",
            Self::Signal(_) => "signal",
            Self::LifecycleFailure { primary, .. } => primary.stable_code(),
            Self::ReportedFailure { source, .. } => source.stable_code(),
        }
    }
}

#[derive(Debug, Clone)]
struct LiveRunAttemptEvidence {
    session_started_at_ms: u64,
    config_source: Option<LiveConfigFileEvidence>,
    config_fingerprint: String,
    evidence_config_fingerprint: String,
    executable_sha256: String,
    host_identity_sha256: Option<String>,
}

#[derive(Debug)]
pub struct PreparedLiveRun {
    config: LiveConfig,
    options: LiveRunOptions,
    evidence: LiveRunAttemptEvidence,
    connectivity_plan: ChaosConnectivityPlan,
}

pub fn prepare_live_path(
    path: impl AsRef<Path>,
    options: LiveRunOptions,
) -> Result<PreparedLiveRun, LiveRuntimeError> {
    let (config, config_source) = load_live_config_with_evidence(path)?;
    prepare_live_with_config_source(config, options, Some(config_source))
}

pub fn prepare_live(
    config: LiveConfig,
    options: LiveRunOptions,
) -> Result<PreparedLiveRun, LiveRuntimeError> {
    prepare_live_with_config_source(config, options, None)
}

fn prepare_live_with_config_source(
    config: LiveConfig,
    options: LiveRunOptions,
    config_source: Option<LiveConfigFileEvidence>,
) -> Result<PreparedLiveRun, LiveRuntimeError> {
    config.ensure_valid()?;
    if options
        .run_duration
        .is_some_and(|duration| duration.is_zero())
    {
        return Err(LiveRuntimeError::InvalidRunDuration);
    }
    if options.mode == LiveMode::Demo {
        if !options.demo_confirmed {
            return Err(LiveRuntimeError::DemoConfirmationRequired);
        }
        if config.venue.environment != TradingEnvironment::Demo {
            return Err(LiveRuntimeError::DemoRequiresSimulatedTrading);
        }
    }
    // Resolve the complete secret-free boundary before evidence collection,
    // credential lookup, signer construction, or any network-capable role.
    let connectivity_plan = ChaosConnectivityPlan::resolve(&config, options.mode)?;
    let evidence = LiveRunAttemptEvidence {
        session_started_at_ms: unix_time_ms(),
        config_source,
        config_fingerprint: config.fingerprint()?,
        evidence_config_fingerprint: config.evidence_fingerprint()?,
        executable_sha256: current_executable_sha256()?,
        host_identity_sha256: config
            .host_guard
            .enabled
            .then(host_identity_sha256)
            .transpose()?,
    };
    Ok(PreparedLiveRun {
        config,
        options,
        evidence,
        connectivity_plan,
    })
}

impl PreparedLiveRun {
    pub fn connectivity_plan(&self) -> &ChaosConnectivityPlan {
        &self.connectivity_plan
    }

    pub fn connectivity_migration_diagnostics(&self) -> Vec<String> {
        self.config.connectivity_migration_diagnostics()
    }

    pub async fn run(self) -> Result<LiveRunReport, LiveRuntimeError> {
        if self.options.mode == LiveMode::Validate {
            return Ok(self.pre_runtime_report(LiveStopReason::Validation, None, 0));
        }
        let build = LiveRuntime::build(
            self.config.clone(),
            self.evidence.clone(),
            self.connectivity_plan.clone(),
            self.options.mode,
            self.options.run_duration,
        )
        .await;
        match build {
            Ok(runtime) => runtime.run().await,
            Err(error @ LiveRuntimeError::ReportedFailure { .. }) => Err(error),
            Err(error) => {
                let report = self.pre_runtime_report(
                    LiveStopReason::RuntimeFailure,
                    Some(live_startup_failure_evidence(&error)),
                    unix_time_ms().saturating_sub(self.evidence.session_started_at_ms),
                );
                Err(LiveRuntimeError::ReportedFailure {
                    source: Box::new(error),
                    report: Box::new(report),
                })
            }
        }
    }

    fn pre_runtime_report(
        &self,
        stop_reason: LiveStopReason,
        failure: Option<LiveFailureEvidence>,
        elapsed_ms: u64,
    ) -> LiveRunReport {
        let readiness = StartupGate::new(&self.config).snapshot();
        LiveRunReport {
            schema_version: LIVE_RUN_REPORT_SCHEMA_VERSION,
            session_id: None,
            session_started_at_ms: self.evidence.session_started_at_ms,
            config_source: self.evidence.config_source.clone(),
            config_fingerprint: self.evidence.config_fingerprint.clone(),
            evidence_config_fingerprint: self.evidence.evidence_config_fingerprint.clone(),
            java_reference_revision: PINNED_JAVA_REVISION.to_string(),
            reap_version: env!("CARGO_PKG_VERSION").to_string(),
            executable_sha256: self.evidence.executable_sha256.clone(),
            host_identity_sha256: self.evidence.host_identity_sha256.clone(),
            account_identity_sha256s: BTreeMap::new(),
            mode: self.options.mode,
            stop_reason,
            failure,
            elapsed_ms,
            reached_ready: false,
            time_to_ready_ms: None,
            readiness_loss_count: 0,
            max_readiness_outage_ms: 0,
            reconciliation_drift_events: 0,
            book_recovery_events: 0,
            stream_stale_events: 0,
            connection_disconnect_events: 0,
            public_connection_disconnect_events: 0,
            private_connection_disconnect_events: 0,
            order_transport_disconnect_events: 0,
            order_transport_stale_events: 0,
            ambiguous_submit_events: 0,
            ambiguous_cancel_events: 0,
            partial_fill_events: 0,
            fill_convergence_timeout_events: 0,
            order_convergence_timeout_events: 0,
            restored_safety_latches: 0,
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
            latency_evidence: LiveLatencyEvidence::default(),
            clean_soak: false,
        }
    }
}

pub async fn run_live_path(
    path: impl AsRef<Path>,
    options: LiveRunOptions,
) -> Result<LiveRunReport, LiveRuntimeError> {
    prepare_live_path(path, options)?.run().await
}

pub async fn run_live(
    config: LiveConfig,
    options: LiveRunOptions,
) -> Result<LiveRunReport, LiveRuntimeError> {
    prepare_live(config, options)?.run().await
}

async fn bootstrap_accounts(
    config: &LiveConfig,
    restored_orders: &HashMap<String, Vec<reap_core::OrderUpdate>>,
    mode: LiveMode,
    planned_order_accounts: &HashSet<String>,
    maintenance_relevance: &MaintenanceRelevancePlan,
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
        let connection = ConnectionSettings::new(
            config.venue.rest_url.clone(),
            config.venue.environment.is_demo(),
            Duration::from_millis(config.runtime.rest_connect_timeout_ms),
            Duration::from_millis(config.runtime.rest_request_timeout_ms),
        )
        .map_err(|error| LiveRuntimeError::Bootstrap {
            account_id: account.id.clone(),
            message: error.to_string(),
        })?;
        let credential_env = CredentialEnvNames::new(
            account.api_key_env.clone(),
            account.secret_key_env.clone(),
            account.passphrase_env.clone(),
        )
        .map_err(|error| LiveRuntimeError::Bootstrap {
            account_id: account.id.clone(),
            message: error.to_string(),
        })?;
        let trade_modes = account
            .trade_modes
            .iter()
            .map(|(symbol, mode)| (symbol.clone(), (*mode).into()))
            .collect();
        let (
            readiness,
            live_reconciliation,
            forbidden_observer,
            private_state_sessions,
            bound_order_gateway,
            safety,
        ) = match (mode, planned_order_accounts.contains(&account.id)) {
            (LiveMode::Observe, _) | (LiveMode::Demo, false) => {
                let mut roles = observe_from_env(
                    connection,
                    credential_env,
                    config.venue.enable_vip_fills_channel,
                )
                .map_err(|error| LiveRuntimeError::Bootstrap {
                    account_id: account.id.clone(),
                    message: error.to_string(),
                })?;
                let private_state_sessions =
                    roles.take_private_state_sessions().ok_or_else(|| {
                        LiveRuntimeError::Bootstrap {
                            account_id: account.id.clone(),
                            message: "private state session authority was already consumed"
                                .to_string(),
                        }
                    })?;
                (
                    Arc::new(roles.readiness()) as Arc<dyn ReadinessPort>,
                    roles.reconciliation(),
                    roles.forbidden_observer(),
                    private_state_sessions,
                    None,
                    None,
                )
            }
            (LiveMode::Demo, true) => {
                let mut roles = demo_from_env(
                    connection,
                    credential_env,
                    account.id.clone(),
                    config.venue.enable_vip_fills_channel,
                )
                .map_err(|error| LiveRuntimeError::Bootstrap {
                    account_id: account.id.clone(),
                    message: error.to_string(),
                })?;
                let reconciliation = roles.observe().reconciliation();
                let bound_order_gateway = roles
                    .take_bound_order_gateway(trade_modes, config.runtime.pacing_policy())
                    .map_err(|error| LiveRuntimeError::GatewaySetup {
                        account_id: account.id.clone(),
                        message: error.to_string(),
                    })?;
                let safety = roles
                    .take_safety()
                    .ok_or_else(|| LiveRuntimeError::GatewaySetup {
                        account_id: account.id.clone(),
                        message: "demo live-safety authority was already consumed".to_string(),
                    })?;
                let private_state_sessions =
                    roles.take_private_state_sessions().ok_or_else(|| {
                        LiveRuntimeError::Bootstrap {
                            account_id: account.id.clone(),
                            message: "private state session authority was already consumed"
                                .to_string(),
                        }
                    })?;
                (
                    Arc::new(roles.observe().readiness()) as Arc<dyn ReadinessPort>,
                    reconciliation,
                    roles.observe().forbidden_observer(),
                    private_state_sessions,
                    Some(bound_order_gateway),
                    Some(Arc::new(safety) as Arc<dyn SafetyPort>),
                )
            }
            (LiveMode::Validate, _) => {
                return Err(LiveRuntimeError::Bootstrap {
                    account_id: account.id.clone(),
                    message: "validate mode cannot construct network authority".to_string(),
                });
            }
        };
        let reconciliation = OkxReconciliationClient::new(
            Arc::new(live_reconciliation),
            config.runtime.pacing_policy(),
        );
        let clock_skew_ms = rest_clock_skew_ms(readiness.as_ref())
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
        if seeds.is_empty() {
            let statuses = readiness
                .system_status()
                .await
                .map_err(|error| LiveRuntimeError::ExchangeStatusCheck(error.to_string()))?;
            if let Some(reason) = exchange_status_block_reason(
                &statuses,
                maintenance_relevance,
                unix_time_ms(),
                config.runtime.exchange_status_lead_ms,
            ) {
                return Err(LiveRuntimeError::ExchangeStatus(reason));
            }
        }
        let account_config = readiness
            .account_config()
            .await
            .map_err(|error| bootstrap_error(&account.id, "account config", error.to_string()))?;
        let balance_economics = readiness
            .account_balance_snapshot()
            .await
            .map_err(|error| bootstrap_error(&account.id, "account balance", error.to_string()))?;
        let balance = balance_economics.account_update();
        let position_risks = readiness
            .account_positions_snapshot(None, None)
            .await
            .map_err(|error| {
                bootstrap_error(&account.id, "account positions", error.to_string())
            })?;
        let positions = position_risks.account_update();
        let (mut open_orders, recent_fills) = reconciliation
            .fetch_remote_state(
                None,
                None,
                config.runtime.max_order_reconciliation_pages,
                config.runtime.max_fill_reconciliation_pages,
            )
            .await
            .map_err(|error| {
                bootstrap_error(
                    &account.id,
                    "open orders and recent fills",
                    error.to_string(),
                )
            })?;
        let mut remote_ids = open_orders
            .iter()
            .map(remote_order_id)
            .collect::<HashSet<_>>();
        for restored in restored_orders.get(&account.id).into_iter().flatten() {
            if remote_ids.contains(&restored.order_id) {
                continue;
            }
            let details = match reconciliation
                .fetch_order_details(&restored.symbol, &restored.order_id)
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
        for (index, instrument) in config.instruments_for_account(&account.id).enumerate() {
            if index > 0 {
                tokio::time::sleep(Duration::from_millis(
                    OKX_MIN_ACCOUNT_INSTRUMENT_REQUEST_INTERVAL_MS,
                ))
                .await;
            }
            let metadata = readiness
                .account_instrument(okx_instrument_type(instrument.kind), &instrument.symbol)
                .await
                .map_err(|error| {
                    bootstrap_error(
                        &account.id,
                        &format!("instrument {}", instrument.symbol),
                        error.to_string(),
                    )
                })?;
            instruments.insert(instrument.symbol.clone(), metadata);
        }
        let instrument_guard = ExchangeInstrumentGuard {
            sweep_interval_ms: config.runtime.exchange_fee_check_interval_ms,
            change_lead_ms: config.runtime.exchange_instrument_change_lead_ms,
            expectations: exchange_instrument_expectations(config, &account.id, &instruments)?,
        };
        verify_initial_exchange_instruments(&account.id, &instrument_guard, unix_time_ms())?;
        verify_initial_exchange_fees(&account.id, readiness.as_ref(), &instrument_guard).await?;
        snapshots.insert(
            account.id.clone(),
            AccountBootstrapSnapshot {
                account_config,
                instruments,
                balance_economics,
                position_risks,
                balance,
                positions,
                open_orders,
                recent_fills,
            },
        );
        seeds.push(AccountSeed {
            account_id: account.id.clone(),
            readiness,
            reconciliation,
            forbidden_observer,
            private_state_sessions,
            bound_order_gateway,
            safety,
            instrument_guard,
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
        last_fill_fee: None,
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

fn recovered_safety_latch_count(recovered: &RecoveredStorage) -> u64 {
    let total = usize::from(recovered.global_safety_latch.is_some())
        .saturating_add(recovered.account_safety_latches.len())
        .saturating_add(recovered.symbol_safety_latches.len());
    u64::try_from(total).unwrap_or(u64::MAX)
}

fn proven_active_recovered_orders(
    config: &LiveConfig,
    recovered: &mut RecoveredStorage,
) -> Vec<(
    reap_core::OrderUpdate,
    reap_storage::ProvenRegularSubmitRequest,
)> {
    let requests = std::mem::take(&mut recovered.proven_regular_submit_requests);
    let mut orders = requests
        .into_values()
        .filter_map(|proof| {
            let update = recovered.latest_orders.get(proof.client_order_id())?;
            if !matches!(
                update.status,
                OrderStatus::PendingNew | OrderStatus::Live | OrderStatus::PartiallyFilled
            ) {
                return None;
            }
            let account_id = config
                .account_for_symbol(&update.symbol)
                .map(|account| account.id.as_str())?;
            (proof.account_id() == account_id
                && proof.symbol() == update.symbol
                && proof.client_order_id() == update.order_id)
                .then(|| (update.clone(), proof))
        })
        .collect::<Vec<_>>();
    orders.sort_by(|(left, _), (right, _)| left.order_id.cmp(&right.order_id));
    orders
}

fn restore_active_order_bindings(
    coordinator: &mut LiveCoordinator,
    recovered: &mut RecoveredStorage,
) -> Result<(), LiveRuntimeError> {
    let bindings = std::mem::take(&mut recovered.proven_regular_order_bindings);
    for binding in bindings.into_values() {
        let account_id = binding.account_id().to_string();
        if !coordinator.manages_account(&account_id) {
            return Err(LiveRuntimeError::BootstrapVerification(format!(
                "recovered order binding references unknown account {account_id}; retain the journal and reconcile before changing account identity"
            )));
        }
        let active_order_is_restored =
            coordinator.private_state(&account_id).is_some_and(|state| {
                state
                    .order_reducer()
                    .contains_order(binding.client_order_id())
            });
        if active_order_is_restored {
            coordinator.restore_order_binding(binding)?;
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
        SystemEventKind::OrderTransportStale => (
            AlertSeverity::Critical,
            "order_gateway",
            "order_transport_stale",
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
        | SystemEventKind::OrderTransportHeartbeat
        | SystemEventKind::OrderTransportRecovered
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
    coordinator: LiveCoordinator,
    composition: CompositionState,
    connectivity: ConnectivityState,
    dispatch: DispatchState,
    readiness_safety: ReadinessSafetyState,
    reconciliation: ReconciliationState,
    shutdown: ShutdownState,
}

impl LiveRuntime {
    async fn build(
        config: LiveConfig,
        attempt: LiveRunAttemptEvidence,
        connectivity_plan: ChaosConnectivityPlan,
        mode: LiveMode,
        run_duration: Option<Duration>,
    ) -> Result<Self, LiveRuntimeError> {
        validate_runtime_connectivity_plan(&config, &connectivity_plan, mode)?;
        let maintenance_relevance = connectivity_plan.maintenance_relevance().clone();
        let forbidden_policy = ForbiddenSentinelPolicy::from_plan(
            connectivity_plan.forbidden_proof_policy(),
            config.runtime.max_order_reconciliation_pages,
            config.runtime.pacing_policy(),
        )
        .map_err(LiveRuntimeError::Subscription)?;
        let planned_public_subscriptions = runtime_public_subscriptions(&connectivity_plan)?;
        let public_subscriptions = planned_public_subscriptions
            .iter()
            .map(|planned| planned.subscription.clone())
            .collect::<Vec<_>>();
        let public_plans = partition_subscriptions(
            &public_subscriptions,
            config.runtime.max_subscriptions_per_socket,
        )
        .map_err(|error| LiveRuntimeError::Subscription(error.to_string()))?;
        validate_public_socket_plans(&planned_public_subscriptions, &public_plans)?;
        let mut private_plans_by_account = private_socket_plans_by_account(&connectivity_plan)?;
        let mut order_session_counts = planned_order_session_counts(&connectivity_plan)?;
        let planned_order_accounts = order_session_counts.keys().cloned().collect::<HashSet<_>>();
        let LiveRunAttemptEvidence {
            session_started_at_ms,
            config_source,
            config_fingerprint,
            evidence_config_fingerprint,
            executable_sha256,
            host_identity_sha256,
        } = attempt;
        let mut storage_lease = acquire_storage_lease(&config.storage.path)?;
        let journal_path = storage_lease.journal_path().to_path_buf();
        let host_preflight = if config.host_guard.enabled {
            Some(check_host_health(&config.host_guard, &journal_path)?)
        } else {
            None
        };
        let connection_attempt_interval =
            Duration::from_millis(config.runtime.connection_attempt_interval_ms);
        let connection_attempt_pacer = match (
            &config.runtime.connection_attempt_pacer_path,
            connection_attempt_interval.is_zero(),
        ) {
            (Some(path), false) => {
                ConnectionAttemptPacer::process_shared(connection_attempt_interval, path)?
            }
            _ => ConnectionAttemptPacer::new(connection_attempt_interval),
        };
        let mut alert_runtime = alert_webhook_from_env(&config.alerts)?
            .map(start_webhook_alerts)
            .transpose()?;
        let alert_sink = alert_runtime.as_ref().map(AlertRuntime::sink);
        let alert_failures = alert_runtime.as_mut().map(AlertRuntime::take_failures);
        let operator_config = config.operator.clone();
        let operator_secret = operator_secret_from_env(&operator_config)?;
        let fill_convergence = FillConvergenceGuard::new(&config);
        let order_convergence =
            OrderStateConvergenceGuard::new(config.runtime.order_state_convergence_timeout_ms);
        let mut recovered = recover_leased_jsonl(&mut storage_lease)?;
        validate_recovered_safety_latches(&config, &recovered)?;
        let restored_safety_latches = recovered_safety_latch_count(&recovered);
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
        let recovered_orders = proven_active_recovered_orders(&config, &mut recovered);
        let mut restored_by_account: HashMap<String, Vec<reap_core::OrderUpdate>> = HashMap::new();
        for (update, _) in &recovered_orders {
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
        let (mut verified, mut seeds, snapshots) = bootstrap_accounts(
            &config,
            &restored_by_account,
            mode,
            &planned_order_accounts,
            &maintenance_relevance,
        )
        .await?;
        let mut approval_scopes = HashMap::new();
        for seed in &mut seeds {
            let Some(gateway) = seed.bound_order_gateway.as_mut() else {
                continue;
            };
            let scope =
                gateway
                    .take_approval_scope()
                    .map_err(|error| LiveRuntimeError::GatewaySetup {
                        account_id: seed.account_id.clone(),
                        message: format!("failed to take regular approval scope: {error}"),
                    })?;
            approval_scopes.insert(seed.account_id.clone(), scope);
        }
        let account_identity_sha256s = account_identity_sha256s(&config, &snapshots)?;
        let session_id = format!("{:x}", unix_time_ns());
        let mut startup_records = Vec::new();
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
                    fill_ids.insert(FillKey::new(fill.symbol.clone(), fill.fill_id.clone()));
                }
            }
            verified
                .baseline_fill_ids
                .insert(account.id.clone(), fill_ids);
            if !recovered.baseline_fill_ids.contains_key(&account.id) {
                let mut baseline_fill_ids = exchange_baseline.into_iter().collect::<Vec<_>>();
                baseline_fill_ids.sort();
                startup_records.push(StorageRecord::Bootstrap(BootstrapRecord {
                    ts_ms: unix_time_ms(),
                    account_id: account.id.clone(),
                    strategy_name: config.strategy.strategy_name.clone(),
                    config_fingerprint: config_fingerprint.clone(),
                    baseline_fill_ids,
                }));
            }
            let account_identity_sha256 = account_identity_sha256s
                .get(&account.id)
                .cloned()
                .ok_or_else(|| {
                    LiveRuntimeError::Provenance(format!(
                        "missing account identity for runtime session account {}",
                        account.id
                    ))
                })?;
            startup_records.push(StorageRecord::SessionStart(SessionStartRecord {
                ts_ms: session_started_at_ms,
                session_id: session_id.clone(),
                account_id: account.id.clone(),
                strategy_name: config.strategy.strategy_name.clone(),
                config_fingerprint: config_fingerprint.clone(),
                account_identity_sha256,
            }));
        }
        let mut coordinator = LiveCoordinator::new_with_order_transports(
            config.clone(),
            verified,
            approval_scopes,
            session_id.clone(),
        )?;
        // Apply recovered halt state before replaying anything that can produce an intent.
        // Reapplying it after reconciliation below generates cancels for restored live orders.
        let _ = restore_safety_latches(&mut coordinator, &recovered)?;
        let mut initial_outputs = vec![CoordinatorOutput {
            actions: Vec::new(),
            records: startup_records,
        }];
        for (update, proof) in recovered_orders {
            initial_outputs.push(coordinator.restore_owned_order(proof, update)?);
        }
        restore_active_order_bindings(&mut coordinator, &mut recovered)?;
        for account in &config.accounts {
            let snapshot = snapshots.get(&account.id).ok_or_else(|| {
                LiveRuntimeError::BootstrapVerification(format!(
                    "missing reconciliation snapshot for {}",
                    account.id
                ))
            })?;
            for fill in &snapshot.recent_fills {
                let should_apply = coordinator.private_state(&account.id).is_some_and(|state| {
                    let order_id =
                        state.resolve_order_id(&fill.client_order_id, &fill.exchange_order_id);
                    state.order_reducer().contains_order(&order_id)
                        && !state.has_seen_fill(&fill.symbol, &fill.fill_id)
                });
                if should_apply {
                    initial_outputs.push(coordinator.process_feed(FeedOutput::PrivateFill {
                        account_id: Some(account.id.clone()),
                        fill: fill.clone(),
                    })?);
                }
            }
            for remote in &snapshot.open_orders {
                let known = coordinator.private_state(&account.id).is_some_and(|state| {
                    let order_id =
                        state.resolve_order_id(&remote.client_order_id, &remote.exchange_order_id);
                    state.order_reducer().contains_order(&order_id)
                });
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
        let (forbidden_tx, forbidden_rx) = mpsc::channel(config.runtime.event_channel_capacity);
        let mut host_guard = config
            .host_guard
            .enabled
            .then(|| start_host_guard(config.host_guard.clone(), journal_path));
        let host_failures = host_guard.as_mut().map(HostGuardRuntime::take_failures);
        let mut feeds = Vec::new();
        let mut feed_tasks = StartupTaskGroup::default();
        let mut sources = Vec::new();

        let public_adapter: Arc<dyn VenueAdapter> = Arc::new(OkxAdapter::new(
            &config.venue.public_ws_url,
            &config.venue.private_ws_url,
        ));
        let _public_connection_capability = okx_capability_registration("OKX-CONNECTION-PUBLIC")
            .expect("live public connection must remain in the OKX capability registry");
        let mut public_feed = try_spawn_supervised_feed(
            Arc::clone(&public_adapter),
            public_plans.clone(),
            reap_feed::no_bootstrap(),
            config.runtime.feed_channel_capacity,
            connection_attempt_pacer.clone(),
            ReconnectPolicy::default(),
        )
        .map_err(|error| LiveRuntimeError::Subscription(error.to_string()))?;
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
        let mut order_tasks = StartupTaskGroup::default();
        let mut reconcile_senders = HashMap::new();
        let mut reconcile_tasks = StartupTaskGroup::default();
        let mut order_ws_runtimes = Vec::new();
        let mut order_ws_status_tasks = StartupTaskGroup::default();
        let mut safety_senders = HashMap::new();
        let mut safety_tasks = StartupTaskGroup::default();
        let mut forbidden_tasks = StartupTaskGroup::default();
        for (seed_index, seed) in seeds.into_iter().enumerate() {
            let AccountSeed {
                account_id,
                readiness,
                reconciliation,
                forbidden_observer,
                private_state_sessions,
                bound_order_gateway,
                safety,
                instrument_guard,
            } = seed;
            let private_plans = private_plans_by_account
                .remove(&account_id)
                .ok_or_else(|| {
                    LiveRuntimeError::Subscription(format!(
                        "connectivity plan has no private state session for account {account_id}"
                    ))
                })?;
            if private_plans.len() != 1 {
                return Err(LiveRuntimeError::Subscription(format!(
                    "connectivity plan must provide exactly one private state socket plan for account {account_id}, received {}",
                    private_plans.len()
                )));
            }
            let planned_session_count = order_session_counts.remove(&account_id);
            let mutation_role_count =
                usize::from(bound_order_gateway.is_some()) + usize::from(safety.is_some());
            let expected_mutation_role_count = if planned_session_count.is_some() {
                2
            } else {
                0
            };
            if mutation_role_count != expected_mutation_role_count {
                return Err(LiveRuntimeError::GatewaySetup {
                    account_id,
                    message: format!(
                        "planned order-lane authority requires exactly {expected_mutation_role_count} mutation roles, bootstrap produced {mutation_role_count}"
                    ),
                });
            }
            let deadman_timeout_secs = planned_session_count.and(
                safety
                    .as_ref()
                    .map(|_| config.runtime.cancel_all_after_timeout_secs),
            );
            if let (Some(timeout_secs), Some(safety)) = (deadman_timeout_secs, safety.as_ref()) {
                safety
                    .cancel_all_after(timeout_secs)
                    .await
                    .map_err(|error| LiveRuntimeError::GatewaySetup {
                        account_id: account_id.clone(),
                        message: format!("failed to arm Cancel All After: {error}"),
                    })?;
            }
            let (safety_tx, safety_rx) = mpsc::channel(8);
            safety_senders.insert(account_id.clone(), safety_tx);
            let expected_account_config = snapshots
                .get(&account_id)
                .expect("bootstrap snapshot must exist for every account seed")
                .account_config
                .clone();
            safety_tasks.push(tokio::spawn(run_account_safety_task(
                account_id.clone(),
                readiness,
                safety,
                expected_account_config,
                safety_rx,
                control_tx.clone(),
                deadman_timeout_secs,
                config.runtime.cancel_all_after_heartbeat_ms,
                config.runtime.exchange_clock_check_interval_ms,
                config.runtime.max_exchange_clock_skew_ms,
                ExchangeStatusGuard {
                    enabled: seed_index == 0,
                    relevance: maintenance_relevance.clone(),
                    check_interval_ms: config.runtime.exchange_status_check_interval_ms,
                    lead_ms: config.runtime.exchange_status_lead_ms,
                },
                instrument_guard,
            )));
            forbidden_tasks.push(tokio::spawn(run_forbidden_order_sentinel(
                account_id.clone(),
                Arc::new(forbidden_observer) as Arc<dyn ForbiddenOrderObserverPort>,
                forbidden_policy.clone(),
                forbidden_tx.clone(),
            )));
            let private_adapter: Arc<dyn VenueAdapter> = Arc::new(
                OkxAdapter::new(&config.venue.public_ws_url, &config.venue.private_ws_url)
                    .with_account_id(&account_id),
            );
            let _private_connection_capability =
                okx_capability_registration("OKX-CONNECTION-PRIVATE-STATE")
                    .expect("private state connection must remain in the OKX capability registry");
            let private_bootstrap = private_state_sessions
                .bootstrap_factory(
                    account_id.clone(),
                    private_plans[0].clone(),
                    &config.venue.private_ws_url,
                )
                .map_err(|error| LiveRuntimeError::GatewaySetup {
                    account_id: account_id.clone(),
                    message: format!(
                        "failed to bind private state bootstrap to its websocket destination: {error}"
                    ),
                })?;
            let mut private_feed = try_spawn_supervised_feed(
                Arc::clone(&private_adapter),
                private_plans.clone(),
                private_bootstrap,
                config.runtime.feed_channel_capacity,
                connection_attempt_pacer.clone(),
                ReconnectPolicy::default(),
            )
            .map_err(|error| LiveRuntimeError::Subscription(error.to_string()))?;
            let source_id = sources.len();
            sources.push(FeedSourceState::private(
                private_adapter,
                account_id.clone(),
                &private_plans,
            ));
            spawn_feed_forwarders(source_id, &mut private_feed, &feed_tx, &mut feed_tasks);
            feeds.push(private_feed);

            match (planned_session_count, bound_order_gateway) {
                (Some(session_count), Some(bound_order_gateway)) => {
                    if session_count != 1 {
                        return Err(LiveRuntimeError::GatewaySetup {
                            account_id: account_id.clone(),
                            message: format!(
                                "regular order command plan must contain exactly one session, found {session_count}"
                            ),
                        });
                    }
                    let order_ws_config = OrderCommandWebsocketConfig::new(
                        account_id.clone(),
                        config.venue.order_ws_url().to_string(),
                        config.runtime.order_channel_capacity,
                        Duration::from_millis(config.runtime.order_request_expiry_ms),
                        Duration::from_millis(config.runtime.order_websocket_ack_timeout_ms),
                        connection_attempt_pacer.clone(),
                        ReconnectPolicy::default(),
                    )
                    .map_err(|error| LiveRuntimeError::GatewaySetup {
                        account_id: account_id.clone(),
                        message: format!(
                            "invalid regular order command websocket configuration: {error}"
                        ),
                    })?;
                    let (gateway, order_ws_runtime, mut order_ws_status) = bound_order_gateway
                        .start_and_install(order_ws_config)
                        .map_err(|error| LiveRuntimeError::GatewaySetup {
                            account_id: account_id.clone(),
                            message: format!(
                                "failed to start and install regular order command websocket: {error}"
                            ),
                        })?;
                    let order_status_events = control_tx.clone();
                    order_ws_status_tasks.push(tokio::spawn(async move {
                        while let Some(status) = order_ws_status.recv().await {
                            if order_status_events
                                .send(RuntimeEvent::OrderTransport(status))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                    }));
                    order_ws_runtimes.push(order_ws_runtime);
                    let (order_tx, order_rx) = mpsc::channel(config.runtime.order_channel_capacity);
                    order_senders.insert(account_id.clone(), order_tx);
                    order_tasks.push(tokio::spawn(run_order_task(
                        account_id.clone(),
                        gateway,
                        order_rx,
                        control_tx.clone(),
                        session_count,
                        config.runtime.order_channel_capacity,
                    )));
                }
                (Some(_), _) => {
                    return Err(LiveRuntimeError::GatewaySetup {
                        account_id,
                        message: "planned regular order lane has no bound gateway authority"
                            .to_string(),
                    });
                }
                (None, _) => {}
            }

            let (reconcile_tx, reconcile_rx) = mpsc::channel(8);
            reconcile_senders.insert(account_id.clone(), reconcile_tx);
            reconcile_tasks.push(tokio::spawn(run_reconcile_task(
                account_id,
                reconciliation,
                reconcile_rx,
                control_tx.clone(),
                config.runtime.ambiguous_submit_grace_ms,
                config.runtime.max_order_reconciliation_pages,
                config.runtime.max_fill_reconciliation_pages,
            )));
        }
        if let Some(account_id) = private_plans_by_account.keys().next() {
            return Err(LiveRuntimeError::Subscription(format!(
                "connectivity plan private state session has no runtime account seed: {account_id}"
            )));
        }
        if let Some(account_id) = order_session_counts.keys().next() {
            return Err(LiveRuntimeError::Subscription(format!(
                "connectivity plan order command lane has no runtime account seed: {account_id}"
            )));
        }

        let mut runtime = Self {
            coordinator,
            composition: CompositionState {
                session_id,
                session_started_at_ms,
                config_source,
                config_fingerprint,
                evidence_config_fingerprint,
                executable_sha256,
                host_identity_sha256,
                account_identity_sha256s,
                mode,
                run_duration,
                storage: Some(storage),
                storage_sink,
                evidence: RuntimeEvidence::default(),
                latency: LiveLatencyCollector::default(),
            },
            connectivity: ConnectivityState {
                processor: FeedProcessor::new(
                    config.runtime.dedup_capacity_per_stream,
                    config.runtime.max_sequence_buffer,
                ),
                feed_rx,
                order_ws_runtimes,
                order_ws_status_tasks: order_ws_status_tasks.take(),
                feeds,
                feed_tasks: feed_tasks.take(),
                sources,
                public_feed_index,
                max_feed_age_ms: config.risk.max_feed_age_ms,
            },
            dispatch: DispatchState {
                control_rx,
                order_senders,
                order_tasks: order_tasks.take(),
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
            },
            readiness_safety: ReadinessSafetyState {
                forbidden_rx,
                safety_senders,
                safety_tasks: safety_tasks.take(),
                forbidden_tasks: forbidden_tasks.take(),
                readiness_timeout_ms: config.runtime.readiness_timeout_ms,
                timer_interval_ms: config.runtime.timer_interval_ms,
                host_guard,
                host_failures,
                host_checks: u64::from(host_preflight.is_some()),
                host_last_snapshot: host_preflight.clone(),
                host_preflight,
            },
            reconciliation: ReconciliationState {
                senders: reconcile_senders,
                tasks: reconcile_tasks.take(),
                inflight: HashSet::new(),
                cancel_inflight: HashSet::new(),
                last_attempt: HashMap::new(),
                fill_convergence: FillConvergenceGuard::default(),
                order_convergence,
            },
            shutdown: ShutdownState {
                timeout_ms: config.runtime.shutdown_timeout_ms,
                teardown_timeout_ms: config.runtime.teardown_timeout_ms,
                safety_latch_sync_timeout_ms: config.runtime.safety_latch_sync_timeout_ms,
                in_progress: false,
                storage_error: None,
                preserve_deadman: false,
                reconciliation_requested: HashSet::new(),
                reconciled_accounts: HashSet::new(),
            },
        };
        for output in initial_outputs {
            if let Err(primary) = runtime.commit_output(output).await {
                let context = format!("runtime initialization failure: {primary}");
                return Err(runtime.close_after_error(primary, &context).await);
            }
        }
        runtime
            .composition
            .evidence
            .begin_live_session(restored_safety_latches);
        runtime.reconciliation.fill_convergence = fill_convergence;
        if let Some(secret) = operator_secret {
            let (operator_tx, operator_rx) =
                mpsc::channel(operator_config.command_channel_capacity);
            match start_operator_service(&operator_config, secret, operator_tx).await {
                Ok(service) => {
                    runtime.dispatch.operator_service = Some(service);
                    runtime.dispatch.operator_rx = Some(operator_rx);
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
        let readiness_at_stop = self.coordinator.readiness();
        let elapsed_ms = unix_time_ms().saturating_sub(self.composition.session_started_at_ms);
        let stop_result = self.graceful_stop(context).await;
        let shutdown_result = self.shutdown().await;
        let mut additional = Vec::new();
        if let Err(error) = stop_result {
            additional.push(("fail-closed cleanup", error));
        }
        if let Err(error) = shutdown_result {
            additional.push(("runtime teardown", error));
        }
        let error = combine_lifecycle_errors(primary, additional);
        let reached_ready = readiness_at_stop.is_ready();
        let outcome = RunLoopOutcome {
            stop_reason: LiveStopReason::RuntimeFailure,
            elapsed_ms,
            reached_ready,
            time_to_ready_ms: reached_ready.then_some(elapsed_ms),
            readiness_loss_count: 0,
            max_readiness_outage_ms: 0,
            readiness_at_stop,
        };
        let report = self.build_report(outcome, Some(live_failure_evidence(&error)));
        LiveRuntimeError::ReportedFailure {
            source: Box::new(error),
            report: Box::new(report),
        }
    }

    async fn run(mut self) -> Result<LiveRunReport, LiveRuntimeError> {
        let loop_result = self.run_loop().await;
        let runtime_alert_result = match &loop_result {
            Ok(_) => Ok(()),
            Err(failure) => self.emit_runtime_failure_alert(&failure.error),
        };
        let stop_context = match &loop_result {
            Ok(outcome) => match outcome.stop_reason {
                LiveStopReason::OperatorSignal => "operator signal".to_string(),
                LiveStopReason::OperatorCommand => self
                    .dispatch
                    .operator_shutdown_reason
                    .clone()
                    .unwrap_or_else(|| "authenticated operator command".to_string()),
                LiveStopReason::DurationElapsed => "bounded duration elapsed".to_string(),
                LiveStopReason::ReadinessTimeout => "bounded readiness timeout".to_string(),
                LiveStopReason::Validation => "validation".to_string(),
                LiveStopReason::RuntimeFailure => "runtime failure".to_string(),
            },
            Err(failure) => format!("runtime failure: {}", failure.error),
        };
        let stop_result = self.graceful_stop(&stop_context).await;
        let shutdown_result = self.shutdown().await;
        let (mut outcome, failure) = match loop_result {
            Ok(outcome) => {
                let failure = match stop_result {
                    Ok(()) => shutdown_result.err(),
                    Err(primary) => {
                        let additional = shutdown_result
                            .err()
                            .map(|error| vec![("runtime teardown", error)])
                            .unwrap_or_default();
                        Some(combine_lifecycle_errors(primary, additional))
                    }
                };
                (outcome, failure)
            }
            Err(RunLoopFailure {
                error: primary,
                outcome,
            }) => {
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
                (outcome, Some(combine_lifecycle_errors(primary, additional)))
            }
        };
        if failure.is_some() {
            outcome.stop_reason = LiveStopReason::RuntimeFailure;
        }
        let failure_evidence = failure.as_ref().map(live_failure_evidence);
        let report = self.build_report(outcome, failure_evidence);
        match failure {
            Some(source) => Err(LiveRuntimeError::ReportedFailure {
                source: Box::new(source),
                report: Box::new(report),
            }),
            None => Ok(report),
        }
    }

    fn build_report(
        &self,
        outcome: RunLoopOutcome,
        failure: Option<LiveFailureEvidence>,
    ) -> LiveRunReport {
        let readiness = self.coordinator.readiness();
        let dropped_storage_records = self.composition.storage_sink.dropped_records();
        let active_orders_after_shutdown = self.coordinator.active_order_count();
        let evidence = self.composition.evidence;
        let clean_soak = qualifies_as_clean_soak(
            &outcome,
            evidence,
            dropped_storage_records,
            active_orders_after_shutdown,
            self.dispatch.alert_stats.failed,
        );
        LiveRunReport {
            schema_version: LIVE_RUN_REPORT_SCHEMA_VERSION,
            session_id: Some(self.composition.session_id.clone()),
            session_started_at_ms: self.composition.session_started_at_ms,
            config_source: self.composition.config_source.clone(),
            config_fingerprint: self.composition.config_fingerprint.clone(),
            evidence_config_fingerprint: self.composition.evidence_config_fingerprint.clone(),
            java_reference_revision: PINNED_JAVA_REVISION.to_string(),
            reap_version: env!("CARGO_PKG_VERSION").to_string(),
            executable_sha256: self.composition.executable_sha256.clone(),
            host_identity_sha256: self.composition.host_identity_sha256.clone(),
            account_identity_sha256s: self.composition.account_identity_sha256s.clone(),
            mode: self.composition.mode,
            stop_reason: outcome.stop_reason,
            failure,
            elapsed_ms: outcome.elapsed_ms,
            reached_ready: outcome.reached_ready,
            time_to_ready_ms: outcome.time_to_ready_ms,
            readiness_loss_count: outcome.readiness_loss_count,
            max_readiness_outage_ms: outcome.max_readiness_outage_ms,
            reconciliation_drift_events: evidence.reconciliation_drift_events,
            book_recovery_events: evidence.book_recovery_events,
            stream_stale_events: evidence.stream_stale_events,
            connection_disconnect_events: evidence.connection_disconnect_events,
            public_connection_disconnect_events: evidence.public_connection_disconnect_events,
            private_connection_disconnect_events: evidence.private_connection_disconnect_events,
            order_transport_disconnect_events: evidence.order_transport_disconnect_events,
            order_transport_stale_events: evidence.order_transport_stale_events,
            ambiguous_submit_events: evidence.ambiguous_submit_events,
            ambiguous_cancel_events: evidence.ambiguous_cancel_events,
            partial_fill_events: evidence.partial_fill_events,
            fill_convergence_timeout_events: evidence.fill_convergence_timeout_events,
            order_convergence_timeout_events: evidence.order_convergence_timeout_events,
            restored_safety_latches: evidence.restored_safety_latches,
            operator_commands: evidence.operator_commands,
            operator_mutations: evidence.operator_mutations,
            max_storage_queue_depth: evidence.max_storage_queue_depth,
            alerts_delivered: self.dispatch.alert_stats.delivered,
            alert_delivery_failures: self.dispatch.alert_stats.failed,
            alert_failure_notifications_dropped: self
                .dispatch
                .alert_stats
                .failure_notifications_dropped,
            max_alert_queue_depth: self.dispatch.alert_stats.max_queue_depth,
            host_preflight: self.readiness_safety.host_preflight.clone(),
            host_checks: self.readiness_safety.host_checks,
            host_last_snapshot: self.readiness_safety.host_last_snapshot.clone(),
            readiness_at_stop: outcome.readiness_at_stop,
            readiness,
            dropped_storage_records,
            active_orders_after_shutdown,
            latency_evidence: self.composition.latency.report(),
            clean_soak,
        }
    }

    async fn run_loop(&mut self) -> Result<RunLoopOutcome, RunLoopFailure> {
        let started = Instant::now();
        let mut readiness_tracker = ReadinessTracker::default();
        let initial_readiness = self.coordinator.readiness();
        readiness_tracker.observe(0, &initial_readiness);
        let mut last_phase = initial_readiness.phase;
        let mut timer = tokio::time::interval(Duration::from_millis(
            self.readiness_safety.timer_interval_ms,
        ));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let shutdown = shutdown_signal();
        tokio::pin!(shutdown);
        let run_duration = self.composition.run_duration;
        let duration_elapsed = async move {
            match run_duration {
                Some(duration) => tokio::time::sleep(duration).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::pin!(duration_elapsed);

        loop {
            let iteration: Result<Option<RunLoopOutcome>, LiveRuntimeError> = async {
                tokio::select! {
                    biased;
                    signal = &mut shutdown => {
                        signal?;
                        self.drain_queued_events().await?;
                        Ok(Some(readiness_tracker.finish(
                            LiveStopReason::OperatorSignal,
                            elapsed_ms(&started),
                            self.coordinator.readiness(),
                        )))
                    }
                    _ = &mut duration_elapsed => {
                        self.drain_queued_events().await?;
                        Ok(Some(readiness_tracker.finish(
                            LiveStopReason::DurationElapsed,
                            elapsed_ms(&started),
                            self.coordinator.readiness(),
                        )))
                    }
                    failure = receive_alert_failure(&mut self.dispatch.alert_failures) => {
                        let failure = failure.ok_or(LiveRuntimeError::AlertMonitorClosed)?;
                        self.dispatch.observed_alert_delivery_failures = self
                            .dispatch
                            .observed_alert_delivery_failures
                            .saturating_add(1);
                        tracing::error!(
                            event_id = %failure.event_id,
                            code = %failure.code,
                            attempts = failure.attempts,
                            reason = %failure.reason,
                            "external alert delivery failed"
                        );
                        if self.dispatch.alert_delivery_failure_is_fatal {
                            Err(LiveRuntimeError::AlertDelivery {
                                code: failure.code,
                                attempts: failure.attempts,
                                reason: failure.reason,
                            })
                        } else {
                            Ok(None)
                        }
                    }
                    failure = receive_host_failure(&mut self.readiness_safety.host_failures) => {
                        let failure = failure.ok_or(LiveRuntimeError::HostGuardClosed)?;
                        Err(failure.into())
                    }
                    event = self.dispatch.control_rx.recv() => {
                        let event = event.ok_or(LiveRuntimeError::EventChannelClosed)?;
                        self.handle_runtime_event(event).await?;
                        Ok(None)
                    }
                    event = self.readiness_safety.forbidden_rx.recv() => {
                        let event = event.ok_or(LiveRuntimeError::EventChannelClosed)?;
                        self.handle_forbidden_order_event(event).await?;
                        Ok(None)
                    }
                    operator = receive_operator(&mut self.dispatch.operator_rx) => {
                        let operator = operator.ok_or(LiveRuntimeError::OperatorChannelClosed)?;
                        self.handle_operator_envelope(operator).await?;
                        Ok(None)
                    }
                    _ = timer.tick() => {
                        let now_ms = unix_time_ms();
                        for event in self.connectivity.processor.mark_stale(
                            now_ms,
                            self.coordinator_risk_max_feed_age(),
                        ) {
                            let output = self.coordinator.process_event(NormalizedEvent::System(event));
                            self.commit_output(output).await?;
                        }
                        for breach in self.reconciliation.fill_convergence.expire(now_ms) {
                            self.composition.evidence.observe_fill_convergence_timeout();
                            let output = self.coordinator.reconciliation_fault(
                                &breach.account_id,
                                now_ms,
                                breach.symbol,
                                breach.reason,
                            )?;
                            self.commit_output(output).await?;
                        }
                        for breach in self.reconciliation.order_convergence.expire(now_ms) {
                            self.composition.evidence.observe_order_convergence_timeout();
                            for order_id in &breach.expired_cancel_order_ids {
                                self.reconciliation.cancel_inflight
                                    .remove(&(breach.account_id.clone(), order_id.clone()));
                            }
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
                        Ok(None)
                    }
                    event = self.connectivity.feed_rx.recv() => {
                        let event = event.ok_or(LiveRuntimeError::EventChannelClosed)?;
                        self.handle_runtime_event(event).await?;
                        Ok(None)
                    }
                }
            }
            .await;

            match iteration {
                Ok(Some(outcome)) => return Ok(outcome),
                Ok(None) => {}
                Err(error) => {
                    return Err(RunLoopFailure {
                        error,
                        outcome: readiness_tracker.finish(
                            LiveStopReason::RuntimeFailure,
                            elapsed_ms(&started),
                            self.coordinator.readiness(),
                        ),
                    });
                }
            }

            let readiness = self.coordinator.readiness();
            let elapsed_ms = elapsed_ms(&started);
            readiness_tracker.observe(elapsed_ms, &readiness);
            if readiness.phase != last_phase {
                tracing::info!(from = ?last_phase, to = ?readiness.phase, ?readiness, "live readiness changed");
                last_phase = readiness.phase;
            }
            if self.dispatch.operator_shutdown_reason.is_some() {
                return Ok(readiness_tracker.finish(
                    LiveStopReason::OperatorCommand,
                    elapsed_ms,
                    readiness,
                ));
            }
            if !readiness_tracker.reached_ready
                && started.elapsed()
                    > Duration::from_millis(self.readiness_safety.readiness_timeout_ms)
            {
                if self.composition.run_duration.is_some() {
                    let outcome = readiness_tracker.finish(
                        LiveStopReason::ReadinessTimeout,
                        elapsed_ms,
                        readiness,
                    );
                    return Ok(outcome);
                }
                return Err(RunLoopFailure {
                    error: LiveRuntimeError::ReadinessTimeout(
                        self.readiness_safety.readiness_timeout_ms,
                    ),
                    outcome: readiness_tracker.finish(
                        LiveStopReason::RuntimeFailure,
                        elapsed_ms,
                        readiness,
                    ),
                });
            }
        }
    }

    fn coordinator_risk_max_feed_age(&self) -> u64 {
        self.connectivity.max_feed_age_ms
    }

    async fn graceful_stop(&mut self, reason: &str) -> Result<(), LiveRuntimeError> {
        if self.composition.mode != LiveMode::Demo {
            return Ok(());
        }
        self.shutdown.in_progress = true;
        let result = match tokio::time::timeout(
            Duration::from_millis(self.shutdown.timeout_ms),
            self.graceful_stop_inner(reason),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(self.shutdown_unresolved_error()),
        };
        match (result, self.shutdown.storage_error.take()) {
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
                && self.reconciliation.inflight.is_empty()
                && self.shutdown.reconciled_accounts.len() == self.dispatch.order_senders.len()
            {
                if !self.shutdown.preserve_deadman {
                    self.disable_deadman_all().await?;
                }
                return Ok(());
            }
            tokio::select! {
                biased;
                event = self.dispatch.control_rx.recv(), if !self.dispatch.control_rx.is_closed() => {
                    if let Some(event) = event {
                        self.handle_runtime_event(event).await?;
                    }
                }
                event = self.readiness_safety.forbidden_rx.recv(), if !self.readiness_safety.forbidden_rx.is_closed() => {
                    if let Some(event) = event {
                        self.handle_forbidden_order_event(event).await?;
                    }
                }
                event = self.connectivity.feed_rx.recv(), if !self.connectivity.feed_rx.is_closed() => {
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
                .chain(self.reconciliation.inflight.iter().cloned())
                .chain(
                    self.dispatch
                        .order_senders
                        .keys()
                        .filter(|account_id| {
                            !self.shutdown.reconciled_accounts.contains(*account_id)
                        })
                        .cloned(),
                )
                .collect::<HashSet<_>>()
                .len(),
        }
    }

    async fn drain_shutdown_events(&mut self) -> Result<(), LiveRuntimeError> {
        let pending_control = self.dispatch.control_rx.len();
        for _ in 0..pending_control {
            match self.dispatch.control_rx.try_recv() {
                Ok(event) => self.handle_runtime_event(event).await?,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        let pending_forbidden = self.readiness_safety.forbidden_rx.len();
        for _ in 0..pending_forbidden {
            match self.readiness_safety.forbidden_rx.try_recv() {
                Ok(event) => self.handle_forbidden_order_event(event).await?,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        let pending_feed = self.connectivity.feed_rx.len();
        for _ in 0..pending_feed {
            match self.connectivity.feed_rx.try_recv() {
                Ok(event) => self.handle_runtime_event(event).await?,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        Ok(())
    }

    async fn drain_queued_events(&mut self) -> Result<(), LiveRuntimeError> {
        let pending_control = self.dispatch.control_rx.len();
        for _ in 0..pending_control {
            match self.dispatch.control_rx.try_recv() {
                Ok(event) => self.handle_runtime_event(event).await?,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return Err(LiveRuntimeError::EventChannelClosed);
                }
            }
        }
        let pending_forbidden = self.readiness_safety.forbidden_rx.len();
        for _ in 0..pending_forbidden {
            match self.readiness_safety.forbidden_rx.try_recv() {
                Ok(event) => self.handle_forbidden_order_event(event).await?,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return Err(LiveRuntimeError::EventChannelClosed);
                }
            }
        }
        let pending_feed = self.connectivity.feed_rx.len();
        for _ in 0..pending_feed {
            match self.connectivity.feed_rx.try_recv() {
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
        self.composition.evidence.operator_commands = self
            .composition
            .evidence
            .operator_commands
            .saturating_add(1);
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
                self.composition.evidence.operator_mutations = self
                    .composition
                    .evidence
                    .operator_mutations
                    .saturating_add(1);
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
                self.composition.evidence.operator_mutations = self
                    .composition
                    .evidence
                    .operator_mutations
                    .saturating_add(1);
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
                self.composition.evidence.operator_mutations = self
                    .composition
                    .evidence
                    .operator_mutations
                    .saturating_add(1);
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
                self.composition.evidence.operator_mutations = self
                    .composition
                    .evidence
                    .operator_mutations
                    .saturating_add(1);
                Ok(OperatorResponse::accepted(
                    request_id,
                    "symbol resumed",
                    Some(self.operator_status()),
                ))
            }
            OperatorCommand::Shutdown { reason } => {
                self.coordinator.set_order_entry_enabled(false);
                self.composition.evidence.operator_mutations = self
                    .composition
                    .evidence
                    .operator_mutations
                    .saturating_add(1);
                self.dispatch.operator_shutdown_reason = Some(format!(
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
            shutdown_in_progress: self.shutdown.in_progress
                || self.dispatch.operator_shutdown_reason.is_some(),
        }
    }

    async fn handle_forbidden_order_event(
        &mut self,
        mut event: ForbiddenOrderEvent,
    ) -> Result<(), LiveRuntimeError> {
        event.expire_delayed_zero_proof(unix_time_ms());
        let alert = event.state.alert_code().map(|code| {
            let reason = event
                .state
                .failure_reason()
                .expect("nonzero forbidden state must have a failure reason");
            let mut alert = AlertEvent::new(
                AlertSeverity::Critical,
                "forbidden_order_sentinel",
                code,
                format!(
                    "account {}: {reason}; run the separate reap-emergency executable",
                    event.account_id
                ),
            )
            .with_attribute("account_id", &event.account_id);
            alert.ts_ms = event.observed_at_ms;
            alert
        });
        let output = self.coordinator.on_forbidden_order_event(event)?;
        // Canonical regular cancellation/reconciliation dispatch stays ahead of
        // telemetry work when the proof becomes invalid.
        self.commit_output(output).await?;
        if let Some(alert) = alert {
            self.emit_alert(alert)?;
        }
        Ok(())
    }

    async fn handle_runtime_event(&mut self, event: RuntimeEvent) -> Result<(), LiveRuntimeError> {
        match event {
            RuntimeEvent::Raw {
                source_id,
                envelope,
            } => {
                let (account_id, adapter, private_source) = {
                    let source = self.connectivity.sources.get(source_id).ok_or_else(|| {
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
                    for output in self
                        .connectivity
                        .processor
                        .process_from(&envelope.conn_id, event)
                    {
                        let strategy_visible_ns = unix_time_ns();
                        self.observe_feed_latency(
                            &output,
                            envelope.recv_ts_ns,
                            strategy_visible_ns,
                        );
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
                        let output = self
                            .coordinator
                            .process_feed_at(output, envelope.recv_ts_ns / 1_000_000)?;
                        let convergence_visible_ns = unix_time_ns();
                        if let Some(account_id) = private_account_id {
                            self.observe_account_convergence(
                                &account_id,
                                &output,
                                convergence_visible_ns,
                            );
                        }
                        if private_order_event {
                            self.observe_fill_convergence(&output, convergence_visible_ns, true);
                        }
                        self.commit_output(output).await?;
                    }
                }
                if private_state_frame {
                    let events = self
                        .connectivity
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
                if status.kind == ConnectionStatusKind::Fatal {
                    return Err(LiveRuntimeError::ConnectionPacerRuntime(format!(
                        "{}: {}",
                        status.conn_id, status.reason
                    )));
                }
                if status.kind == ConnectionStatusKind::Disconnected {
                    self.composition.evidence.observe_disconnect(status.private);
                }
                let (events, public_connectivity) = {
                    let source = self
                        .connectivity
                        .sources
                        .get_mut(source_id)
                        .ok_or_else(|| {
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
            RuntimeEvent::OrderTransport(status) => {
                let kind = match status.kind {
                    OrderCommandWebsocketStatusKind::Ready => {
                        SystemEventKind::OrderTransportRecovered
                    }
                    OrderCommandWebsocketStatusKind::Heartbeat => {
                        SystemEventKind::OrderTransportHeartbeat
                    }
                    OrderCommandWebsocketStatusKind::Disconnected => {
                        self.composition
                            .evidence
                            .observe_order_transport_disconnect();
                        SystemEventKind::OrderTransportStale
                    }
                    OrderCommandWebsocketStatusKind::Fatal => {
                        return Err(LiveRuntimeError::ConnectionPacerRuntime(status.reason));
                    }
                };
                let output = self
                    .coordinator
                    .process_event(NormalizedEvent::System(SystemEvent {
                        ts_ms: status.ts_ms,
                        kind,
                        venue: Some(Venue::Okx),
                        account_id: Some(status.account_id),
                        symbol: None,
                        reason: format!(
                            "{} ({}/{} sessions ready)",
                            status.reason, status.ready_sessions, status.total_sessions
                        ),
                    }));
                self.commit_output(output).await?;
            }
            RuntimeEvent::SubmitComplete {
                account_id,
                symbol,
                outcome,
                ts_ms,
                latency_us,
            } => {
                if let Some(latency_us) = latency_us {
                    self.composition.latency.observe_us(
                        BacktestLatencyClass::MatchingNew,
                        &symbol,
                        LiveLatencySemantics::StrategyDispatchToOrderAckUpperBound,
                        latency_us,
                    );
                }
                let output = self
                    .coordinator
                    .on_submit_outcome(&account_id, outcome, ts_ms)?;
                self.commit_output(output).await?;
            }
            RuntimeEvent::SubmitFailed {
                account_id,
                client_order_id,
                symbol,
                ts_ms,
                ambiguous,
                reason,
            } => {
                self.composition.latency.observe_operation_failure(
                    BacktestLatencyClass::MatchingNew,
                    &symbol,
                    LiveLatencySemantics::StrategyDispatchToOrderAckUpperBound,
                );
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
                symbol,
                outcome,
                ts_ms,
                latency_us,
            } => {
                self.composition.latency.observe_us(
                    BacktestLatencyClass::MatchingCancel,
                    &symbol,
                    LiveLatencySemantics::StrategyDispatchToOrderAckUpperBound,
                    latency_us,
                );
                let output = self
                    .coordinator
                    .on_cancel_outcome(&account_id, outcome, ts_ms)?;
                self.commit_output(output).await?;
            }
            RuntimeEvent::CancelFailed {
                account_id,
                client_order_id,
                symbol,
                ts_ms,
                ambiguous,
                reason,
            } => {
                self.composition.latency.observe_operation_failure(
                    BacktestLatencyClass::MatchingCancel,
                    &symbol,
                    LiveLatencySemantics::StrategyDispatchToOrderAckUpperBound,
                );
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
                self.reconciliation.inflight.remove(&account_id);
                self.apply_remote_recovery(&account_id, &remote_orders, &remote_fills)
                    .await?;
                let order_convergence = &self.reconciliation.order_convergence;
                self.reconciliation
                    .cancel_inflight
                    .retain(|(cancel_account, order_id)| {
                        cancel_account != &account_id
                            || order_convergence.has_pending_cancel(cancel_account, order_id)
                    });
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
                let censored_fill_latencies = self
                    .reconciliation
                    .fill_convergence
                    .observe_authoritative(&account_id, remote_account_ts_ms);
                self.composition
                    .latency
                    .observe_dropped_observations(censored_fill_latencies as u64);
                self.commit_output(account_output).await?;
                let pending_order_state = self
                    .reconciliation
                    .order_convergence
                    .pending_reason(&account_id);
                let clean = report.is_clean() && pending_order_state.is_none();
                if self.shutdown.in_progress
                    && self.shutdown.reconciliation_requested.contains(&account_id)
                {
                    if is_zero_order_reconciliation(&report) {
                        self.shutdown.reconciled_accounts.insert(account_id.clone());
                    } else {
                        self.shutdown.reconciled_accounts.remove(&account_id);
                    }
                }
                let reason = if clean {
                    "REST orders, fills, balances, positions, and canonical private state agree"
                        .to_string()
                } else if report.is_clean() {
                    pending_order_state
                        .expect("non-clean order convergence must include a pending reason")
                } else {
                    let mut reason = format!("{:?}", report.issues);
                    if let Some(pending) = pending_order_state {
                        reason.push_str("; ");
                        reason.push_str(&pending);
                    }
                    reason
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
                self.reconciliation.inflight.remove(&account_id);
                self.shutdown.reconciled_accounts.remove(&account_id);
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
            RuntimeEvent::Fatal(failure) => return Err(failure.into()),
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

    fn observe_feed_latency(
        &mut self,
        output: &FeedOutput,
        received_ns: u64,
        strategy_visible_ns: u64,
    ) {
        match output {
            FeedOutput::Event(NormalizedEvent::Market(event)) => {
                let class = match event {
                    MarketEvent::Depth(_) => BacktestLatencyClass::MarketDepth,
                    MarketEvent::Trade { .. } => BacktestLatencyClass::HistoricalTrade,
                    MarketEvent::IndexPrice { .. }
                    | MarketEvent::FundingRate { .. }
                    | MarketEvent::BurstSignal { .. }
                    | MarketEvent::PriceLimits { .. } => BacktestLatencyClass::ReferenceData,
                };
                self.composition.latency.observe_ns(
                    class,
                    event.symbol(),
                    LiveLatencySemantics::HostReceiveToStrategyVisibility,
                    received_ns,
                    strategy_visible_ns,
                );
            }
            FeedOutput::PrivateOrder { update, .. } => {
                self.composition.latency.observe_exchange_ms(
                    BacktestLatencyClass::OrderUpdate,
                    &update.symbol,
                    update.ts_ms,
                    strategy_visible_ns,
                );
            }
            FeedOutput::PrivateFill { fill, .. } => {
                self.composition.latency.observe_exchange_ms(
                    BacktestLatencyClass::OrderUpdate,
                    &fill.symbol,
                    fill.ts_ms,
                    strategy_visible_ns,
                );
            }
            FeedOutput::Event(_)
            | FeedOutput::PrivateAccount { .. }
            | FeedOutput::Duplicate(_)
            | FeedOutput::RecoveryRequired(_)
            | FeedOutput::System(_) => {}
        }
    }

    fn observe_account_convergence(
        &mut self,
        account_id: &str,
        output: &CoordinatorOutput,
        observed_ns: u64,
    ) {
        for record in &output.records {
            if let StorageRecord::Normalized(NormalizedEvent::Account(update)) = record {
                let result = self.reconciliation.fill_convergence.observe_account_at(
                    account_id,
                    update,
                    observed_ns / 1_000_000,
                    observed_ns,
                );
                for observation in result.observations {
                    self.composition.latency.observe_ns(
                        BacktestLatencyClass::OrderFill,
                        &observation.symbol,
                        LiveLatencySemantics::FillToAccountStateVisibility,
                        observation.first_observed_ns,
                        observation.state_visible_ns,
                    );
                }
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
            let should_apply = self
                .coordinator
                .private_state(account_id)
                .is_some_and(|state| {
                    let order_id =
                        state.resolve_order_id(&fill.client_order_id, &fill.exchange_order_id);
                    state.order_reducer().contains_order(&order_id)
                        && !state.has_seen_fill(&fill.symbol, &fill.fill_id)
                });
            if should_apply {
                let output = self.coordinator.process_feed(FeedOutput::PrivateFill {
                    account_id: Some(account_id.to_string()),
                    fill: fill.clone(),
                })?;
                self.observe_fill_convergence(&output, unix_time_ns(), false);
                self.commit_output(output).await?;
            }
        }
        for remote in remote_orders {
            let known = self
                .coordinator
                .private_state(account_id)
                .is_some_and(|state| {
                    let order_id =
                        state.resolve_order_id(&remote.client_order_id, &remote.exchange_order_id);
                    state.order_reducer().contains_order(&order_id)
                });
            if known {
                let output = self.coordinator.process_feed(FeedOutput::PrivateOrder {
                    account_id: Some(account_id.to_string()),
                    update: private_update_from_remote(remote.clone()),
                })?;
                self.observe_fill_convergence(&output, unix_time_ns(), false);
                self.commit_output(output).await?;
            }
        }
        Ok(())
    }

    async fn commit_output(&mut self, output: CoordinatorOutput) -> Result<(), LiveRuntimeError> {
        self.observe_order_convergence(&output, unix_time_ms());
        let alerts = if self.dispatch.alert_sink.is_some() {
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

    fn observe_fill_convergence(
        &mut self,
        output: &CoordinatorOutput,
        observed_ns: u64,
        collect_latency: bool,
    ) {
        for record in &output.records {
            if let StorageRecord::Order {
                account_id: Some(account_id),
                update,
            } = record
            {
                let result = self.reconciliation.fill_convergence.observe_fill_at(
                    account_id,
                    update,
                    observed_ns / 1_000_000,
                    observed_ns,
                    collect_latency,
                );
                if collect_latency {
                    if result.dropped_latency_observation {
                        self.composition.latency.observe_dropped_observation();
                    }
                    for observation in result.observations {
                        self.composition.latency.observe_ns(
                            BacktestLatencyClass::OrderFill,
                            &observation.symbol,
                            LiveLatencySemantics::FillToAccountStateVisibility,
                            observation.first_observed_ns,
                            observation.state_visible_ns,
                        );
                    }
                }
            }
        }
    }

    fn observe_order_convergence(&mut self, output: &CoordinatorOutput, observed_ms: u64) {
        for record in &output.records {
            if let StorageRecord::Order {
                account_id: Some(account_id),
                update,
            } = record
            {
                self.reconciliation.order_convergence.observe_order(
                    account_id,
                    update,
                    observed_ms,
                );
                if matches!(
                    update.status,
                    OrderStatus::Filled | OrderStatus::Cancelled | OrderStatus::Rejected
                ) {
                    self.reconciliation
                        .cancel_inflight
                        .remove(&(account_id.clone(), update.order_id.clone()));
                }
            }
        }
    }

    fn emit_alert(&self, alert: AlertEvent) -> Result<(), LiveRuntimeError> {
        if let Some(sink) = &self.dispatch.alert_sink {
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
        self.observe_order_convergence(&output, unix_time_ms());
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
        tracing::debug!(
            account_id = action.account_id(),
            symbol = action.symbol(),
            client_order_id = action.client_order_id(),
            reason = action.reason(),
            "dispatching approved shutdown cancellation"
        );
        let enqueued_at = Instant::now();
        let cancel_key = (
            action.account_id().to_string(),
            action.client_order_id().to_string(),
        );
        let symbol = action.symbol().to_string();
        if !self
            .reconciliation
            .cancel_inflight
            .insert(cancel_key.clone())
        {
            return Ok(());
        }
        if let Err(error) = self.record_storage(StorageRecord::OrderRequest(OrderRequestRecord {
            ts_ms: action.ts_ms(),
            account_id: action.account_id().to_string(),
            operation: OrderOperation::Cancel,
            idempotency_key: None,
            client_order_id: Some(action.client_order_id().to_string()),
            exchange_order_id: None,
            symbol: action.symbol().to_string(),
        })) {
            self.reconciliation.cancel_inflight.remove(&cancel_key);
            return Err(error);
        }
        let sender = match self.order_sender(action.account_id()) {
            Ok(sender) => sender.clone(),
            Err(error) => {
                self.reconciliation.cancel_inflight.remove(&cancel_key);
                return Err(error);
            }
        };
        if sender
            .send(OrderTaskCommand::Cancel {
                action,
                enqueued_at,
            })
            .await
            .is_err()
        {
            self.reconciliation.cancel_inflight.remove(&cancel_key);
            return Err(LiveRuntimeError::OrderQueueUnavailable(cancel_key.0));
        }
        self.reconciliation.order_convergence.observe_cancel(
            &cancel_key.0,
            &cancel_key.1,
            &symbol,
            unix_time_ms(),
        );
        Ok(())
    }

    fn record_storage(&mut self, record: StorageRecord) -> Result<(), LiveRuntimeError> {
        self.composition.evidence.observe_record(&record);
        if let Err(error) = self.composition.storage_sink.try_record(record) {
            if !self.shutdown.in_progress {
                return Err(error.into());
            }
            tracing::error!(%error, "storage unavailable during fail-closed shutdown");
            self.shutdown
                .storage_error
                .get_or_insert_with(|| error.to_string());
            return Ok(());
        }
        self.composition.evidence.max_storage_queue_depth = self
            .composition
            .evidence
            .max_storage_queue_depth
            .max(self.composition.storage_sink.queue_depth());
        Ok(())
    }

    async fn record_durable_storage(
        &mut self,
        record: StorageRecord,
    ) -> Result<(), LiveRuntimeError> {
        self.composition.evidence.observe_record(&record);
        self.composition.evidence.max_storage_queue_depth =
            self.composition.evidence.max_storage_queue_depth.max(
                self.composition
                    .storage_sink
                    .queue_depth()
                    .saturating_add(1),
            );
        let result = tokio::time::timeout(
            Duration::from_millis(self.shutdown.safety_latch_sync_timeout_ms),
            self.composition.storage_sink.record_durable(record),
        )
        .await;
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                self.shutdown.preserve_deadman = true;
                Err(error.into())
            }
            Err(_) => {
                self.shutdown.preserve_deadman = true;
                Err(LiveRuntimeError::SafetyLatchSyncTimeout(
                    self.shutdown.safety_latch_sync_timeout_ms,
                ))
            }
        }
    }

    fn dispatch_action(&mut self, action: LiveAction) -> Result<(), LiveRuntimeError> {
        match action {
            LiveAction::Submit(action) => {
                let enqueued_at = Instant::now();
                self.record_storage(StorageRecord::OrderRequest(OrderRequestRecord {
                    ts_ms: action.ts_ms(),
                    account_id: action.account_id().to_string(),
                    operation: OrderOperation::Submit,
                    idempotency_key: Some(action.idempotency_key().to_string()),
                    client_order_id: Some(action.client_order_id().to_string()),
                    exchange_order_id: None,
                    symbol: action.order().symbol.clone(),
                }))?;
                self.order_sender(action.account_id())?
                    .try_send(OrderTaskCommand::Submit {
                        action,
                        enqueued_at,
                    })
                    .map_err(|_| {
                        LiveRuntimeError::OrderQueueUnavailable("submit account queue".to_string())
                    })?;
            }
            LiveAction::Cancel(action) => {
                let enqueued_at = Instant::now();
                let cancel_key = (
                    action.account_id().to_string(),
                    action.client_order_id().to_string(),
                );
                let symbol = action.symbol().to_string();
                if !self
                    .reconciliation
                    .cancel_inflight
                    .insert(cancel_key.clone())
                {
                    return Ok(());
                }
                if let Err(error) =
                    self.record_storage(StorageRecord::OrderRequest(OrderRequestRecord {
                        ts_ms: action.ts_ms(),
                        account_id: action.account_id().to_string(),
                        operation: OrderOperation::Cancel,
                        idempotency_key: None,
                        client_order_id: Some(action.client_order_id().to_string()),
                        exchange_order_id: None,
                        symbol: action.symbol().to_string(),
                    }))
                {
                    self.reconciliation.cancel_inflight.remove(&cancel_key);
                    return Err(error);
                }
                let sender = match self.order_sender(action.account_id()) {
                    Ok(sender) => sender.clone(),
                    Err(error) => {
                        self.reconciliation.cancel_inflight.remove(&cancel_key);
                        return Err(error);
                    }
                };
                if sender
                    .try_send(OrderTaskCommand::Cancel {
                        action,
                        enqueued_at,
                    })
                    .is_err()
                {
                    self.reconciliation.cancel_inflight.remove(&cancel_key);
                    return Err(LiveRuntimeError::OrderQueueUnavailable(cancel_key.0));
                }
                self.reconciliation.order_convergence.observe_cancel(
                    &cancel_key.0,
                    &cancel_key.1,
                    &symbol,
                    unix_time_ms(),
                );
            }
            LiveAction::RecoverBook(request) => {
                let routes = self.connectivity.feeds[self.connectivity.public_feed_index]
                    .request_recovery(&request);
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
        tracing::debug!(
            account_id = action.account_id,
            requested_at_ms = action.ts_ms,
            reason = action.reason,
            "dispatching reconciliation"
        );
        if !self
            .reconciliation
            .inflight
            .insert(action.account_id.clone())
        {
            return Ok(());
        }
        self.reconciliation
            .last_attempt
            .insert(action.account_id.clone(), Instant::now());
        let orders = match self.reconciliation_order_refs(&action.account_id) {
            Ok(orders) => orders,
            Err(error) => {
                self.reconciliation.inflight.remove(&action.account_id);
                return Err(error);
            }
        };
        let sender = match self.reconcile_sender(&action.account_id) {
            Ok(sender) => sender.clone(),
            Err(error) => {
                self.reconciliation.inflight.remove(&action.account_id);
                return Err(error);
            }
        };
        sender
            .try_send(ReconcileTaskCommand::Reconcile {
                restored_orders: orders,
                command_flush: None,
            })
            .map_err(|_| {
                self.reconciliation.inflight.remove(&action.account_id);
                LiveRuntimeError::OrderQueueUnavailable(action.account_id)
            })
    }

    async fn dispatch_shutdown_reconcile(
        &mut self,
        action: ReconcileAction,
    ) -> Result<(), LiveRuntimeError> {
        tracing::debug!(
            account_id = action.account_id,
            requested_at_ms = action.ts_ms,
            reason = action.reason,
            "dispatching shutdown reconciliation"
        );
        if !self
            .reconciliation
            .inflight
            .insert(action.account_id.clone())
        {
            return Ok(());
        }
        self.reconciliation
            .last_attempt
            .insert(action.account_id.clone(), Instant::now());
        let order_sender = match self.order_sender(&action.account_id) {
            Ok(sender) => sender.clone(),
            Err(error) => {
                self.reconciliation.inflight.remove(&action.account_id);
                return Err(error);
            }
        };
        let (flushed_tx, flushed_rx) = oneshot::channel();
        if order_sender
            .send(OrderTaskCommand::Flush(flushed_tx))
            .await
            .is_err()
        {
            self.reconciliation.inflight.remove(&action.account_id);
            return Err(LiveRuntimeError::OrderQueueUnavailable(action.account_id));
        }
        let orders = match self.reconciliation_order_refs(&action.account_id) {
            Ok(orders) => orders,
            Err(error) => {
                self.reconciliation.inflight.remove(&action.account_id);
                return Err(error);
            }
        };
        let sender = match self.reconcile_sender(&action.account_id) {
            Ok(sender) => sender.clone(),
            Err(error) => {
                self.reconciliation.inflight.remove(&action.account_id);
                return Err(error);
            }
        };
        if sender
            .send(ReconcileTaskCommand::Reconcile {
                restored_orders: orders,
                command_flush: Some(flushed_rx),
            })
            .await
            .is_err()
        {
            self.reconciliation.inflight.remove(&action.account_id);
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
            .dispatch
            .order_senders
            .keys()
            .filter(|account_id| !self.shutdown.reconciled_accounts.contains(*account_id))
            .filter(|account_id| !self.reconciliation.inflight.contains(*account_id))
            .filter(|account_id| {
                !self.shutdown.reconciliation_requested.contains(*account_id)
                    || force
                    || self
                        .reconciliation
                        .last_attempt
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
            self.shutdown.reconciliation_requested.insert(account_id);
        }
        Ok(())
    }

    fn retry_reconciliation(&mut self, ts_ms: u64) -> Result<(), LiveRuntimeError> {
        let readiness = self.coordinator.readiness();
        let accounts = readiness
            .missing_reconciliation
            .into_iter()
            .filter(|account_id| !self.reconciliation.inflight.contains(account_id))
            .filter(|account_id| {
                self.reconciliation
                    .last_attempt
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
        self.dispatch
            .order_senders
            .get(account_id)
            .ok_or_else(|| LiveRuntimeError::OrderQueueUnavailable(account_id.to_string()))
    }

    fn reconcile_sender(
        &self,
        account_id: &str,
    ) -> Result<&mpsc::Sender<ReconcileTaskCommand>, LiveRuntimeError> {
        self.reconciliation
            .senders
            .get(account_id)
            .ok_or_else(|| LiveRuntimeError::OrderQueueUnavailable(account_id.to_string()))
    }

    async fn disable_deadman_all(&mut self) -> Result<(), LiveRuntimeError> {
        let mut acknowledgements = Vec::new();
        for (account_id, sender) in &self.readiness_safety.safety_senders {
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
        let timeout_ms = self.shutdown.teardown_timeout_ms;
        match tokio::time::timeout(Duration::from_millis(timeout_ms), self.shutdown_inner()).await {
            Ok(result) => result,
            Err(_) => {
                self.abort_remaining_teardown();
                tokio::task::yield_now().await;
                Err(LiveRuntimeError::TeardownTimeout(timeout_ms))
            }
        }
    }

    async fn shutdown_inner(&mut self) -> Result<(), LiveRuntimeError> {
        let mut errors = Vec::new();
        if let Some(host_guard) = self.readiness_safety.host_guard.as_mut() {
            host_guard.request_shutdown();
        }
        if let Some(service) = self.dispatch.operator_service.as_mut() {
            service.request_shutdown();
        }
        for feed in &self.connectivity.feeds {
            feed.request_shutdown();
        }
        for runtime in &self.connectivity.order_ws_runtimes {
            runtime.request_shutdown();
        }
        for sender in self.dispatch.order_senders.values() {
            let _ = sender.try_send(OrderTaskCommand::Shutdown);
        }
        self.dispatch.order_senders.clear();
        for sender in self.reconciliation.senders.values() {
            let _ = sender.try_send(ReconcileTaskCommand::Shutdown);
        }
        self.reconciliation.senders.clear();
        for sender in self.readiness_safety.safety_senders.values() {
            let _ = sender.try_send(SafetyTaskCommand::Shutdown);
        }
        self.readiness_safety.safety_senders.clear();
        self.dispatch.control_rx.close();
        self.readiness_safety.forbidden_rx.close();
        self.connectivity.feed_rx.close();
        for task in &self.readiness_safety.forbidden_tasks {
            task.abort();
        }
        if let Some(host_guard) = self.readiness_safety.host_guard.take() {
            match host_guard.shutdown().await {
                Ok(stats) => {
                    self.readiness_safety.host_checks = self
                        .readiness_safety
                        .host_checks
                        .saturating_add(stats.checks);
                    if let Some(snapshot) = stats.last_snapshot {
                        self.readiness_safety.host_last_snapshot = Some(snapshot);
                    }
                }
                Err(error) => errors.push(("host guard", LiveRuntimeError::Join(error))),
            }
        }
        if let Some(failures) = &mut self.readiness_safety.host_failures
            && let Ok(error) = failures.try_recv()
        {
            errors.push(("host health", error.into()));
        }
        self.readiness_safety.host_failures.take();
        if let Some(service) = self.dispatch.operator_service.take()
            && let Err(error) = service.shutdown().await
        {
            errors.push(("operator service", LiveRuntimeError::Operator(error)));
        }
        self.dispatch.operator_rx.take();
        for feed in self.connectivity.feeds.drain(..) {
            feed.shutdown().await;
        }
        for task in &mut self.connectivity.feed_tasks {
            if let Err(error) = task.await {
                errors.push(("feed task", LiveRuntimeError::Join(error)));
            }
        }
        self.connectivity.feed_tasks.clear();
        for task in &mut self.dispatch.order_tasks {
            if let Err(error) = task.await {
                errors.push(("order task", LiveRuntimeError::Join(error)));
            }
        }
        self.dispatch.order_tasks.clear();
        for task in &mut self.reconciliation.tasks {
            if let Err(error) = task.await {
                errors.push(("reconciliation task", LiveRuntimeError::Join(error)));
            }
        }
        self.reconciliation.tasks.clear();
        for runtime in self.connectivity.order_ws_runtimes.drain(..) {
            if let Err(error) = runtime.shutdown().await {
                errors.push(("order websocket", LiveRuntimeError::Join(error)));
            }
        }
        for task in &mut self.connectivity.order_ws_status_tasks {
            if let Err(error) = task.await {
                errors.push(("order websocket status", LiveRuntimeError::Join(error)));
            }
        }
        self.connectivity.order_ws_status_tasks.clear();
        for task in &mut self.readiness_safety.safety_tasks {
            if let Err(error) = task.await {
                errors.push(("safety task", LiveRuntimeError::Join(error)));
            }
        }
        self.readiness_safety.safety_tasks.clear();
        for task in &mut self.readiness_safety.forbidden_tasks {
            if let Err(error) = task.await
                && !error.is_cancelled()
            {
                errors.push(("forbidden-order sentinel", LiveRuntimeError::Join(error)));
            }
        }
        self.readiness_safety.forbidden_tasks.clear();
        if let Some(storage) = self.composition.storage.as_mut()
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
        self.dispatch.alert_sink.take();
        if let Some(alert_runtime) = self.dispatch.alert_runtime.take() {
            match tokio::time::timeout(
                Duration::from_millis(self.dispatch.alert_shutdown_timeout_ms),
                alert_runtime.shutdown(),
            )
            .await
            {
                Ok(Ok(stats)) => {
                    self.dispatch.alert_stats = stats;
                    let unobserved_failures = stats
                        .failed
                        .saturating_sub(self.dispatch.observed_alert_delivery_failures);
                    if self.dispatch.alert_delivery_failure_is_fatal && unobserved_failures > 0 {
                        errors.push((
                            "alert delivery",
                            LiveRuntimeError::AlertFailuresDuringShutdown(unobserved_failures),
                        ));
                    }
                }
                Ok(Err(error)) => errors.push(("alert service", error.into())),
                Err(_) => errors.push((
                    "alert service",
                    LiveRuntimeError::AlertShutdownTimeout(self.dispatch.alert_shutdown_timeout_ms),
                )),
            }
        }
        self.dispatch.alert_failures.take();
        if errors.is_empty() {
            return Ok(());
        }
        let (_, first) = errors.remove(0);
        let additional = errors;
        Err(combine_lifecycle_errors(first, additional))
    }

    fn abort_remaining_teardown(&mut self) {
        self.shutdown.preserve_deadman = true;
        self.dispatch.order_senders.clear();
        self.reconciliation.senders.clear();
        self.readiness_safety.safety_senders.clear();
        self.dispatch.control_rx.close();
        self.readiness_safety.forbidden_rx.close();
        self.connectivity.feed_rx.close();

        for task in self
            .connectivity
            .feed_tasks
            .iter()
            .chain(self.dispatch.order_tasks.iter())
            .chain(self.reconciliation.tasks.iter())
            .chain(self.connectivity.order_ws_status_tasks.iter())
            .chain(self.readiness_safety.safety_tasks.iter())
            .chain(self.readiness_safety.forbidden_tasks.iter())
        {
            task.abort();
        }
        self.connectivity.feed_tasks.clear();
        self.dispatch.order_tasks.clear();
        self.reconciliation.tasks.clear();
        self.connectivity.order_ws_status_tasks.clear();
        self.readiness_safety.safety_tasks.clear();
        self.readiness_safety.forbidden_tasks.clear();

        self.connectivity.feeds.clear();
        self.connectivity.order_ws_runtimes.clear();
        self.dispatch.operator_service.take();
        self.dispatch.operator_rx.take();
        self.readiness_safety.host_guard.take();
        self.readiness_safety.host_failures.take();
        self.composition.storage.take();
        self.dispatch.alert_sink.take();
        self.dispatch.alert_runtime.take();
        self.dispatch.alert_failures.take();
    }
}
fn current_executable_sha256() -> Result<String, LiveRuntimeError> {
    hash_current_executable().map_err(LiveRuntimeError::Provenance)
}

fn host_identity_sha256() -> Result<String, LiveRuntimeError> {
    hash_host_identity().map_err(LiveRuntimeError::Provenance)
}

fn account_identity_sha256s(
    config: &LiveConfig,
    snapshots: &HashMap<String, AccountBootstrapSnapshot>,
) -> Result<BTreeMap<String, String>, LiveRuntimeError> {
    config
        .accounts
        .iter()
        .map(|account| {
            let snapshot = snapshots.get(&account.id).ok_or_else(|| {
                LiveRuntimeError::Provenance(format!(
                    "missing bootstrap identity for account {}",
                    account.id
                ))
            })?;
            Ok((
                account.id.clone(),
                okx_account_identity_sha256(
                    config.venue.environment,
                    &account.id,
                    &snapshot.account_config.user_id,
                    &snapshot.account_config.main_user_id,
                ),
            ))
        })
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

fn elapsed_us(started: Instant) -> u64 {
    duration_us_ceil(started.elapsed())
}

fn duration_us_ceil(duration: Duration) -> u64 {
    (duration.as_nanos().saturating_add(999) / 1_000).min(u64::MAX as u128) as u64
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

#[cfg(test)]
#[path = "../tests/runtime_unit/mod.rs"]
mod tests;
