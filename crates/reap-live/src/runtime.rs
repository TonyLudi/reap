use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reap_core::{
    BacktestLatencyClass, Channel, ConnId, FeedPriority, FillKey, MarketEvent, NormalizedEvent,
    OrderStatus, PINNED_JAVA_REVISION, Subscription, SystemEvent, SystemEventKind, TimerEvent,
    Venue,
};
use reap_feed::{
    ConnectionAttemptPacer, ConnectionStatus, ConnectionStatusKind, FeedOutput, FeedProcessor,
    ReconnectPolicy, SocketPlan, partition_subscriptions, try_spawn_supervised_feed,
};
#[cfg(test)]
use reap_okx_live_adapter::OrderCommandWebsocketLifecycle;
use reap_okx_live_adapter::{
    ConnectionSettings, CredentialEnvNames, OrderCommandWebsocketConfig,
    OrderCommandWebsocketStatusKind, demo_from_env, observe_from_env,
};
#[cfg(test)]
use reap_order::OkxOrderGateway;
use reap_order::{OkxReconciliationClient, okx_order_dispatch_key, reconcile_full_state};
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
    OKX_MIN_ACCOUNT_INSTRUMENT_REQUEST_INTERVAL_MS, OKX_MIN_TRADE_FEE_REQUEST_INTERVAL_MS,
    OkxAdapter, OkxInstrument, OkxInstrumentType, OkxSystemEnvironment, OkxSystemServiceType,
    OkxSystemStatus, OkxSystemStatusState, OkxTradeFeeRate, RestError, okx_capability_registration,
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
    LiveLatencyEvidence, LiveLatencySemantics, LiveMode, MaintenanceRelevancePlan,
    MaintenanceServicePlan, OperatorCommand, OperatorEnvelope, OperatorError, OperatorResponse,
    OperatorService, OperatorStatus, PrivateChannelPlan, PublicChannelPlan,
    PublicRedundancyConsumer, ReadinessSnapshot, ReconciliationResult, RequirementUse, StartupGate,
    TradingEnvironment, VerifiedBootstrap, alert_webhook_from_env, check_host_health,
    load_live_config_with_evidence, okx_instrument_type, operator_secret_from_env,
    start_host_guard, start_operator_service, verify_bootstrap,
};

