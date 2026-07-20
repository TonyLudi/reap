use std::collections::BTreeSet;

use crate::ChaosExecutionPurpose;
use reap_core::PINNED_JAVA_REVISION;
use serde::Deserialize;
use serde_json::{Value, json};

use super::*;

const ARRIVAL_NS: u64 = 1_000_000_000;
const CALLBACK_DELAY_NS: u64 = 100_000;
const WORKER_INTERVAL_NS: u64 = 5_000_000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct JavaTradeFixture {
    schema_version: u32,
    java_revision: String,
    java_call_path: Vec<String>,
    callback_delay_ns: u64,
    worker_interval_ns: u64,
    worker_interval_sources: Vec<String>,
    pending_hedge_expiry_ms: TimeMs,
    base_depth: OrderBook,
    atomic_cases: Vec<AtomicCase>,
    sequence_cases: Vec<SequenceCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AtomicCase {
    id: String,
    ignore_best_level: bool,
    trade: FixtureTrade,
    expected: AtomicExpected,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureTrade {
    source_ts_ms: TimeMs,
    price: f64,
    qty: f64,
    taker_side: Side,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AtomicExpected {
    book_side: Side,
    crossed_raw_best: bool,
    first_valid_price: f64,
    due_offset_ns: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SequenceCase {
    id: String,
    ignore_best_level: bool,
    steps: Vec<SequenceStep>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum SequenceStep {
    Depth {
        source_ts_ms: TimeMs,
        arrival_ns: u64,
        local_now_ms: TimeMs,
        strategy_is_live: bool,
        expect: StepExpectation,
    },
    Trade {
        source_ts_ms: TimeMs,
        price: f64,
        qty: f64,
        taker_side: Side,
        arrival_ns: u64,
        local_now_ms: TimeMs,
        strategy_is_live: bool,
        expect: StepExpectation,
    },
    PendingHedge {
        hedge_side: Side,
        price: f64,
        qty: f64,
        local_now_ms: TimeMs,
        expect: StepExpectation,
    },
    AdvanceTime {
        local_now_ms: TimeMs,
        expect: StepExpectation,
    },
    InstallBaseBookWithoutDepth {
        source_ts_ms: TimeMs,
        expect: StepExpectation,
    },
    Service {
        now_ns: u64,
        local_now_ms: TimeMs,
        strategy_is_live: bool,
        expect: StepExpectation,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StepExpectation {
    effective_buy_px: Option<f64>,
    effective_sell_px: Option<f64>,
    #[serde(default)]
    buy_levels_empty: bool,
    #[serde(default)]
    sell_levels_empty: bool,
    next_due: Option<DeadlineExpectation>,
    intent_count: Option<usize>,
    #[serde(default)]
    pending_debug_contains: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum DeadlineExpectation {
    None,
    At { ns: u64 },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NormalizedGolden {
    schema_version: u32,
    java_revision: String,
    scenario_fixture: String,
    lifecycle: NormalizedLifecycle,
    event_outputs: Vec<ExpectedOutput>,
    final_state: ExpectedFinalState,
    burst_conflation: BurstConflationGolden,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NormalizedLifecycle {
    strategy_is_live: bool,
    event_arrival_ns: Vec<u64>,
    event_observed_now_ms: Vec<TimeMs>,
    service_horizon_ns: u64,
    service_points: Vec<ExpectedServicePoint>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedServicePoint {
    now_ns: u64,
    observed_now_ms: TimeMs,
    expected_next_due_ns: Option<u64>,
    output: ExpectedOutput,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedOutput {
    typed: Vec<ExpectedTypedIntent>,
    legacy: Vec<OrderIntent>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedTypedIntent {
    purpose: String,
    legacy: OrderIntent,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedFinalState {
    effective_sell_price: f64,
    next_due_ns: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BurstConflationGolden {
    first_arrival_ns: u64,
    first_due_ns: u64,
    second_arrival_ns: u64,
    second_due_ns: u64,
    third_arrival_ns: u64,
    third_due_ns: u64,
    trailing_due_ns: u64,
    latest_effective_sell_price: f64,
    first_due_output: ExpectedOutput,
    trailing_due_output: ExpectedOutput,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RngInterleavingFixture {
    schema_version: u32,
    java_revision: String,
    java_call_path: Vec<String>,
    random_seed: u64,
    configuration: RngFixtureConfiguration,
    random_draws: Vec<RngFixtureDraw>,
    initial_orders: Vec<RngFixtureOrder>,
    timeline: RngFixtureTimeline,
    service_points: Vec<RngFixtureServicePoint>,
    next_random_bits_hex: String,
    scope_note: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RngFixtureConfiguration {
    multi_level_symbol: String,
    num_quote_levels: usize,
    min_level_spread: f64,
    max_level_spread: f64,
    force_quote_update_ms: TimeMs,
    multi_level_debounce_size_usd: f64,
    trade_affected_symbol: String,
    trade_affected_num_quote_levels: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RngFixtureDraw {
    ordinal: usize,
    value: f64,
    bits_hex: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RngFixtureOrder {
    order_id: String,
    symbol: String,
    side: Side,
    reason: String,
    price: f64,
    qty: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RngFixtureTimeline {
    reference_depth_processing_ns: u64,
    reference_depth_finish_ms: TimeMs,
    hedge_depth_processing_ns: u64,
    hedge_depth_finish_ms: TimeMs,
    pending_depth_receipt_ns: u64,
    pending_depth_processing_ns: u64,
    pending_depth_processing_ms: TimeMs,
    pending_depth_due_ns: u64,
    first_trade_arrival_ns: u64,
    first_callback_due_ns: u64,
    second_trade_arrival_ns: u64,
    second_callback_due_ns: u64,
    trailing_service_ms: TimeMs,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RngFixtureServicePoint {
    now_ns: u64,
    expected_next_due_ns: Option<u64>,
    output: Vec<OrderIntent>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerClockFixture {
    schema_version: u32,
    java_revision: String,
    java_call_path: Vec<String>,
    derivation: String,
    immediate_depth: ImmediateDepthClockCase,
    immediate_callback: ImmediateCallbackClockCase,
    direct_timer: DirectTimerClockCase,
    scope_note: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImmediateDepthClockCase {
    id: String,
    decision_ns: u64,
    decision_ms: TimeMs,
    work_start_ms: TimeMs,
    finish_ms: TimeMs,
    next_decision_ns: u64,
    next_decision_ms: TimeMs,
    expected_next_due_ns: u64,
    expected_worker_clock_reads: u8,
    expected_last_work_ms: TimeMs,
    expected_last_finish_ms: TimeMs,
    expected_next_disposition: String,
    expected_output: ExpectedOutput,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImmediateCallbackClockCase {
    id: String,
    seed_decision_ns: u64,
    seed_ms: TimeMs,
    trade_receipt_ns: u64,
    trade_processing_ns: u64,
    trade_processing_ms: TimeMs,
    callback_decision_ms: TimeMs,
    work_start_ms: TimeMs,
    finish_ms: TimeMs,
    next_decision_ns: u64,
    next_decision_ms: TimeMs,
    expected_next_due_ns: u64,
    expected_worker_clock_reads: u8,
    expected_last_work_ms: TimeMs,
    expected_last_finish_ms: TimeMs,
    expected_next_disposition: String,
    expected_output: ExpectedOutput,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectTimerClockCase {
    id: String,
    seed_decision_ns: u64,
    seed_ms: TimeMs,
    throttled_decision_ns: u64,
    throttled_decision_ms: TimeMs,
    timer_due_ns: u64,
    scheduled_time_ms: TimeMs,
    work_start_ms: TimeMs,
    finish_ms: TimeMs,
    post_timer_decision_ns: u64,
    post_timer_decision_ms: TimeMs,
    expected_next_due_ns: u64,
    expected_worker_clock_reads: u8,
    expected_last_work_ms: TimeMs,
    expected_last_finish_ms: TimeMs,
    expected_next_disposition: String,
    expected_output: ExpectedOutput,
}

fn java_fixture() -> JavaTradeFixture {
    serde_json::from_str(include_str!(
        "../../../../fixtures/java/chaos_trade_implied_depth_v2.json"
    ))
    .unwrap()
}

fn rng_interleaving_fixture() -> RngInterleavingFixture {
    serde_json::from_str(include_str!(
        "../../../../fixtures/java/chaos_trade_rng_interleaving_v1.json"
    ))
    .unwrap()
}

fn worker_clock_fixture() -> WorkerClockFixture {
    serde_json::from_str(include_str!(
        "../../../../fixtures/java/chaos_pricing_worker_clock_v1.json"
    ))
    .unwrap()
}

fn normalized_fixture() -> (Vec<NormalizedEvent>, NormalizedGolden) {
    let raw = include_str!("../../../../fixtures/normalized/chaos_trade_implied_depth.jsonl");
    let events = raw
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .map(|line| serde_json::from_str::<NormalizedEvent>(line).unwrap())
        .collect::<Vec<_>>();
    let golden = serde_json::from_str(include_str!(
        "../../../../fixtures/normalized/chaos_trade_implied_depth_intents_v2.json"
    ))
    .unwrap();
    (events, golden)
}

fn implied_depth_strategy(ignore_best_level: bool) -> ChaosStrategy {
    let mut cfg = config();
    cfg.ignore_best_level = ignore_best_level;
    ChaosStrategy::new(cfg).unwrap()
}

fn base_book(symbol: &str, ts_ms: TimeMs) -> OrderBook {
    OrderBook {
        symbol: symbol.to_string(),
        ts_ms,
        bids: vec![
            Level::new(99.0, 10.0),
            Level::new(98.0, 10.0),
            Level::new(97.0, 10.0),
        ],
        asks: vec![
            Level::new(101.0, 10.0),
            Level::new(102.0, 10.0),
            Level::new(103.0, 10.0),
        ],
    }
}

fn depth(symbol: &str, ts_ms: TimeMs) -> MarketEvent {
    MarketEvent::Depth(base_book(symbol, ts_ms))
}

fn trade(symbol: &str, ts_ms: TimeMs, price: f64, qty: f64, taker_side: Side) -> MarketEvent {
    MarketEvent::Trade {
        ts_ms,
        symbol: symbol.to_string(),
        price,
        qty,
        taker_side,
    }
}

fn owned_trade(
    strategy: &mut ChaosStrategy,
    source_ts_ms: TimeMs,
    price: f64,
    qty: f64,
    taker_side: Side,
    delivery: (u64, TimeMs, bool),
) -> Vec<ChaosExecutionIntent> {
    let (arrival_ns, observed_now_ms, strategy_is_live) = delivery;
    strategy.on_owned_execution_event_at(
        StrategyEvent::Market(trade("BTC-PERP", source_ts_ms, price, qty, taker_side)),
        arrival_ns,
        arrival_ns,
        observed_now_ms,
        strategy_is_live,
    )
}

fn implied_state(ignore_best_level: bool) -> InstrumentState {
    let mut state = InstrumentState::new(InstrumentConfig {
        symbol: "BTC-PERP".to_string(),
        tick_size: 1.0,
        lot_size: 1.0,
        min_trade_size: 1.0,
        ..InstrumentConfig::default()
    });
    state.ignore_best_level = ignore_best_level;
    state.book = Some(base_book("BTC-PERP", 1));
    state
}

fn apply_state_trade(state: &mut InstrumentState, price: f64, qty: f64, taker_side: Side) -> bool {
    let book = state.book.clone();
    state
        .implied_depth
        .on_public_trade(book.as_ref(), price, qty, taker_side)
}

fn first_px(state: &InstrumentState, side: Side) -> f64 {
    state.effective_levels(side)[0].px
}

fn legacy_projection(intents: &[ChaosExecutionIntent]) -> Value {
    json!(
        intents
            .iter()
            .map(ChaosExecutionIntent::to_order_intent)
            .collect::<Vec<_>>()
    )
}

fn typed_projection(intents: &[ChaosExecutionIntent]) -> Value {
    json!(
        intents
            .iter()
            .map(|intent| {
                json!({
                    "purpose": intent.purpose().as_str(),
                    "legacy": intent.to_order_intent(),
                })
            })
            .collect::<Vec<_>>()
    )
}

fn expected_typed_projection(output: &ExpectedOutput) -> Value {
    json!(
        output
            .typed
            .iter()
            .map(|intent| {
                json!({
                    "purpose": intent.purpose.as_str(),
                    "legacy": &intent.legacy,
                })
            })
            .collect::<Vec<_>>()
    )
}

fn expected_legacy_projection(output: &ExpectedOutput) -> Value {
    json!(&output.legacy)
}

fn quote_reprice_strategy() -> ChaosStrategy {
    let mut cfg = config();
    for instrument in &mut cfg.instruments {
        instrument.debounce_width = 0.0;
        instrument.debounce_size_usd = 0.0;
        instrument.debounce_ms = 0;
    }
    ChaosStrategy::new(cfg).unwrap()
}

fn rng_interleaving_strategy(fixture: &RngInterleavingFixture) -> ChaosStrategy {
    let mut cfg = config();
    for instrument in &mut cfg.instruments {
        instrument.debounce_width = 0.0;
        instrument.debounce_size_usd = 0.0;
        instrument.debounce_ms = 0;
        if instrument.symbol == fixture.configuration.multi_level_symbol {
            instrument.num_quote_levels = fixture.configuration.num_quote_levels;
            instrument.min_level_spread = fixture.configuration.min_level_spread;
            instrument.max_level_spread = fixture.configuration.max_level_spread;
            instrument.force_quote_update_ms = fixture.configuration.force_quote_update_ms;
            instrument.debounce_size_usd = fixture.configuration.multi_level_debounce_size_usd;
        }
        if instrument.symbol == fixture.configuration.trade_affected_symbol {
            instrument.num_quote_levels = fixture.configuration.trade_affected_num_quote_levels;
        }
    }
    ChaosStrategy::new(cfg).unwrap()
}

fn rng_reference_depth(ts_ms: TimeMs) -> StrategyEvent {
    StrategyEvent::Market(MarketEvent::Depth(OrderBook::one_level(
        "BTC-USDT",
        ts_ms,
        Level::new(50_000.0, 2.0),
        Level::new(50_001.0, 2.0),
    )))
}

fn rng_hedge_depth(ts_ms: TimeMs) -> StrategyEvent {
    StrategyEvent::Market(MarketEvent::Depth(OrderBook {
        symbol: "BTC-PERP".to_string(),
        ts_ms,
        bids: vec![
            Level::new(50_003.0, 10_000.0),
            Level::new(50_002.0, 10_000.0),
            Level::new(50_001.0, 10_000.0),
        ],
        asks: vec![
            Level::new(50_004.0, 10_000.0),
            Level::new(50_005.0, 10_000.0),
            Level::new(50_006.0, 10_000.0),
        ],
    }))
}

fn assert_and_install_rng_fixture_orders(
    strategy: &mut ChaosStrategy,
    intents: &[ChaosExecutionIntent],
    expected: &[RngFixtureOrder],
) {
    assert_eq!(intents.len(), expected.len());
    for (intent, expected) in intents.iter().zip(expected) {
        let OrderIntent::NewOrder(order) = intent.to_order_intent() else {
            panic!("initial RNG fixture output must contain only quotes");
        };
        assert_eq!(intent.purpose(), ChaosExecutionPurpose::Quote);
        assert_eq!(order.symbol, expected.symbol);
        assert_eq!(order.side, expected.side);
        assert_eq!(order.reason, expected.reason);
        if !expected.reason.starts_with("quote:") {
            assert_eq!(
                order.price.to_bits(),
                expected.price.to_bits(),
                "{} top price: actual {}, expected {}",
                expected.order_id,
                order.price,
                expected.price
            );
        }
        assert_eq!(
            order.qty.to_bits(),
            expected.qty.to_bits(),
            "{} quantity: actual {}, expected {}",
            expected.order_id,
            order.qty,
            expected.qty
        );

        let pending_output = strategy.on_execution_event(&StrategyEvent::Order(OrderUpdate {
            ts_ms: 3_905,
            order_id: expected.order_id.clone(),
            symbol: order.symbol,
            side: order.side,
            event: OrderEvent::PendingNew,
            status: OrderStatus::PendingNew,
            price: order.price,
            time_in_force: Some(order.time_in_force),
            qty: order.qty,
            open_qty: order.qty,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            last_fill_fee: None,
            reason: order.reason,
        }));
        assert!(pending_output.is_empty());
    }
}

fn drain_private_depth_work_before(
    strategy: &mut ChaosStrategy,
    cutoff_ns: u64,
    strategy_is_live: bool,
) {
    while let Some(due_ns) = strategy.next_trade_reprice_due_ns() {
        if due_ns >= cutoff_ns {
            break;
        }
        let output =
            strategy.service_one_due_trade_reprice(due_ns, due_ns / 1_000_000, strategy_is_live);
        assert!(
            output.is_empty(),
            "depth-only shared-worker service before {cutoff_ns}ns must not add output"
        );
    }
}

fn deliver_rng_fixture_event(
    strategy: &mut ChaosStrategy,
    event: StrategyEvent,
    arrival_ns: u64,
    observed_now_ms: TimeMs,
    borrowed: bool,
) -> Vec<ChaosExecutionIntent> {
    if borrowed {
        strategy.on_execution_event_at(&event, arrival_ns, arrival_ns, observed_now_ms, true)
    } else {
        strategy.on_owned_execution_event_at(event, arrival_ns, arrival_ns, observed_now_ms, true)
    }
}

fn run_rng_interleaving_fixture(
    fixture: &RngInterleavingFixture,
    borrowed: bool,
) -> (ChaosStrategy, Vec<Vec<OrderIntent>>) {
    let timeline = &fixture.timeline;
    let mut strategy = rng_interleaving_strategy(fixture);
    let reference_output = deliver_rng_fixture_event(
        &mut strategy,
        rng_reference_depth(1),
        timeline.reference_depth_processing_ns,
        timeline.reference_depth_finish_ms,
        borrowed,
    );
    assert!(reference_output.is_empty());

    let initial = deliver_rng_fixture_event(
        &mut strategy,
        rng_hedge_depth(1),
        timeline.hedge_depth_processing_ns,
        timeline.hedge_depth_finish_ms,
        borrowed,
    );
    assert_and_install_rng_fixture_orders(&mut strategy, &initial, &fixture.initial_orders);

    let pending_depth = deliver_rng_fixture_event(
        &mut strategy,
        rng_hedge_depth(2),
        timeline.pending_depth_processing_ns,
        timeline.pending_depth_processing_ms,
        borrowed,
    );
    assert!(pending_depth.is_empty());
    assert_eq!(
        strategy.next_trade_reprice_due_ns(),
        Some(timeline.pending_depth_due_ns)
    );

    for (source_ts_ms, arrival_ns, expected_callback_due_ns, qty) in [
        (
            3,
            timeline.first_trade_arrival_ns,
            timeline.first_callback_due_ns,
            5_000.0,
        ),
        (
            4,
            timeline.second_trade_arrival_ns,
            timeline.second_callback_due_ns,
            4_999.0,
        ),
    ] {
        let output = deliver_rng_fixture_event(
            &mut strategy,
            StrategyEvent::Market(trade("BTC-PERP", source_ts_ms, 50_005.0, qty, Side::Buy)),
            arrival_ns,
            timeline.pending_depth_processing_ms,
            borrowed,
        );
        assert!(output.is_empty());
        assert!(
            strategy.next_trade_reprice_due_ns().unwrap() <= expected_callback_due_ns,
            "the retained callback must be inserted without displacing an earlier callback"
        );
    }

    let mut service_outputs = Vec::new();
    for point in &fixture.service_points {
        let observed_now_ms = if point.now_ns == timeline.pending_depth_due_ns {
            timeline.trailing_service_ms
        } else {
            point.now_ns / 1_000_000
        };
        let output = strategy.service_one_due_trade_reprice(point.now_ns, observed_now_ms, true);
        let legacy = output
            .iter()
            .map(ChaosExecutionIntent::to_order_intent)
            .collect::<Vec<_>>();
        assert_eq!(
            serde_json::to_value(&legacy).unwrap(),
            serde_json::to_value(&point.output).unwrap(),
            "RNG fixture output at {}ns",
            point.now_ns
        );
        assert_eq!(
            strategy.next_trade_reprice_due_ns(),
            point.expected_next_due_ns
        );
        service_outputs.push(legacy);
    }
    (strategy, service_outputs)
}

fn execute_java_sequence(case: &SequenceCase) {
    let mut strategy = implied_depth_strategy(case.ignore_best_level);
    for (step_index, step) in case.steps.iter().enumerate() {
        let (intents, expect) = match step {
            SequenceStep::Depth {
                source_ts_ms,
                arrival_ns,
                local_now_ms,
                strategy_is_live,
                expect,
            } => (
                strategy.on_owned_execution_event_at(
                    StrategyEvent::Market(depth("BTC-PERP", *source_ts_ms)),
                    *arrival_ns,
                    *arrival_ns,
                    *local_now_ms,
                    *strategy_is_live,
                ),
                expect,
            ),
            SequenceStep::Trade {
                source_ts_ms,
                price,
                qty,
                taker_side,
                arrival_ns,
                local_now_ms,
                strategy_is_live,
                expect,
            } => (
                owned_trade(
                    &mut strategy,
                    *source_ts_ms,
                    *price,
                    *qty,
                    *taker_side,
                    (*arrival_ns, *local_now_ms, *strategy_is_live),
                ),
                expect,
            ),
            SequenceStep::PendingHedge {
                hedge_side,
                price,
                qty,
                local_now_ms,
                expect,
            } => {
                strategy
                    .entities
                    .get_mut("BTC-PERP")
                    .unwrap()
                    .implied_depth
                    .update_our_hedge(*hedge_side, *price, *qty, *local_now_ms);
                (Vec::new(), expect)
            }
            SequenceStep::AdvanceTime {
                local_now_ms,
                expect,
            } => {
                strategy.advance_time(*local_now_ms);
                (Vec::new(), expect)
            }
            SequenceStep::InstallBaseBookWithoutDepth {
                source_ts_ms,
                expect,
            } => {
                strategy.entities.get_mut("BTC-PERP").unwrap().book =
                    Some(base_book("BTC-PERP", *source_ts_ms));
                (Vec::new(), expect)
            }
            SequenceStep::Service {
                now_ns,
                local_now_ms,
                strategy_is_live,
                expect,
            } => (
                strategy.service_one_due_trade_reprice(*now_ns, *local_now_ms, *strategy_is_live),
                expect,
            ),
        };

        let context = format!("{} step {step_index}", case.id);
        if let Some(intent_count) = expect.intent_count {
            assert_eq!(intents.len(), intent_count, "{context}");
        }
        let entity = strategy.entity("BTC-PERP").unwrap();
        let buy_px = entity
            .effective_levels(Side::Buy)
            .first()
            .map(|level| level.px);
        let sell_px = entity
            .effective_levels(Side::Sell)
            .first()
            .map(|level| level.px);
        if let Some(expected) = expect.effective_buy_px {
            assert_eq!(buy_px, Some(expected), "{context}");
        }
        if let Some(expected) = expect.effective_sell_px {
            assert_eq!(sell_px, Some(expected), "{context}");
        }
        if expect.buy_levels_empty {
            assert_eq!(buy_px, None, "{context}");
        }
        if expect.sell_levels_empty {
            assert_eq!(sell_px, None, "{context}");
        }
        if let Some(next_due) = &expect.next_due {
            let expected = match next_due {
                DeadlineExpectation::None => None,
                DeadlineExpectation::At { ns } => Some(*ns),
            };
            assert_eq!(strategy.next_trade_reprice_due_ns(), expected, "{context}");
        }
        let pending_debug = format!("{:?}", entity.implied_depth);
        for fragment in &expect.pending_debug_contains {
            assert!(
                pending_debug.contains(fragment),
                "{context}: `{pending_debug}` does not contain `{fragment}`"
            );
        }
    }
}

#[test]
fn pinned_java_truth_fixture_drives_raw_and_implied_depth_boundaries() {
    let fixture = java_fixture();
    assert_eq!(fixture.schema_version, 2);
    assert_eq!(fixture.java_revision, PINNED_JAVA_REVISION);
    assert_eq!(fixture.callback_delay_ns, CALLBACK_DELAY_NS);
    assert_eq!(fixture.worker_interval_ns, WORKER_INTERVAL_NS);
    assert_eq!(
        fixture
            .worker_interval_sources
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "chaos/chaos-iarb2/src/test/resources/Iarb2CalcCoinSkewTestConfig.json:6",
            "chaos/chaos-iarb2/src/test/resources/Iarb2CalcTestConfig.json:6",
            "chaos/chaos-iarb2/src/test/resources/Iarb2CalcTestConfig2.json:6",
            "chaos/chaos-iarb2/src/test/resources/SampleIarb2Config.json:5",
        ])
    );
    assert_eq!(fixture.worker_interval_sources.len(), 4);
    assert_eq!(fixture.pending_hedge_expiry_ms, 30);
    assert_eq!(fixture.base_depth, base_book("BTC-PERP", 1));
    assert_eq!(
        fixture
            .java_call_path
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec![
            "chaos/chaos-core/src/main/java/app/metcoin/chaos/ChaosStrategyBase.java:302-331",
            "chaos/chaos-core/src/main/java/app/metcoin/chaos/model/entity/OkEntity.java:62-67,94-115,166-213",
            "chaos/chaos-core/src/main/java/app/metcoin/chaos/model/entity/ExchEntityBase.java:137-145,157-166",
            "chaos/chaos-iarb2/src/main/java/app/metcoin/chaos/iarb2/Iarb2Strategy.java:180-182,204-215,399-405,846-869",
            "chaos/chaos-core/src/main/java/app/metcoin/chaos/model/ChaosEntity.java:240-262",
            "chaos/chaos-core/src/main/java/app/metcoin/chaos/worker/ChaosTimedConflationWorker.java:15-59",
            "chaos/chaos-core/src/main/java/app/metcoin/chaos/worker/ChaosConflationWorker.java:46-103",
            "metcoin-parent/metcoin-utils/metcoin-base-utils/src/main/java/app/metcoin/util/number/NumberUtil.java:26-46,156-207,281-308",
        ]
    );

    let expected_atomic_cases = BTreeSet::from([
        "aggressive_buy_below_half",
        "aggressive_buy_exact_half",
        "aggressive_sell_exact_half",
        "fuzzy_equal_but_raw_unequal",
        "ignore_best_below_half",
        "ignore_best_exact_half",
        "passive_buy_maps_to_ask",
        "passive_sell_maps_to_bid",
        "raw_top_equal_below_half",
        "raw_top_equal_exact_half",
    ]);
    let actual_atomic_cases = fixture
        .atomic_cases
        .iter()
        .map(|case| case.id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_atomic_cases, expected_atomic_cases);
    assert_eq!(actual_atomic_cases.len(), fixture.atomic_cases.len());

    for case in &fixture.atomic_cases {
        let mut strategy = implied_depth_strategy(case.ignore_best_level);
        strategy.on_market_event(&MarketEvent::Depth(fixture.base_depth.clone()));

        let immediate = owned_trade(
            &mut strategy,
            case.trade.source_ts_ms,
            case.trade.price,
            case.trade.qty,
            case.trade.taker_side,
            (ARRIVAL_NS, 10, true),
        );

        assert!(immediate.is_empty(), "{}", case.id);
        assert_eq!(
            case.expected.book_side,
            case.trade.taker_side.reverse(),
            "{}",
            case.id
        );
        assert_eq!(
            strategy
                .entity("BTC-PERP")
                .unwrap()
                .effective_levels(case.expected.book_side)[0]
                .px,
            case.expected.first_valid_price,
            "{}",
            case.id
        );
        assert_eq!(
            strategy.next_trade_reprice_due_ns(),
            case.expected
                .due_offset_ns
                .map(|offset| ARRIVAL_NS + offset),
            "{}",
            case.id
        );
        assert_eq!(
            case.expected.crossed_raw_best,
            case.expected.due_offset_ns.is_some(),
            "{}",
            case.id
        );
    }
}

#[test]
fn pinned_java_sequence_fixture_is_strict_and_executable() {
    let fixture = java_fixture();
    let expected_sequences = BTreeSet::from([
        "pending_hedge_exact_and_fuzzy_boundaries",
        "same_price_pending_quantity_accumulates_and_depth_expiry_resets",
        "pending_hedge_strict_local_depth_expiry",
        "alternating_passive_then_crossing_trade_clears_opposite",
        "stale_source_timestamp_latest_arrival_wins",
        "trade_before_depth_is_retained_then_cleared",
        "older_depth_clears_state_without_cancelling_callback",
        "repeated_callbacks_and_inclusive_deadline",
        "shared_worker_depth_timer_fires_before_trade",
        "shared_worker_trade_joins_depth_timer",
        "non_live_crossing_mutates_without_schedule",
    ]);
    let actual_sequences = fixture
        .sequence_cases
        .iter()
        .map(|case| case.id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_sequences, expected_sequences);
    assert_eq!(actual_sequences.len(), fixture.sequence_cases.len());

    for case in &fixture.sequence_cases {
        execute_java_sequence(case);
    }
}

#[test]
fn pinned_java_same_price_pending_quantity_accumulates_and_resets() {
    let fixture = java_fixture();
    let case = fixture
        .sequence_cases
        .iter()
        .find(|case| case.id == "same_price_pending_quantity_accumulates_and_depth_expiry_resets")
        .unwrap();
    execute_java_sequence(case);
}

#[test]
fn fixture_schemas_reject_unknown_fields_in_nested_evidence() {
    let java_raw = include_str!("../../../../fixtures/java/chaos_trade_implied_depth_v2.json");
    let mut java: Value = serde_json::from_str(java_raw).unwrap();
    java["atomic_cases"][0]
        .as_object_mut()
        .unwrap()
        .insert("unreviewed".to_string(), json!(true));
    assert!(serde_json::from_value::<JavaTradeFixture>(java).is_err());

    let mut java: Value = serde_json::from_str(java_raw).unwrap();
    java["sequence_cases"][0]["steps"][0]
        .as_object_mut()
        .unwrap()
        .insert("unreviewed".to_string(), json!(true));
    assert!(serde_json::from_value::<JavaTradeFixture>(java).is_err());

    let golden_raw =
        include_str!("../../../../fixtures/normalized/chaos_trade_implied_depth_intents_v2.json");
    let mut golden: Value = serde_json::from_str(golden_raw).unwrap();
    golden["lifecycle"]["service_points"][0]
        .as_object_mut()
        .unwrap()
        .insert("unreviewed".to_string(), json!(true));
    assert!(serde_json::from_value::<NormalizedGolden>(golden).is_err());
}

#[test]
fn pinned_java_pending_hedge_boundaries_and_ignore_best_are_exact() {
    for (pending_price, expected_ask) in [
        (102.0, 102.0),
        (102.000_000_000_000_5, 102.0),
        (102.000_000_000_002, 103.0),
        (103.0, 103.0),
    ] {
        let mut state = implied_state(false);
        assert!(apply_state_trade(&mut state, 102.0, 4.0, Side::Buy));
        state
            .implied_depth
            .update_our_hedge(Side::Buy, pending_price, 1.0, 100);
        assert_eq!(first_px(&state, Side::Sell), expected_ask);
    }

    let mut allowed = implied_state(true);
    assert!(apply_state_trade(&mut allowed, 102.0, 4.0, Side::Buy));
    allowed
        .implied_depth
        .update_our_hedge(Side::Buy, 102.0, 1.0, 100);
    assert_eq!(first_px(&allowed, Side::Sell), 102.0);

    let mut rejected = implied_state(true);
    assert!(apply_state_trade(&mut rejected, 102.0, 4.0, Side::Buy));
    rejected
        .implied_depth
        .update_our_hedge(Side::Buy, 103.0, 1.0, 100);
    assert_eq!(first_px(&rejected, Side::Sell), 103.0);
}

#[test]
fn no_trade_one_level_ignore_best_preserves_the_existing_fallback() {
    let mut state = implied_state(true);
    state.book = Some(OrderBook::one_level(
        "BTC-PERP",
        1,
        Level::new(99.0, 10.0),
        Level::new(101.0, 10.0),
    ));
    assert_eq!(first_px(&state, Side::Buy), 99.0);
    assert_eq!(first_px(&state, Side::Sell), 101.0);
}

#[test]
fn pending_hedge_expires_strictly_on_local_depth_time_only() {
    let mut strategy = implied_depth_strategy(false);
    strategy.on_market_event(&depth("BTC-PERP", 1));
    strategy
        .entities
        .get_mut("BTC-PERP")
        .unwrap()
        .implied_depth
        .update_our_hedge(Side::Buy, 103.0, 1.0, 100);

    strategy.advance_time(10_000);
    strategy.on_market_event(&trade("BTC-PERP", 2, 102.0, 4.0, Side::Buy));
    assert_eq!(
        strategy
            .entity("BTC-PERP")
            .unwrap()
            .effective_levels(Side::Sell)[0]
            .px,
        103.0
    );

    strategy.on_owned_execution_event_at(
        StrategyEvent::Market(depth("BTC-PERP", 0)),
        0,
        0,
        130,
        false,
    );
    strategy.on_market_event(&trade("BTC-PERP", 1, 102.0, 4.0, Side::Buy));
    assert_eq!(
        strategy
            .entity("BTC-PERP")
            .unwrap()
            .effective_levels(Side::Sell)[0]
            .px,
        103.0
    );

    strategy.on_owned_execution_event_at(
        StrategyEvent::Market(depth("BTC-PERP", 0)),
        0,
        0,
        131,
        false,
    );
    strategy.on_market_event(&trade("BTC-PERP", 0, 102.0, 4.0, Side::Buy));
    assert_eq!(
        strategy
            .entity("BTC-PERP")
            .unwrap()
            .effective_levels(Side::Sell)[0]
            .px,
        102.0
    );
}

#[test]
fn pending_hedge_state_commits_only_after_local_send_acceptance() {
    let mut strategy = implied_depth_strategy(false);
    strategy.on_market_event(&depth("BTC-PERP", 1));
    strategy.on_market_event(&trade("BTC-PERP", 2, 102.0, 4.0, Side::Buy));
    assert_eq!(
        strategy
            .entity("BTC-PERP")
            .unwrap()
            .effective_levels(Side::Sell)[0]
            .px,
        102.0
    );

    let hedge = ChaosExecutionIntent::hedge(
        "BTC-PERP".to_string(),
        Side::Buy,
        1.0,
        103.0,
        "hedge:fixture:103".to_string(),
        crate::execution::ChaosHedgeCommit::new(
            std::sync::Arc::<str>::from("BTC-PERP"),
            Side::Buy,
            103.0,
            1.0,
        ),
    );
    assert_eq!(
        strategy
            .entity("BTC-PERP")
            .unwrap()
            .effective_levels(Side::Sell)[0]
            .px,
        102.0,
        "intent construction must not mutate implied depth"
    );

    let lowered = strategy
        .with_locally_sent_intent(
            hedge,
            || 100,
            |intent| Ok::<_, std::convert::Infallible>(intent.into_order_intent()),
        )
        .unwrap();
    assert!(matches!(lowered, OrderIntent::NewOrder(_)));
    assert_eq!(
        strategy
            .entity("BTC-PERP")
            .unwrap()
            .effective_levels(Side::Sell)[0]
            .px,
        103.0
    );
}

#[test]
fn borrowed_and_owned_timed_depth_and_trade_paths_are_equivalent() {
    let mut borrowed = implied_depth_strategy(false);
    let mut owned = implied_depth_strategy(false);

    let initial_depth = StrategyEvent::Market(depth("BTC-PERP", 1));
    let borrowed_initial = borrowed.on_execution_event_at(&initial_depth, 10, 10, 100, true);
    let owned_initial = owned.on_owned_execution_event_at(initial_depth.clone(), 10, 10, 100, true);
    assert_eq!(
        legacy_projection(&borrowed_initial),
        legacy_projection(&owned_initial)
    );

    for strategy in [&mut borrowed, &mut owned] {
        strategy
            .entities
            .get_mut("BTC-PERP")
            .unwrap()
            .implied_depth
            .update_our_hedge(Side::Buy, 103.0, 1.0, 100);
    }
    let boundary_depth = StrategyEvent::Market(depth("BTC-PERP", 0));
    let borrowed_depth = borrowed.on_execution_event_at(&boundary_depth, 20, 20, 130, true);
    let owned_depth = owned.on_owned_execution_event_at(boundary_depth.clone(), 20, 20, 130, true);
    assert_eq!(
        legacy_projection(&borrowed_depth),
        legacy_projection(&owned_depth)
    );

    let crossing = StrategyEvent::Market(trade("BTC-PERP", 0, 102.0, 4.0, Side::Buy));
    let borrowed_trade =
        borrowed.on_execution_event_at(&crossing, ARRIVAL_NS, ARRIVAL_NS, 130, true);
    let owned_trade =
        owned.on_owned_execution_event_at(crossing.clone(), ARRIVAL_NS, ARRIVAL_NS, 130, true);
    assert_eq!(
        legacy_projection(&borrowed_trade),
        legacy_projection(&owned_trade)
    );
    assert_eq!(
        borrowed.next_trade_reprice_due_ns(),
        owned.next_trade_reprice_due_ns()
    );
    assert_eq!(
        borrowed
            .entity("BTC-PERP")
            .unwrap()
            .effective_levels(Side::Sell)[0]
            .px,
        owned
            .entity("BTC-PERP")
            .unwrap()
            .effective_levels(Side::Sell)[0]
            .px
    );
    assert_eq!(
        borrowed
            .entity("BTC-PERP")
            .unwrap()
            .effective_levels(Side::Sell)[0]
            .px,
        103.0
    );

    let expired_depth = StrategyEvent::Market(depth("BTC-PERP", 0));
    let borrowed_expired =
        borrowed.on_execution_event_at(&expired_depth, ARRIVAL_NS + 1, ARRIVAL_NS + 1, 131, true);
    let owned_expired =
        owned.on_owned_execution_event_at(expired_depth, ARRIVAL_NS + 1, ARRIVAL_NS + 1, 131, true);
    assert_eq!(
        legacy_projection(&borrowed_expired),
        legacy_projection(&owned_expired)
    );
    assert_eq!(
        borrowed
            .entity("BTC-PERP")
            .unwrap()
            .effective_levels(Side::Sell)[0]
            .px,
        101.0
    );
    assert_eq!(
        borrowed.next_trade_reprice_due_ns(),
        owned.next_trade_reprice_due_ns()
    );

    let due_ns = ARRIVAL_NS + CALLBACK_DELAY_NS;
    let borrowed_due = borrowed.service_one_due_trade_reprice(due_ns, 131, false);
    let owned_due = owned.service_one_due_trade_reprice(due_ns, 131, false);
    assert_eq!(
        legacy_projection(&borrowed_due),
        legacy_projection(&owned_due)
    );
    assert_eq!(
        borrowed.next_trade_reprice_due_ns(),
        owned.next_trade_reprice_due_ns()
    );
}

#[test]
fn alternating_trades_coexist_until_a_crossing_trade_clears_the_opposite_side() {
    let mut state = implied_state(false);

    assert!(!apply_state_trade(&mut state, 101.0, 5.0, Side::Buy));
    assert!(!apply_state_trade(&mut state, 99.0, 5.0, Side::Sell));
    assert_eq!(first_px(&state, Side::Sell), 102.0);
    assert_eq!(first_px(&state, Side::Buy), 98.0);

    assert!(apply_state_trade(&mut state, 102.0, 5.0, Side::Buy));
    assert_eq!(first_px(&state, Side::Sell), 103.0);
    assert_eq!(first_px(&state, Side::Buy), 99.0);
}

#[test]
fn arrival_order_wins_over_stale_trade_timestamps_and_depth_clears_trade_state() {
    let mut strategy = implied_depth_strategy(false);
    strategy.on_market_event(&depth("BTC-PERP", 10));
    strategy.on_market_event(&trade("BTC-PERP", 1_000, 102.0, 5.0, Side::Buy));
    assert_eq!(
        strategy
            .entity("BTC-PERP")
            .unwrap()
            .effective_levels(Side::Sell)[0]
            .px,
        103.0
    );

    strategy.on_market_event(&trade("BTC-PERP", 1, 101.0, 4.0, Side::Buy));
    assert_eq!(
        strategy
            .entity("BTC-PERP")
            .unwrap()
            .effective_levels(Side::Sell)[0]
            .px,
        101.0
    );

    strategy.on_market_event(&depth("BTC-PERP", 0));
    assert_eq!(
        strategy
            .entity("BTC-PERP")
            .unwrap()
            .effective_levels(Side::Sell)[0]
            .px,
        101.0
    );
}

#[test]
fn trade_before_depth_is_retained_until_on_depth_clears_it() {
    let mut state = InstrumentState::new(InstrumentConfig {
        symbol: "BTC-PERP".to_string(),
        tick_size: 1.0,
        lot_size: 1.0,
        min_trade_size: 1.0,
        ..InstrumentConfig::default()
    });
    assert!(!apply_state_trade(&mut state, 102.0, 5.0, Side::Buy));

    state.book = Some(base_book("BTC-PERP", 1));
    assert_eq!(first_px(&state, Side::Sell), 103.0);

    state.implied_depth.on_depth(1);
    assert_eq!(first_px(&state, Side::Sell), 101.0);
}

#[test]
fn depth_clears_implied_state_without_cancelling_scheduled_callbacks() {
    let mut strategy = implied_depth_strategy(false);
    strategy.on_market_event(&depth("BTC-PERP", 10));
    owned_trade(
        &mut strategy,
        10,
        102.0,
        5.0,
        Side::Buy,
        (ARRIVAL_NS, 10, true),
    );
    let due_ns = ARRIVAL_NS + CALLBACK_DELAY_NS;
    assert_eq!(strategy.next_trade_reprice_due_ns(), Some(due_ns));

    strategy.on_owned_execution_event_at(
        StrategyEvent::Market(depth("BTC-PERP", 1)),
        ARRIVAL_NS + 1,
        ARRIVAL_NS + 1,
        11,
        true,
    );
    assert_eq!(
        strategy
            .entity("BTC-PERP")
            .unwrap()
            .effective_levels(Side::Sell)[0]
            .px,
        101.0
    );
    assert_eq!(strategy.next_trade_reprice_due_ns(), Some(due_ns));
}

#[test]
fn non_live_crossing_trade_mutates_implied_depth_without_scheduling() {
    let mut strategy = implied_depth_strategy(false);
    strategy.on_market_event(&depth("BTC-PERP", 1));
    let immediate = owned_trade(
        &mut strategy,
        2,
        102.0,
        5.0,
        Side::Buy,
        (ARRIVAL_NS, 2, false),
    );

    assert!(immediate.is_empty());
    assert_eq!(
        strategy
            .entity("BTC-PERP")
            .unwrap()
            .effective_levels(Side::Sell)[0]
            .px,
        103.0
    );
    assert_eq!(strategy.next_trade_reprice_due_ns(), None);
}

#[test]
fn repeated_crossing_trades_keep_exact_inclusive_callback_deadlines() {
    let mut strategy = implied_depth_strategy(false);
    strategy.on_market_event(&depth("BTC-PERP", 1));
    owned_trade(
        &mut strategy,
        2,
        102.0,
        5.0,
        Side::Buy,
        (ARRIVAL_NS, 2, true),
    );
    owned_trade(
        &mut strategy,
        1,
        102.0,
        5.0,
        Side::Buy,
        (ARRIVAL_NS + 20_000, 3, true),
    );

    let first_due = ARRIVAL_NS + CALLBACK_DELAY_NS;
    let second_due = first_due + 20_000;
    assert_eq!(strategy.next_trade_reprice_due_ns(), Some(first_due));
    assert!(
        strategy
            .service_one_due_trade_reprice(first_due - 1, 5, false)
            .is_empty()
    );
    assert_eq!(strategy.next_trade_reprice_due_ns(), Some(first_due));
    assert!(
        strategy
            .service_one_due_trade_reprice(first_due, 5, false)
            .is_empty()
    );
    assert_eq!(strategy.next_trade_reprice_due_ns(), Some(second_due));
    assert!(
        strategy
            .service_one_due_trade_reprice(second_due, 5, false)
            .is_empty()
    );
    assert_eq!(
        strategy.next_trade_reprice_due_ns(),
        Some(second_due + WORKER_INTERVAL_NS)
    );
}

#[test]
fn dense_pending_depth_callbacks_preserve_java_random_cursor() {
    let fixture = rng_interleaving_fixture();
    assert_eq!(fixture.schema_version, 1);
    assert_eq!(fixture.java_revision, PINNED_JAVA_REVISION);
    assert_eq!(fixture.random_seed, 1);
    assert_eq!(fixture.random_draws.len(), 5);
    assert_eq!(fixture.initial_orders.len(), 8);
    assert_eq!(fixture.timeline.pending_depth_receipt_ns, 3_905_900_000);
    assert!(
        fixture.timeline.pending_depth_receipt_ns < fixture.timeline.pending_depth_processing_ns
    );
    assert_eq!(
        fixture.timeline.first_callback_due_ns,
        fixture.timeline.first_trade_arrival_ns + CALLBACK_DELAY_NS
    );
    assert_eq!(
        fixture.timeline.second_callback_due_ns,
        fixture.timeline.second_trade_arrival_ns + CALLBACK_DELAY_NS
    );
    assert_eq!(
        fixture.timeline.second_trade_arrival_ns - fixture.timeline.first_trade_arrival_ns,
        50_000
    );
    assert_eq!(fixture.java_call_path.len(), 6);
    assert!(
        fixture
            .java_call_path
            .iter()
            .any(|path| path.contains("FakeRandomProviderImpl.java"))
    );
    assert!(
        fixture
            .scope_note
            .contains("unaffected by the public trade")
    );

    let mut java_random = JavaRandom::new(fixture.random_seed);
    for (index, draw) in fixture.random_draws.iter().enumerate() {
        assert_eq!(draw.ordinal, index + 1);
        assert_eq!(draw.value.to_bits(), java_random.next_f64().to_bits());
        assert_eq!(
            u64::from_str_radix(&draw.bits_hex, 16).unwrap(),
            draw.value.to_bits()
        );
    }

    let (owned, owned_outputs) = run_rng_interleaving_fixture(&fixture, false);
    let (borrowed, borrowed_outputs) = run_rng_interleaving_fixture(&fixture, true);
    assert_eq!(
        serde_json::to_value(&owned_outputs).unwrap(),
        serde_json::to_value(&borrowed_outputs).unwrap()
    );

    let expected_next_bits = u64::from_str_radix(&fixture.next_random_bits_hex, 16).unwrap();
    for strategy in [owned, borrowed] {
        let mut probe = strategy.pricing.random.clone();
        assert_eq!(
            probe.next_f64().to_bits(),
            expected_next_bits,
            "callbacks and the unchanged multi-level trailing worker must consume no new draw"
        );
    }
}

#[test]
fn ordinary_owned_depths_retire_compatibility_timers_on_a_causal_clock() {
    let mut strategy = implied_depth_strategy(false);
    for now_ms in 1_000..=1_010 {
        let output =
            strategy.on_owned_execution_event(StrategyEvent::Market(depth("BTC-PERP", now_ms)));
        assert!(output.is_empty());
    }

    let trade_receipt_ns = 1_010_500_000;
    let output = strategy.on_owned_execution_event_at(
        StrategyEvent::Market(trade("BTC-PERP", 1_010, 102.0, 5.0, Side::Buy)),
        trade_receipt_ns,
        trade_receipt_ns,
        1_010,
        true,
    );
    assert!(output.is_empty());
    assert_eq!(
        strategy.next_trade_reprice_due_ns(),
        Some(trade_receipt_ns + CALLBACK_DELAY_NS)
    );

    let mut wake_deadlines = Vec::new();
    while let Some(due_ns) = strategy.take_new_trade_reprice_wake_deadline_ns() {
        wake_deadlines.push(due_ns);
    }
    assert_eq!(
        wake_deadlines,
        [1_015_000_000, trade_receipt_ns + CALLBACK_DELAY_NS],
        "the no-arrival wrapper must retain only Java's one causally pending timer"
    );
}

#[test]
fn worker_clock_reads_match_java_for_depth_callback_and_direct_timer_paths() {
    let fixture = worker_clock_fixture();
    assert_eq!(fixture.schema_version, 1);
    assert_eq!(fixture.java_revision, PINNED_JAVA_REVISION);
    assert_eq!(
        fixture.java_call_path,
        [
            "chaos/chaos-iarb2/src/main/java/app/metcoin/chaos/iarb2/Iarb2Strategy.java:205-214",
            "chaos/chaos-iarb2/src/main/java/app/metcoin/chaos/iarb2/Iarb2Strategy.java:399-405",
            "chaos/chaos-core/src/main/java/app/metcoin/chaos/worker/ChaosTimedConflationWorker.java:45-53",
            "chaos/chaos-core/src/main/java/app/metcoin/chaos/worker/ChaosConflationWorker.java:46-103",
        ]
    );
    assert!(fixture.derivation.contains("line 47"));
    assert!(fixture.derivation.contains("line 72"));
    assert!(fixture.derivation.contains("line 78"));
    assert!(fixture.scope_note.contains("add no exchange connectivity"));

    let depth_case = &fixture.immediate_depth;
    assert_eq!(depth_case.id, "depth_decision_start_finish_regression");
    assert_eq!(
        depth_case.expected_last_work_ms,
        depth_case.work_start_ms.max(depth_case.decision_ms)
    );
    assert_eq!(
        depth_case.expected_last_finish_ms,
        depth_case.finish_ms.max(depth_case.expected_last_work_ms)
    );
    assert_eq!(
        depth_case.expected_next_disposition,
        "scheduled_for_one_millisecond"
    );
    let mut depth_strategy = implied_depth_strategy(false);
    let mut depth_clock_reads = 0_u8;
    let depth_output = depth_strategy.on_owned_live_execution_event_at_with_finish_clock(
        StrategyEvent::Market(depth("BTC-PERP", 1)),
        depth_case.decision_ns,
        depth_case.decision_ns,
        depth_case.decision_ms,
        true,
        || {
            depth_clock_reads += 1;
            match depth_clock_reads {
                1 => depth_case.work_start_ms,
                2 => depth_case.finish_ms,
                _ => panic!("immediate depth sampled more than work start and finish"),
            }
        },
    );
    assert_eq!(
        typed_projection(&depth_output),
        expected_typed_projection(&depth_case.expected_output)
    );
    assert_eq!(
        legacy_projection(&depth_output),
        serde_json::to_value(&depth_case.expected_output.legacy).unwrap()
    );
    assert_eq!(
        depth_clock_reads, depth_case.expected_worker_clock_reads,
        "{}",
        depth_case.id
    );
    depth_strategy.on_owned_live_execution_event_at_with_finish_clock(
        StrategyEvent::Market(depth("BTC-PERP", 2)),
        depth_case.next_decision_ns,
        depth_case.next_decision_ns,
        depth_case.next_decision_ms,
        true,
        || panic!("throttled depth must not sample a worker clock"),
    );
    assert_eq!(
        depth_strategy.next_trade_reprice_due_ns(),
        Some(depth_case.expected_next_due_ns),
        "D/S/F=2000/2001/2000 must debounce from Java's clamped 2001 finish"
    );

    let callback_case = &fixture.immediate_callback;
    assert_eq!(
        callback_case.id,
        "trade_callback_decision_start_finish_regression"
    );
    assert_eq!(
        callback_case.expected_last_work_ms,
        callback_case
            .work_start_ms
            .max(callback_case.callback_decision_ms)
    );
    assert_eq!(
        callback_case.expected_last_finish_ms,
        callback_case
            .finish_ms
            .max(callback_case.expected_last_work_ms)
    );
    assert_eq!(
        callback_case.expected_next_disposition,
        "scheduled_for_one_millisecond"
    );
    let mut callback_strategy = implied_depth_strategy(false);
    callback_strategy.on_owned_live_execution_event_at_with_finish_clock(
        StrategyEvent::Market(depth("BTC-PERP", 1)),
        callback_case.seed_decision_ns,
        callback_case.seed_decision_ns,
        callback_case.seed_ms,
        true,
        || callback_case.seed_ms,
    );
    callback_strategy.on_owned_live_execution_event_at_with_finish_clock(
        StrategyEvent::Market(trade("BTC-PERP", 2, 102.0, 5.0, Side::Buy)),
        callback_case.trade_receipt_ns,
        callback_case.trade_processing_ns,
        callback_case.trade_processing_ms,
        true,
        || panic!("trade delivery must not run the deferred pricing worker"),
    );
    let mut callback_clock_reads = 0_u8;
    let callback_output = callback_strategy.service_one_due_trade_reprice_with_clocks(
        || {
            (
                callback_case.trade_receipt_ns + CALLBACK_DELAY_NS,
                callback_case.callback_decision_ms,
            )
        },
        true,
        || {
            callback_clock_reads += 1;
            match callback_clock_reads {
                1 => callback_case.work_start_ms,
                2 => callback_case.finish_ms,
                _ => panic!("immediate callback sampled more than work start and finish"),
            }
        },
    );
    assert_eq!(
        typed_projection(&callback_output),
        expected_typed_projection(&callback_case.expected_output)
    );
    assert_eq!(
        legacy_projection(&callback_output),
        serde_json::to_value(&callback_case.expected_output.legacy).unwrap()
    );
    assert_eq!(
        callback_clock_reads, callback_case.expected_worker_clock_reads,
        "{}",
        callback_case.id
    );
    callback_strategy.on_owned_live_execution_event_at_with_finish_clock(
        StrategyEvent::Market(depth("BTC-PERP", 3)),
        callback_case.next_decision_ns,
        callback_case.next_decision_ns,
        callback_case.next_decision_ms,
        true,
        || panic!("throttled post-callback depth must not sample a worker clock"),
    );
    assert_eq!(
        callback_strategy.next_trade_reprice_due_ns(),
        Some(callback_case.expected_next_due_ns),
        "D/S/F=1005/1006/1005 must debounce from Java's clamped 1006 finish"
    );

    let timer_case = &fixture.direct_timer;
    assert_eq!(timer_case.id, "scheduled_timer_start_finish_only");
    assert_eq!(
        timer_case.expected_last_work_ms,
        timer_case.work_start_ms.max(timer_case.scheduled_time_ms)
    );
    assert_eq!(
        timer_case.expected_last_finish_ms,
        timer_case.finish_ms.max(timer_case.expected_last_work_ms)
    );
    assert_eq!(
        timer_case.expected_next_disposition,
        "scheduled_for_one_millisecond"
    );
    let mut timer_strategy = implied_depth_strategy(false);
    timer_strategy.on_owned_live_execution_event_at_with_finish_clock(
        StrategyEvent::Market(depth("BTC-PERP", 1)),
        timer_case.seed_decision_ns,
        timer_case.seed_decision_ns,
        timer_case.seed_ms,
        true,
        || timer_case.seed_ms,
    );
    timer_strategy.on_owned_live_execution_event_at_with_finish_clock(
        StrategyEvent::Market(depth("BTC-PERP", 2)),
        timer_case.throttled_decision_ns,
        timer_case.throttled_decision_ns,
        timer_case.throttled_decision_ms,
        true,
        || panic!("throttled depth must not sample a worker clock"),
    );
    assert_eq!(
        timer_strategy.next_trade_reprice_due_ns(),
        Some(timer_case.timer_due_ns)
    );
    let mut timer_finish_reads = 0_u8;
    let timer_output = timer_strategy.service_one_due_trade_reprice_with_clocks(
        || (timer_case.timer_due_ns, timer_case.work_start_ms),
        true,
        || {
            timer_finish_reads += 1;
            if timer_finish_reads == 1 {
                timer_case.finish_ms
            } else {
                panic!("direct timer start is already sampled; only finish remains")
            }
        },
    );
    assert_eq!(
        typed_projection(&timer_output),
        expected_typed_projection(&timer_case.expected_output)
    );
    assert_eq!(
        legacy_projection(&timer_output),
        serde_json::to_value(&timer_case.expected_output.legacy).unwrap()
    );
    assert_eq!(
        timer_finish_reads, timer_case.expected_worker_clock_reads,
        "{}",
        timer_case.id
    );
    timer_strategy.on_owned_live_execution_event_at_with_finish_clock(
        StrategyEvent::Market(depth("BTC-PERP", 3)),
        timer_case.post_timer_decision_ns,
        timer_case.post_timer_decision_ns,
        timer_case.post_timer_decision_ms,
        true,
        || panic!("throttled post-timer depth must not sample a worker clock"),
    );
    assert_eq!(
        timer_strategy.next_trade_reprice_due_ns(),
        Some(timer_case.expected_next_due_ns)
    );
}

#[test]
fn five_millisecond_worker_conflates_callbacks_while_latest_trade_state_wins() {
    let (events, golden) = normalized_fixture();
    let burst = &golden.burst_conflation;
    let mut strategy = quote_reprice_strategy();
    for (index, event) in events.iter().take(6).enumerate() {
        drain_private_depth_work_before(
            &mut strategy,
            golden.lifecycle.event_arrival_ns[index],
            golden.lifecycle.strategy_is_live,
        );
        let intents = strategy.on_owned_execution_event_at(
            event.clone().into_strategy_event(),
            golden.lifecycle.event_arrival_ns[index],
            golden.lifecycle.event_arrival_ns[index],
            golden.lifecycle.event_observed_now_ms[index],
            golden.lifecycle.strategy_is_live,
        );
        assert_eq!(
            typed_projection(&intents),
            expected_typed_projection(&golden.event_outputs[index])
        );
    }
    drain_private_depth_work_before(
        &mut strategy,
        burst.first_arrival_ns,
        golden.lifecycle.strategy_is_live,
    );

    owned_trade(
        &mut strategy,
        900,
        50_005.0,
        5_000.0,
        Side::Buy,
        (burst.first_arrival_ns, 900, true),
    );
    assert_eq!(
        burst.first_due_ns,
        burst.first_arrival_ns + CALLBACK_DELAY_NS
    );
    let first_refresh = strategy.service_one_due_trade_reprice(burst.first_due_ns, 1_000, true);
    assert_eq!(
        typed_projection(&first_refresh),
        expected_typed_projection(&burst.first_due_output)
    );
    assert_eq!(
        legacy_projection(&first_refresh),
        expected_legacy_projection(&burst.first_due_output)
    );

    owned_trade(
        &mut strategy,
        800,
        50_005.0,
        5_000.0,
        Side::Buy,
        (burst.second_arrival_ns, 1_000, true),
    );
    owned_trade(
        &mut strategy,
        1,
        50_005.0,
        4_999.0,
        Side::Buy,
        (burst.third_arrival_ns, 1_000, true),
    );

    assert_eq!(
        burst.second_due_ns,
        burst.second_arrival_ns + CALLBACK_DELAY_NS
    );
    assert_eq!(
        burst.third_due_ns,
        burst.third_arrival_ns + CALLBACK_DELAY_NS
    );
    assert_eq!(
        strategy.next_trade_reprice_due_ns(),
        Some(burst.second_due_ns)
    );
    assert!(
        strategy
            .service_one_due_trade_reprice(burst.second_due_ns, 1_000, true)
            .is_empty()
    );
    assert_eq!(
        strategy.next_trade_reprice_due_ns(),
        Some(burst.third_due_ns)
    );
    assert!(
        strategy
            .service_one_due_trade_reprice(burst.third_due_ns, 1_000, true)
            .is_empty()
    );
    assert_eq!(
        strategy.next_trade_reprice_due_ns(),
        Some(burst.trailing_due_ns)
    );
    assert_eq!(
        burst.trailing_due_ns,
        burst.second_due_ns + WORKER_INTERVAL_NS
    );
    assert_eq!(
        strategy
            .entity("BTC-PERP")
            .unwrap()
            .effective_levels(Side::Sell)[0]
            .px,
        burst.latest_effective_sell_price
    );
    let trailing_refresh =
        strategy.service_one_due_trade_reprice(burst.trailing_due_ns, 1_005, true);
    assert_eq!(strategy.next_trade_reprice_due_ns(), None);
    assert_eq!(
        typed_projection(&trailing_refresh),
        expected_typed_projection(&burst.trailing_due_output)
    );
    assert_eq!(
        legacy_projection(&trailing_refresh),
        expected_legacy_projection(&burst.trailing_due_output)
    );
    assert_eq!(
        strategy
            .entity("BTC-PERP")
            .unwrap()
            .effective_levels(Side::Sell)[0]
            .px,
        burst.latest_effective_sell_price
    );
}

#[test]
fn normalized_scenario_and_golden_drive_owned_and_borrowed_lifecycle_to_horizon() {
    let raw = include_str!("../../../../fixtures/normalized/chaos_trade_implied_depth.jsonl");
    assert!(raw.contains("# schema_version=2"));
    assert!(raw.contains(&format!("# java_revision={PINNED_JAVA_REVISION}")));
    assert!(raw.contains("# golden=chaos_trade_implied_depth_intents_v2.json"));
    assert!(raw.contains(
        "# lifecycle=events are delivered in file order at the arrival/local clocks in the paired golden; timed Live depth and deferred trade callbacks share the exact pinned pricing worker, and private due work is serviced before later arrivals; listed trade service points continue through the inclusive horizon"
    ));
    assert!(raw.contains(
        "# java_call_path=ChaosStrategyBase.onPublicTrade->OkEntity.onPublicTrade/isDepthUpdatedOnTrade->Iarb2Strategy.onPublicTrade->ChaosTimedConflationWorker/ChaosConflationWorker"
    ));

    let (events, golden) = normalized_fixture();
    assert_eq!(golden.schema_version, 2);
    assert_eq!(golden.java_revision, PINNED_JAVA_REVISION);
    assert_eq!(golden.scenario_fixture, "chaos_trade_implied_depth.jsonl");
    assert_eq!(events.len(), golden.event_outputs.len());
    assert_eq!(events.len(), golden.lifecycle.event_arrival_ns.len());
    assert_eq!(events.len(), golden.lifecycle.event_observed_now_ms.len());

    let mut owned = quote_reprice_strategy();
    let mut borrowed = quote_reprice_strategy();
    for (index, event) in events.iter().enumerate() {
        let event = event.clone().into_strategy_event();
        let arrival_ns = golden.lifecycle.event_arrival_ns[index];
        let observed_now_ms = golden.lifecycle.event_observed_now_ms[index];
        drain_private_depth_work_before(&mut owned, arrival_ns, golden.lifecycle.strategy_is_live);
        drain_private_depth_work_before(
            &mut borrowed,
            arrival_ns,
            golden.lifecycle.strategy_is_live,
        );
        let owned_output = owned.on_owned_execution_event_at(
            event.clone(),
            arrival_ns,
            arrival_ns,
            observed_now_ms,
            golden.lifecycle.strategy_is_live,
        );
        let borrowed_output = borrowed.on_execution_event_at(
            &event,
            arrival_ns,
            arrival_ns,
            observed_now_ms,
            golden.lifecycle.strategy_is_live,
        );
        let expected = &golden.event_outputs[index];

        assert_eq!(
            typed_projection(&owned_output),
            typed_projection(&borrowed_output),
            "typed event output {index}"
        );
        assert_eq!(
            legacy_projection(&owned_output),
            legacy_projection(&borrowed_output),
            "legacy event output {index}"
        );
        assert_eq!(
            typed_projection(&owned_output),
            expected_typed_projection(expected),
            "typed golden event output {index}"
        );
        assert_eq!(
            legacy_projection(&owned_output),
            expected_legacy_projection(expected),
            "legacy golden event output {index}"
        );
    }

    for (index, point) in golden.lifecycle.service_points.iter().enumerate() {
        assert!(point.now_ns <= golden.lifecycle.service_horizon_ns);
        let owned_output = owned.service_one_due_trade_reprice(
            point.now_ns,
            point.observed_now_ms,
            golden.lifecycle.strategy_is_live,
        );
        let borrowed_output = borrowed.service_one_due_trade_reprice(
            point.now_ns,
            point.observed_now_ms,
            golden.lifecycle.strategy_is_live,
        );
        assert_eq!(
            typed_projection(&owned_output),
            typed_projection(&borrowed_output),
            "typed service output {index}"
        );
        assert_eq!(
            legacy_projection(&owned_output),
            legacy_projection(&borrowed_output),
            "legacy service output {index}"
        );
        assert_eq!(
            typed_projection(&owned_output),
            expected_typed_projection(&point.output),
            "typed golden service output {index}"
        );
        assert_eq!(
            legacy_projection(&owned_output),
            expected_legacy_projection(&point.output),
            "legacy golden service output {index}"
        );
        assert_eq!(
            owned.next_trade_reprice_due_ns(),
            point.expected_next_due_ns,
            "owned next deadline after service {index}"
        );
        assert_eq!(
            borrowed.next_trade_reprice_due_ns(),
            point.expected_next_due_ns,
            "borrowed next deadline after service {index}"
        );
    }

    assert_eq!(
        golden.lifecycle.service_points.last().unwrap().now_ns,
        golden.lifecycle.service_horizon_ns
    );
    assert_eq!(
        owned
            .entity("BTC-PERP")
            .unwrap()
            .effective_levels(Side::Sell)[0]
            .px,
        golden.final_state.effective_sell_price
    );
    assert_eq!(
        borrowed
            .entity("BTC-PERP")
            .unwrap()
            .effective_levels(Side::Sell)[0]
            .px,
        golden.final_state.effective_sell_price
    );
    assert_eq!(
        owned.next_trade_reprice_due_ns(),
        golden.final_state.next_due_ns
    );
    assert_eq!(
        borrowed.next_trade_reprice_due_ns(),
        golden.final_state.next_due_ns
    );
}
