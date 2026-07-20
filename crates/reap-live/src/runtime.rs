use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(test)]
use reap_core::Venue;
use reap_core::{Channel, NormalizedEvent, PINNED_JAVA_REVISION, TimerEvent};
#[cfg(test)]
use reap_feed::ConnectionStatusKind;
#[cfg(test)]
use reap_feed::FeedProcessor;
use reap_feed::{ConnectionStatus, FeedOutput};
#[cfg(test)]
use reap_okx_live_adapter::OrderCommandWebsocketLifecycle;
#[cfg(test)]
use reap_order::OkxOrderGateway;
#[cfg(test)]
use reap_storage::StorageConfig;
use reap_storage::{StorageError, StorageRecord};
use reap_telemetry::AlertError;
#[cfg(test)]
use reap_telemetry::{AlertRuntime, AlertStats, start_webhook_alerts};
#[cfg(test)]
use reap_venue::VenueAdapter;
#[cfg(test)]
use reap_venue::okx::{
    OkxAdapter, OkxInstrument, OkxSystemEnvironment, OkxSystemServiceType, OkxSystemStatus,
    OkxSystemStatusState, OkxTradeFeeRate,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
#[cfg(test)]
use tokio::sync::mpsc;

use crate::convergence::{FillConvergenceGuard, OrderStateConvergenceGuard};
use crate::coordinator::SubmitAction;
use crate::provenance::{
    current_executable_sha256 as hash_current_executable,
    host_identity_sha256 as hash_host_identity, okx_account_identity_sha256,
};
use crate::safety_contracts::LiveFaultFailureCode;
use crate::{
    AccountBootstrapSnapshot, ChaosConnectivityPlan, ChaosConnectivityPlanError, CoordinatorError,
    HostHealthError, HostHealthSnapshot, LiveConfig, LiveConfigError, LiveConfigFileEvidence,
    LiveCoordinator, LiveLatencyCollector, LiveLatencyEvidence, LiveMode, OperatorEnvelope,
    OperatorError, OperatorService, ReadinessSnapshot, StartupGate, TradingEnvironment,
    load_live_config_with_evidence,
};
#[cfg(test)]
use crate::{HostGuardRuntime, ReconciliationResult, start_operator_service};

mod bootstrap;
mod commit;
mod composition;
mod connectivity;
mod dispatch;
mod lifecycle;
mod operator_flow;
mod planning;
mod readiness_safety;
mod reconciliation;
mod recovery;
mod shutdown;
mod startup;

use commit::receive_alert_failure;
#[cfg(test)]
use composition::RuntimeEvidence;
#[cfg(test)]
use composition::truncate_utf8;
use composition::{
    CompositionState, ReadinessTracker, RunLoopFailure, RunLoopOutcome,
    live_startup_failure_evidence,
};
use connectivity::ConnectivityState;
#[cfg(test)]
use connectivity::FeedSourceState;
#[cfg(test)]
use dispatch::RuntimeTaskFailure;
#[cfg(test)]
use dispatch::run_order_task;
use dispatch::{DispatchState, RuntimeEvent};
use operator_flow::receive_operator;
#[cfg(test)]
use planning::{
    planned_order_session_counts, private_socket_plans_by_account, runtime_public_subscriptions,
    validate_private_state_socket_count,
};
#[cfg(test)]
use readiness_safety::{
    ExchangeInstrumentExpectation, ExchangeInstrumentGuard, ReadinessPort, SafetyPort,
    exchange_fee_drift_reason, exchange_fee_request_interval_ms, exchange_instrument_drift_reason,
    exchange_status_block_reason, verify_initial_exchange_fees,
    verify_initial_exchange_instruments,
};
#[cfg(test)]
use readiness_safety::{ExchangeStatusGuard, run_account_safety_task};
use readiness_safety::{ReadinessSafetyState, receive_host_failure};
use reconciliation::ReconciliationState;
#[cfg(test)]
use reconciliation::run_reconcile_task;
#[cfg(test)]
use recovery::{
    proven_active_recovered_orders, recovered_safety_latch_count, restore_active_order_bindings,
    restore_safety_latches, validate_recovered_safety_latches,
};
#[cfg(test)]
use shutdown::StartupTaskGroup;
#[cfg(test)]
use shutdown::is_zero_order_reconciliation;
use shutdown::{ShutdownState, shutdown_signal};
use startup::{
    AuthenticatedStartup, CoordinatorStartup, RuntimeResources, StartupPlan, StartupRecovery,
    finish_startup,
};

pub const LIVE_RUN_REPORT_SCHEMA_VERSION: u32 = 8;
pub const MAX_LIVE_FAILURE_CODE_BYTES: usize = 64;
pub const MAX_LIVE_FAILURE_MESSAGE_BYTES: usize = 4_096;

mod scheduling;
use scheduling::{monotonic_now_ns, wait_until_monotonic_ns};

struct SchedulingState {
    origin: Instant,
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
    scheduling: SchedulingState,
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
        let plan = StartupPlan::resolve(config, attempt, connectivity_plan, mode, run_duration)?;
        let recovered = StartupRecovery::open(plan)?;
        let authenticated = AuthenticatedStartup::bootstrap(recovered).await?;
        let restored = CoordinatorStartup::restore(authenticated)?;
        let resources = RuntimeResources::start(restored).await?;
        let (runtime, finalization) = resources.into_runtime();
        finish_startup(runtime, finalization).await
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
            let trade_reprice_wait = wait_until_monotonic_ns(
                self.scheduling.origin(),
                self.coordinator.next_trade_reprice_due_ns(),
            );
            tokio::pin!(trade_reprice_wait);
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
                    _ = &mut trade_reprice_wait => {
                        let scheduling_origin = self.scheduling.origin();
                        let output = self
                            .coordinator
                            .service_one_due_trade_reprice_with_clocks(
                                move || {
                                    (monotonic_now_ns(scheduling_origin), unix_time_ms())
                                },
                                unix_time_ms,
                            );
                        self.commit_output(output).await?;
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
                received_at,
            } => {
                let arrival_ns = self.scheduling.captured_receipt_ns(received_at);
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
                        let scheduling_origin = self.scheduling.origin();
                        let output = self.coordinator.process_feed_received_at(
                            output,
                            envelope.recv_ts_ns / 1_000_000,
                            arrival_ns,
                            move || (monotonic_now_ns(scheduling_origin), unix_time_ms()),
                            unix_time_ms,
                        )?;
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