mod composition;
mod connectivity;
mod dispatch;
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
use dispatch::{
    DispatchState, OrderTaskCommand, ReconcileOrderRef, ReconcileTaskCommand, RuntimeEvent,
    RuntimeTaskFailure, SafetyTaskCommand, order_dispatch_lane, run_order_task,
};
use readiness_safety::{
    ExchangeInstrumentExpectation, ExchangeInstrumentGuard, ExchangeStatusGuard, ReadinessPort,
    ReadinessSafetyState, SafetyPort,
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

async fn rest_clock_skew_ms(client: &dyn ReadinessPort) -> Result<u64, RestError> {
    let before_ms = unix_time_ms();
    let exchange_ms = client.server_time_ms().await?;
    let after_ms = unix_time_ms();
    let midpoint_ms = before_ms.saturating_add(after_ms.saturating_sub(before_ms) / 2);
    Ok(midpoint_ms.abs_diff(exchange_ms))
}

fn exchange_instrument_expectations(
    config: &LiveConfig,
    account_id: &str,
    instruments: &HashMap<String, OkxInstrument>,
) -> Result<Vec<ExchangeInstrumentExpectation>, LiveRuntimeError> {
    config
        .instruments_for_account(account_id)
        .map(|configured| {
            let metadata = instruments.get(&configured.symbol).ok_or_else(|| {
                LiveRuntimeError::ExchangeInstrumentCheck(format!(
                    "account {account_id} has no instrument metadata for {}",
                    configured.symbol
                ))
            })?;
            let expected_type = okx_instrument_type(configured.kind);
            if metadata.instrument_type != expected_type {
                return Err(LiveRuntimeError::ExchangeInstrumentDrift(format!(
                    "account {account_id} {} metadata type is {:?}, expected {:?}",
                    configured.symbol, metadata.instrument_type, expected_type
                )));
            }
            let group_id = metadata.trade_fee_group_id.trim();
            if group_id.is_empty() {
                return Err(LiveRuntimeError::ExchangeFeeCheck(format!(
                    "account {account_id} {} has no OKX trade-fee groupId",
                    configured.symbol
                )));
            }
            let (instrument_id, instrument_family) = match expected_type {
                OkxInstrumentType::Spot | OkxInstrumentType::Margin => {
                    (Some(configured.symbol.clone()), None)
                }
                OkxInstrumentType::Swap
                | OkxInstrumentType::Futures
                | OkxInstrumentType::Option => {
                    let family = metadata.instrument_family.trim();
                    if family.is_empty() {
                        return Err(LiveRuntimeError::ExchangeFeeCheck(format!(
                            "account {account_id} {} has no OKX instFamily for fee lookup",
                            configured.symbol
                        )));
                    }
                    (None, Some(family.to_string()))
                }
            };
            Ok(ExchangeInstrumentExpectation {
                symbol: configured.symbol.clone(),
                instrument_type: expected_type,
                instrument_id,
                instrument_family,
                group_id: group_id.to_string(),
                configured_maker_cost: configured.maker_fee,
                configured_taker_cost: configured.taker_fee,
                expected_instrument: metadata.clone(),
            })
        })
        .collect()
}

async fn fetch_exchange_fee(
    client: &dyn ReadinessPort,
    expectation: &ExchangeInstrumentExpectation,
) -> Result<OkxTradeFeeRate, RestError> {
    client
        .account_trade_fee(
            expectation.instrument_type,
            expectation.instrument_id.as_deref(),
            expectation.instrument_family.as_deref(),
            &expectation.group_id,
        )
        .await
}

fn exchange_fee_drift_reason(
    expectation: &ExchangeInstrumentExpectation,
    rate: &OkxTradeFeeRate,
) -> Option<String> {
    const RATE_EPSILON: f64 = 1e-12;

    let maker_cost = rate.maker_cost_rate();
    let taker_cost = rate.taker_cost_rate();
    let maker_understated = expectation.configured_maker_cost + RATE_EPSILON < maker_cost;
    let taker_understated = expectation.configured_taker_cost + RATE_EPSILON < taker_cost;
    (maker_understated || taker_understated).then(|| {
        format!(
            "{} group {} level {} configured maker/taker costs {}/{} understate authenticated costs {}/{} at {}",
            expectation.symbol,
            rate.group_id,
            rate.level,
            expectation.configured_maker_cost,
            expectation.configured_taker_cost,
            maker_cost,
            taker_cost,
            rate.timestamp_ms
        )
    })
}

fn exchange_instrument_drift_reason(
    expectation: &ExchangeInstrumentExpectation,
    current: &OkxInstrument,
    now_ms: u64,
    change_lead_ms: u64,
) -> Option<String> {
    let expected = &expectation.expected_instrument;
    if current.state != "live" {
        return Some(format!(
            "{} state changed from {:?} to {:?}",
            expectation.symbol, expected.state, current.state
        ));
    }

    macro_rules! check_field {
        ($name:literal, $expected:expr, $current:expr) => {
            if $expected != $current {
                return Some(format!(
                    "{} {} changed from {:?} to {:?}",
                    expectation.symbol, $name, $expected, $current
                ));
            }
        };
    }

    check_field!("symbol", expected.symbol, current.symbol);
    check_field!(
        "instrument type",
        expected.instrument_type,
        current.instrument_type
    );
    check_field!(
        "instrument family",
        expected.instrument_family,
        current.instrument_family
    );
    check_field!(
        "fee group",
        expected.trade_fee_group_id,
        current.trade_fee_group_id
    );
    check_field!("underlying", expected.underlying, current.underlying);
    check_field!(
        "base currency",
        expected.base_currency,
        current.base_currency
    );
    check_field!(
        "quote currency",
        expected.quote_currency,
        current.quote_currency
    );
    check_field!(
        "settle currency",
        expected.settle_currency,
        current.settle_currency
    );
    check_field!(
        "contract type",
        expected.contract_type,
        current.contract_type
    );
    check_field!(
        "contract value",
        expected.contract_value,
        current.contract_value
    );
    check_field!(
        "contract value currency",
        expected.contract_value_currency,
        current.contract_value_currency
    );
    check_field!("tick size", expected.tick_size, current.tick_size);
    check_field!("lot size", expected.lot_size, current.lot_size);
    check_field!("minimum size", expected.min_size, current.min_size);
    check_field!(
        "maximum limit-order size",
        expected.max_limit_size,
        current.max_limit_size
    );
    check_field!(
        "maximum market-order size",
        expected.max_market_size,
        current.max_market_size
    );
    check_field!(
        "maximum limit-order amount",
        expected.max_limit_amount_usd,
        current.max_limit_amount_usd
    );
    check_field!(
        "maximum market-order amount",
        expected.max_market_amount_usd,
        current.max_market_amount_usd
    );

    let cutoff_ms = now_ms.saturating_add(change_lead_ms);
    current
        .upcoming_changes
        .iter()
        .filter(|change| change.effective_time_ms <= cutoff_ms)
        .min_by_key(|change| change.effective_time_ms)
        .map(|change| {
            format!(
                "{} announced {} change to {} effective at {} inside the {}ms guard lead",
                expectation.symbol,
                change.parameter.as_okx_str(),
                change.new_value,
                change.effective_time_ms,
                change_lead_ms
            )
        })
}

fn verify_initial_exchange_instruments(
    account_id: &str,
    guard: &ExchangeInstrumentGuard,
    now_ms: u64,
) -> Result<(), LiveRuntimeError> {
    for expectation in &guard.expectations {
        if let Some(reason) = exchange_instrument_drift_reason(
            expectation,
            &expectation.expected_instrument,
            now_ms,
            guard.change_lead_ms,
        ) {
            return Err(LiveRuntimeError::ExchangeInstrumentDrift(format!(
                "account {account_id}: {reason}"
            )));
        }
    }
    Ok(())
}

async fn verify_initial_exchange_fees(
    account_id: &str,
    client: &dyn ReadinessPort,
    guard: &ExchangeInstrumentGuard,
) -> Result<(), LiveRuntimeError> {
    for (index, expectation) in guard.expectations.iter().enumerate() {
        if index > 0 {
            tokio::time::sleep(Duration::from_millis(OKX_MIN_TRADE_FEE_REQUEST_INTERVAL_MS)).await;
        }
        let rate = fetch_exchange_fee(client, expectation)
            .await
            .map_err(|error| {
                LiveRuntimeError::ExchangeFeeCheck(format!(
                    "account {account_id} {}: {error}",
                    expectation.symbol
                ))
            })?;
        if let Some(reason) = exchange_fee_drift_reason(expectation, &rate) {
            return Err(LiveRuntimeError::ExchangeFeeDrift(format!(
                "account {account_id}: {reason}"
            )));
        }
    }
    Ok(())
}

fn exchange_status_block_reason(
    statuses: &[OkxSystemStatus],
    relevance: &MaintenanceRelevancePlan,
    now_ms: u64,
    lead_ms: u64,
) -> Option<String> {
    let _maintenance_capability = okx_capability_registration("OKX-MAINTENANCE-FILTER")
        .expect("maintenance filter must remain in the OKX capability registry");
    let expected_environment = match relevance.environment() {
        TradingEnvironment::Demo => OkxSystemEnvironment::Demo,
        TradingEnvironment::Production => OkxSystemEnvironment::Production,
    };
    statuses.iter().find_map(|status| {
        let planned_service = match status.service_type {
            OkxSystemServiceType::WebSocket => Some(MaintenanceServicePlan::Websocket),
            OkxSystemServiceType::Trading => Some(MaintenanceServicePlan::Trading),
            OkxSystemServiceType::TradingAccounts => Some(MaintenanceServicePlan::TradingAccounts),
            OkxSystemServiceType::TradingProducts => Some(MaintenanceServicePlan::TradingProducts),
            OkxSystemServiceType::Other => Some(MaintenanceServicePlan::OtherAmbiguous),
            OkxSystemServiceType::BlockTrading
            | OkxSystemServiceType::TradingBot
            | OkxSystemServiceType::SpreadTrading
            | OkxSystemServiceType::CopyTrading => None,
        };
        let plan_relevant_service =
            planned_service.is_some_and(|service| relevance.services().contains(&service));
        let inside_guard_window = match status.state {
            OkxSystemStatusState::Scheduled => {
                status.begin_time_ms <= now_ms.saturating_add(lead_ms)
            }
            OkxSystemStatusState::Ongoing | OkxSystemStatusState::PreOpen => true,
            OkxSystemStatusState::Completed | OkxSystemStatusState::Canceled => false,
        };
        (relevance.unified_system()
            && status.system.eq_ignore_ascii_case("unified")
            && status.environment == expected_environment
            && plan_relevant_service
            && inside_guard_window)
            .then(|| {
                format!(
                    "{:?} {:?} maintenance {:?} from {} to {} ({}ms lead): {}",
                    status.environment,
                    status.service_type,
                    status.state,
                    status.begin_time_ms,
                    status.end_time_ms,
                    lead_ms,
                    status.title.trim()
                )
            })
    })
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

async fn run_exchange_instrument_guard(
    account_id: String,
    client: Arc<dyn ReadinessPort>,
    guard: ExchangeInstrumentGuard,
) -> RuntimeTaskFailure {
    if guard.expectations.is_empty() {
        return std::future::pending::<RuntimeTaskFailure>().await;
    }
    let request_interval_ms =
        exchange_fee_request_interval_ms(guard.sweep_interval_ms, guard.expectations.len());
    let mut interval = tokio::time::interval(Duration::from_millis(request_interval_ms));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval.tick().await;
    let mut next = 0;

    loop {
        interval.tick().await;
        let expectation = &guard.expectations[next];
        let instrument = match client
            .account_instrument(expectation.instrument_type, &expectation.symbol)
            .await
        {
            Ok(instrument) => instrument,
            Err(error) => {
                return RuntimeTaskFailure::ExchangeInstrumentCheck(format!(
                    "account {account_id} {}: {error}",
                    expectation.symbol
                ));
            }
        };
        if let Some(reason) = exchange_instrument_drift_reason(
            expectation,
            &instrument,
            unix_time_ms(),
            guard.change_lead_ms,
        ) {
            return RuntimeTaskFailure::ExchangeInstrumentDrift(format!(
                "account {account_id}: {reason}"
            ));
        }
        let rate = match fetch_exchange_fee(client.as_ref(), expectation).await {
            Ok(rate) => rate,
            Err(error) => {
                return RuntimeTaskFailure::ExchangeFeeCheck(format!(
                    "account {account_id} {}: {error}",
                    expectation.symbol
                ));
            }
        };
        if let Some(reason) = exchange_fee_drift_reason(expectation, &rate) {
            return RuntimeTaskFailure::ExchangeFeeDrift(format!("account {account_id}: {reason}"));
        }
        next = (next + 1) % guard.expectations.len();
    }
}

fn exchange_fee_request_interval_ms(sweep_interval_ms: u64, instrument_count: usize) -> u64 {
    debug_assert!(instrument_count > 0);
    (sweep_interval_ms / instrument_count as u64).max(OKX_MIN_TRADE_FEE_REQUEST_INTERVAL_MS)
}

#[allow(clippy::too_many_arguments)]
async fn run_account_safety_task(
    account_id: String,
    readiness: Arc<dyn ReadinessPort>,
    safety: Option<Arc<dyn SafetyPort>>,
    expected_account_config: reap_venue::okx::OkxAccountConfig,
    mut commands: mpsc::Receiver<SafetyTaskCommand>,
    events: mpsc::Sender<RuntimeEvent>,
    mut deadman_timeout_secs: Option<u64>,
    deadman_heartbeat_ms: u64,
    clock_check_interval_ms: u64,
    max_clock_skew_ms: u64,
    exchange_status_guard: ExchangeStatusGuard,
    exchange_instrument_guard: ExchangeInstrumentGuard,
) {
    let mut instrument_task = tokio::spawn(run_exchange_instrument_guard(
        account_id.clone(),
        readiness.clone(),
        exchange_instrument_guard,
    ));
    let mut deadman = tokio::time::interval(Duration::from_millis(deadman_heartbeat_ms));
    deadman.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    deadman.tick().await;
    let mut clock = tokio::time::interval(Duration::from_millis(clock_check_interval_ms));
    clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    clock.tick().await;
    let mut exchange_status = tokio::time::interval(Duration::from_millis(
        exchange_status_guard.check_interval_ms,
    ));
    exchange_status.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    exchange_status.tick().await;

    let terminal_failure = loop {
        tokio::select! {
            instrument_result = &mut instrument_task => {
                break Some(match instrument_result {
                    Ok(failure) => failure,
                    Err(error) => RuntimeTaskFailure::ExchangeInstrumentCheck(format!(
                        "account {account_id} instrument/fee guard task failed: {error}"
                    )),
                });
            }
            command = commands.recv() => {
                let Some(command) = command else { break None; };
                match command {
                    SafetyTaskCommand::DisableDeadMan { result } => {
                        let disabled = match (deadman_timeout_secs, safety.as_ref()) {
                            (Some(_), Some(safety)) => safety.cancel_all_after(0).await.map_err(|error| error.to_string()),
                            (None, _) => Ok(()),
                            (Some(_), None) => Err("dead-man authority is absent".to_string()),
                        };
                        if disabled.is_ok() {
                            deadman_timeout_secs = None;
                        }
                        let _ = result.send(disabled);
                    }
                    SafetyTaskCommand::Shutdown => break None,
                }
            }
            _ = deadman.tick(), if deadman_timeout_secs.is_some() => {
                let timeout_secs = deadman_timeout_secs.expect("guarded dead-man timeout");
                let Some(safety) = safety.as_ref() else {
                    break Some(RuntimeTaskFailure::DeadmanHeartbeat(format!(
                        "account {account_id}: dead-man authority is absent"
                    )));
                };
                if let Err(error) = safety.cancel_all_after(timeout_secs).await {
                    break Some(RuntimeTaskFailure::DeadmanHeartbeat(format!(
                        "account {account_id}: {error}"
                    )));
                }
            }
            _ = clock.tick() => {
                match rest_clock_skew_ms(readiness.as_ref()).await {
                    Ok(skew_ms) if skew_ms <= max_clock_skew_ms => {}
                    Ok(skew_ms) => {
                        break Some(RuntimeTaskFailure::ExchangeClockSkew(format!(
                            "account {account_id} observed {skew_ms}ms; maximum is {max_clock_skew_ms}ms"
                        )));
                    }
                    Err(error) => {
                        break Some(RuntimeTaskFailure::ExchangeClockCheck(format!(
                            "account {account_id}: {error}"
                        )));
                    }
                }
                match readiness.account_config().await {
                    Ok(current) if current == expected_account_config => {}
                    Ok(_) => {
                        break Some(RuntimeTaskFailure::AccountConfigDrift(format!(
                            "account {account_id} configuration or authenticated identity differs from bootstrap"
                        )));
                    }
                    Err(error) => {
                        break Some(RuntimeTaskFailure::AccountConfigCheck(format!(
                            "account {account_id}: {error}"
                        )));
                    }
                }
            }
            _ = exchange_status.tick(), if exchange_status_guard.enabled => {
                match readiness.system_status().await {
                    Ok(statuses) => {
                        if let Some(reason) = exchange_status_block_reason(
                            &statuses,
                            &exchange_status_guard.relevance,
                            unix_time_ms(),
                            exchange_status_guard.lead_ms,
                        ) {
                            break Some(RuntimeTaskFailure::ExchangeStatus(reason));
                        }
                    }
                    Err(error) => {
                        break Some(RuntimeTaskFailure::ExchangeStatusCheck(error.to_string()));
                    }
                }
            }
        }
    };

    if !instrument_task.is_finished() {
        instrument_task.abort();
        let _ = instrument_task.await;
    }
    if let Some(failure) = terminal_failure {
        let _ = events.send(RuntimeEvent::Fatal(failure)).await;
    }
}

#[derive(Debug, Clone)]
struct PlannedPublicSubscription {
    subscription: Subscription,
    redundancy_consumer: Option<PublicRedundancyConsumer>,
    requirements: Vec<RequirementUse>,
}

fn validate_runtime_connectivity_plan(
    config: &LiveConfig,
    plan: &ChaosConnectivityPlan,
    mode: LiveMode,
) -> Result<(), LiveRuntimeError> {
    if plan.mode() != mode {
        return Err(LiveRuntimeError::Subscription(format!(
            "connectivity plan mode {:?} does not match runtime mode {mode:?}",
            plan.mode()
        )));
    }
    if plan.environment() != config.venue.environment {
        return Err(LiveRuntimeError::Subscription(format!(
            "connectivity plan environment {:?} does not match config environment {:?}",
            plan.environment(),
            config.venue.environment
        )));
    }
    let config_accounts = config
        .accounts
        .iter()
        .map(|account| account.id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if plan.account_ids() != config_accounts {
        return Err(LiveRuntimeError::Subscription(
            "connectivity plan account boundary does not match the live config".to_string(),
        ));
    }
    let config_symbols = config
        .strategy
        .instruments
        .iter()
        .map(|instrument| instrument.symbol.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if plan.symbols() != config_symbols {
        return Err(LiveRuntimeError::Subscription(
            "connectivity plan symbol boundary does not match the live config".to_string(),
        ));
    }
    let maximum_public_replicas = plan
        .public_subscriptions()
        .iter()
        .map(|subscription| usize::from(subscription.replica_count()))
        .max()
        .unwrap_or_default();
    if maximum_public_replicas > config.runtime.public_connection_replica_cap() {
        return Err(LiveRuntimeError::Subscription(format!(
            "connectivity plan requires {maximum_public_replicas} public replicas, exceeding the configured safety ceiling {}",
            config.runtime.public_connection_replica_cap()
        )));
    }
    let mut command_shards_by_account = HashMap::<&str, usize>::new();
    for lane in plan.command_lanes() {
        *command_shards_by_account
            .entry(lane.account_id())
            .or_default() += 1;
    }
    let maximum_order_shards = command_shards_by_account
        .into_values()
        .max()
        .unwrap_or_default();
    if maximum_order_shards > config.runtime.order_command_shard_cap() {
        return Err(LiveRuntimeError::Subscription(format!(
            "connectivity plan requires {maximum_order_shards} order shards, exceeding the configured safety ceiling {}",
            config.runtime.order_command_shard_cap()
        )));
    }
    Ok(())
}

fn runtime_public_subscriptions(
    plan: &ChaosConnectivityPlan,
) -> Result<Vec<PlannedPublicSubscription>, LiveRuntimeError> {
    let mut subscriptions = Vec::with_capacity(plan.public_subscriptions().len());
    let mut seen = HashSet::new();
    for planned in plan.public_subscriptions() {
        if planned.replica_count() == 0 {
            return Err(LiveRuntimeError::Subscription(format!(
                "planned public subscription {:?}/{} has zero replicas",
                planned.channel(),
                planned.symbol()
            )));
        }
        if planned.requirements().is_empty() {
            return Err(LiveRuntimeError::Subscription(format!(
                "planned public subscription {:?}/{} has no Chaos requirement",
                planned.channel(),
                planned.symbol()
            )));
        }
        if planned.session_surfaces().is_empty()
            || planned.channel_surface().capability_id() != planned.channel().capability_id()
        {
            return Err(LiveRuntimeError::Subscription(format!(
                "planned public subscription {:?}/{} has invalid capability metadata",
                planned.channel(),
                planned.symbol()
            )));
        }
        if (planned.replica_count() > 1) != planned.redundancy_consumer().is_some() {
            return Err(LiveRuntimeError::Subscription(format!(
                "planned public subscription {:?}/{} has replicas without exact redundancy-consumer metadata",
                planned.channel(),
                planned.symbol()
            )));
        }
        let channel = match planned.channel() {
            PublicChannelPlan::Books => Channel::Books,
            PublicChannelPlan::Trades => Channel::Trades,
            PublicChannelPlan::FundingRate
            | PublicChannelPlan::IndexTickers
            | PublicChannelPlan::MarkPrice
            | PublicChannelPlan::PriceLimit => {
                Channel::Custom(planned.channel_surface().endpoint_or_channel().to_string())
            }
        };
        if !seen.insert((channel.clone(), planned.symbol().to_string())) {
            return Err(LiveRuntimeError::Subscription(format!(
                "connectivity plan repeats public subscription {:?}/{}",
                planned.channel(),
                planned.symbol()
            )));
        }
        let priority = if planned.channel() == PublicChannelPlan::Trades {
            FeedPriority::High
        } else {
            FeedPriority::Critical
        };
        let mut subscription =
            Subscription::public(Venue::Okx, channel, planned.symbol(), priority);
        subscription.connections = usize::from(planned.replica_count());
        subscriptions.push(PlannedPublicSubscription {
            subscription,
            redundancy_consumer: planned.redundancy_consumer(),
            requirements: planned.requirements().to_vec(),
        });
    }
    if subscriptions.is_empty() {
        return Err(LiveRuntimeError::Subscription(
            "connectivity plan has no public subscriptions".to_string(),
        ));
    }
    Ok(subscriptions)
}

fn validate_public_socket_plans(
    subscriptions: &[PlannedPublicSubscription],
    socket_plans: &[SocketPlan],
) -> Result<(), LiveRuntimeError> {
    let mut occurrences = HashMap::<(Channel, Option<String>), usize>::new();
    for socket in socket_plans {
        if socket.private || socket.venue != Venue::Okx {
            return Err(LiveRuntimeError::Subscription(
                "public connectivity plan produced a private or non-OKX socket".to_string(),
            ));
        }
        for subscription in &socket.subscriptions {
            *occurrences
                .entry((subscription.channel.clone(), subscription.symbol.clone()))
                .or_default() += 1;
        }
    }
    if occurrences.len() != subscriptions.len() {
        return Err(LiveRuntimeError::Subscription(
            "public socket plan contains an unplanned subscription".to_string(),
        ));
    }
    for planned in subscriptions {
        let key = (
            planned.subscription.channel.clone(),
            planned.subscription.symbol.clone(),
        );
        let actual = occurrences.get(&key).copied().unwrap_or_default();
        if actual != planned.subscription.connections {
            return Err(LiveRuntimeError::Subscription(format!(
                "public socket plan materialized {actual} replicas for {:?}/{}, expected {}",
                planned.subscription.channel,
                planned.subscription.symbol.as_deref().unwrap_or("<all>"),
                planned.subscription.connections
            )));
        }
        let consumers = planned
            .requirements
            .iter()
            .map(RequirementUse::consumer)
            .collect::<BTreeSet<_>>();
        if consumers.is_empty() || (actual > 1) != planned.redundancy_consumer.is_some() {
            return Err(LiveRuntimeError::Subscription(format!(
                "public socket plan lost consumer metadata for {:?}/{}",
                planned.subscription.channel,
                planned.subscription.symbol.as_deref().unwrap_or("<all>")
            )));
        }
    }
    Ok(())
}

fn private_socket_plans_by_account(
    plan: &ChaosConnectivityPlan,
) -> Result<BTreeMap<String, Vec<SocketPlan>>, LiveRuntimeError> {
    let mut plans_by_account = BTreeMap::new();
    for session in plan.private_state_sessions() {
        validate_private_state_socket_count(session.account_id(), session.socket_count())?;
        if session.channels().is_empty()
            || session.requirements().is_empty()
            || session.session_surfaces().is_empty()
        {
            return Err(LiveRuntimeError::Subscription(format!(
                "private state session for {} is empty or has incomplete metadata",
                session.account_id()
            )));
        }
        if !plan
            .account_ids()
            .iter()
            .any(|account_id| account_id == session.account_id())
        {
            return Err(LiveRuntimeError::Subscription(format!(
                "private state session references unknown account {}",
                session.account_id()
            )));
        }
        let mut channels = Vec::with_capacity(session.channels().len());
        let mut seen_channels = HashSet::new();
        let mut binding_requirements = BTreeSet::new();
        for binding in session.channels() {
            if binding.requirements().is_empty()
                || binding.surface().capability_id() != binding.channel().capability_id()
            {
                return Err(LiveRuntimeError::Subscription(format!(
                    "private state channel {:?} for {} has incomplete requirement metadata",
                    binding.channel(),
                    session.account_id()
                )));
            }
            let channel = match binding.channel() {
                PrivateChannelPlan::Account => Channel::Account,
                PrivateChannelPlan::Fills => Channel::Fills,
                PrivateChannelPlan::Orders => Channel::Orders,
                PrivateChannelPlan::Positions => Channel::Positions,
            };
            if !seen_channels.insert(channel.clone()) {
                return Err(LiveRuntimeError::Subscription(format!(
                    "private state session for {} repeats channel {:?}",
                    session.account_id(),
                    channel
                )));
            }
            binding_requirements.extend(binding.requirements().iter().cloned());
            channels.push(Subscription::private(
                Venue::Okx,
                channel,
                FeedPriority::Critical,
            ));
        }
        let session_requirements = session
            .requirements()
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if binding_requirements != session_requirements {
            return Err(LiveRuntimeError::Subscription(format!(
                "private state session for {} does not preserve its channel consumers",
                session.account_id()
            )));
        }
        let account_plans = vec![SocketPlan {
            conn_id: ConnId::new(format!("okx-private-{}-r0", session.account_id())),
            venue: Venue::Okx,
            private: true,
            subscriptions: channels,
        }];
        if plans_by_account
            .insert(session.account_id().to_string(), account_plans)
            .is_some()
        {
            return Err(LiveRuntimeError::Subscription(format!(
                "connectivity plan repeats private state session for {}",
                session.account_id()
            )));
        }
    }
    let planned_accounts = plans_by_account.keys().cloned().collect::<Vec<_>>();
    if planned_accounts != plan.account_ids() {
        return Err(LiveRuntimeError::Subscription(
            "connectivity plan must define exactly one private state session per account"
                .to_string(),
        ));
    }
    Ok(plans_by_account)
}

fn validate_private_state_socket_count(
    account_id: &str,
    socket_count: u16,
) -> Result<(), LiveRuntimeError> {
    if socket_count != 1 {
        return Err(LiveRuntimeError::Subscription(format!(
            "private state session for {account_id} must use exactly one socket, configured {socket_count}"
        )));
    }
    Ok(())
}

fn planned_order_session_counts(
    plan: &ChaosConnectivityPlan,
) -> Result<BTreeMap<String, usize>, LiveRuntimeError> {
    if plan.mode() != LiveMode::Demo && !plan.command_lanes().is_empty() {
        return Err(LiveRuntimeError::Subscription(
            "order command lanes are only valid in demo mode".to_string(),
        ));
    }
    let planned_accounts = plan
        .account_ids()
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut lanes_by_account = BTreeMap::<String, Vec<_>>::new();
    for lane in plan.command_lanes() {
        if !planned_accounts.contains(lane.account_id()) {
            return Err(LiveRuntimeError::Subscription(format!(
                "order command lane references unknown account {}",
                lane.account_id()
            )));
        }
        if lane.dispatch_families().is_empty()
            || lane.requirements().is_empty()
            || lane.session_surfaces().is_empty()
        {
            return Err(LiveRuntimeError::Subscription(format!(
                "order command lane {} for {} is empty or has incomplete consumer metadata",
                lane.lane_index(),
                lane.account_id()
            )));
        }
        lanes_by_account
            .entry(lane.account_id().to_string())
            .or_default()
            .push(lane);
    }
    let mut counts = BTreeMap::new();
    for (account_id, mut lanes) in lanes_by_account {
        lanes.sort_by_key(|lane| lane.lane_index());
        let lane_count = lanes.len();
        if lane_count != 1 {
            return Err(LiveRuntimeError::Subscription(format!(
                "account {account_id} must have exactly one order command lane, found {lane_count}"
            )));
        }
        let mut families = BTreeSet::new();
        for (expected_index, lane) in lanes.into_iter().enumerate() {
            if usize::from(lane.lane_index()) != expected_index {
                return Err(LiveRuntimeError::Subscription(format!(
                    "account {account_id} order lane indices must be contiguous from zero"
                )));
            }
            for family in lane.dispatch_families() {
                if family.trim().is_empty()
                    || okx_order_dispatch_key(family) != *family
                    || !families.insert(family.clone())
                {
                    return Err(LiveRuntimeError::Subscription(format!(
                        "account {account_id} has an invalid or duplicate order dispatch family {family:?}"
                    )));
                }
                if order_dispatch_lane(family, lane_count) != expected_index {
                    return Err(LiveRuntimeError::Subscription(format!(
                        "account {account_id} dispatch family {family} does not route to planned lane {expected_index}"
                    )));
                }
            }
        }
        counts.insert(account_id, lane_count);
    }
    Ok(counts)
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
mod tests {
    use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use reap_core::{
        AccountUpdate, Balance, Level, MarketEvent, OrderBook, OrderEvent, OrderUpdate, Side,
        TimeInForce,
    };
    use reap_feed::SupervisedFeed;
    use reap_order::{
        CancelOrderTransportError, ClientOrderIdGenerator, OrderTransportError, OwnedRegularOrders,
        PacingPolicy, PreparedRegularCancel, PreparedRegularSubmit, PrivateStateReducer,
        ReconcileReport, RegularExecution, RegularExecutionPolicy, RegularExecutionProfile,
        RegularReconciliation,
    };
    use reap_risk::{
        InstrumentOrderLimits, InstrumentRiskModel, RiskLimits, StablecoinGuardConfig,
    };
    use reap_storage::{
        OrderAckStatus, StorageRuntime, StorageSink, acquire_storage_lease, recover_jsonl,
        recover_leased_jsonl, start_jsonl_storage,
    };
    use reap_strategy::{
        ChaosConfig, ChaosExecutionIntent, ChaosStrategy, InstrumentConfig, InstrumentKindConfig,
        ReferenceDataKind, RiskGroupConfig,
    };
    use reap_telemetry::AlertSink;
    use reap_venue::okx::{
        OkxAccountConfig, OkxAccountLevel, OkxApiKeyPermission, OkxFillPage, OkxInstrumentChange,
        OkxInstrumentChangeParameter, OkxInstrumentType, OkxOrderAck, OkxPositionMode,
        OkxRegularOrderPage, OkxTradeMode, RestError, parse_okx_account_balance_response_json,
        parse_okx_account_config_response_json, parse_okx_account_instruments_response_json,
        parse_okx_account_positions_response_json, parse_okx_cancel_all_after_response_json,
        parse_okx_fill_page_response_json, parse_okx_order_ack_response_json,
        parse_okx_order_details_response_json, parse_okx_regular_order_page_response_json,
        parse_okx_server_time_response_json, parse_okx_system_status_response_json,
        parse_okx_trade_fee_response_json,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::{Notify, Semaphore, oneshot};
    use tokio::task::JoinHandle;

    use crate::forbidden_orders::ForbiddenOrderState;
    use crate::{
        LiveAccountConfig, LiveStorageConfig, OkxTradeModeConfig, OkxVenueConfig, RuntimeConfig,
        VerifiedInstrument,
    };

    use super::*;

    #[test]
    fn production_runtime_keeps_single_owner_responsibility_state() {
        let runtime_source = include_str!("runtime.rs");
        let (production_runtime, _) = runtime_source
            .split_once("#[cfg(test)]\nmod tests")
            .expect("runtime test module marker");
        let responsibility_modules = [
            include_str!("runtime/composition.rs"),
            include_str!("runtime/connectivity.rs"),
            include_str!("runtime/dispatch.rs"),
            include_str!("runtime/readiness_safety.rs"),
            include_str!("runtime/reconciliation.rs"),
            include_str!("runtime/shutdown.rs"),
        ];

        assert_eq!(
            production_runtime
                .matches("coordinator: LiveCoordinator")
                .count(),
            1,
            "LiveRuntime must remain the sole LiveCoordinator owner",
        );
        for state_field in [
            "composition: CompositionState",
            "connectivity: ConnectivityState",
            "dispatch: DispatchState",
            "readiness_safety: ReadinessSafetyState",
            "reconciliation: ReconciliationState",
            "shutdown: ShutdownState",
        ] {
            assert!(
                production_runtime.contains(state_field),
                "LiveRuntime is missing responsibility state `{state_field}`",
            );
        }

        for source in std::iter::once(production_runtime).chain(responsibility_modules) {
            let compact_source = source
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>();
            assert!(
                !compact_source.contains("Arc<Mutex"),
                "production runtime ownership must not use Arc<Mutex<_>>",
            );
        }
        for source in responsibility_modules {
            assert!(
                !source.contains("use super::*;"),
                "responsibility modules must declare explicit dependencies",
            );
        }
    }

    struct TestRuntimeParts {
        session_id: String,
        session_started_at_ms: u64,
        config_source: Option<LiveConfigFileEvidence>,
        config_fingerprint: String,
        evidence_config_fingerprint: String,
        executable_sha256: String,
        host_identity_sha256: Option<String>,
        account_identity_sha256s: BTreeMap<String, String>,
        mode: LiveMode,
        run_duration: Option<Duration>,
        coordinator: LiveCoordinator,
        processor: FeedProcessor,
        storage: Option<StorageRuntime>,
        storage_sink: StorageSink,
        control_rx: mpsc::Receiver<RuntimeEvent>,
        feed_rx: mpsc::Receiver<RuntimeEvent>,
        forbidden_rx: mpsc::Receiver<ForbiddenOrderEvent>,
        order_senders: HashMap<String, mpsc::Sender<OrderTaskCommand>>,
        order_tasks: Vec<JoinHandle<()>>,
        reconcile_senders: HashMap<String, mpsc::Sender<ReconcileTaskCommand>>,
        reconcile_tasks: Vec<JoinHandle<()>>,
        order_ws_runtimes: Vec<OrderCommandWebsocketLifecycle>,
        order_ws_status_tasks: Vec<JoinHandle<()>>,
        safety_senders: HashMap<String, mpsc::Sender<SafetyTaskCommand>>,
        safety_tasks: Vec<JoinHandle<()>>,
        forbidden_tasks: Vec<JoinHandle<()>>,
        feeds: Vec<SupervisedFeed>,
        feed_tasks: Vec<JoinHandle<()>>,
        sources: Vec<FeedSourceState>,
        public_feed_index: usize,
        reconcile_inflight: HashSet<String>,
        cancel_inflight: HashSet<(String, String)>,
        last_reconcile_attempt: HashMap<String, Instant>,
        fill_convergence: FillConvergenceGuard,
        order_convergence: OrderStateConvergenceGuard,
        readiness_timeout_ms: u64,
        timer_interval_ms: u64,
        max_feed_age_ms: u64,
        shutdown_timeout_ms: u64,
        teardown_timeout_ms: u64,
        safety_latch_sync_timeout_ms: u64,
        evidence: RuntimeEvidence,
        latency: LiveLatencyCollector,
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

    impl TestRuntimeParts {
        fn into_runtime(self) -> LiveRuntime {
            LiveRuntime {
                coordinator: self.coordinator,
                composition: CompositionState {
                    session_id: self.session_id,
                    session_started_at_ms: self.session_started_at_ms,
                    config_source: self.config_source,
                    config_fingerprint: self.config_fingerprint,
                    evidence_config_fingerprint: self.evidence_config_fingerprint,
                    executable_sha256: self.executable_sha256,
                    host_identity_sha256: self.host_identity_sha256,
                    account_identity_sha256s: self.account_identity_sha256s,
                    mode: self.mode,
                    run_duration: self.run_duration,
                    storage: self.storage,
                    storage_sink: self.storage_sink,
                    evidence: self.evidence,
                    latency: self.latency,
                },
                connectivity: ConnectivityState {
                    processor: self.processor,
                    feed_rx: self.feed_rx,
                    order_ws_runtimes: self.order_ws_runtimes,
                    order_ws_status_tasks: self.order_ws_status_tasks,
                    feeds: self.feeds,
                    feed_tasks: self.feed_tasks,
                    sources: self.sources,
                    public_feed_index: self.public_feed_index,
                    max_feed_age_ms: self.max_feed_age_ms,
                },
                dispatch: DispatchState {
                    control_rx: self.control_rx,
                    order_senders: self.order_senders,
                    order_tasks: self.order_tasks,
                    operator_service: self.operator_service,
                    operator_rx: self.operator_rx,
                    operator_shutdown_reason: self.operator_shutdown_reason,
                    alert_runtime: self.alert_runtime,
                    alert_sink: self.alert_sink,
                    alert_failures: self.alert_failures,
                    alert_shutdown_timeout_ms: self.alert_shutdown_timeout_ms,
                    alert_delivery_failure_is_fatal: self.alert_delivery_failure_is_fatal,
                    observed_alert_delivery_failures: self.observed_alert_delivery_failures,
                    alert_stats: self.alert_stats,
                },
                readiness_safety: ReadinessSafetyState {
                    forbidden_rx: self.forbidden_rx,
                    safety_senders: self.safety_senders,
                    safety_tasks: self.safety_tasks,
                    forbidden_tasks: self.forbidden_tasks,
                    readiness_timeout_ms: self.readiness_timeout_ms,
                    timer_interval_ms: self.timer_interval_ms,
                    host_guard: self.host_guard,
                    host_failures: self.host_failures,
                    host_preflight: self.host_preflight,
                    host_checks: self.host_checks,
                    host_last_snapshot: self.host_last_snapshot,
                },
                reconciliation: ReconciliationState {
                    senders: self.reconcile_senders,
                    tasks: self.reconcile_tasks,
                    inflight: self.reconcile_inflight,
                    cancel_inflight: self.cancel_inflight,
                    last_attempt: self.last_reconcile_attempt,
                    fill_convergence: self.fill_convergence,
                    order_convergence: self.order_convergence,
                },
                shutdown: ShutdownState {
                    timeout_ms: self.shutdown_timeout_ms,
                    teardown_timeout_ms: self.teardown_timeout_ms,
                    safety_latch_sync_timeout_ms: self.safety_latch_sync_timeout_ms,
                    in_progress: self.shutdown_in_progress,
                    storage_error: self.shutdown_storage_error,
                    preserve_deadman: self.preserve_deadman_on_shutdown,
                    reconciliation_requested: self.shutdown_reconciliation_requested,
                    reconciled_accounts: self.shutdown_reconciled_accounts,
                },
            }
        }
    }

    #[derive(Clone)]
    struct HttpResponse {
        #[allow(dead_code)]
        status: u16,
        body: String,
    }

    struct TaskDropSignal(Option<oneshot::Sender<()>>);

    impl Drop for TaskDropSignal {
        fn drop(&mut self) {
            if let Some(signal) = self.0.take() {
                let _ = signal.send(());
            }
        }
    }

    fn recover_storage_records(
        records: impl IntoIterator<Item = StorageRecord>,
    ) -> RecoveredStorage {
        let journal = records
            .into_iter()
            .map(|record| {
                serde_json::to_string(&serde_json::json!({
                    "schema_version": 7,
                    "record": record,
                }))
                .unwrap()
            })
            .collect::<Vec<_>>()
            .join("\n");
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runtime-recovery.jsonl");
        std::fs::write(&path, format!("{journal}\n")).unwrap();
        let mut lease = acquire_storage_lease(&path).unwrap();
        recover_leased_jsonl(&mut lease).unwrap()
    }

    fn recovered_submit_proof(
        account_id: &str,
        symbol: &str,
        client_order_id: &str,
    ) -> reap_storage::ProvenRegularSubmitRequest {
        recover_storage_records([StorageRecord::OrderRequest(OrderRequestRecord {
            ts_ms: 1,
            account_id: account_id.to_string(),
            operation: OrderOperation::Submit,
            idempotency_key: Some(format!("proof-{client_order_id}")),
            client_order_id: Some(client_order_id.to_string()),
            exchange_order_id: None,
            symbol: symbol.to_string(),
        })])
        .proven_regular_submit_requests
        .into_values()
        .next()
        .expect("test journal must produce a durable submit proof")
    }

    fn unwrap_startup_failure(error: LiveRuntimeError) -> (LiveRuntimeError, LiveRunReport) {
        let (source, report) = match error {
            LiveRuntimeError::ReportedFailure { source, report } => (source, report),
            other => panic!("expected reported startup failure, got {other}"),
        };
        assert!(report.session_id.is_none());
        assert_eq!(report.stop_reason, LiveStopReason::RuntimeFailure);
        assert!(report.account_identity_sha256s.is_empty());
        assert!(!report.reached_ready);
        assert!(!report.clean_soak);
        assert!(
            report
                .failure
                .as_ref()
                .is_some_and(|failure| failure.message.contains("not exchange-zero proof"))
        );
        (*source, *report)
    }

    #[derive(Clone)]
    struct RuntimeMockRoles {
        responses: Arc<Mutex<VecDeque<Result<HttpResponse, RestError>>>>,
    }

    impl RuntimeMockRoles {
        fn next(&self) -> Result<HttpResponse, RestError> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected runtime mock HTTP request")
        }
    }

    #[async_trait]
    impl RegularExecution for RuntimeMockRoles {
        async fn place_regular_order(
            &self,
            _order: PreparedRegularSubmit,
        ) -> Result<OkxOrderAck, OrderTransportError> {
            Err(OrderTransportError::Unavailable(
                "runtime mock command websocket is not installed".to_string(),
            ))
        }

        async fn cancel_regular_order(
            &self,
            order: PreparedRegularCancel,
        ) -> Result<OkxOrderAck, CancelOrderTransportError> {
            Err(CancelOrderTransportError::pre_send_unavailable(
                "runtime mock command websocket is not installed",
                order,
            ))
        }

        async fn cancel_regular_order_via_rest(
            &self,
            _order: PreparedRegularCancel,
        ) -> Result<OkxOrderAck, OrderTransportError> {
            let response = self
                .next()
                .map_err(|error| OrderTransportError::Ambiguous(error.to_string()))?;
            parse_okx_order_ack_response_json(response.body.as_bytes(), "cancel order")
                .map_err(|error| OrderTransportError::Ambiguous(error.to_string()))
        }
    }

    #[async_trait]
    impl RegularReconciliation for RuntimeMockRoles {
        async fn regular_pending_orders_page(
            &self,
            _instrument_type: Option<&str>,
            _symbol: Option<&str>,
            _after: Option<&str>,
        ) -> Result<OkxRegularOrderPage, RestError> {
            let response = self.next()?;
            parse_okx_regular_order_page_response_json(response.body.as_bytes())
        }

        async fn recent_fills_page(
            &self,
            _instrument_type: Option<&str>,
            _symbol: Option<&str>,
            _after: Option<&str>,
        ) -> Result<OkxFillPage, RestError> {
            let response = self.next()?;
            parse_okx_fill_page_response_json(response.body.as_bytes())
        }

        async fn account_balance(&self) -> Result<AccountUpdate, RestError> {
            let response = self.next()?;
            Ok(parse_okx_account_balance_response_json(response.body.as_bytes())?.account_update())
        }

        async fn account_positions(&self) -> Result<AccountUpdate, RestError> {
            let response = self.next()?;
            Ok(
                parse_okx_account_positions_response_json(response.body.as_bytes())?
                    .account_update(),
            )
        }

        async fn order_details(
            &self,
            _symbol: &str,
            _client_order_id: &str,
        ) -> Result<RemoteOrder, RestError> {
            let response = self.next()?;
            Ok(parse_okx_order_details_response_json(response.body.as_bytes())?.order)
        }

        async fn server_time_ms(&self) -> Result<u64, RestError> {
            let response = self.next()?;
            parse_okx_server_time_response_json(response.body.as_bytes())
        }
    }

    struct GatedOrderTransport {
        started: mpsc::UnboundedSender<String>,
        gates: OrderGates,
        active: Arc<AtomicUsize>,
        max_active: Arc<AtomicUsize>,
    }

    type OrderGates = Arc<Mutex<HashMap<String, Arc<Semaphore>>>>;
    type GatedOrderHarness = (
        GatedOrderTransport,
        mpsc::UnboundedReceiver<String>,
        OrderGates,
        Arc<AtomicUsize>,
    );

    #[async_trait]
    impl RegularExecution for GatedOrderTransport {
        async fn place_regular_order(
            &self,
            order: PreparedRegularSubmit,
        ) -> Result<OkxOrderAck, OrderTransportError> {
            self.execute(&order.order().symbol, order.client_order_id())
                .await
        }

        async fn cancel_regular_order(
            &self,
            order: PreparedRegularCancel,
        ) -> Result<OkxOrderAck, CancelOrderTransportError> {
            self.execute(order.symbol(), order.client_order_id())
                .await
                .map_err(CancelOrderTransportError::failed)
        }

        async fn cancel_regular_order_via_rest(
            &self,
            _order: PreparedRegularCancel,
        ) -> Result<OkxOrderAck, OrderTransportError> {
            unreachable!("gated command tests never use REST cancellation fallback")
        }
    }

    impl GatedOrderTransport {
        async fn execute(
            &self,
            symbol: &str,
            client_order_id: &str,
        ) -> Result<OkxOrderAck, OrderTransportError> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            self.started.send(symbol.to_string()).unwrap();
            let gate = self
                .gates
                .lock()
                .unwrap()
                .get(symbol)
                .unwrap_or_else(|| panic!("missing gate for {symbol}"))
                .clone();
            gate.acquire().await.unwrap().forget();
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(OkxOrderAck {
                exchange_order_id: format!("exchange-{client_order_id}"),
                client_order_id: client_order_id.to_string(),
            })
        }
    }

    fn runtime_order_gateway(
        symbols: &[&str],
        responses: Vec<Result<HttpResponse, RestError>>,
    ) -> (
        OkxOrderGateway,
        RegularExecutionPolicy,
        ClientOrderIdGenerator,
    ) {
        let responses = Arc::new(Mutex::new(responses.into()));
        let execution = Box::new(RuntimeMockRoles {
            responses: Arc::clone(&responses),
        });
        runtime_order_gateway_from_parts(symbols, responses, execution)
    }

    fn runtime_order_gateway_with_execution(
        symbols: &[&str],
        responses: Vec<Result<HttpResponse, RestError>>,
        execution: Box<dyn RegularExecution>,
    ) -> (
        OkxOrderGateway,
        RegularExecutionPolicy,
        ClientOrderIdGenerator,
    ) {
        runtime_order_gateway_from_parts(symbols, Arc::new(Mutex::new(responses.into())), execution)
    }

    fn runtime_order_gateway_from_parts(
        symbols: &[&str],
        responses: Arc<Mutex<VecDeque<Result<HttpResponse, RestError>>>>,
        execution: Box<dyn RegularExecution>,
    ) -> (
        OkxOrderGateway,
        RegularExecutionPolicy,
        ClientOrderIdGenerator,
    ) {
        let mut gateway = OkxOrderGateway::new(
            "main",
            execution,
            Arc::new(RuntimeMockRoles { responses }),
            symbols
                .iter()
                .map(|symbol| ((*symbol).to_string(), OkxTradeMode::Cash))
                .collect(),
            PacingPolicy::default(),
        )
        .unwrap();
        let profiles = symbols.iter().map(|symbol| execution_profile(symbol));
        let (profile_set, client_order_id_generator) = gateway
            .take_approval_scope()
            .unwrap()
            .bind_profiles_and_client_id_generator(profiles, "test", 1)
            .unwrap();
        let policy = RegularExecutionPolicy::from_profile_sets([profile_set]).unwrap();
        (gateway, policy, client_order_id_generator)
    }

    fn gated_order_transport(symbols: &[&str]) -> GatedOrderHarness {
        let (started_tx, started_rx) = mpsc::unbounded_channel();
        let gates = Arc::new(Mutex::new(
            symbols
                .iter()
                .map(|symbol| ((*symbol).to_string(), Arc::new(Semaphore::new(0))))
                .collect(),
        ));
        let max_active = Arc::new(AtomicUsize::new(0));
        (
            GatedOrderTransport {
                started: started_tx,
                gates: Arc::clone(&gates),
                active: Arc::new(AtomicUsize::new(0)),
                max_active: Arc::clone(&max_active),
            },
            started_rx,
            gates,
            max_active,
        )
    }

    fn execution_profile(symbol: &str) -> RegularExecutionProfile {
        RegularExecutionProfile::new(
            symbol,
            "main",
            InstrumentRiskModel::Spot,
            InstrumentOrderLimits {
                max_limit_quantity: 1_000_000.0,
                max_limit_notional_usd: None,
            },
            0.1,
            0.0001,
            0.0001,
            true,
            false,
            true,
        )
    }

    fn strategy_quote(symbol: &str) -> ChaosExecutionIntent {
        let hedge_symbol = format!("{symbol}-HEDGE");
        let mut strategy = ChaosStrategy::new(ChaosConfig {
            ref_symbol: symbol.to_string(),
            delta_limit_usd: 50_000.0,
            active_hedge_threshold_usd: 1_000.0,
            min_hedge_interval_ms: 0,
            risk_groups: vec![RiskGroupConfig {
                name: "main".to_string(),
                symbols: vec![symbol.to_string(), hedge_symbol.clone()],
                soft_delta_limit_usd: 25_000.0,
                hard_delta_limit_usd: 40_000.0,
                live_order_limit_usd: 100_000.0,
                ..RiskGroupConfig::default()
            }],
            instruments: vec![
                InstrumentConfig {
                    symbol: symbol.to_string(),
                    risk_group: "main".to_string(),
                    kind: InstrumentKindConfig::Spot,
                    tick_size: 0.1,
                    lot_size: 0.0001,
                    min_trade_size: 0.0001,
                    max_order_size_usd: 5_000.0,
                    min_order_size_usd: 100.0,
                    max_order_size: 1.0,
                    ..InstrumentConfig::default()
                },
                InstrumentConfig {
                    symbol: hedge_symbol.clone(),
                    risk_group: "main".to_string(),
                    kind: InstrumentKindConfig::Future,
                    tick_size: 0.1,
                    lot_size: 1.0,
                    min_trade_size: 1.0,
                    contract_value: 0.001,
                    max_order_size_usd: 5_000.0,
                    min_order_size_usd: 100.0,
                    max_order_size: 200.0,
                    min_position: -10_000.0,
                    max_position: 10_000.0,
                    ..InstrumentConfig::default()
                },
            ],
            ..ChaosConfig::default()
        })
        .unwrap();
        let depths = [
            OrderBook {
                symbol: symbol.to_string(),
                ts_ms: 1,
                bids: vec![Level {
                    px: 50_000.0,
                    qty: 2.0,
                }],
                asks: vec![Level {
                    px: 50_001.0,
                    qty: 2.0,
                }],
            },
            OrderBook {
                symbol: hedge_symbol,
                ts_ms: 1,
                bids: vec![Level {
                    px: 50_003.0,
                    qty: 10_000.0,
                }],
                asks: vec![Level {
                    px: 50_004.0,
                    qty: 10_000.0,
                }],
            },
        ];
        depths
            .into_iter()
            .flat_map(|book| {
                strategy.on_execution_event(
                    &NormalizedEvent::Market(MarketEvent::Depth(book)).into_strategy_event(),
                )
            })
            .find(|intent| {
                intent
                    .as_quote()
                    .is_some_and(|quote| quote.symbol() == symbol)
            })
            .expect("strategy fixture must emit a typed quote")
    }

    fn submit_action(
        policy: &RegularExecutionPolicy,
        client_order_ids: &ClientOrderIdGenerator,
        symbol: &str,
        id: &str,
    ) -> SubmitAction {
        let approved = policy
            .authorize_submit(strategy_quote(symbol))
            .expect("typed strategy quote must satisfy regular execution policy");
        let mut owned = OwnedRegularOrders::default();
        let mut private_state = PrivateStateReducer::new();
        let (_, reserved) = owned
            .reserve_local(
                approved,
                client_order_ids.next(unix_time_ms()),
                &mut private_state,
                unix_time_ms(),
            )
            .expect("test order must establish ownership");
        SubmitAction::new(unix_time_ms(), format!("decision-{id}"), reserved)
    }

    fn cancel_action(
        policy: &RegularExecutionPolicy,
        client_order_ids: &ClientOrderIdGenerator,
        symbol: &str,
        _client_order_id: &str,
        reason: &str,
    ) -> CancelAction {
        let approved_submit = policy
            .authorize_submit(strategy_quote(symbol))
            .expect("typed strategy quote must satisfy regular execution policy");
        let mut owned = OwnedRegularOrders::default();
        let mut private_state = PrivateStateReducer::new();
        let generated_client_order_id = client_order_ids.next(unix_time_ms());
        let client_order_id = generated_client_order_id.as_str().to_string();
        owned
            .reserve_local(
                approved_submit,
                generated_client_order_id,
                &mut private_state,
                unix_time_ms(),
            )
            .expect("test order must establish ownership");
        let approved_cancel = policy
            .authorize_cancel(
                &client_order_id,
                reason,
                &owned,
                &HashMap::from([("main".to_string(), private_state)]),
            )
            .expect("owned canonical order must be cancellable");
        CancelAction::new(unix_time_ms(), approved_cancel)
    }

    fn release_order(gates: &OrderGates, symbol: &str) {
        gates.lock().unwrap().get(symbol).unwrap().add_permits(1);
    }

    async fn receive_started(receiver: &mut mpsc::UnboundedReceiver<String>) -> String {
        tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("order operation did not start")
            .expect("order start channel closed")
    }

    #[tokio::test]
    async fn order_task_is_bounded_and_serializes_each_underlying() {
        let symbols = [
            "BTC-USDT-SWAP",
            "BTC-USDT-260925",
            "ETH-USDT-SWAP",
            "SOL-USDT-SWAP",
        ];
        let (transport, mut started, gates, max_active) = gated_order_transport(&symbols);
        let (gateway, policy, client_order_ids) =
            runtime_order_gateway_with_execution(&symbols, Vec::new(), Box::new(transport));
        let (command_tx, command_rx) = mpsc::channel(16);
        let (event_tx, mut event_rx) = mpsc::channel(16);
        let task = tokio::spawn(run_order_task(
            "main".to_string(),
            gateway,
            command_rx,
            event_tx,
            2,
            16,
        ));

        for (symbol, id) in [
            (symbols[0], "btc-swap"),
            (symbols[1], "btc-future"),
            (symbols[2], "eth"),
        ] {
            command_tx
                .send(OrderTaskCommand::Submit {
                    action: submit_action(&policy, &client_order_ids, symbol, id),
                    enqueued_at: Instant::now(),
                })
                .await
                .unwrap();
        }

        let first = receive_started(&mut started).await;
        let second = receive_started(&mut started).await;
        assert_eq!(
            HashSet::from([first, second]),
            HashSet::from([symbols[0].to_string(), symbols[2].to_string()])
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(25), started.recv())
                .await
                .is_err(),
            "the worker exceeded its in-flight bound"
        );

        release_order(&gates, symbols[0]);
        assert_eq!(receive_started(&mut started).await, symbols[1]);
        command_tx
            .send(OrderTaskCommand::Submit {
                action: submit_action(&policy, &client_order_ids, symbols[3], "sol"),
                enqueued_at: Instant::now(),
            })
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(25), started.recv())
                .await
                .is_err(),
            "a later underlying started before a bounded slot was free"
        );

        release_order(&gates, symbols[2]);
        assert_eq!(receive_started(&mut started).await, symbols[3]);
        release_order(&gates, symbols[1]);
        release_order(&gates, symbols[3]);

        let (flushed_tx, flushed_rx) = oneshot::channel();
        command_tx
            .send(OrderTaskCommand::Flush(flushed_tx))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), flushed_rx)
            .await
            .expect("command flush timed out")
            .unwrap();
        for _ in 0..4 {
            assert!(matches!(
                event_rx.recv().await,
                Some(RuntimeEvent::SubmitComplete { .. })
            ));
        }
        assert_eq!(max_active.load(Ordering::SeqCst), 2);

        command_tx.send(OrderTaskCommand::Shutdown).await.unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn order_task_rejects_authority_for_a_different_account_before_preparation() {
        let symbol = "BTC-USDT-SWAP";
        let (transport, mut started, _gates, _) = gated_order_transport(&[symbol]);
        let (gateway, policy, client_order_ids) =
            runtime_order_gateway_with_execution(&[symbol], Vec::new(), Box::new(transport));
        let (command_tx, command_rx) = mpsc::channel(2);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let task = tokio::spawn(run_order_task(
            "other-account".to_string(),
            gateway,
            command_rx,
            event_tx,
            1,
            2,
        ));

        command_tx
            .send(OrderTaskCommand::Submit {
                action: submit_action(&policy, &client_order_ids, symbol, "wrong-account"),
                enqueued_at: Instant::now(),
            })
            .await
            .unwrap();
        let event = event_rx.recv().await.expect("worker must fail closed");
        assert!(matches!(
            event,
            RuntimeEvent::Fatal(RuntimeTaskFailure::Gateway(message))
                if message.contains("received submit authority for account main")
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(25), started.recv())
                .await
                .is_err(),
            "wrong-account authority reached the order transport"
        );

        command_tx.send(OrderTaskCommand::Shutdown).await.unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn reconciliation_completes_while_an_order_command_is_blocked() {
        let symbol = "BTC-USDT-SWAP";
        let responses = vec![
            Ok(HttpResponse {
                status: 200,
                body: r#"{"code":"0","msg":"","data":[]}"#.to_string(),
            }),
            Ok(HttpResponse {
                status: 200,
                body: r#"{"code":"0","msg":"","data":[]}"#.to_string(),
            }),
            Ok(HttpResponse {
                status: 200,
                body: r#"{"code":"0","msg":"","data":[{"uTime":"100","details":[{"ccy":"USDT","cashBal":"100","availBal":"90","eq":"100","liab":"0","maxLoan":"0"}]}]}"#.to_string(),
            }),
            Ok(HttpResponse {
                status: 200,
                body: r#"{"code":"0","msg":"","data":[]}"#.to_string(),
            }),
        ];
        let (transport, mut started, gates, _) = gated_order_transport(&[symbol]);
        let (gateway, policy, client_order_ids) =
            runtime_order_gateway_with_execution(&[symbol], responses, Box::new(transport));
        let io = gateway.reconciliation_client();
        let (command_tx, command_rx) = mpsc::channel(8);
        let (reconcile_tx, reconcile_rx) = mpsc::channel(2);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let order_task = tokio::spawn(run_order_task(
            "main".to_string(),
            gateway,
            command_rx,
            event_tx.clone(),
            1,
            8,
        ));
        let reconcile_task = tokio::spawn(run_reconcile_task(
            "main".to_string(),
            io,
            reconcile_rx,
            event_tx,
            10_000,
            2,
            2,
        ));

        command_tx
            .send(OrderTaskCommand::Submit {
                action: submit_action(&policy, &client_order_ids, symbol, "blocked"),
                enqueued_at: Instant::now(),
            })
            .await
            .unwrap();
        assert_eq!(receive_started(&mut started).await, symbol);
        reconcile_tx
            .send(ReconcileTaskCommand::Reconcile {
                restored_orders: Vec::new(),
                command_flush: None,
            })
            .await
            .unwrap();

        let event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .expect("reconciliation was blocked behind order acknowledgement")
            .expect("runtime event channel closed");
        let RuntimeEvent::RemoteState {
            remote_orders,
            remote_account,
            ..
        } = event
        else {
            panic!("blocked command completed before independent reconciliation");
        };
        assert!(remote_orders.is_empty());
        assert_eq!(remote_account.balances.len(), 1);

        release_order(&gates, symbol);
        let (flushed_tx, flushed_rx) = oneshot::channel();
        command_tx
            .send(OrderTaskCommand::Flush(flushed_tx))
            .await
            .unwrap();
        flushed_rx.await.unwrap();
        assert!(matches!(
            event_rx.recv().await,
            Some(RuntimeEvent::SubmitComplete { .. })
        ));

        command_tx.send(OrderTaskCommand::Shutdown).await.unwrap();
        reconcile_tx
            .send(ReconcileTaskCommand::Shutdown)
            .await
            .unwrap();
        order_task.await.unwrap();
        reconcile_task.await.unwrap();
    }

    #[test]
    fn latency_duration_rounds_up_to_microseconds() {
        assert_eq!(duration_us_ceil(Duration::ZERO), 0);
        assert_eq!(duration_us_ceil(Duration::from_nanos(1)), 1);
        assert_eq!(duration_us_ceil(Duration::from_nanos(1_000)), 1);
        assert_eq!(duration_us_ceil(Duration::from_nanos(1_001)), 2);
    }

    #[test]
    fn runtime_failure_evidence_has_a_stable_code_and_utf8_bound() {
        let error = combine_lifecycle_errors(
            LiveRuntimeError::GatewayTask("\u{20ac}".repeat(2_000)),
            vec![("runtime teardown", LiveRuntimeError::EventChannelClosed)],
        );

        let evidence = live_failure_evidence(&error);

        assert_eq!(evidence.code, "gateway_task");
        assert!(evidence.message.len() <= MAX_LIVE_FAILURE_MESSAGE_BYTES);
        assert!(evidence.message.ends_with('\u{20ac}'));
        assert_eq!(truncate_utf8("a\u{20ac}b".to_string(), 3), "a");
        assert_eq!(
            live_failure_evidence(&LiveRuntimeError::DeadmanHeartbeat("test".to_string())).code,
            "deadman_heartbeat"
        );
        assert_eq!(
            live_failure_evidence(&LiveRuntimeError::ExchangeClockSkew("test".to_string())).code,
            "exchange_clock_skew"
        );
        assert_eq!(
            live_failure_evidence(&LiveRuntimeError::ExchangeClockCheck("test".to_string())).code,
            "exchange_clock_check"
        );
        assert_eq!(
            live_failure_evidence(&LiveRuntimeError::ExchangeStatus("test".to_string())).code,
            "exchange_status"
        );
        assert_eq!(
            live_failure_evidence(&LiveRuntimeError::ExchangeStatusCheck("test".to_string())).code,
            "exchange_status_check"
        );
        assert_eq!(
            live_failure_evidence(&LiveRuntimeError::ExchangeFeeDrift("test".to_string())).code,
            "exchange_fee_drift"
        );
        assert_eq!(
            live_failure_evidence(&LiveRuntimeError::ExchangeFeeCheck("test".to_string())).code,
            "exchange_fee_check"
        );
        assert_eq!(
            live_failure_evidence(&LiveRuntimeError::ExchangeInstrumentDrift(
                "test".to_string()
            ))
            .code,
            "exchange_instrument_drift"
        );
        assert_eq!(
            live_failure_evidence(&LiveRuntimeError::ExchangeInstrumentCheck(
                "test".to_string()
            ))
            .code,
            "exchange_instrument_check"
        );
        assert_eq!(
            live_failure_evidence(&LiveRuntimeError::AccountConfigDrift("test".to_string())).code,
            "account_config_drift"
        );
        assert_eq!(
            live_failure_evidence(&LiveRuntimeError::AccountConfigCheck("test".to_string())).code,
            "account_config_check"
        );
    }

    #[test]
    fn pseudonymous_identity_hash_is_stable_and_field_delimited() {
        let first = crate::provenance::identity_sha256(b"account", &[b"ab", b"c"]);
        assert_eq!(
            first,
            crate::provenance::identity_sha256(b"account", &[b"ab", b"c"])
        );
        assert_ne!(
            first,
            crate::provenance::identity_sha256(b"account", &[b"a", b"bc"])
        );
        assert_ne!(
            first,
            crate::provenance::identity_sha256(b"host", &[b"ab", b"c"])
        );
        assert_eq!(first.len(), 64);
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RecordedRoleRequest {
        path: String,
        body: String,
    }

    struct SafetyMockPort {
        responses: Arc<Mutex<VecDeque<Result<String, RestError>>>>,
        requests: Arc<Mutex<Vec<RecordedRoleRequest>>>,
    }

    impl SafetyMockPort {
        fn next(
            &self,
            path: impl Into<String>,
            body: impl Into<String>,
        ) -> Result<String, RestError> {
            self.requests.lock().unwrap().push(RecordedRoleRequest {
                path: path.into(),
                body: body.into(),
            });
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("mock role response")
        }
    }

    #[async_trait]
    impl ReadinessPort for SafetyMockPort {
        async fn server_time_ms(&self) -> Result<u64, RestError> {
            let body = self.next("/api/v5/public/time", "")?;
            parse_okx_server_time_response_json(body.as_bytes())
        }

        async fn system_status(&self) -> Result<Vec<OkxSystemStatus>, RestError> {
            let body = self.next("/api/v5/system/status", "")?;
            parse_okx_system_status_response_json(body.as_bytes())
        }

        async fn account_config(&self) -> Result<OkxAccountConfig, RestError> {
            let body = self.next("/api/v5/account/config", "")?;
            parse_okx_account_config_response_json(body.as_bytes())
        }

        async fn account_balance_snapshot(
            &self,
        ) -> Result<reap_venue::okx::OkxAccountBalanceSnapshot, RestError> {
            let body = self.next("/api/v5/account/balance", "")?;
            parse_okx_account_balance_response_json(body.as_bytes())
        }

        async fn account_positions_snapshot(
            &self,
            _instrument_type: Option<OkxInstrumentType>,
            _symbol: Option<&str>,
        ) -> Result<reap_venue::okx::OkxAccountPositionsSnapshot, RestError> {
            let body = self.next("/api/v5/account/positions", "")?;
            parse_okx_account_positions_response_json(body.as_bytes())
        }

        async fn account_instrument(
            &self,
            instrument_type: OkxInstrumentType,
            symbol: &str,
        ) -> Result<OkxInstrument, RestError> {
            let path = format!(
                "/api/v5/account/instruments?instType={}&instId={symbol}",
                instrument_type.as_str()
            );
            let body = self.next(path, "")?;
            parse_okx_account_instruments_response_json(body.as_bytes())?
                .into_iter()
                .next()
                .ok_or(RestError::EmptyData {
                    operation: "account instrument",
                })
        }

        async fn account_trade_fee(
            &self,
            instrument_type: OkxInstrumentType,
            instrument_id: Option<&str>,
            instrument_family: Option<&str>,
            group_id: &str,
        ) -> Result<OkxTradeFeeRate, RestError> {
            let selector = instrument_id
                .map(|value| format!("&instId={value}"))
                .or_else(|| instrument_family.map(|value| format!("&instFamily={value}")))
                .unwrap_or_default();
            let path = format!(
                "/api/v5/account/trade-fee?instType={}{}",
                instrument_type.as_str(),
                selector
            );
            let body = self.next(path, "")?;
            parse_okx_trade_fee_response_json(body.as_bytes())?
                .into_iter()
                .find(|rate| rate.group_id == group_id)
                .ok_or(RestError::EmptyData {
                    operation: "account trade fee",
                })
        }
    }

    #[async_trait]
    impl SafetyPort for SafetyMockPort {
        async fn cancel_all_after(&self, timeout_secs: u64) -> Result<(), RestError> {
            let body = format!(r#"{{"timeOut":"{timeout_secs}"}}"#);
            let response = self.next("/api/v5/trade/cancel-all-after", body)?;
            parse_okx_cancel_all_after_response_json(response.as_bytes(), timeout_secs)
        }
    }

    struct BlockingFeePort {
        fee_started: Arc<Notify>,
    }

    #[async_trait]
    impl ReadinessPort for BlockingFeePort {
        async fn account_instrument(
            &self,
            _instrument_type: OkxInstrumentType,
            _symbol: &str,
        ) -> Result<OkxInstrument, RestError> {
            Ok(
                parse_okx_account_instruments_response_json(spot_instrument_response().as_bytes())?
                    .into_iter()
                    .next()
                    .unwrap(),
            )
        }

        async fn account_trade_fee(
            &self,
            _instrument_type: OkxInstrumentType,
            _instrument_id: Option<&str>,
            _instrument_family: Option<&str>,
            _group_id: &str,
        ) -> Result<OkxTradeFeeRate, RestError> {
            self.fee_started.notify_one();
            std::future::pending().await
        }
    }

    struct BlockingInstrumentPort {
        instrument_started: Arc<Notify>,
    }

    #[async_trait]
    impl ReadinessPort for BlockingInstrumentPort {
        async fn account_instrument(
            &self,
            _instrument_type: OkxInstrumentType,
            _symbol: &str,
        ) -> Result<OkxInstrument, RestError> {
            self.instrument_started.notify_one();
            std::future::pending().await
        }

        async fn account_trade_fee(
            &self,
            _instrument_type: OkxInstrumentType,
            _instrument_id: Option<&str>,
            _instrument_family: Option<&str>,
            _group_id: &str,
        ) -> Result<OkxTradeFeeRate, RestError> {
            panic!("fee should not run while the instrument request is blocked")
        }
    }

    struct NotifyingSafety {
        deadman_seen: Arc<Notify>,
    }

    #[async_trait]
    impl SafetyPort for NotifyingSafety {
        async fn cancel_all_after(&self, _timeout_secs: u64) -> Result<(), RestError> {
            self.deadman_seen.notify_one();
            Ok(())
        }
    }

    fn safety_client(
        responses: Vec<Result<&str, RestError>>,
    ) -> (Arc<SafetyMockPort>, Arc<Mutex<Vec<RecordedRoleRequest>>>) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let port = Arc::new(SafetyMockPort {
            responses: Arc::new(Mutex::new(
                responses
                    .into_iter()
                    .map(|response| response.map(str::to_string))
                    .collect(),
            )),
            requests: Arc::clone(&requests),
        });
        (port, requests)
    }

    fn safety_account_config() -> OkxAccountConfig {
        OkxAccountConfig {
            account_level: OkxAccountLevel::SingleCurrencyMargin,
            position_mode: OkxPositionMode::NetMode,
            account_stp_mode: "cancel_maker".to_string(),
            user_id: "7".to_string(),
            main_user_id: "6".to_string(),
            api_key_label: "reap-demo".to_string(),
            api_key_permissions: BTreeSet::from([
                OkxApiKeyPermission::ReadOnly,
                OkxApiKeyPermission::Trade,
            ]),
            api_key_ip_bindings: BTreeSet::from(["203.0.113.5".to_string()]),
            enable_spot_borrow: Some(false),
            auto_loan: Some(false),
            spot_borrow_auto_repay: Some(false),
        }
    }

    fn exchange_status_guard(enabled: bool, check_interval_ms: u64) -> ExchangeStatusGuard {
        ExchangeStatusGuard {
            enabled,
            relevance: ChaosConnectivityPlan::resolve(&config(), LiveMode::Demo)
                .unwrap()
                .maintenance_relevance()
                .clone(),
            check_interval_ms,
            lead_ms: 60_000,
        }
    }

    fn exchange_instrument_guard(
        check_interval_ms: u64,
        expectations: Vec<ExchangeInstrumentExpectation>,
    ) -> ExchangeInstrumentGuard {
        ExchangeInstrumentGuard {
            sweep_interval_ms: check_interval_ms,
            change_lead_ms: 3_600_000,
            expectations,
        }
    }

    fn exchange_instrument_expectation(
        configured_maker_cost: f64,
        configured_taker_cost: f64,
    ) -> ExchangeInstrumentExpectation {
        ExchangeInstrumentExpectation {
            symbol: "BTC-USDT".to_string(),
            instrument_type: OkxInstrumentType::Spot,
            instrument_id: Some("BTC-USDT".to_string()),
            instrument_family: None,
            group_id: "1".to_string(),
            configured_maker_cost,
            configured_taker_cost,
            expected_instrument: spot_instrument(Vec::new()),
        }
    }

    fn spot_instrument(upcoming_changes: Vec<OkxInstrumentChange>) -> OkxInstrument {
        OkxInstrument {
            symbol: "BTC-USDT".to_string(),
            instrument_type: OkxInstrumentType::Spot,
            instrument_family: String::new(),
            trade_fee_group_id: "1".to_string(),
            underlying: String::new(),
            base_currency: "BTC".to_string(),
            quote_currency: "USDT".to_string(),
            settle_currency: String::new(),
            contract_type: None,
            contract_value: None,
            contract_value_currency: String::new(),
            tick_size: 0.1,
            lot_size: 0.001,
            min_size: 0.001,
            max_limit_size: 100.0,
            max_market_size: 1_000_000.0,
            max_limit_amount_usd: Some(1_000_000.0),
            max_market_amount_usd: Some(1_000_000.0),
            state: "live".to_string(),
            upcoming_changes,
        }
    }

    fn spot_instrument_response() -> &'static str {
        r#"{"code":"0","msg":"","data":[{"instId":"BTC-USDT","instType":"SPOT","instFamily":"","groupId":"1","baseCcy":"BTC","quoteCcy":"USDT","settleCcy":"","ctType":"","ctVal":"","ctValCcy":"","tickSz":"0.1","lotSz":"0.001","minSz":"0.001","maxLmtSz":"100","maxMktSz":"1000000","maxLmtAmt":"1000000","maxMktAmt":"1000000","state":"live","upcChg":[]}]}"#
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
        strategy.reference_data_stale_threshold_ms = Some(120_000);
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
                api_key_policy: crate::OkxApiKeyPolicyConfig::default(),
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
            missing_order_transports: Vec::new(),
            missing_stablecoin_rates: Vec::new(),
            missing_strategy_references: Vec::new(),
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
                forced_repayment_indicator: None,
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
                        order_limits: InstrumentOrderLimits {
                            max_limit_quantity: 1_000_000.0,
                            max_limit_notional_usd: instrument
                                .kind
                                .is_spot()
                                .then_some(1_000_000.0),
                        },
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
            quote_stp_verified_accounts: config
                .accounts
                .iter()
                .map(|account| account.id.clone())
                .collect(),
        }
    }

    fn ready_coordinator(
        config: &LiveConfig,
        now_ms: u64,
        gateway_actions_enabled: bool,
    ) -> LiveCoordinator {
        let update = account_update(now_ms);
        let mut approval_scopes = HashMap::new();
        if gateway_actions_enabled {
            for account in &config.accounts {
                let responses = Arc::new(Mutex::new(VecDeque::new()));
                let mut gateway = OkxOrderGateway::new(
                    account.id.clone(),
                    Box::new(RuntimeMockRoles {
                        responses: Arc::clone(&responses),
                    }),
                    Arc::new(RuntimeMockRoles { responses }),
                    account
                        .trade_modes
                        .iter()
                        .map(|(symbol, mode)| (symbol.clone(), (*mode).into()))
                        .collect(),
                    PacingPolicy::default(),
                )
                .unwrap();
                approval_scopes.insert(account.id.clone(), gateway.take_approval_scope().unwrap());
            }
        }
        let mut coordinator = LiveCoordinator::new(
            config.clone(),
            verified(config, update.clone()),
            approval_scopes,
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
        coordinator.process_event(NormalizedEvent::System(SystemEvent {
            ts_ms: now_ms,
            kind: SystemEventKind::OrderTransportRecovered,
            venue: Some(Venue::Okx),
            account_id: Some("main".to_string()),
            symbol: None,
            reason: "test order sockets".to_string(),
        }));
        coordinator
            .on_forbidden_order_event(ForbiddenOrderEvent {
                account_id: "main".to_string(),
                observed_at_ms: now_ms,
                state: ForbiddenOrderState::VerifiedZero {
                    expires_at_ms: now_ms + 30_000,
                },
            })
            .unwrap();
        for requirement in config.strategy.reference_data_requirements() {
            let event = match requirement.kind {
                ReferenceDataKind::IndexPrice => MarketEvent::IndexPrice {
                    ts_ms: now_ms,
                    symbol: requirement.symbol,
                    price: 100.0,
                },
                ReferenceDataKind::FundingRate => MarketEvent::FundingRate {
                    ts_ms: now_ms,
                    symbol: requirement.symbol,
                    rate: 0.0001,
                    funding_time_ms: now_ms + 28_800_000,
                    settlement: None,
                },
                ReferenceDataKind::MarkPrice => MarketEvent::PriceLimits {
                    ts_ms: now_ms,
                    symbol: requirement.symbol,
                    mark_price: 100.0,
                    limit_down: 0.0,
                    limit_up: 0.0,
                },
                ReferenceDataKind::PriceLimits => MarketEvent::PriceLimits {
                    ts_ms: now_ms,
                    symbol: requirement.symbol,
                    mark_price: 0.0,
                    limit_down: 50.0,
                    limit_up: 150.0,
                },
            };
            coordinator.process_event(NormalizedEvent::Market(event));
        }
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
        assert_eq!(recovered_safety_latch_count(&recovered), 3);

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
    fn restart_restores_exchange_binding_for_active_order() {
        let config = config();
        let mut coordinator = ready_coordinator(&config, unix_time_ms(), true);
        coordinator
            .restore_owned_order(
                recovered_submit_proof("main", "BTC-USDT", "restored-live"),
                OrderUpdate {
                    ts_ms: 2,
                    order_id: "restored-live".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    side: Side::Buy,
                    event: OrderEvent::New,
                    status: OrderStatus::Live,
                    price: 100.0,
                    time_in_force: Some(reap_core::TimeInForce::PostOnly),
                    qty: 1.0,
                    open_qty: 1.0,
                    filled_qty: 0.0,
                    avg_fill_price: 0.0,
                    last_fill_qty: 0.0,
                    last_fill_price: 0.0,
                    last_fill_liquidity: None,
                    last_fill_fee: None,
                    reason: "restored quote".to_string(),
                },
            )
            .unwrap();
        let mut recovered = recover_storage_records([
            StorageRecord::OrderRequest(OrderRequestRecord {
                ts_ms: 1,
                account_id: "main".to_string(),
                operation: OrderOperation::Submit,
                idempotency_key: Some("decision-1".to_string()),
                client_order_id: Some("restored-live".to_string()),
                exchange_order_id: None,
                symbol: "BTC-USDT".to_string(),
            }),
            StorageRecord::OrderAck(reap_storage::OrderAckRecord {
                ts_ms: 2,
                account_id: "main".to_string(),
                operation: OrderOperation::Submit,
                client_order_id: "restored-live".to_string(),
                exchange_order_id: Some("exchange-1".to_string()),
                status: OrderAckStatus::Accepted,
                message: "accepted".to_string(),
            }),
        ]);

        restore_active_order_bindings(&mut coordinator, &mut recovered).unwrap();

        assert_eq!(
            coordinator
                .private_state("main")
                .unwrap()
                .canonical_order_id("exchange-1"),
            Some("restored-live")
        );
    }

    #[test]
    fn recovered_order_binding_account_must_match_live_config() {
        let config = config();
        let mut coordinator = ready_coordinator(&config, unix_time_ms(), true);
        let mut recovered = recover_storage_records([
            StorageRecord::OrderRequest(OrderRequestRecord {
                ts_ms: 1,
                account_id: "removed".to_string(),
                operation: OrderOperation::Submit,
                idempotency_key: Some("decision-1".to_string()),
                client_order_id: Some("order-1".to_string()),
                exchange_order_id: None,
                symbol: "BTC-USDT".to_string(),
            }),
            StorageRecord::OrderAck(reap_storage::OrderAckRecord {
                ts_ms: 2,
                account_id: "removed".to_string(),
                operation: OrderOperation::Submit,
                client_order_id: "order-1".to_string(),
                exchange_order_id: Some("exchange-1".to_string()),
                status: OrderAckStatus::Accepted,
                message: "accepted".to_string(),
            }),
        ]);

        let error = restore_active_order_bindings(&mut coordinator, &mut recovered).unwrap_err();

        assert!(error.to_string().contains("unknown account removed"));
    }

    #[test]
    fn restart_restores_only_orders_with_durable_regular_submit_proof() {
        let config = config();
        let update = OrderUpdate {
            ts_ms: 2,
            order_id: "foreign-or-legacy".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            event: OrderEvent::New,
            status: OrderStatus::Live,
            price: 100.0,
            time_in_force: Some(TimeInForce::PostOnly),
            qty: 1.0,
            open_qty: 1.0,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            last_fill_fee: None,
            reason: "private observation".to_string(),
        };
        let mut unproven = recover_storage_records([StorageRecord::Order {
            account_id: Some("main".to_string()),
            update: update.clone(),
        }]);
        assert!(proven_active_recovered_orders(&config, &mut unproven).is_empty());

        let mut proven = recover_storage_records([
            StorageRecord::OrderRequest(OrderRequestRecord {
                ts_ms: 1,
                account_id: "main".to_string(),
                operation: OrderOperation::Submit,
                idempotency_key: Some("decision-owned".to_string()),
                client_order_id: Some(update.order_id.clone()),
                exchange_order_id: None,
                symbol: update.symbol.clone(),
            }),
            StorageRecord::Order {
                account_id: Some("main".to_string()),
                update: update.clone(),
            },
        ]);
        let restored = proven_active_recovered_orders(&config, &mut proven);
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].0.order_id, update.order_id);
        assert_eq!(restored[0].0.symbol, update.symbol);
        assert_eq!(restored[0].0.status, update.status);
        assert_eq!(restored[0].1.account_id(), "main");
        assert_eq!(restored[0].1.client_order_id(), update.order_id);
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
            .restore_owned_order(
                recovered_submit_proof("main", "BTC-USDT", "restored-live"),
                OrderUpdate {
                    ts_ms: 2,
                    order_id: "restored-live".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    side: Side::Buy,
                    event: OrderEvent::New,
                    status: OrderStatus::Live,
                    price: 100.0,
                    time_in_force: Some(reap_core::TimeInForce::PostOnly),
                    qty: 1.0,
                    open_qty: 1.0,
                    filled_qty: 0.0,
                    avg_fill_price: 0.0,
                    last_fill_qty: 0.0,
                    last_fill_price: 0.0,
                    last_fill_liquidity: None,
                    last_fill_fee: None,
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
                    LiveAction::Cancel(cancel) if cancel.client_order_id() == "restored-live"
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
    fn runtime_evidence_classifies_fault_campaign_events() {
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
        evidence.observe_record(&system(SystemEventKind::OrderTransportStale));
        for operation in [OrderOperation::Submit, OrderOperation::Cancel] {
            evidence.observe_record(&StorageRecord::OrderAck(reap_storage::OrderAckRecord {
                ts_ms: 1,
                account_id: "main".to_string(),
                operation,
                client_order_id: "client-1".to_string(),
                exchange_order_id: None,
                status: OrderAckStatus::Ambiguous,
                message: "test ambiguity".to_string(),
            }));
        }
        evidence.observe_record(&StorageRecord::Order {
            account_id: Some("main".to_string()),
            update: OrderUpdate {
                ts_ms: 1,
                order_id: "client-1".to_string(),
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                event: OrderEvent::PartialFill,
                status: OrderStatus::PartiallyFilled,
                price: 100.0,
                time_in_force: Some(reap_core::TimeInForce::PostOnly),
                qty: 1.0,
                open_qty: 0.5,
                filled_qty: 0.5,
                avg_fill_price: 100.0,
                last_fill_qty: 0.5,
                last_fill_price: 100.0,
                last_fill_liquidity: None,
                last_fill_fee: None,
                reason: "partial fill".to_string(),
            },
        });
        evidence.observe_fill_convergence_timeout();
        evidence.observe_order_convergence_timeout();
        evidence.observe_disconnect(false);
        evidence.observe_disconnect(true);
        evidence.observe_order_transport_disconnect();

        assert_eq!(evidence.reconciliation_drift_events, 1);
        assert_eq!(evidence.book_recovery_events, 1);
        assert_eq!(evidence.stream_stale_events, 2);
        assert_eq!(evidence.order_transport_stale_events, 1);
        assert_eq!(evidence.connection_disconnect_events, 3);
        assert_eq!(evidence.public_connection_disconnect_events, 1);
        assert_eq!(evidence.private_connection_disconnect_events, 1);
        assert_eq!(evidence.order_transport_disconnect_events, 1);
        assert_eq!(evidence.ambiguous_submit_events, 1);
        assert_eq!(evidence.ambiguous_cancel_events, 1);
        assert_eq!(evidence.partial_fill_events, 1);
        assert_eq!(evidence.fill_convergence_timeout_events, 1);
        assert_eq!(evidence.order_convergence_timeout_events, 1);
    }

    #[test]
    fn live_session_evidence_excludes_bootstrap_order_outcomes() {
        let mut evidence = RuntimeEvidence {
            ambiguous_submit_events: 1,
            ambiguous_cancel_events: 1,
            partial_fill_events: 1,
            max_storage_queue_depth: 7,
            ..RuntimeEvidence::default()
        };

        evidence.begin_live_session(3);

        assert_eq!(evidence.ambiguous_submit_events, 0);
        assert_eq!(evidence.ambiguous_cancel_events, 0);
        assert_eq!(evidence.partial_fill_events, 0);
        assert_eq!(evidence.restored_safety_latches, 3);
        assert_eq!(evidence.max_storage_queue_depth, 7);
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

    #[test]
    fn preparation_resolves_the_secret_free_plan_without_credentials() {
        let mut config = config();
        let missing_prefix = format!("REAP_PHASE1_MISSING_{}", std::process::id());
        config.accounts[0].api_key_env = format!("{missing_prefix}_KEY");
        config.accounts[0].secret_key_env = format!("{missing_prefix}_SECRET");
        config.accounts[0].passphrase_env = format!("{missing_prefix}_PASSPHRASE");
        for name in [
            &config.accounts[0].api_key_env,
            &config.accounts[0].secret_key_env,
            &config.accounts[0].passphrase_env,
        ] {
            assert!(std::env::var_os(name).is_none());
        }

        let prepared = prepare_live(
            config,
            LiveRunOptions {
                mode: LiveMode::Observe,
                demo_confirmed: false,
                run_duration: Some(Duration::from_millis(1)),
            },
        )
        .unwrap();

        assert_eq!(prepared.connectivity_plan().mode(), LiveMode::Observe);
        assert_eq!(prepared.connectivity_plan().sha256().len(), 64);
        assert!(prepared.connectivity_plan().regular_mutations().is_empty());
    }

    #[test]
    fn unsupported_burst_input_fails_preparation_before_credentials() {
        let mut config = config();
        let missing_prefix = format!("REAP_PHASE1_BURST_MISSING_{}", std::process::id());
        config.accounts[0].api_key_env = format!("{missing_prefix}_KEY");
        config.accounts[0].secret_key_env = format!("{missing_prefix}_SECRET");
        config.accounts[0].passphrase_env = format!("{missing_prefix}_PASSPHRASE");
        config.strategy.act_on_burst = true;

        let error = prepare_live(
            config,
            LiveRunOptions {
                mode: LiveMode::Observe,
                demo_confirmed: false,
                run_duration: None,
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            LiveRuntimeError::Config(LiveConfigError::Invalid(ref message))
                if message.contains("strategy.act_on_burst is unsupported by live modes")
        ));
    }

    #[tokio::test]
    async fn startup_task_group_aborts_owned_tasks_on_early_exit() {
        let (started_tx, started_rx) = oneshot::channel();
        let (dropped_tx, dropped_rx) = oneshot::channel();
        let mut tasks = StartupTaskGroup::default();
        tasks.push(tokio::spawn(async move {
            let _drop_signal = TaskDropSignal(Some(dropped_tx));
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        }));
        started_rx.await.unwrap();

        drop(tasks);

        tokio::time::timeout(Duration::from_secs(1), dropped_rx)
            .await
            .expect("startup task was not aborted")
            .unwrap();
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
        assert!(report.failure.is_none());
        assert!(!report.reached_ready);
        assert!(!report.clean_soak);
        assert_eq!(report.schema_version, LIVE_RUN_REPORT_SCHEMA_VERSION);
        assert_eq!(report.java_reference_revision, PINNED_JAVA_REVISION);
        assert_eq!(report.executable_sha256.len(), 64);
        assert!(report.host_identity_sha256.is_none());
        assert!(report.account_identity_sha256s.is_empty());
        assert!(report.latency_evidence.series.is_empty());
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
        let (_forbidden_tx, forbidden_rx) = mpsc::channel(16);
        let runtime = TestRuntimeParts {
            session_id: "test-alert-session".to_string(),
            session_started_at_ms: unix_time_ms(),
            config_source: None,
            config_fingerprint: "test-config".to_string(),
            evidence_config_fingerprint: "test-evidence-config".to_string(),
            executable_sha256: "a".repeat(64),
            host_identity_sha256: None,
            account_identity_sha256s: BTreeMap::new(),
            mode: LiveMode::Observe,
            run_duration: Some(Duration::from_millis(25)),
            coordinator,
            processor: FeedProcessor::new(16, 16),
            storage: Some(storage),
            storage_sink,
            control_rx,
            feed_rx,
            forbidden_rx,
            order_senders: HashMap::new(),
            order_tasks: Vec::new(),
            reconcile_senders: HashMap::new(),
            reconcile_tasks: Vec::new(),
            order_ws_runtimes: Vec::new(),
            order_ws_status_tasks: Vec::new(),
            safety_senders: HashMap::new(),
            safety_tasks: Vec::new(),
            forbidden_tasks: Vec::new(),
            feeds: Vec::new(),
            feed_tasks: Vec::new(),
            sources: Vec::new(),
            public_feed_index: 0,
            reconcile_inflight: HashSet::new(),
            cancel_inflight: HashSet::new(),
            last_reconcile_attempt: HashMap::new(),
            fill_convergence: FillConvergenceGuard::default(),
            order_convergence: OrderStateConvergenceGuard::new(5_000),
            readiness_timeout_ms: 1_000,
            timer_interval_ms: 100,
            max_feed_age_ms: 60_000,
            shutdown_timeout_ms: 100,
            teardown_timeout_ms: 1_000,
            safety_latch_sync_timeout_ms: 1_000,
            evidence: RuntimeEvidence::default(),
            latency: LiveLatencyCollector::default(),
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
        }
        .into_runtime();

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

    #[tokio::test]
    async fn stalled_teardown_is_aborted_and_reported_within_the_deadline() {
        struct AbortNotice(Option<oneshot::Sender<()>>);

        impl Drop for AbortNotice {
            fn drop(&mut self) {
                if let Some(sender) = self.0.take() {
                    let _ = sender.send(());
                }
            }
        }

        let config = config();
        let coordinator = ready_coordinator(&config, unix_time_ms(), false);
        let path = std::env::temp_dir().join(format!(
            "reap-teardown-timeout-{}-{}.jsonl",
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
        let (_forbidden_tx, forbidden_rx) = mpsc::channel(16);
        let (aborted_tx, aborted_rx) = oneshot::channel();
        let stalled_task = tokio::spawn(async move {
            let _notice = AbortNotice(Some(aborted_tx));
            std::future::pending::<()>().await;
        });
        let runtime = TestRuntimeParts {
            session_id: "test-teardown-timeout".to_string(),
            session_started_at_ms: unix_time_ms(),
            config_source: None,
            config_fingerprint: "test-config".to_string(),
            evidence_config_fingerprint: "test-evidence-config".to_string(),
            executable_sha256: "a".repeat(64),
            host_identity_sha256: None,
            account_identity_sha256s: BTreeMap::new(),
            mode: LiveMode::Observe,
            run_duration: Some(Duration::from_millis(5)),
            coordinator,
            processor: FeedProcessor::new(16, 16),
            storage: Some(storage),
            storage_sink,
            control_rx,
            feed_rx,
            forbidden_rx,
            order_senders: HashMap::new(),
            order_tasks: Vec::new(),
            reconcile_senders: HashMap::new(),
            reconcile_tasks: Vec::new(),
            order_ws_runtimes: Vec::new(),
            order_ws_status_tasks: Vec::new(),
            safety_senders: HashMap::new(),
            safety_tasks: Vec::new(),
            forbidden_tasks: Vec::new(),
            feeds: Vec::new(),
            feed_tasks: vec![stalled_task],
            sources: Vec::new(),
            public_feed_index: 0,
            reconcile_inflight: HashSet::new(),
            cancel_inflight: HashSet::new(),
            last_reconcile_attempt: HashMap::new(),
            fill_convergence: FillConvergenceGuard::default(),
            order_convergence: OrderStateConvergenceGuard::new(5_000),
            readiness_timeout_ms: 1_000,
            timer_interval_ms: 100,
            max_feed_age_ms: 60_000,
            shutdown_timeout_ms: 100,
            teardown_timeout_ms: 25,
            safety_latch_sync_timeout_ms: 1_000,
            evidence: RuntimeEvidence::default(),
            latency: LiveLatencyCollector::default(),
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
        }
        .into_runtime();

        let error = tokio::time::timeout(Duration::from_secs(1), runtime.run())
            .await
            .expect("teardown must honor its application deadline")
            .unwrap_err();
        drop(control_tx);
        drop(feed_tx);
        tokio::time::timeout(Duration::from_secs(1), aborted_rx)
            .await
            .expect("stalled task must be aborted")
            .expect("abort notice sender must remain live until cancellation");

        let LiveRuntimeError::ReportedFailure { source, report } = error else {
            panic!("teardown timeout must retain a typed run report");
        };
        assert!(matches!(*source, LiveRuntimeError::TeardownTimeout(25)));
        assert_eq!(report.stop_reason, LiveStopReason::RuntimeFailure);
        assert_eq!(report.failure.as_ref().unwrap().code, "teardown_timeout");
        assert!(!report.clean_soak);

        let lease =
            acquire_storage_lease(&path).expect("aborted writer must release journal lease");
        drop(lease);
        let _ = std::fs::remove_file(path.with_extension("jsonl.lock"));
        let _ = std::fs::remove_file(path);
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
        let (_forbidden_tx, forbidden_rx) = mpsc::channel(16);
        let (operator_tx, operator_rx) = mpsc::channel(16);
        let operator_service =
            start_operator_service(&operator_config, SECRET.to_vec(), operator_tx)
                .await
                .unwrap();
        let runtime = TestRuntimeParts {
            session_id: "test-operator-session".to_string(),
            session_started_at_ms: unix_time_ms(),
            config_source: None,
            config_fingerprint: "test-config".to_string(),
            evidence_config_fingerprint: "test-evidence-config".to_string(),
            executable_sha256: "a".repeat(64),
            host_identity_sha256: None,
            account_identity_sha256s: BTreeMap::new(),
            mode: LiveMode::Observe,
            run_duration: None,
            coordinator,
            processor: FeedProcessor::new(16, 16),
            storage: Some(storage),
            storage_sink,
            control_rx,
            feed_rx,
            forbidden_rx,
            order_senders: HashMap::new(),
            order_tasks: Vec::new(),
            reconcile_senders: HashMap::new(),
            reconcile_tasks: Vec::new(),
            order_ws_runtimes: Vec::new(),
            order_ws_status_tasks: Vec::new(),
            safety_senders: HashMap::new(),
            safety_tasks: Vec::new(),
            forbidden_tasks: Vec::new(),
            feeds: Vec::new(),
            feed_tasks: Vec::new(),
            sources: Vec::new(),
            public_feed_index: 0,
            reconcile_inflight: HashSet::new(),
            cancel_inflight: HashSet::new(),
            last_reconcile_attempt: HashMap::new(),
            fill_convergence: FillConvergenceGuard::default(),
            order_convergence: OrderStateConvergenceGuard::new(5_000),
            readiness_timeout_ms: 1_000,
            timer_interval_ms: 100,
            max_feed_age_ms: 60_000,
            shutdown_timeout_ms: 1_000,
            teardown_timeout_ms: 1_000,
            safety_latch_sync_timeout_ms: 1_000,
            evidence: RuntimeEvidence::default(),
            latency: LiveLatencyCollector::default(),
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
        }
        .into_runtime();
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
            .restore_owned_order(
                recovered_submit_proof("main", "BTC-USDT", "client-live"),
                OrderUpdate {
                    ts_ms: now_ms,
                    order_id: "client-live".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    side: Side::Buy,
                    event: OrderEvent::New,
                    status: OrderStatus::Live,
                    price: 100.0,
                    time_in_force: Some(reap_core::TimeInForce::PostOnly),
                    qty: 1.0,
                    open_qty: 1.0,
                    filled_qty: 0.0,
                    avg_fill_price: 0.0,
                    last_fill_qty: 0.0,
                    last_fill_price: 0.0,
                    last_fill_liquidity: None,
                    last_fill_fee: None,
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
        let (_forbidden_tx, forbidden_rx) = mpsc::channel(16);
        let (order_tx, mut order_rx) = mpsc::channel(16);
        let (reconcile_tx, mut reconcile_rx) = mpsc::channel(16);
        let cancel_observed = Arc::new(AtomicBool::new(false));
        let reconcile_received = Arc::new(Notify::new());
        let task_cancel_observed = Arc::clone(&cancel_observed);
        let task_reconcile_received = Arc::clone(&reconcile_received);
        let order_task = tokio::spawn(async move {
            while let Some(command) = order_rx.recv().await {
                match command {
                    OrderTaskCommand::Cancel { action, .. } => {
                        assert_eq!(action.client_order_id(), "client-live");
                        task_cancel_observed.store(true, Ordering::SeqCst);
                    }
                    OrderTaskCommand::Flush(waiter) => {
                        assert!(task_cancel_observed.load(Ordering::SeqCst));
                        task_reconcile_received.notified().await;
                        waiter.send(()).unwrap();
                    }
                    OrderTaskCommand::Submit { .. } => panic!("shutdown dispatched a submit"),
                    OrderTaskCommand::Shutdown => return,
                }
            }
        });
        let reconcile_cancel_observed = Arc::clone(&cancel_observed);
        let task_reconcile_received = Arc::clone(&reconcile_received);
        let task_events = control_tx.clone();
        let reconcile_task = tokio::spawn(async move {
            while let Some(command) = reconcile_rx.recv().await {
                match command {
                    ReconcileTaskCommand::Reconcile {
                        restored_orders: orders,
                        command_flush,
                    } => {
                        task_reconcile_received.notify_one();
                        if let Some(command_flush) = command_flush {
                            command_flush.await.unwrap();
                        }
                        assert!(reconcile_cancel_observed.load(Ordering::SeqCst));
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
                    ReconcileTaskCommand::Shutdown => return,
                }
            }
        });
        control_tx
            .send(RuntimeEvent::Fatal(RuntimeTaskFailure::Gateway(
                "injected runtime failure".to_string(),
            )))
            .await
            .unwrap();
        let mut runtime = TestRuntimeParts {
            session_id: "test-shutdown-session".to_string(),
            session_started_at_ms: unix_time_ms(),
            config_source: None,
            config_fingerprint: "test-config".to_string(),
            evidence_config_fingerprint: "test-evidence-config".to_string(),
            executable_sha256: "a".repeat(64),
            host_identity_sha256: None,
            account_identity_sha256s: BTreeMap::new(),
            mode: LiveMode::Demo,
            run_duration: None,
            coordinator,
            processor: FeedProcessor::new(16, 16),
            storage: None,
            storage_sink,
            control_rx,
            feed_rx,
            forbidden_rx,
            order_senders: HashMap::from([("main".to_string(), order_tx)]),
            order_tasks: vec![order_task],
            reconcile_senders: HashMap::from([("main".to_string(), reconcile_tx)]),
            reconcile_tasks: vec![reconcile_task],
            order_ws_runtimes: Vec::new(),
            order_ws_status_tasks: Vec::new(),
            safety_senders: HashMap::new(),
            safety_tasks: Vec::new(),
            forbidden_tasks: Vec::new(),
            feeds: Vec::new(),
            feed_tasks: Vec::new(),
            sources: Vec::new(),
            public_feed_index: 0,
            reconcile_inflight: HashSet::new(),
            cancel_inflight: HashSet::new(),
            last_reconcile_attempt: HashMap::new(),
            fill_convergence: FillConvergenceGuard::default(),
            order_convergence: OrderStateConvergenceGuard::new(5_000),
            readiness_timeout_ms: 1_000,
            timer_interval_ms: 100,
            max_feed_age_ms: 60_000,
            shutdown_timeout_ms: 1_000,
            teardown_timeout_ms: 1_000,
            safety_latch_sync_timeout_ms: 1_000,
            evidence: RuntimeEvidence::default(),
            latency: LiveLatencyCollector::default(),
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
        }
        .into_runtime();

        let (_authority_gateway, cancel_policy, client_order_ids) =
            runtime_order_gateway(&["BTC-USDT"], Vec::new());
        assert!(matches!(
            runtime.dispatch_action(LiveAction::Cancel(cancel_action(
                &cancel_policy,
                &client_order_ids,
                "BTC-USDT",
                "client-live",
                "injected pre-shutdown storage failure",
            ))),
            Err(LiveRuntimeError::Storage(StorageError::Closed))
        ));
        assert!(runtime.reconciliation.cancel_inflight.is_empty());

        let error = runtime.run().await.unwrap_err();
        drop(control_tx);
        drop(feed_tx);
        let _ = std::fs::remove_file(path);

        let LiveRuntimeError::ReportedFailure { source, report } = error else {
            panic!("runtime failure must retain its post-cleanup evidence report");
        };
        assert_eq!(report.stop_reason, LiveStopReason::RuntimeFailure);
        assert!(!report.clean_soak);
        assert_eq!(report.active_orders_after_shutdown, 0);
        let failure = report.failure.as_ref().expect("failure evidence");
        assert_eq!(failure.code, "gateway_task");
        assert!(failure.message.contains("injected runtime failure"));

        let error = *source;
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
            client.clone(),
            Some(client),
            safety_account_config(),
            command_rx,
            event_tx,
            Some(30),
            60_000,
            60_000,
            1_000,
            exchange_status_guard(false, 60_000),
            exchange_instrument_guard(60_000, Vec::new()),
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
            client.clone(),
            Some(client),
            safety_account_config(),
            command_rx,
            event_tx,
            Some(30),
            1,
            60_000,
            1_000,
            exchange_status_guard(false, 60_000),
            exchange_instrument_guard(60_000, Vec::new()),
        ));

        let event = tokio::time::timeout(Duration::from_millis(100), event_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            event,
            RuntimeEvent::Fatal(RuntimeTaskFailure::DeadmanHeartbeat(message))
                if message.contains("injected heartbeat failure")
        ));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn blocked_exchange_fee_check_does_not_delay_deadman_heartbeat() {
        let fee_started = Arc::new(Notify::new());
        let deadman_seen = Arc::new(Notify::new());
        let readiness: Arc<dyn ReadinessPort> = Arc::new(BlockingFeePort {
            fee_started: Arc::clone(&fee_started),
        });
        let safety: Arc<dyn SafetyPort> = Arc::new(NotifyingSafety {
            deadman_seen: Arc::clone(&deadman_seen),
        });
        let (command_tx, command_rx) = mpsc::channel(2);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let task = tokio::spawn(run_account_safety_task(
            "main".to_string(),
            readiness,
            Some(safety),
            safety_account_config(),
            command_rx,
            event_tx,
            Some(30),
            500,
            60_000,
            1_000,
            exchange_status_guard(false, 60_000),
            exchange_instrument_guard(400, vec![exchange_instrument_expectation(0.001, 0.001)]),
        ));

        tokio::time::timeout(Duration::from_secs(1), fee_started.notified())
            .await
            .expect("fee request did not start");
        tokio::time::timeout(Duration::from_secs(1), deadman_seen.notified())
            .await
            .expect("deadman heartbeat was blocked by fee request");
        assert!(event_rx.try_recv().is_err());

        command_tx.send(SafetyTaskCommand::Shutdown).await.unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn blocked_exchange_instrument_check_does_not_delay_deadman_heartbeat() {
        let instrument_started = Arc::new(Notify::new());
        let deadman_seen = Arc::new(Notify::new());
        let readiness: Arc<dyn ReadinessPort> = Arc::new(BlockingInstrumentPort {
            instrument_started: Arc::clone(&instrument_started),
        });
        let safety: Arc<dyn SafetyPort> = Arc::new(NotifyingSafety {
            deadman_seen: Arc::clone(&deadman_seen),
        });
        let (command_tx, command_rx) = mpsc::channel(2);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let task = tokio::spawn(run_account_safety_task(
            "main".to_string(),
            readiness,
            Some(safety),
            safety_account_config(),
            command_rx,
            event_tx,
            Some(30),
            500,
            60_000,
            1_000,
            exchange_status_guard(false, 60_000),
            exchange_instrument_guard(400, vec![exchange_instrument_expectation(0.001, 0.001)]),
        ));

        tokio::time::timeout(Duration::from_secs(1), instrument_started.notified())
            .await
            .expect("instrument request did not start");
        tokio::time::timeout(Duration::from_secs(1), deadman_seen.notified())
            .await
            .expect("deadman heartbeat was blocked by instrument request");
        assert!(event_rx.try_recv().is_err());

        command_tx.send(SafetyTaskCommand::Shutdown).await.unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn periodic_exchange_clock_skew_has_a_typed_failure() {
        let (client, requests) =
            safety_client(vec![Ok(r#"{"code":"0","msg":"","data":[{"ts":"0"}]}"#)]);
        let (_command_tx, command_rx) = mpsc::channel(2);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let task = tokio::spawn(run_account_safety_task(
            "main".to_string(),
            client,
            None,
            safety_account_config(),
            command_rx,
            event_tx,
            None,
            60_000,
            1,
            1,
            exchange_status_guard(false, 60_000),
            exchange_instrument_guard(60_000, Vec::new()),
        ));

        let event = tokio::time::timeout(Duration::from_millis(100), event_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            event,
            RuntimeEvent::Fatal(RuntimeTaskFailure::ExchangeClockSkew(message))
                if message.contains("maximum is 1ms")
        ));
        task.await.unwrap();
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn account_configuration_drift_is_fatal() {
        let (client, requests) = safety_client(vec![
            Ok(r#"{"code":"0","msg":"","data":[{"ts":"0"}]}"#),
            Ok(
                r#"{"code":"0","msg":"","data":[{"acctLv":"2","posMode":"net_mode","acctStpMode":"cancel_maker","uid":"7","mainUid":"6","label":"reap-demo","perm":"read_only,trade","ip":"203.0.113.5","enableSpotBorrow":true,"autoLoan":false,"spotBorrowAutoRepay":false}]}"#,
            ),
        ]);
        let (_command_tx, command_rx) = mpsc::channel(2);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let task = tokio::spawn(run_account_safety_task(
            "main".to_string(),
            client,
            None,
            safety_account_config(),
            command_rx,
            event_tx,
            None,
            60_000,
            1,
            u64::MAX,
            exchange_status_guard(false, 60_000),
            exchange_instrument_guard(60_000, Vec::new()),
        ));

        let event = tokio::time::timeout(Duration::from_millis(100), event_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            event,
            RuntimeEvent::Fatal(RuntimeTaskFailure::AccountConfigDrift(message))
                if message.contains("configuration or authenticated identity differs")
        ));
        task.await.unwrap();
        assert_eq!(requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn account_stp_mode_drift_is_fatal() {
        let (client, requests) = safety_client(vec![
            Ok(r#"{"code":"0","msg":"","data":[{"ts":"0"}]}"#),
            Ok(
                r#"{"code":"0","msg":"","data":[{"acctLv":"2","posMode":"net_mode","acctStpMode":"cancel_taker","uid":"7","mainUid":"6","label":"reap-demo","perm":"read_only,trade","ip":"203.0.113.5","enableSpotBorrow":false,"autoLoan":false,"spotBorrowAutoRepay":false}]}"#,
            ),
        ]);
        let (_command_tx, command_rx) = mpsc::channel(2);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let task = tokio::spawn(run_account_safety_task(
            "main".to_string(),
            client,
            None,
            safety_account_config(),
            command_rx,
            event_tx,
            None,
            60_000,
            1,
            u64::MAX,
            exchange_status_guard(false, 60_000),
            exchange_instrument_guard(60_000, Vec::new()),
        ));

        let event = tokio::time::timeout(Duration::from_millis(100), event_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            event,
            RuntimeEvent::Fatal(RuntimeTaskFailure::AccountConfigDrift(message))
                if message.contains("configuration or authenticated identity differs")
        ));
        task.await.unwrap();
        assert_eq!(requests.lock().unwrap().len(), 2);
    }

    #[test]
    fn exchange_status_guard_matches_planned_service_scope_and_environment() {
        let relevance = ChaosConnectivityPlan::resolve(&config(), LiveMode::Demo)
            .unwrap()
            .maintenance_relevance()
            .clone();
        let status = |service_type, environment, state, begin_time_ms| OkxSystemStatus {
            title: "maintenance".to_string(),
            description: String::new(),
            state,
            begin_time_ms,
            end_time_ms: begin_time_ms.saturating_add(60_000),
            pre_open_begin_time_ms: None,
            service_type,
            maintenance_type: reap_venue::okx::OkxSystemMaintenanceType::Scheduled,
            environment,
            system: "unified".to_string(),
        };
        let now_ms = 1_000_000;
        let lead_ms = 60_000;

        let trading = status(
            OkxSystemServiceType::Trading,
            OkxSystemEnvironment::Demo,
            OkxSystemStatusState::Scheduled,
            now_ms + lead_ms,
        );
        assert!(exchange_status_block_reason(&[trading], &relevance, now_ms, lead_ms).is_some());

        let too_early = status(
            OkxSystemServiceType::TradingAccounts,
            OkxSystemEnvironment::Demo,
            OkxSystemStatusState::Scheduled,
            now_ms + lead_ms + 1,
        );
        assert!(exchange_status_block_reason(&[too_early], &relevance, now_ms, lead_ms).is_none());

        let copy_trading = status(
            OkxSystemServiceType::CopyTrading,
            OkxSystemEnvironment::Demo,
            OkxSystemStatusState::Ongoing,
            1,
        );
        let production = status(
            OkxSystemServiceType::Trading,
            OkxSystemEnvironment::Production,
            OkxSystemStatusState::Ongoing,
            1,
        );
        let completed = status(
            OkxSystemServiceType::TradingProducts,
            OkxSystemEnvironment::Demo,
            OkxSystemStatusState::Completed,
            1,
        );
        assert!(
            exchange_status_block_reason(
                &[copy_trading, production, completed],
                &relevance,
                now_ms,
                lead_ms
            )
            .is_none()
        );
    }

    #[test]
    fn spread_only_ongoing_maintenance_is_irrelevant_to_the_planned_scope() {
        let relevance = ChaosConnectivityPlan::resolve(&config(), LiveMode::Demo)
            .unwrap()
            .maintenance_relevance()
            .clone();
        let spread = OkxSystemStatus {
            title: "spread maintenance".to_string(),
            description: String::new(),
            state: OkxSystemStatusState::Ongoing,
            begin_time_ms: 1,
            end_time_ms: 60_001,
            pre_open_begin_time_ms: None,
            service_type: OkxSystemServiceType::SpreadTrading,
            maintenance_type: reap_venue::okx::OkxSystemMaintenanceType::Scheduled,
            environment: OkxSystemEnvironment::Demo,
            system: "unified".to_string(),
        };

        assert!(exchange_status_block_reason(&[spread], &relevance, 1_000_000, 60_000).is_none());
    }

    #[test]
    fn ambiguous_ongoing_maintenance_blocks_the_planned_scope() {
        let relevance = ChaosConnectivityPlan::resolve(&config(), LiveMode::Demo)
            .unwrap()
            .maintenance_relevance()
            .clone();
        let ambiguous = OkxSystemStatus {
            title: "ambiguous maintenance".to_string(),
            description: String::new(),
            state: OkxSystemStatusState::Ongoing,
            begin_time_ms: 1,
            end_time_ms: 60_001,
            pre_open_begin_time_ms: None,
            service_type: OkxSystemServiceType::Other,
            maintenance_type: reap_venue::okx::OkxSystemMaintenanceType::Scheduled,
            environment: OkxSystemEnvironment::Demo,
            system: "unified".to_string(),
        };

        assert!(
            exchange_status_block_reason(&[ambiguous], &relevance, 1_000_000, 60_000).is_some()
        );
    }

    #[test]
    fn exchange_fee_guard_converts_signed_rates_and_allows_conservative_config() {
        let mut expectation = exchange_instrument_expectation(0.0002, 0.0005);
        let rate = OkxTradeFeeRate {
            instrument_type: OkxInstrumentType::Spot,
            group_id: "1".to_string(),
            level: "Lv1".to_string(),
            maker_rate: -0.0002,
            taker_rate: -0.0005,
            timestamp_ms: 1,
        };
        assert!(exchange_fee_drift_reason(&expectation, &rate).is_none());

        expectation.configured_maker_cost = 0.0003;
        expectation.configured_taker_cost = 0.0006;
        assert!(exchange_fee_drift_reason(&expectation, &rate).is_none());

        expectation.configured_maker_cost = 0.0001;
        let reason = exchange_fee_drift_reason(&expectation, &rate).unwrap();
        assert!(reason.contains("understate authenticated costs"));

        expectation.configured_maker_cost = -0.0002;
        let rebate = OkxTradeFeeRate {
            maker_rate: 0.0001,
            ..rate
        };
        assert!(exchange_fee_drift_reason(&expectation, &rebate).is_some());
        expectation.configured_maker_cost = 0.0;
        assert!(exchange_fee_drift_reason(&expectation, &rebate).is_none());
    }

    #[test]
    fn exchange_instrument_guard_detects_rule_drift_and_announced_changes() {
        let expectation = exchange_instrument_expectation(0.001, 0.001);
        let mut current = expectation.expected_instrument.clone();
        assert!(exchange_instrument_drift_reason(&expectation, &current, 1_000, 100).is_none());

        current.tick_size = 0.01;
        let reason = exchange_instrument_drift_reason(&expectation, &current, 1_000, 100).unwrap();
        assert!(reason.contains("tick size changed"));

        current = expectation.expected_instrument.clone();
        current.max_limit_size = 0.5;
        let reason = exchange_instrument_drift_reason(&expectation, &current, 1_000, 100).unwrap();
        assert!(reason.contains("maximum limit-order size changed"));

        current = expectation.expected_instrument.clone();
        current.upcoming_changes.push(OkxInstrumentChange {
            parameter: OkxInstrumentChangeParameter::MinimumSize,
            new_value: 0.01,
            effective_time_ms: 1_101,
        });
        assert!(exchange_instrument_drift_reason(&expectation, &current, 1_000, 100).is_none());
        current.upcoming_changes[0].effective_time_ms = 1_100;
        let reason = exchange_instrument_drift_reason(&expectation, &current, 1_000, 100).unwrap();
        assert!(reason.contains("announced minSz change"));

        current = expectation.expected_instrument.clone();
        current.state = "post_only".to_string();
        let reason = exchange_instrument_drift_reason(&expectation, &current, 1_000, 100).unwrap();
        assert!(reason.contains("state changed"));
    }

    #[test]
    fn initial_announced_instrument_change_is_typed() {
        let mut expectation = exchange_instrument_expectation(0.001, 0.001);
        expectation
            .expected_instrument
            .upcoming_changes
            .push(OkxInstrumentChange {
                parameter: OkxInstrumentChangeParameter::TickSize,
                new_value: 0.01,
                effective_time_ms: 1_100,
            });
        let mut guard = exchange_instrument_guard(400, vec![expectation]);
        guard.change_lead_ms = 100;

        let error = verify_initial_exchange_instruments("main", &guard, 1_000).unwrap_err();

        assert!(matches!(
            error,
            LiveRuntimeError::ExchangeInstrumentDrift(message)
                if message.contains("account main") && message.contains("tickSz")
        ));
    }

    #[test]
    fn exchange_fee_request_spacing_finishes_within_the_sweep_deadline() {
        assert_eq!(exchange_fee_request_interval_ms(1_001, 2), 500);
        assert_eq!(exchange_fee_request_interval_ms(800, 2), 400);
    }

    #[tokio::test]
    async fn initial_exchange_fee_understatement_is_typed() {
        let (client, requests) = safety_client(vec![Ok(
            r#"{"code":"0","msg":"","data":[{"feeGroup":[{"groupId":"1","maker":"-0.0008","taker":"-0.001"}],"instType":"SPOT","level":"Lv1","ts":"1763979985847"}]}"#,
        )]);
        let guard = exchange_instrument_guard(
            60_000,
            vec![exchange_instrument_expectation(0.0002, 0.0005)],
        );

        let error = verify_initial_exchange_fees("main", client.as_ref(), &guard)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            LiveRuntimeError::ExchangeFeeDrift(message)
                if message.contains("account main") && message.contains("BTC-USDT")
        ));
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn periodic_exchange_fee_understatement_is_fatal() {
        let (client, requests) = safety_client(vec![
            Ok(spot_instrument_response()),
            Ok(
                r#"{"code":"0","msg":"","data":[{"feeGroup":[{"groupId":"1","maker":"-0.0008","taker":"-0.001"}],"instType":"SPOT","level":"Lv1","ts":"1763979985847"}]}"#,
            ),
        ]);
        let (_command_tx, command_rx) = mpsc::channel(2);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let task = tokio::spawn(run_account_safety_task(
            "main".to_string(),
            client,
            None,
            safety_account_config(),
            command_rx,
            event_tx,
            None,
            60_000,
            60_000,
            1_000,
            exchange_status_guard(false, 60_000),
            exchange_instrument_guard(400, vec![exchange_instrument_expectation(0.0002, 0.0005)]),
        ));

        let event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            event,
            RuntimeEvent::Fatal(RuntimeTaskFailure::ExchangeFeeDrift(message))
                if message.contains("BTC-USDT") && message.contains("0.001")
        ));
        task.await.unwrap();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].path,
            "/api/v5/account/instruments?instType=SPOT&instId=BTC-USDT"
        );
        assert_eq!(
            requests[1].path,
            "/api/v5/account/trade-fee?instType=SPOT&instId=BTC-USDT"
        );
    }

    #[tokio::test]
    async fn periodic_exchange_fee_check_failure_is_typed() {
        let (client, _) = safety_client(vec![
            Ok(spot_instrument_response()),
            Err(RestError::Transport("injected fee failure".to_string())),
        ]);
        let (_command_tx, command_rx) = mpsc::channel(2);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let task = tokio::spawn(run_account_safety_task(
            "main".to_string(),
            client,
            None,
            safety_account_config(),
            command_rx,
            event_tx,
            None,
            60_000,
            60_000,
            1_000,
            exchange_status_guard(false, 60_000),
            exchange_instrument_guard(400, vec![exchange_instrument_expectation(0.001, 0.001)]),
        ));

        let event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            event,
            RuntimeEvent::Fatal(RuntimeTaskFailure::ExchangeFeeCheck(message))
                if message.contains("injected fee failure")
        ));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn periodic_exchange_instrument_maximum_drift_is_fatal() {
        let changed = r#"{"code":"0","msg":"","data":[{"instId":"BTC-USDT","instType":"SPOT","instFamily":"","groupId":"1","baseCcy":"BTC","quoteCcy":"USDT","settleCcy":"","ctType":"","ctVal":"","ctValCcy":"","tickSz":"0.1","lotSz":"0.001","minSz":"0.001","maxLmtSz":"99","maxMktSz":"1000000","maxLmtAmt":"1000000","maxMktAmt":"1000000","state":"live","upcChg":[]}]}"#;
        let (client, requests) = safety_client(vec![Ok(changed)]);
        let (_command_tx, command_rx) = mpsc::channel(2);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let task = tokio::spawn(run_account_safety_task(
            "main".to_string(),
            client,
            None,
            safety_account_config(),
            command_rx,
            event_tx,
            None,
            60_000,
            60_000,
            1_000,
            exchange_status_guard(false, 60_000),
            exchange_instrument_guard(400, vec![exchange_instrument_expectation(0.001, 0.001)]),
        ));

        let event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            event,
            RuntimeEvent::Fatal(RuntimeTaskFailure::ExchangeInstrumentDrift(message))
                if message.contains("maximum limit-order size changed")
        ));
        task.await.unwrap();
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn periodic_exchange_instrument_check_failure_is_typed() {
        let (client, _) = safety_client(vec![Err(RestError::Transport(
            "injected instrument failure".to_string(),
        ))]);
        let (_command_tx, command_rx) = mpsc::channel(2);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let task = tokio::spawn(run_account_safety_task(
            "main".to_string(),
            client,
            None,
            safety_account_config(),
            command_rx,
            event_tx,
            None,
            60_000,
            60_000,
            1_000,
            exchange_status_guard(false, 60_000),
            exchange_instrument_guard(400, vec![exchange_instrument_expectation(0.001, 0.001)]),
        ));

        let event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            event,
            RuntimeEvent::Fatal(RuntimeTaskFailure::ExchangeInstrumentCheck(message))
                if message.contains("injected instrument failure")
        ));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn periodic_relevant_exchange_status_is_fatal() {
        let (client, requests) = safety_client(vec![Ok(
            r#"{"code":"0","msg":"","data":[{"begin":"1","end":"60001","env":"2","maintType":"2","serviceType":"5","state":"ongoing","system":"unified","title":"Trading maintenance"}]}"#,
        )]);
        let (_command_tx, command_rx) = mpsc::channel(2);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let task = tokio::spawn(run_account_safety_task(
            "main".to_string(),
            client,
            None,
            safety_account_config(),
            command_rx,
            event_tx,
            None,
            60_000,
            60_000,
            1_000,
            exchange_status_guard(true, 1),
            exchange_instrument_guard(60_000, Vec::new()),
        ));

        let event = tokio::time::timeout(Duration::from_millis(100), event_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            event,
            RuntimeEvent::Fatal(RuntimeTaskFailure::ExchangeStatus(message))
                if message.contains("Trading maintenance")
        ));
        task.await.unwrap();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/api/v5/system/status");
    }

    #[tokio::test]
    async fn periodic_exchange_status_check_failure_is_typed() {
        let (client, _) = safety_client(vec![Err(RestError::Transport(
            "injected status failure".to_string(),
        ))]);
        let (_command_tx, command_rx) = mpsc::channel(2);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let task = tokio::spawn(run_account_safety_task(
            "main".to_string(),
            client,
            None,
            safety_account_config(),
            command_rx,
            event_tx,
            None,
            60_000,
            60_000,
            1_000,
            exchange_status_guard(true, 1),
            exchange_instrument_guard(60_000, Vec::new()),
        ));

        let event = tokio::time::timeout(Duration::from_millis(100), event_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            event,
            RuntimeEvent::Fatal(RuntimeTaskFailure::ExchangeStatusCheck(message))
                if message.contains("injected status failure")
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
        assert!(stale[0].reason.ends_with(": test"));
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
        assert!(stale[0].reason.ends_with(": test"));
    }

    #[test]
    fn public_plan_materializes_exact_replicas_and_explicit_trades() {
        let mut config = config();
        let connectivity_plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();
        let subscriptions = runtime_public_subscriptions(&connectivity_plan).unwrap();

        assert!(subscriptions.iter().all(|planned| {
            let expected = if planned.subscription.channel == Channel::Books {
                2
            } else {
                1
            };
            planned.subscription.connections == expected
                && (expected > 1) == planned.redundancy_consumer.is_some()
                && !planned.requirements.is_empty()
        }));
        let configured_symbols = config
            .strategy
            .instruments
            .iter()
            .map(|instrument| instrument.symbol.as_str())
            .collect::<BTreeSet<_>>();
        let trade_symbols = subscriptions
            .iter()
            .filter(|planned| planned.subscription.channel == Channel::Trades)
            .map(|planned| {
                assert_eq!(planned.subscription.connections, 1);
                planned.subscription.symbol.as_deref().unwrap()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(trade_symbols, configured_symbols);
        assert!(subscriptions.iter().any(|subscription| {
            subscription.subscription.channel == Channel::Custom("price-limit".to_string())
                && subscription.subscription.symbol.as_deref() == Some("BTC-USDT")
                && subscription.subscription.priority == FeedPriority::Critical
        }));
        assert!(subscriptions.iter().any(|subscription| {
            subscription.subscription.channel == Channel::Custom("mark-price".to_string())
                && subscription.subscription.symbol.as_deref() == Some("BTC-PERP")
        }));
        assert!(!subscriptions.iter().any(|subscription| {
            subscription.subscription.channel == Channel::Custom("funding-rate".to_string())
        }));

        config.strategy.instruments[1].kind = InstrumentKindConfig::LinearSwap;
        let connectivity_plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();
        assert!(
            runtime_public_subscriptions(&connectivity_plan)
                .unwrap()
                .iter()
                .any(|subscription| {
                    subscription.subscription.channel == Channel::Custom("funding-rate".to_string())
                        && subscription.subscription.symbol.as_deref() == Some("BTC-PERP")
                        && subscription.subscription.priority == FeedPriority::Critical
                        && subscription.subscription.connections == 1
                })
        );
    }

    #[test]
    fn public_plan_packs_stablecoin_requirement_without_extra_replica() {
        let mut config = config();
        config.risk.stablecoin_guards = vec![StablecoinGuardConfig {
            symbol: "USDT-USD".to_string(),
            max_downside_deviation: 0.01,
        }];
        config.strategy.instruments[0].index_symbol = Some("USDT-USD".to_string());

        let connectivity_plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();
        let subscriptions = runtime_public_subscriptions(&connectivity_plan).unwrap();
        let stablecoin = subscriptions
            .iter()
            .filter(|subscription| {
                subscription.subscription.channel == Channel::Custom("index-tickers".to_string())
                    && subscription.subscription.symbol.as_deref() == Some("USDT-USD")
            })
            .collect::<Vec<_>>();

        assert_eq!(stablecoin.len(), 1);
        assert_eq!(stablecoin[0].subscription.priority, FeedPriority::Critical);
        assert_eq!(stablecoin[0].subscription.connections, 1);
        assert!(stablecoin[0].requirements.len() >= 2);
    }

    #[test]
    fn private_sessions_are_packed_per_account_and_unused_account_waits_only_for_positions() {
        let mut config = config();
        let mut unused = config.accounts[0].clone();
        unused.id = "unused".to_string();
        unused.api_key_env = "UNUSED_KEY".to_string();
        unused.secret_key_env = "UNUSED_SECRET".to_string();
        unused.passphrase_env = "UNUSED_PASS".to_string();
        unused.id_prefix = "unused".to_string();
        unused.node_id = 2;
        unused.trade_modes.clear();
        config.accounts.push(unused);
        config.venue.enable_vip_fills_channel = true;
        let connectivity_plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();

        let mut plans = private_socket_plans_by_account(&connectivity_plan).unwrap();
        assert_eq!(plans["main"].len(), 1);
        assert!(
            plans["main"][0]
                .subscriptions
                .iter()
                .any(|subscription| subscription.channel == Channel::Orders)
        );
        let unused = plans.remove("unused").unwrap();
        assert_eq!(unused.len(), 1);
        assert_eq!(unused[0].subscriptions.len(), 1);
        assert_eq!(unused[0].subscriptions[0].channel, Channel::Positions);

        let conn_id = unused[0].conn_id.0.clone();
        let adapter: Arc<dyn VenueAdapter> = Arc::new(OkxAdapter::default());
        let mut source = FeedSourceState::private(adapter, "unused".to_string(), &unused);
        assert!(
            source
                .on_status(status(&conn_id, ConnectionStatusKind::Ready))
                .is_empty()
        );
        let ready = source.on_private_data(Channel::Positions, 7);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].kind, SystemEventKind::PrivateStreamRecovered);
    }

    #[test]
    fn private_session_socket_overcount_is_rejected_by_runtime_composition() {
        let error = validate_private_state_socket_count("main", 2).unwrap_err();
        assert!(
            matches!(
                &error,
                LiveRuntimeError::Subscription(message)
                    if message.contains("must use exactly one socket, configured 2")
            ),
            "unexpected private socket overcount error: {error}"
        );
    }

    #[test]
    fn packed_private_session_is_permutation_safe_and_disconnect_resets_its_data_round() {
        let mut config = config();
        config.venue.enable_vip_fills_channel = true;
        let connectivity_plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();
        let packed = private_socket_plans_by_account(&connectivity_plan)
            .unwrap()
            .remove("main")
            .unwrap();
        assert_eq!(packed.len(), 1);
        assert_eq!(
            packed[0]
                .subscriptions
                .iter()
                .map(|subscription| subscription.channel.clone())
                .collect::<HashSet<_>>(),
            HashSet::from([
                Channel::Orders,
                Channel::Fills,
                Channel::Account,
                Channel::Positions,
            ])
        );
        let conn_id = packed[0].conn_id.0.clone();
        let channels = [
            Channel::Orders,
            Channel::Fills,
            Channel::Account,
            Channel::Positions,
        ];
        let mut permutations = 0;
        for first in 0..channels.len() {
            for second in 0..channels.len() {
                if second == first {
                    continue;
                }
                for third in 0..channels.len() {
                    if third == first || third == second {
                        continue;
                    }
                    for fourth in 0..channels.len() {
                        if fourth == first || fourth == second || fourth == third {
                            continue;
                        }
                        permutations += 1;
                        let adapter: Arc<dyn VenueAdapter> = Arc::new(OkxAdapter::default());
                        let mut source =
                            FeedSourceState::private(adapter, "main".to_string(), &packed);
                        assert!(
                            source
                                .on_status(status(&conn_id, ConnectionStatusKind::Ready,))
                                .is_empty()
                        );
                        let mut saw_account = false;
                        let mut saw_positions = false;
                        let mut recovered = false;
                        for (offset, channel) in [first, second, third, fourth]
                            .into_iter()
                            .map(|index| channels[index].clone())
                            .enumerate()
                        {
                            saw_account |= channel == Channel::Account;
                            saw_positions |= channel == Channel::Positions;
                            let events = source.on_private_data(channel, offset as u64 + 2);
                            if saw_account && saw_positions && !recovered {
                                assert_eq!(events.len(), 1);
                                assert_eq!(events[0].kind, SystemEventKind::PrivateStreamRecovered);
                                recovered = true;
                            } else {
                                assert!(events.is_empty());
                            }
                        }
                        assert!(recovered);
                    }
                }
            }
        }
        assert_eq!(permutations, 24);

        let adapter: Arc<dyn VenueAdapter> = Arc::new(OkxAdapter::default());
        let mut source = FeedSourceState::private(adapter, "main".to_string(), &packed);
        assert!(
            source
                .on_status(status(&conn_id, ConnectionStatusKind::Ready))
                .is_empty()
        );
        assert!(source.on_private_data(Channel::Account, 10).is_empty());
        assert_eq!(
            source.on_private_data(Channel::Positions, 11)[0].kind,
            SystemEventKind::PrivateStreamRecovered
        );
        let stale = source.on_status(status(&conn_id, ConnectionStatusKind::Disconnected));
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].kind, SystemEventKind::PrivateStreamStale);
        assert!(
            source
                .on_status(status(&conn_id, ConnectionStatusKind::Ready))
                .is_empty()
        );
        assert!(source.on_private_data(Channel::Orders, 12).is_empty());
        assert!(source.on_private_data(Channel::Fills, 13).is_empty());
        assert!(source.on_private_data(Channel::Positions, 14).is_empty());
        let recovered = source.on_private_data(Channel::Account, 15);
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].kind, SystemEventKind::PrivateStreamRecovered);
    }

    #[test]
    fn command_sessions_exist_only_for_nonempty_planned_lanes() {
        let mut config = config();
        let mut unused = config.accounts[0].clone();
        unused.id = "unused".to_string();
        unused.api_key_env = "UNUSED_KEY".to_string();
        unused.secret_key_env = "UNUSED_SECRET".to_string();
        unused.passphrase_env = "UNUSED_PASS".to_string();
        unused.id_prefix = "unused".to_string();
        unused.node_id = 2;
        unused.trade_modes.clear();
        config.accounts.push(unused);

        let demo_plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Demo).unwrap();
        let counts = planned_order_session_counts(&demo_plan).unwrap();
        assert_eq!(counts, BTreeMap::from([("main".to_string(), 1)]));
        let mut startup =
            StartupGate::new_with_order_transports(&config, counts.keys().cloned().collect())
                .unwrap();
        assert_eq!(
            startup.snapshot().missing_order_transports,
            vec!["main".to_string()]
        );
        startup
            .mark_order_transport("unused", false, "unplanned account has no lane")
            .unwrap();
        assert_eq!(
            startup.snapshot().missing_order_transports,
            vec!["main".to_string()]
        );
        assert!(
            !startup
                .snapshot()
                .faults
                .contains_key("order_transport:unused")
        );
        let observe_plan = ChaosConnectivityPlan::resolve(&config, LiveMode::Observe).unwrap();
        assert!(
            planned_order_session_counts(&observe_plan)
                .unwrap()
                .is_empty()
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

        let (source, _) = unwrap_startup_failure(error);
        assert!(matches!(
            source,
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

        let (source, _) = unwrap_startup_failure(error);
        assert!(matches!(
            source,
            LiveRuntimeError::Host(HostHealthError::Unhealthy { ref code, .. })
                if code == "disk_low"
        ));
        let lease = acquire_storage_lease(&path).unwrap();
        let lock_path = lease.lock_path().to_path_buf();
        drop(lease);
        let _ = std::fs::remove_file(lock_path);
    }

    #[tokio::test]
    async fn connection_pacer_preflight_fails_before_credentials_or_network_and_releases_lease() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("live.jsonl");
        let mut config = config();
        config.storage.path = path.clone();
        config.runtime.connection_attempt_pacer_path =
            Some(directory.path().join("missing").join("connect.pacer"));

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

        let (source, _) = unwrap_startup_failure(error);
        assert!(matches!(source, LiveRuntimeError::ConnectionPacer(_)));
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
        production.venue.public_ws_url = "wss://ws.okx.com:8443/ws/v5/public".to_string();
        production.venue.private_ws_url = "wss://ws.okx.com:8443/ws/v5/private".to_string();
        production.risk.stablecoin_guards = vec![StablecoinGuardConfig {
            symbol: "USDT-USD".to_string(),
            max_downside_deviation: 0.01,
        }];
        production.accounts[0].api_key_policy.require_ip_binding = true;
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
