use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use reap_core::FillKey;
use reap_feed::{
    ConnectionAttemptPacer, FeedProcessor, ReconnectPolicy, SocketPlan, partition_subscriptions,
    try_spawn_supervised_feed,
};
use reap_okx_live_adapter::OrderCommandWebsocketConfig;
use reap_order::{RegularApprovalScope, reconcile_full_state};
use reap_storage::{
    BootstrapRecord, RecoveredStorage, SessionStartRecord, StorageConfig, StorageLease,
    StorageRecord, acquire_storage_lease, recover_leased_jsonl, start_jsonl_storage_with_lease,
};
use reap_telemetry::{
    AlertDeliveryFailure, AlertRuntime, AlertSink, AlertStats, start_webhook_alerts,
};
use reap_venue::VenueAdapter;
use reap_venue::okx::{OkxAdapter, okx_capability_registration};
use tokio::sync::mpsc;

use crate::forbidden_orders::{
    ForbiddenOrderObserverPort, ForbiddenSentinelPolicy, run_forbidden_order_sentinel,
};
use crate::{
    AccountBootstrapSnapshot, ChaosConnectivityPlan, CoordinatorError, CoordinatorOutput,
    HostGuardRuntime, HostHealthSnapshot, LiveConfig, LiveConfigFileEvidence, LiveCoordinator,
    LiveLatencyCollector, LiveMode, MaintenanceRelevancePlan, OperatorConfig, ReconciliationResult,
    VerifiedBootstrap, alert_webhook_from_env, check_host_health, operator_secret_from_env,
    start_host_guard, start_operator_service,
};

use super::SchedulingState;
use super::bootstrap::{AccountSeed, bootstrap_accounts};
use super::composition::{CompositionState, RuntimeEvidence};
use super::connectivity::{ConnectivityState, FeedSourceState, spawn_feed_forwarders};
use super::dispatch::{DispatchState, RuntimeEvent, run_order_task};
use super::planning::{
    planned_order_session_counts, private_socket_plans_by_account, runtime_public_subscriptions,
    validate_public_socket_plans, validate_runtime_connectivity_plan,
};
use super::readiness_safety::{ExchangeStatusGuard, ReadinessSafetyState, run_account_safety_task};
use super::reconciliation::{ReconciliationState, run_reconcile_task};
use super::recovery::{
    RecoveredActiveOrders, private_update_from_remote, recovered_safety_latch_count,
    restore_active_order_bindings, restore_safety_latches, validate_recovered_safety_latches,
};
use super::shutdown::{ShutdownState, StartupTaskGroup};
use super::{
    FillConvergenceGuard, LiveRunAttemptEvidence, LiveRuntime, LiveRuntimeError,
    OrderStateConvergenceGuard, account_identity_sha256s, unix_time_ms, unix_time_ns,
};

pub(super) struct StartupPlan {
    config: LiveConfig,
    attempt: LiveRunAttemptEvidence,
    mode: LiveMode,
    run_duration: Option<Duration>,
    maintenance_relevance: MaintenanceRelevancePlan,
    forbidden_policy: ForbiddenSentinelPolicy,
    public_plans: Vec<SocketPlan>,
    private_plans_by_account: BTreeMap<String, Vec<SocketPlan>>,
    order_session_counts: BTreeMap<String, usize>,
    planned_order_accounts: HashSet<String>,
}

impl StartupPlan {
    pub(super) fn resolve(
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
        let private_plans_by_account = private_socket_plans_by_account(&connectivity_plan)?;
        let order_session_counts = planned_order_session_counts(&connectivity_plan)?;
        let planned_order_accounts = order_session_counts.keys().cloned().collect::<HashSet<_>>();
        Ok(Self {
            config,
            attempt,
            mode,
            run_duration,
            maintenance_relevance,
            forbidden_policy,
            public_plans,
            private_plans_by_account,
            order_session_counts,
            planned_order_accounts,
        })
    }
}

