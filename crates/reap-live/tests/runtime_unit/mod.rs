use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use reap_core::{
    AccountUpdate, Balance, ConnId, FeedPriority, Level, MarketEvent, OrderBook, OrderEvent,
    OrderStatus, OrderUpdate, Side, Subscription, SystemEvent, SystemEventKind, TimeInForce,
};
use reap_feed::{SocketPlan, SupervisedFeed};
use reap_order::{
    CancelOrderTransportError, ClientOrderIdGenerator, OrderTransportError, OwnedRegularOrders,
    PacingPolicy, PreparedRegularCancel, PreparedRegularSubmit, PrivateStateReducer,
    ReconcileReport, RegularExecution, RegularExecutionPolicy, RegularExecutionProfile,
    RegularReconciliation,
};
use reap_risk::{InstrumentOrderLimits, InstrumentRiskModel, RiskLimits, StablecoinGuardConfig};
use reap_storage::{
    OrderAckStatus, OrderOperation, OrderRequestRecord, RecoveredStorage, SafetyLatchRecord,
    SafetyLatchScope, SafetyLatchSource, StorageRuntime, StorageSink, acquire_storage_lease,
    recover_jsonl, recover_leased_jsonl, start_jsonl_storage,
};
use reap_strategy::{
    ChaosConfig, ChaosExecutionIntent, ChaosStrategy, InstrumentConfig, InstrumentKindConfig,
    ReferenceDataKind, RiskGroupConfig,
};
use reap_telemetry::{AlertDeliveryFailure, AlertEvent, AlertSeverity, AlertSink};
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
use reap_venue::{PrivateOrderState, RemoteOrder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Notify, Semaphore, oneshot};
use tokio::task::JoinHandle;

use crate::coordinator::{CancelAction, LiveAction};
use crate::forbidden_orders::{ForbiddenOrderEvent, ForbiddenOrderState};
use crate::{
    LiveAccountConfig, LiveStorageConfig, OkxTradeModeConfig, OkxVenueConfig, OperatorCommand,
    RuntimeConfig, VerifiedBootstrap, VerifiedInstrument,
};

use super::commit::alert_for_storage_record;
use super::dispatch::{OrderTaskCommand, ReconcileTaskCommand, SafetyTaskCommand};
use super::*;

mod dispatch;
mod lifecycle;
mod planning;
mod recovery;
mod safety;
mod startup;

#[test]
fn production_runtime_keeps_single_owner_responsibility_state() {
    let runtime_source = include_str!("../../src/runtime.rs");
    let (production_runtime, _) = runtime_source
        .split_once("#[cfg(test)]\n#[path = \"../tests/runtime_unit/mod.rs\"]\nmod tests")
        .expect("runtime test module marker");
    let responsibility_modules = [
        include_str!("../../src/runtime/bootstrap.rs"),
        include_str!("../../src/runtime/commit.rs"),
        include_str!("../../src/runtime/composition.rs"),
        include_str!("../../src/runtime/connectivity.rs"),
        include_str!("../../src/runtime/dispatch.rs"),
        include_str!("../../src/runtime/operator_flow.rs"),
        include_str!("../../src/runtime/planning.rs"),
        include_str!("../../src/runtime/readiness_safety.rs"),
        include_str!("../../src/runtime/reconciliation.rs"),
        include_str!("../../src/runtime/recovery.rs"),
        include_str!("../../src/runtime/shutdown.rs"),
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

fn recover_storage_records(records: impl IntoIterator<Item = StorageRecord>) -> RecoveredStorage {
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
        Ok(parse_okx_account_positions_response_json(response.body.as_bytes())?.account_update())
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

fn config() -> LiveConfig {
    let mut strategy: ChaosConfig =
        toml::from_str(include_str!("../../../../examples/iarb2-basic.toml")).unwrap();
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
                        max_limit_notional_usd: instrument.kind.is_spot().then_some(1_000_000.0),
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
