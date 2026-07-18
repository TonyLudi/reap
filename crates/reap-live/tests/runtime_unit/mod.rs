use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use reap_core::{
    AccountUpdate, Balance, ConnId, FeedPriority, Level, MarketEvent, OrderBook, OrderEvent,
    OrderUpdate, Side, Subscription, TimeInForce,
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
    let runtime_source = include_str!("../../src/runtime.rs");
    let (production_runtime, _) = runtime_source
        .split_once("#[cfg(test)]\n#[path = \"../tests/runtime_unit/mod.rs\"]\nmod tests")
        .expect("runtime test module marker");
    let responsibility_modules = [
        include_str!("../../src/runtime/composition.rs"),
        include_str!("../../src/runtime/connectivity.rs"),
        include_str!("../../src/runtime/dispatch.rs"),
        include_str!("../../src/runtime/planning.rs"),
        include_str!("../../src/runtime/readiness_safety.rs"),
        include_str!("../../src/runtime/reconciliation.rs"),
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

struct TaskDropSignal(Option<oneshot::Sender<()>>);

impl Drop for TaskDropSignal {
    fn drop(&mut self) {
        if let Some(signal) = self.0.take() {
            let _ = signal.send(());
        }
    }
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
    fn next(&self, path: impl Into<String>, body: impl Into<String>) -> Result<String, RestError> {
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

    let lease = acquire_storage_lease(&path).expect("aborted writer must release journal lease");
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
    let operator_service = start_operator_service(&operator_config, SECRET.to_vec(), operator_tx)
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

    assert!(exchange_status_block_reason(&[ambiguous], &relevance, 1_000_000, 60_000).is_some());
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
                    let mut source = FeedSourceState::private(adapter, "main".to_string(), &packed);
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
        StartupGate::new_with_order_transports(&config, counts.keys().cloned().collect()).unwrap();
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