struct StartupFoundation {
    config: LiveConfig,
    mode: LiveMode,
    run_duration: Option<Duration>,
    maintenance_relevance: MaintenanceRelevancePlan,
    forbidden_policy: ForbiddenSentinelPolicy,
    public_plans: Vec<SocketPlan>,
    private_plans_by_account: BTreeMap<String, Vec<SocketPlan>>,
    order_session_counts: BTreeMap<String, usize>,
    session_started_at_ms: u64,
    config_source: Option<LiveConfigFileEvidence>,
    config_fingerprint: String,
    evidence_config_fingerprint: String,
    executable_sha256: String,
    host_identity_sha256: Option<String>,
    storage_lease: StorageLease,
    journal_path: PathBuf,
    host_preflight: Option<HostHealthSnapshot>,
    connection_attempt_pacer: ConnectionAttemptPacer,
    alert_runtime: Option<AlertRuntime>,
    alert_sink: Option<AlertSink>,
    alert_failures: Option<mpsc::Receiver<AlertDeliveryFailure>>,
    operator_config: OperatorConfig,
    operator_secret: Option<Vec<u8>>,
    fill_convergence: FillConvergenceGuard,
    order_convergence: OrderStateConvergenceGuard,
    recovered: RecoveredStorage,
    restored_safety_latches: u64,
}

pub(super) struct StartupRecovery {
    foundation: StartupFoundation,
    recovered_orders: RecoveredActiveOrders,
    restored_by_account: HashMap<String, Vec<reap_core::OrderUpdate>>,
    planned_order_accounts: HashSet<String>,
}

impl StartupRecovery {
    pub(super) fn open(plan: StartupPlan) -> Result<Self, LiveRuntimeError> {
        let StartupPlan {
            config,
            attempt,
            mode,
            run_duration,
            maintenance_relevance,
            forbidden_policy,
            public_plans,
            private_plans_by_account,
            order_session_counts,
            planned_order_accounts,
        } = plan;
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
        let recovered_orders = RecoveredActiveOrders::take(&config, &mut recovered);
        let restored_by_account = recovered_orders.restored_by_account(&config)?;
        Ok(Self {
            foundation: StartupFoundation {
                config,
                mode,
                run_duration,
                maintenance_relevance,
                forbidden_policy,
                public_plans,
                private_plans_by_account,
                order_session_counts,
                session_started_at_ms,
                config_source,
                config_fingerprint,
                evidence_config_fingerprint,
                executable_sha256,
                host_identity_sha256,
                storage_lease,
                journal_path,
                host_preflight,
                connection_attempt_pacer,
                alert_runtime,
                alert_sink,
                alert_failures,
                operator_config,
                operator_secret,
                fill_convergence,
                order_convergence,
                recovered,
                restored_safety_latches,
            },
            recovered_orders,
            restored_by_account,
            planned_order_accounts,
        })
    }
}

pub(super) struct AuthenticatedStartup {
    foundation: StartupFoundation,
    recovered_orders: RecoveredActiveOrders,
    verified: VerifiedBootstrap,
    seeds: Vec<AccountSeed>,
    snapshots: HashMap<String, AccountBootstrapSnapshot>,
    approval_scopes: HashMap<String, RegularApprovalScope>,
}

impl AuthenticatedStartup {
    pub(super) async fn bootstrap(recovery: StartupRecovery) -> Result<Self, LiveRuntimeError> {
        let StartupRecovery {
            foundation,
            recovered_orders,
            restored_by_account,
            planned_order_accounts,
        } = recovery;
        let (verified, mut seeds, snapshots) = bootstrap_accounts(
            &foundation.config,
            &restored_by_account,
            foundation.mode,
            &planned_order_accounts,
            &foundation.maintenance_relevance,
        )
        .await?;
        let approval_scopes = take_regular_approval_scopes(&mut seeds)?;
        Ok(Self {
            foundation,
            recovered_orders,
            verified,
            seeds,
            snapshots,
            approval_scopes,
        })
    }
}

