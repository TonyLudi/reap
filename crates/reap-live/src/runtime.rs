use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(test)]
use reap_core::Venue;
use reap_core::{Channel, FillKey, NormalizedEvent, PINNED_JAVA_REVISION, TimerEvent};
#[cfg(test)]
use reap_feed::ConnectionStatusKind;
use reap_feed::{
    ConnectionAttemptPacer, ConnectionStatus, FeedOutput, FeedProcessor, ReconnectPolicy,
    partition_subscriptions, try_spawn_supervised_feed,
};
use reap_okx_live_adapter::OrderCommandWebsocketConfig;
#[cfg(test)]
use reap_okx_live_adapter::OrderCommandWebsocketLifecycle;
#[cfg(test)]
use reap_order::OkxOrderGateway;
use reap_order::reconcile_full_state;
use reap_storage::{
    BootstrapRecord, SessionStartRecord, StorageConfig, StorageError, StorageRecord,
    acquire_storage_lease, recover_leased_jsonl, start_jsonl_storage_with_lease,
};
use reap_telemetry::{AlertError, AlertRuntime, AlertStats, start_webhook_alerts};
use reap_venue::VenueAdapter;
use reap_venue::okx::{OkxAdapter, okx_capability_registration};
#[cfg(test)]
use reap_venue::okx::{
    OkxInstrument, OkxSystemEnvironment, OkxSystemServiceType, OkxSystemStatus,
    OkxSystemStatusState, OkxTradeFeeRate,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;

use crate::convergence::{FillConvergenceGuard, OrderStateConvergenceGuard};
use crate::coordinator::SubmitAction;
use crate::forbidden_orders::{
    ForbiddenOrderObserverPort, ForbiddenSentinelPolicy, run_forbidden_order_sentinel,
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
    LiveLatencyEvidence, LiveMode, OperatorEnvelope, OperatorError, OperatorService,
    ReadinessSnapshot, ReconciliationResult, StartupGate, TradingEnvironment,
    alert_webhook_from_env, check_host_health, load_live_config_with_evidence,
    operator_secret_from_env, start_host_guard, start_operator_service,
};

mod bootstrap;
mod commit;
mod composition;
mod connectivity;
mod dispatch;
mod operator_flow;
mod planning;
mod readiness_safety;
mod reconciliation;
mod recovery;
mod shutdown;

use bootstrap::{AccountSeed, bootstrap_accounts};
use commit::receive_alert_failure;
#[cfg(test)]
use composition::truncate_utf8;
use composition::{
    CompositionState, ReadinessTracker, RunLoopFailure, RunLoopOutcome, RuntimeEvidence,
    combine_lifecycle_errors, live_failure_evidence, live_startup_failure_evidence,
    qualifies_as_clean_soak,
};
use connectivity::{ConnectivityState, FeedSourceState, spawn_feed_forwarders};
#[cfg(test)]
use dispatch::RuntimeTaskFailure;
use dispatch::{DispatchState, RuntimeEvent, run_order_task};
use operator_flow::receive_operator;
#[cfg(test)]
use planning::validate_private_state_socket_count;
use planning::{
    planned_order_session_counts, private_socket_plans_by_account, runtime_public_subscriptions,
    validate_public_socket_plans, validate_runtime_connectivity_plan,
};
#[cfg(test)]
use readiness_safety::{
    ExchangeInstrumentExpectation, ExchangeInstrumentGuard, ReadinessPort, SafetyPort,
    exchange_fee_drift_reason, exchange_fee_request_interval_ms, exchange_instrument_drift_reason,
    exchange_status_block_reason, verify_initial_exchange_fees,
    verify_initial_exchange_instruments,
};
use readiness_safety::{
    ExchangeStatusGuard, ReadinessSafetyState, receive_host_failure, run_account_safety_task,
};
use reconciliation::{ReconciliationState, run_reconcile_task};
use recovery::{
    private_update_from_remote, proven_active_recovered_orders, recovered_safety_latch_count,
    restore_active_order_bindings, restore_safety_latches, validate_recovered_safety_latches,
};
#[cfg(test)]
use shutdown::is_zero_order_reconciliation;
use shutdown::{ShutdownState, StartupTaskGroup, shutdown_signal};

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
            event @ RuntimeEvent::Connection { .. } => {
                connectivity::handle_runtime_event(self, event).await?;
            }
            event @ (RuntimeEvent::OrderTransport(_)
            | RuntimeEvent::SubmitComplete { .. }
            | RuntimeEvent::SubmitFailed { .. }
            | RuntimeEvent::CancelComplete { .. }
            | RuntimeEvent::CancelFailed { .. }) => {
                dispatch::handle_runtime_event(self, event).await?;
            }
            event @ (RuntimeEvent::RemoteState { .. } | RuntimeEvent::ReconcileFailed { .. }) => {
                reconciliation::handle_runtime_event(self, event).await?;
            }
            RuntimeEvent::Fatal(failure) => return Err(failure.into()),
        }
        Ok(())
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

#[cfg(test)]
#[path = "../tests/runtime_unit/mod.rs"]
mod tests;
