use super::*;
use reap_core::{OrderIntent, TimeMs};
use reap_strategy::ChaosExecutionPurpose;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
struct NormalizedGolden {
    lifecycle: NormalizedLifecycle,
    event_outputs: Vec<ExpectedOutput>,
    final_state: ExpectedFinalState,
}

#[derive(Debug, Deserialize)]
struct NormalizedLifecycle {
    strategy_is_live: bool,
    event_arrival_ns: Vec<u64>,
    event_observed_now_ms: Vec<TimeMs>,
    service_horizon_ns: u64,
    service_points: Vec<ExpectedServicePoint>,
}

#[derive(Debug, Deserialize)]
struct ExpectedServicePoint {
    now_ns: u64,
    observed_now_ms: TimeMs,
    expected_next_due_ns: Option<u64>,
    output: ExpectedOutput,
}

#[derive(Debug, Deserialize)]
struct ExpectedOutput {
    typed: Vec<ExpectedTypedIntent>,
    legacy: Vec<OrderIntent>,
}

#[derive(Debug, Deserialize)]
struct ExpectedTypedIntent {
    purpose: String,
    legacy: OrderIntent,
}

#[derive(Debug, Deserialize)]
struct ExpectedFinalState {
    next_due_ns: Option<u64>,
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

fn normalized_fixture_config() -> ChaosConfig {
    let mut config = config();
    config.delta_limit_usd = 50_000.0;
    config.active_hedge_threshold_usd = 1_000.0;
    config.risk_groups[0].soft_delta_limit_usd = 25_000.0;
    config.risk_groups[0].hard_delta_limit_usd = 40_000.0;
    config.risk_groups[0].delta_stop_limit_usd = 60_000.0;
    let future = config
        .instruments
        .iter_mut()
        .find(|instrument| instrument.symbol == "BTC-PERP")
        .unwrap();
    future.max_order_size_usd = 5_000.0;
    future.max_order_size = 200.0;
    future.min_position = -10_000.0;
    future.max_position = 10_000.0;
    for instrument in &mut config.instruments {
        instrument.debounce_width = 0.0;
        instrument.debounce_size_usd = 0.0;
        instrument.debounce_ms = 0;
    }
    config
}

fn fixture_execution_with_order_entry_blocked() -> BacktestExecutionConfig {
    BacktestExecutionConfig {
        // This unused valuation route keeps simulated order entry closed. The
        // fixture's explicit PendingNew updates then represent exactly the
        // four quoted orders in the golden, without duplicate matcher orders.
        currency_rates: vec![BacktestCurrencyRateConfig {
            currency: "UNUSED".to_string(),
            index_symbol: "UNUSED-USD".to_string(),
            max_age_ms: 60_000,
        }],
        ..BacktestExecutionConfig::default()
    }
}

fn take_chaos_intent_trace(
    runner: &mut BacktestRunner,
) -> Vec<(ChaosExecutionPurpose, OrderIntent)> {
    runner.take_chaos_intent_trace()
}

fn typed_trace_projection(trace: &[(ChaosExecutionPurpose, OrderIntent)]) -> Value {
    json!(
        trace
            .iter()
            .map(|(purpose, legacy)| {
                json!({
                    "purpose": purpose.as_str(),
                    "legacy": legacy,
                })
            })
            .collect::<Vec<_>>()
    )
}

fn legacy_trace_projection(trace: &[(ChaosExecutionPurpose, OrderIntent)]) -> Value {
    json!(trace.iter().map(|(_, legacy)| legacy).collect::<Vec<_>>())
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

fn implied_depth_book(ts_ms: TimeMs) -> NormalizedEvent {
    NormalizedEvent::Market(MarketEvent::Depth(OrderBook {
        symbol: "BTC-PERP".to_string(),
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
    }))
}

fn crossing_trade(ts_ms: TimeMs) -> NormalizedEvent {
    NormalizedEvent::Market(MarketEvent::Trade {
        ts_ms,
        symbol: "BTC-PERP".to_string(),
        price: 102.0,
        qty: 5.0,
        taker_side: Side::Buy,
    })
}

fn seed_implied_books(runner: &mut BacktestRunner, arrival_ns: u64) {
    runner
        .process_replay_event_at(
            NormalizedEvent::Market(MarketEvent::Depth(OrderBook::one_level(
                "BTC-USDT",
                1,
                Level::new(99.0, 10.0),
                Level::new(101.0, 10.0),
            ))),
            arrival_ns,
        )
        .unwrap();
    runner
        .process_replay_event_at(implied_depth_book(1), arrival_ns)
        .unwrap();
}

#[test]
fn trade_reprice_uses_replay_arrival_and_inclusive_nanosecond_wake() {
    let mut runner = BacktestRunner::new(config()).unwrap();
    seed_implied_books(&mut runner, 900_000_000);

    runner
        .process_replay_event_at(crossing_trade(2), 1_000_000_000)
        .unwrap();

    assert_eq!(
        runner.strategy.next_trade_reprice_due_ns(),
        Some(1_000_100_000)
    );
    assert!(
        runner
            .schedule
            .scheduled
            .iter()
            .any(|(&(due_ns, _), action)| due_ns == 1_000_100_000
                && matches!(
                    action,
                    ScheduledAction::TradeRepriceWake {
                        deadline_ns: 1_000_100_000
                    }
                ))
    );

    runner.drain_before(1_000_100_000).unwrap();
    assert_eq!(
        runner.strategy.next_trade_reprice_due_ns(),
        Some(1_000_100_000)
    );
    runner.drain_through(1_000_100_000).unwrap();
    assert_eq!(runner.strategy.next_trade_reprice_due_ns(), None);
}

#[test]
fn first_qualifying_trade_inherits_the_prior_depth_worker_clock() {
    let mut runner = BacktestRunner::new(config()).unwrap();
    runner
        .process_replay_event_at(implied_depth_book(1), 1_000_000_000)
        .unwrap();
    assert!(!runner.replay.trade_reprice_active);
    assert_eq!(runner.strategy.next_trade_reprice_due_ns(), None);

    runner
        .process_replay_event_at(crossing_trade(2), 1_001_000_000)
        .unwrap();
    assert!(runner.replay.trade_reprice_active);
    assert_eq!(
        runner.strategy.next_trade_reprice_due_ns(),
        Some(1_001_100_000)
    );

    runner.drain_through(1_001_100_000).unwrap();
    assert_eq!(
        runner.strategy.next_trade_reprice_due_ns(),
        Some(1_005_100_000),
        "the callback must retain the four milliseconds remaining since the prior depth worker"
    );
}

#[test]
fn causal_activation_promotes_a_pending_depth_wake_and_the_trade_callback() {
    let mut runner = BacktestRunner::new(config()).unwrap();
    runner
        .process_replay_event_at(implied_depth_book(1), 1_000_000_000)
        .unwrap();
    runner
        .process_replay_event_at(implied_depth_book(2), 1_001_000_000)
        .unwrap();

    runner
        .process_replay_event_at(crossing_trade(3), 1_002_000_000)
        .unwrap();
    assert!(runner.replay.trade_reprice_active);
    assert_eq!(
        runner.strategy.next_trade_reprice_due_ns(),
        Some(1_002_100_000)
    );
    for deadline_ns in [1_002_100_000, 1_005_000_000] {
        assert!(
            runner
                .schedule
                .scheduled
                .iter()
                .any(|(&(due_ns, _), action)| due_ns == deadline_ns
                    && matches!(action, ScheduledAction::TradeRepriceWake { .. })),
            "global replay scheduler is missing private wake {deadline_ns}"
        );
    }

    runner.drain_through(1_002_100_000).unwrap();
    assert_eq!(
        runner.strategy.next_trade_reprice_due_ns(),
        Some(1_005_000_000)
    );
}

#[test]
fn delayed_trade_delivery_services_shadow_worker_at_processing_time() {
    let execution = BacktestExecutionConfig {
        market_data_latency_ms: 1,
        ..BacktestExecutionConfig::default()
    };
    let mut runner = BacktestRunner::with_execution_config(config(), execution).unwrap();
    seed_implied_books(&mut runner, 900_000_000);
    runner.drain_through(901_000_000).unwrap();

    runner
        .process_replay_event_at(crossing_trade(2), 905_500_000)
        .unwrap();
    assert!(!runner.replay.trade_reprice_active);

    runner.drain_through(906_500_000).unwrap();
    assert!(runner.replay.trade_reprice_active);
    assert_eq!(
        runner.strategy.next_trade_reprice_due_ns(),
        Some(911_500_000),
        "the shadow worker due between receipt and delivery must be serviced before activation; \
         the past-due receipt callback then starts a fresh five-millisecond trailing interval"
    );
}

#[test]
fn equal_arrival_callbacks_keep_sequence_and_share_one_trailing_worker() {
    let mut runner = BacktestRunner::new(config()).unwrap();
    seed_implied_books(&mut runner, 900_000_000);

    runner
        .process_replay_event_at(crossing_trade(2), 1_000_000_000)
        .unwrap();
    runner
        .process_replay_event_at(crossing_trade(3), 1_000_000_000)
        .unwrap();
    assert_eq!(
        runner.strategy.next_trade_reprice_due_ns(),
        Some(1_000_100_000)
    );
    assert_eq!(
        runner
            .schedule
            .scheduled
            .iter()
            .filter(|&(&(due_ns, _), ref action)| {
                due_ns == 1_000_100_000
                    && matches!(action, ScheduledAction::TradeRepriceWake { .. })
            })
            .count(),
        2,
        "each private callback insertion must receive its own global wake"
    );

    runner.drain_through(1_000_100_000).unwrap();
    assert_eq!(
        runner.strategy.next_trade_reprice_due_ns(),
        Some(1_005_100_000),
        "the second retained callback must schedule the pinned 5ms trailing worker"
    );
    assert!(
        runner
            .schedule
            .scheduled
            .iter()
            .any(|(&(due_ns, _), action)| {
                due_ns == 1_005_100_000
                    && matches!(
                        action,
                        ScheduledAction::TradeRepriceWake {
                            deadline_ns: 1_005_100_000
                        }
                    )
            })
    );
}

#[test]
fn equal_deadline_wakes_preserve_creation_order_across_the_global_scheduler() {
    let mut runner = BacktestRunner::new(config()).unwrap();
    seed_implied_books(&mut runner, 900_000_000);

    runner
        .process_replay_event_at(crossing_trade(2), 1_000_000_000)
        .unwrap();
    runner
        .process_replay_event_at(crossing_trade(3), 1_000_000_000)
        .unwrap();

    let deadline_ns = 1_000_100_000;
    runner.schedule_at(
        deadline_ns,
        ScheduledAction::DeliverStrategy(StrategyEvent::Timer(TimerEvent {
            ts_ms: 1_000,
            name: "same-deadline-collision".to_string(),
        })),
    );

    let colliding = runner
        .schedule
        .scheduled
        .iter()
        .filter(|&(&(due_ns, _), _)| due_ns == deadline_ns)
        .map(|(&(due_ns, sequence), action)| ((due_ns, sequence), action))
        .collect::<Vec<_>>();
    assert_eq!(colliding.len(), 3);
    assert!(matches!(
        colliding[0].1,
        ScheduledAction::TradeRepriceWake { .. }
    ));
    assert!(matches!(
        colliding[1].1,
        ScheduledAction::TradeRepriceWake { .. }
    ));
    assert!(matches!(
        colliding[2].1,
        ScheduledAction::DeliverStrategy(StrategyEvent::Timer(TimerEvent { name, .. }))
            if name == "same-deadline-collision"
    ));
    assert!(colliding[0].0 < colliding[1].0 && colliding[1].0 < colliding[2].0);

    runner.drain_through(deadline_ns).unwrap();
    assert_eq!(
        runner.strategy.next_trade_reprice_due_ns(),
        Some(deadline_ns + 5_000_000)
    );
}

#[test]
fn replay_scheduler_orders_sub_100us_trade_callbacks_before_pending_depth_work() {
    let mut runner = BacktestRunner::new(config()).unwrap();
    seed_implied_books(&mut runner, 900_000_000);

    runner
        .process_replay_event_at(crossing_trade(2), 1_000_000_000)
        .unwrap();
    runner.drain_through(1_000_100_000).unwrap();
    assert!(runner.replay.trade_reprice_active);
    assert_eq!(runner.strategy.next_trade_reprice_due_ns(), None);

    runner
        .process_replay_event_at(implied_depth_book(3), 1_001_000_000)
        .unwrap();
    assert_eq!(
        runner.strategy.next_trade_reprice_due_ns(),
        Some(1_005_000_000),
        "the reached depth must leave one shared trailing worker"
    );

    for (trade_ts_ms, arrival_ns) in [(4, 1_001_050_000), (5, 1_001_100_000)] {
        runner
            .process_replay_event_at(crossing_trade(trade_ts_ms), arrival_ns)
            .unwrap();
    }
    assert_eq!(
        runner.strategy.next_trade_reprice_due_ns(),
        Some(1_001_150_000)
    );

    runner.drain_through(1_001_150_000).unwrap();
    assert_eq!(
        runner.strategy.next_trade_reprice_due_ns(),
        Some(1_001_200_000)
    );
    runner.drain_through(1_001_200_000).unwrap();
    assert_eq!(
        runner.strategy.next_trade_reprice_due_ns(),
        Some(1_005_000_000),
        "both callbacks must conflate into the pending depth refresh"
    );
    runner.drain_through(1_005_000_000).unwrap();
    assert_eq!(runner.strategy.next_trade_reprice_due_ns(), None);
}

#[test]
fn delayed_strategy_delivery_never_moves_replay_time_back_to_the_deadline() {
    let execution = BacktestExecutionConfig {
        market_data_latency_ms: 1,
        ..BacktestExecutionConfig::default()
    };
    let mut runner = BacktestRunner::with_execution_config(config(), execution).unwrap();
    seed_implied_books(&mut runner, 900_000_000);

    runner
        .process_replay_event_at(crossing_trade(2), 1_000_000_000)
        .unwrap();

    assert_eq!(runner.replay.now_ns, 1_000_000_000);
    runner.drain_through(1_001_000_000).unwrap();
    assert_eq!(runner.replay.now_ns, 1_001_000_000);
    assert!(
        runner
            .schedule
            .scheduled
            .keys()
            .all(|(due_ns, _)| *due_ns >= runner.replay.now_ns),
        "past-due private actions must execute at current scheduler time"
    );
}

#[test]
fn a_later_passive_trade_cannot_change_earlier_replay_decisions() {
    let baseline_events = load_normalized_jsonl(
        include_str!("../../../../fixtures/normalized/chaos_quote_hedge.jsonl").as_bytes(),
    )
    .unwrap();
    let mut with_passive_trade = baseline_events.clone();
    let final_ts_ms = with_passive_trade
        .last()
        .map(NormalizedEvent::ts_ms)
        .unwrap_or_default()
        .saturating_add(1);
    with_passive_trade.push(NormalizedEvent::Market(MarketEvent::Trade {
        ts_ms: final_ts_ms,
        symbol: "BTC-PERP".to_string(),
        price: 1.0,
        qty: 1.0,
        taker_side: Side::Buy,
    }));

    let mut baseline_runner = BacktestRunner::new(config()).unwrap();
    let baseline = baseline_runner.run(baseline_events).unwrap();
    let mut appended_runner = BacktestRunner::new(config()).unwrap();
    let appended = appended_runner.run(with_passive_trade).unwrap();

    assert!(!appended_runner.replay.trade_reprice_active);
    assert_eq!(appended.orders_sent, baseline.orders_sent);
    assert_eq!(appended.cancel_requests, baseline.cancel_requests);
    assert_eq!(appended.exchange_activations, baseline.exchange_activations);
    assert_eq!(appended.fills, baseline.fills);
    assert_eq!(
        appended.pending_scheduled_actions,
        baseline.pending_scheduled_actions
    );
}

#[test]
fn normalized_fixture_drives_exact_production_outputs_through_its_inclusive_horizon() {
    let (events, golden) = normalized_fixture();
    assert!(golden.lifecycle.strategy_is_live);
    assert_eq!(events.len(), golden.event_outputs.len());
    assert_eq!(events.len(), golden.lifecycle.event_arrival_ns.len());
    assert_eq!(events.len(), golden.lifecycle.event_observed_now_ms.len());

    let mut runner = BacktestRunner::with_execution_config(
        normalized_fixture_config(),
        fixture_execution_with_order_entry_blocked(),
    )
    .unwrap();
    let _trace_guard = runner.begin_chaos_intent_trace();

    for (index, event) in events.into_iter().enumerate() {
        runner
            .process_replay_event_at(event, golden.lifecycle.event_arrival_ns[index])
            .unwrap();
        assert_eq!(
            time_ms(runner.replay.now_ns),
            golden.lifecycle.event_observed_now_ms[index],
            "production replay observed clock after event {index}"
        );

        let trace = take_chaos_intent_trace(&mut runner);
        let expected = &golden.event_outputs[index];
        assert_eq!(
            typed_trace_projection(&trace),
            expected_typed_projection(expected),
            "typed production output for event {index}"
        );
        assert_eq!(
            legacy_trace_projection(&trace),
            expected_legacy_projection(expected),
            "legacy production output for event {index}"
        );
    }

    for (index, point) in golden.lifecycle.service_points.iter().enumerate() {
        assert!(point.now_ns <= golden.lifecycle.service_horizon_ns);
        assert_eq!(time_ms(point.now_ns), point.observed_now_ms);
        runner.drain_through(point.now_ns).unwrap();

        let trace = take_chaos_intent_trace(&mut runner);
        assert_eq!(
            typed_trace_projection(&trace),
            expected_typed_projection(&point.output),
            "typed production output at service point {index}"
        );
        assert_eq!(
            legacy_trace_projection(&trace),
            expected_legacy_projection(&point.output),
            "legacy production output at service point {index}"
        );
        assert_eq!(
            runner.strategy.next_trade_reprice_due_ns(),
            point.expected_next_due_ns,
            "next private deadline after service point {index}"
        );
    }

    assert_eq!(
        golden.lifecycle.service_points.last().unwrap().now_ns,
        golden.lifecycle.service_horizon_ns
    );
    assert_eq!(runner.replay.now_ns, golden.lifecycle.service_horizon_ns);
    assert_eq!(
        runner.strategy.next_trade_reprice_due_ns(),
        golden.final_state.next_due_ns
    );
    assert!(
        !runner
            .schedule
            .scheduled
            .values()
            .any(|action| matches!(action, ScheduledAction::TradeRepriceWake { .. })),
        "inclusive fixture horizon must leave no global wake for a private action"
    );
}