fn take_regular_approval_scopes(
    seeds: &mut [AccountSeed],
) -> Result<HashMap<String, RegularApprovalScope>, LiveRuntimeError> {
    let mut approval_scopes = HashMap::new();
    for seed in seeds {
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
    Ok(approval_scopes)
}

pub(super) struct CoordinatorStartup {
    foundation: StartupFoundation,
    owner: LiveCoordinator,
    initial_outputs: Vec<CoordinatorOutput>,
    seeds: Vec<AccountSeed>,
    snapshots: HashMap<String, AccountBootstrapSnapshot>,
    account_identity_sha256s: BTreeMap<String, String>,
    session_id: String,
}

impl CoordinatorStartup {
    pub(super) fn restore(startup: AuthenticatedStartup) -> Result<Self, LiveRuntimeError> {
        let AuthenticatedStartup {
            mut foundation,
            recovered_orders,
            mut verified,
            seeds,
            snapshots,
            approval_scopes,
        } = startup;
        let account_identity_sha256s = account_identity_sha256s(&foundation.config, &snapshots)?;
        let session_id = format!("{:x}", unix_time_ns());
        let mut startup_records = Vec::new();
        for account in &foundation.config.accounts {
            let exchange_baseline = verified
                .baseline_fill_ids
                .get(&account.id)
                .cloned()
                .unwrap_or_default();
            let mut fill_ids = foundation
                .recovered
                .baseline_fill_ids
                .get(&account.id)
                .cloned()
                .unwrap_or_else(|| exchange_baseline.clone());
            for fill in &foundation.recovered.fills {
                let fill_account_id = fill.account_id.as_deref().or_else(|| {
                    foundation
                        .config
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
            if !foundation
                .recovered
                .baseline_fill_ids
                .contains_key(&account.id)
            {
                let mut baseline_fill_ids = exchange_baseline.into_iter().collect::<Vec<_>>();
                baseline_fill_ids.sort();
                startup_records.push(StorageRecord::Bootstrap(BootstrapRecord {
                    ts_ms: unix_time_ms(),
                    account_id: account.id.clone(),
                    strategy_name: foundation.config.strategy.strategy_name.clone(),
                    config_fingerprint: foundation.config_fingerprint.clone(),
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
                ts_ms: foundation.session_started_at_ms,
                session_id: session_id.clone(),
                account_id: account.id.clone(),
                strategy_name: foundation.config.strategy.strategy_name.clone(),
                config_fingerprint: foundation.config_fingerprint.clone(),
                account_identity_sha256,
            }));
        }
        let mut owner = LiveCoordinator::new_with_order_transports(
            foundation.config.clone(),
            verified,
            approval_scopes,
            session_id.clone(),
        )?;
        // Apply recovered halt state before replaying anything that can produce an intent.
        // Reapplying it after reconciliation below generates cancels for restored live orders.
        let _ = restore_safety_latches(&mut owner, &foundation.recovered)?;
        let mut initial_outputs = vec![CoordinatorOutput {
            actions: Vec::new(),
            records: startup_records,
        }];
        recovered_orders.restore_into(&mut owner, &mut initial_outputs)?;
        restore_active_order_bindings(&mut owner, &mut foundation.recovered)?;
        for account in &foundation.config.accounts {
            let snapshot = snapshots.get(&account.id).ok_or_else(|| {
                LiveRuntimeError::BootstrapVerification(format!(
                    "missing reconciliation snapshot for {}",
                    account.id
                ))
            })?;
            for fill in &snapshot.recent_fills {
                let should_apply = owner.private_state(&account.id).is_some_and(|state| {
                    let order_id =
                        state.resolve_order_id(&fill.client_order_id, &fill.exchange_order_id);
                    state.order_reducer().contains_order(&order_id)
                        && !state.has_seen_fill(&fill.symbol, &fill.fill_id)
                });
                if should_apply {
                    initial_outputs.push(owner.process_feed(
                        reap_feed::FeedOutput::PrivateFill {
                            account_id: Some(account.id.clone()),
                            fill: fill.clone(),
                        },
                    )?);
                }
            }
            for remote in &snapshot.open_orders {
                let known = owner.private_state(&account.id).is_some_and(|state| {
                    let order_id =
                        state.resolve_order_id(&remote.client_order_id, &remote.exchange_order_id);
                    state.order_reducer().contains_order(&order_id)
                });
                if known {
                    initial_outputs.push(owner.process_feed(
                        reap_feed::FeedOutput::PrivateOrder {
                            account_id: Some(account.id.clone()),
                            update: private_update_from_remote(remote.clone()),
                        },
                    )?);
                }
            }
            let account_snapshot = snapshot.scoped_account_update(&account.id);
            initial_outputs.push(
                owner
                    .apply_authoritative_account_snapshot(&account.id, account_snapshot.clone())?,
            );
            let state = owner
                .private_state(&account.id)
                .ok_or_else(|| CoordinatorError::UnknownAccount(account.id.clone()))?;
            let report = reconcile_full_state(
                state,
                &snapshot.open_orders,
                &snapshot.recent_fills,
                &account_snapshot,
            );
            initial_outputs.push(owner.on_reconciliation(ReconciliationResult {
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
        initial_outputs.extend(restore_safety_latches(&mut owner, &foundation.recovered)?);
        Ok(Self {
            foundation,
            owner,
            initial_outputs,
            seeds,
            snapshots,
            account_identity_sha256s,
            session_id,
        })
    }
}

pub(super) struct RuntimeResources {
    runtime: LiveRuntime,
    feed_tasks: StartupTaskGroup,
    order_tasks: StartupTaskGroup,
    reconcile_tasks: StartupTaskGroup,
    order_ws_status_tasks: StartupTaskGroup,
    safety_tasks: StartupTaskGroup,
    forbidden_tasks: StartupTaskGroup,
    finalization: StartupFinalization,
}

pub(super) struct StartupFinalization {
    initial_outputs: Vec<CoordinatorOutput>,
    restored_safety_latches: u64,
    fill_convergence: FillConvergenceGuard,
    operator_config: OperatorConfig,
    operator_secret: Option<Vec<u8>>,
}

impl RuntimeResources {
    pub(super) async fn start(startup: CoordinatorStartup) -> Result<Self, LiveRuntimeError> {
        let CoordinatorStartup {
            foundation,
            mut owner,
            initial_outputs,
            seeds,
            snapshots,
            account_identity_sha256s,
            session_id,
        } = startup;
        let StartupFoundation {
            config,
            mode,
            run_duration,
            maintenance_relevance,
            forbidden_policy,
            public_plans,
            mut private_plans_by_account,
            mut order_session_counts,
            session_started_at_ms,
            config_source,
            config_fingerprint,
            evidence_config_fingerprint,
            executable_sha256,
            host_identity_sha256,
            storage_lease,
            journal_path,
            host_preflight,
            connection_attempt_pacer,
            alert_runtime,
            alert_sink,
            alert_failures,
            operator_config,
            operator_secret,
            fill_convergence,
            order_convergence,
            recovered: _,
            restored_safety_latches,
        } = foundation;
        let scheduling = SchedulingState::new();
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
        owner.mark_storage_ready(true, "storage file opened");

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

        let runtime = LiveRuntime {
            coordinator: owner,
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
                order_ws_status_tasks: Vec::new(),
                feeds,
                feed_tasks: Vec::new(),
                sources,
                public_feed_index,
                max_feed_age_ms: config.risk.max_feed_age_ms,
            },
            dispatch: DispatchState {
                control_rx,
                order_senders,
                order_tasks: Vec::new(),
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
            scheduling,
            readiness_safety: ReadinessSafetyState {
                forbidden_rx,
                safety_senders,
                safety_tasks: Vec::new(),
                forbidden_tasks: Vec::new(),
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
                tasks: Vec::new(),
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
        Ok(Self {
            runtime,
            feed_tasks,
            order_tasks,
            reconcile_tasks,
            order_ws_status_tasks,
            safety_tasks,
            forbidden_tasks,
            finalization: StartupFinalization {
                initial_outputs,
                restored_safety_latches,
                fill_convergence,
                operator_config,
                operator_secret,
            },
        })
    }

    pub(super) fn into_runtime(mut self) -> (LiveRuntime, StartupFinalization) {
        self.runtime.connectivity.order_ws_status_tasks = self.order_ws_status_tasks.take();
        self.runtime.connectivity.feed_tasks = self.feed_tasks.take();
        self.runtime.dispatch.order_tasks = self.order_tasks.take();
        self.runtime.readiness_safety.safety_tasks = self.safety_tasks.take();
        self.runtime.readiness_safety.forbidden_tasks = self.forbidden_tasks.take();
        self.runtime.reconciliation.tasks = self.reconcile_tasks.take();
        (self.runtime, self.finalization)
    }
}

pub(super) async fn finish_startup(
    mut runtime: LiveRuntime,
    finalization: StartupFinalization,
) -> Result<LiveRuntime, LiveRuntimeError> {
    let StartupFinalization {
        initial_outputs,
        restored_safety_latches,
        fill_convergence,
        operator_config,
        operator_secret,
    } = finalization;
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
        let (operator_tx, operator_rx) = mpsc::channel(operator_config.command_channel_capacity);
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
