use super::*;

#[test]
fn order_remains_fillable_until_delayed_cancel_is_effective() {
    let execution = BacktestExecutionConfig {
        cancel_latency_ms: 10,
        ..BacktestExecutionConfig::default()
    };
    let mut runner = BacktestRunner::with_execution_config(config(), execution).unwrap();
    runner.now_ns = NS_PER_MS;
    runner.matcher_mut("BTC-USDT").unwrap().on_depth_at(
        OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(100.0, 1.0),
            Level::new(101.0, 1.0),
        ),
        1,
    );
    seed_perp_matcher(&mut runner, 1);
    runner
        .accept_intents(vec![OrderIntent::NewOrder(NewOrder {
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            qty: 0.5,
            price: 100.0,
            time_in_force: TimeInForce::PostOnly,
            reduce_only: false,
            self_trade_prevention: None,
            reason: "manual_test".to_string(),
        })])
        .unwrap();
    runner.drain_through(NS_PER_MS).unwrap();
    runner
        .accept_intents(vec![OrderIntent::CancelOrder {
            order_id: "BTC-USDT-1".to_string(),
            reason: "manual_cancel".to_string(),
        }])
        .unwrap();

    runner
        .process_replay_event_at(
            NormalizedEvent::from(MarketEvent::Trade {
                ts_ms: 5,
                symbol: "BTC-USDT".to_string(),
                price: 100.0,
                qty: 1.5,
                taker_side: Side::Sell,
            }),
            5 * NS_PER_MS,
        )
        .unwrap();
    runner
        .process_replay_event_at(
            NormalizedEvent::Timer(TimerEvent {
                ts_ms: 11,
                name: "advance".to_string(),
            }),
            11 * NS_PER_MS,
        )
        .unwrap();
    let report = runner.finish_report().unwrap();

    assert_eq!(report.fills, 1);
    assert_eq!(report.maker_fills, 1);
    assert_eq!(report.cancel_requests, 1);
    assert_eq!(report.cancelled_orders, 0);
    assert_eq!(report.pending_cancel_requests, 0);
}

#[test]
fn nanosecond_arrival_clock_preserves_cancel_before_next_market_event() {
    let execution = BacktestExecutionConfig {
        cancel_latency_ms: 1,
        ..BacktestExecutionConfig::default()
    };
    let mut runner = BacktestRunner::with_execution_config(config(), execution).unwrap();
    runner.now_ns = 100_100_000;
    runner.matcher_mut("BTC-USDT").unwrap().on_depth_at(
        OrderBook::one_level(
            "BTC-USDT",
            100,
            Level::new(100.0, 1.0),
            Level::new(101.0, 1.0),
        ),
        100,
    );
    seed_perp_matcher(&mut runner, 100);
    runner
        .accept_intents(vec![OrderIntent::NewOrder(NewOrder {
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            qty: 0.5,
            price: 100.0,
            time_in_force: TimeInForce::PostOnly,
            reduce_only: false,
            self_trade_prevention: None,
            reason: "nanosecond_test".to_string(),
        })])
        .unwrap();
    runner.drain_through(100_100_000).unwrap();
    runner
        .accept_intents(vec![OrderIntent::CancelOrder {
            order_id: "BTC-USDT-1".to_string(),
            reason: "cancel_before_trade".to_string(),
        }])
        .unwrap();

    runner
        .process_replay_event_at(
            NormalizedEvent::from(MarketEvent::Trade {
                ts_ms: 101,
                symbol: "BTC-USDT".to_string(),
                price: 100.0,
                qty: 1.5,
                taker_side: Side::Sell,
            }),
            101_200_000,
        )
        .unwrap();
    let report = runner.finish_report().unwrap();

    assert_eq!(report.cancelled_orders, 1);
    assert_eq!(report.fills, 0);
    assert_eq!(report.last_arrival_ns, Some(101_200_000));
}
