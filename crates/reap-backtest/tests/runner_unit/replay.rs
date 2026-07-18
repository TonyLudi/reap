use super::*;

#[test]
fn replayed_quote_fill_triggers_hedge_order() {
    let mut runner = BacktestRunner::new(config()).unwrap();
    let mut events = initial_books();
    events.push(NormalizedEvent::from(MarketEvent::Trade {
        ts_ms: 2,
        symbol: "BTC-USDT".to_string(),
        price: 49_000.0,
        qty: 1.0,
        taker_side: Side::Sell,
    }));

    let report = runner.run(events).unwrap();
    assert!(report.orders_sent >= 3);
    assert!(report.fills >= 1);
    assert!(report.taker_fills >= 1);
    assert!(report.final_delta_usd.abs() < 5_000.0);
    assert_eq!(report.execution, BacktestExecutionConfig::default());
}

#[test]
fn normalized_fixture_replays_quote_and_hedge_path() {
    let events = load_normalized_jsonl(
        include_str!("../../../../fixtures/normalized/chaos_quote_hedge.jsonl").as_bytes(),
    )
    .unwrap();
    let mut runner = BacktestRunner::new(config()).unwrap();

    let report = runner.run(events).unwrap();

    assert!(report.orders_sent >= 1);
    assert_eq!(report.fills, 2);
    assert_eq!(report.maker_fills, 1);
    assert_eq!(report.taker_fills, 1);
    assert!(report.final_delta_usd.abs() < 1_000.0);
}

#[test]
fn delayed_entry_is_reported_as_pending_at_end_of_data() {
    let execution = BacktestExecutionConfig {
        order_entry_latency_ms: 10,
        ..BacktestExecutionConfig::default()
    };
    let mut runner = BacktestRunner::with_execution_config(config(), execution).unwrap();

    let report = runner.run(initial_books()).unwrap();

    assert!(report.orders_sent > 0);
    assert_eq!(report.exchange_activations, 0);
    assert_eq!(report.pending_orders, report.orders_sent);
    assert_eq!(report.pending_activation_actions, report.pending_orders);
    assert!(report.pending_scheduled_actions >= report.pending_orders);
}

#[test]
fn delayed_market_data_is_not_delivered_past_end_of_data() {
    let execution = BacktestExecutionConfig {
        market_data_latency_ms: 10,
        ..BacktestExecutionConfig::default()
    };
    let mut runner = BacktestRunner::with_execution_config(config(), execution).unwrap();

    let report = runner.run(initial_books()).unwrap();

    assert_eq!(report.orders_sent, 0);
    assert_eq!(report.pending_scheduled_actions, 2);
    assert_eq!(report.pending_strategy_event_actions, 2);
    assert_eq!(report.pending_orders, 0);
    assert_eq!(report.latency_usage.len(), 2);
    assert!(report.latency_usage.iter().all(|usage| {
        usage.class == BacktestLatencyClass::MarketDepth
            && usage.samples == 1
            && usage.minimum_latency_ms == 10
            && usage.maximum_latency_ms == 10
    }));
}

#[test]
fn symbol_latency_rule_overrides_class_rule_in_the_scheduler() {
    let execution = BacktestExecutionConfig {
        market_data_latency_ms: 99,
        latency_profile: BacktestLatencyProfile {
            seed: 17,
            rules: vec![
                BacktestLatencyRule {
                    class: BacktestLatencyClass::MarketDepth,
                    symbol: None,
                    samples_ms: vec![0],
                },
                BacktestLatencyRule {
                    class: BacktestLatencyClass::MarketDepth,
                    symbol: Some("BTC-PERP".to_string()),
                    samples_ms: vec![10],
                },
            ],
        },
        ..BacktestExecutionConfig::default()
    };
    let mut runner = BacktestRunner::with_execution_config(config(), execution).unwrap();

    let report = runner.run(initial_books()).unwrap();

    assert_eq!(report.orders_sent, 0);
    assert_eq!(report.pending_strategy_event_actions, 1);
    assert_eq!(report.latency_usage.len(), 2);
    assert_eq!(
        report
            .latency_usage
            .iter()
            .find(|usage| usage.symbol == "BTC-USDT")
            .unwrap()
            .maximum_latency_ms,
        0
    );
    assert_eq!(
        report
            .latency_usage
            .iter()
            .find(|usage| usage.symbol == "BTC-PERP")
            .unwrap()
            .maximum_latency_ms,
        10
    );
}

#[test]
fn runner_rejects_unknown_symbol_latency_rule() {
    let execution = BacktestExecutionConfig {
        latency_profile: BacktestLatencyProfile {
            seed: 1,
            rules: vec![BacktestLatencyRule {
                class: BacktestLatencyClass::MarketDepth,
                symbol: Some("ETH-USDT".to_string()),
                samples_ms: vec![1],
            }],
        },
        ..BacktestExecutionConfig::default()
    };

    let error = BacktestRunner::with_execution_config(config(), execution)
        .err()
        .unwrap()
        .to_string();

    assert!(error.contains("outside the strategy instrument/reference/index universe"));
    assert!(error.contains("ETH-USDT"));
}

#[test]
fn input_clock_regressions_are_clamped_and_reported() {
    let mut runner = BacktestRunner::new(config()).unwrap();
    let mut events = initial_books();
    events[0] = NormalizedEvent::from(MarketEvent::Depth(OrderBook::one_level(
        "BTC-USDT",
        10,
        Level::new(50_000.0, 2.0),
        Level::new(50_001.0, 2.0),
    )));

    let report = runner.run(events).unwrap();

    assert_eq!(report.input_clock_regressions, 1);
    assert_eq!(report.max_input_clock_regression_ns, 9 * NS_PER_MS);
    assert_eq!(report.last_arrival_ns, Some(10 * NS_PER_MS));
}

#[test]
fn raw_horizon_extends_metric_duration_past_the_last_normalized_event() {
    let mut runner = BacktestRunner::new(config()).unwrap();
    for event in initial_books() {
        runner.process_replay_event(event).unwrap();
    }
    runner
        .process_replay_event(external_spot_fill(2, 50_000.0))
        .unwrap();
    runner.advance_raw_horizon(3 * NS_PER_MS).unwrap();

    let report = runner.finish_report().unwrap();

    assert_eq!(report.first_arrival_ns, Some(NS_PER_MS));
    assert_eq!(report.last_arrival_ns, Some(2 * NS_PER_MS));
    assert_eq!(report.observed_duration_ns, 2 * NS_PER_MS);
    assert_eq!(report.inventory_open_duration_ns, NS_PER_MS);
    assert_eq!(report.inventory_open_fraction, 0.5);
}

#[test]
fn raw_capture_requires_every_strategy_book() {
    let mut runner = BacktestRunner::new(config()).unwrap();
    replay_raw_capture(
        include_str!("../../../../fixtures/raw/okx/depth-gap.jsonl").as_bytes(),
        |event| runner.process_replay_event(event),
    )
    .unwrap();

    let error = runner.require_all_configured_books().unwrap_err();

    assert!(error.to_string().contains("BTC-PERP"));
}
