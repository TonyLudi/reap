use super::*;
use reap_core::{AccountUpdate, Position, StrategyEvent};
use reap_strategy::{ChaosExecutionIntent, ChaosExecutionPurpose};

fn one_typed_hedge(
    runner: &mut BacktestRunner,
    seed_matchers: bool,
) -> (Vec<ChaosExecutionIntent>, String) {
    let books = [
        OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(50_000.0, 10.0),
            Level::new(50_001.0, 10.0),
        ),
        OrderBook::one_level(
            "BTC-PERP",
            1,
            Level::new(50_003.0, 10_000.0),
            Level::new(50_004.0, 10_000.0),
        ),
    ];
    for book in books {
        if seed_matchers {
            runner
                .matcher_mut(&book.symbol)
                .unwrap()
                .on_depth_at(book.clone(), 1);
        }
        let ignored_quotes = runner
            .strategy
            .on_execution_event(&StrategyEvent::Market(MarketEvent::Depth(book)));
        drop(ignored_quotes);
    }
    runner.replay.now_ns = 7 * NS_PER_MS;
    let hedge = runner
        .strategy
        .on_execution_event(&StrategyEvent::Account(AccountUpdate {
            ts_ms: 7,
            balances: Vec::new(),
            positions: vec![Position {
                symbol: "BTC-USDT".to_string(),
                qty: 0.1,
                avg_price: 50_000.0,
                margin_mode: None,
            }],
            margins: Vec::new(),
        }))
        .into_iter()
        .find(|intent| intent.purpose() == ChaosExecutionPurpose::Hedge)
        .expect("seeded positive spot delta must produce one typed hedge");
    let symbol = hedge
        .as_hedge()
        .expect("hedge purpose must carry a typed hedge")
        .symbol()
        .to_string();
    (vec![hedge], symbol)
}

fn instrument_debug(runner: &BacktestRunner, symbol: &str) -> String {
    format!(
        "{:?}",
        runner
            .strategy
            .entity(symbol)
            .expect("hedge symbol must remain configured")
    )
}

#[test]
fn order_remains_fillable_until_delayed_cancel_is_effective() {
    let execution = BacktestExecutionConfig {
        cancel_latency_ms: 10,
        ..BacktestExecutionConfig::default()
    };
    let mut runner = BacktestRunner::with_execution_config(config(), execution).unwrap();
    runner.replay.now_ns = NS_PER_MS;
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
    runner.replay.now_ns = 100_100_000;
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

#[test]
fn accepted_typed_hedge_commits_only_after_local_matcher_reservation() {
    let mut runner = BacktestRunner::new(config()).unwrap();
    let (intents, hedge_symbol) = one_typed_hedge(&mut runner, true);
    let before = instrument_debug(&runner, &hedge_symbol);

    runner.accept_chaos_intents(intents).unwrap();

    let after = instrument_debug(&runner, &hedge_symbol);
    assert_ne!(
        after, before,
        "local matcher acceptance must commit the hedge's implied-depth transition"
    );
    assert!(
        after.contains("updated_ms: 7"),
        "the committed hedge must use local replay time"
    );
    assert_eq!(runner.orders.orders_sent, 1);
    assert_eq!(
        runner
            .matcher_mut(&hedge_symbol)
            .unwrap()
            .pending_order_count(),
        1
    );
}

#[test]
fn not_ready_typed_hedge_drops_commit_without_mutating_strategy_state() {
    let mut runner = BacktestRunner::new(config()).unwrap();
    let (intents, hedge_symbol) = one_typed_hedge(&mut runner, false);
    let before = instrument_debug(&runner, &hedge_symbol);

    runner.accept_chaos_intents(intents).unwrap();

    assert_eq!(instrument_debug(&runner, &hedge_symbol), before);
    assert_eq!(runner.orders.new_orders_blocked_not_ready, 1);
    assert_eq!(runner.orders.orders_sent, 0);
}

#[test]
fn missing_matcher_rejection_drops_commit_without_mutating_strategy_state() {
    let mut runner = BacktestRunner::new(config()).unwrap();
    let (intents, hedge_symbol) = one_typed_hedge(&mut runner, true);
    let before = instrument_debug(&runner, &hedge_symbol);
    runner
        .orders
        .matchers
        .remove(&hedge_symbol)
        .expect("test must remove the hedge route");

    let error = runner
        .accept_chaos_intents(intents)
        .expect_err("missing matcher must reject before local reservation");

    assert!(
        error
            .to_string()
            .contains(&format!("no matcher configured for symbol {hedge_symbol}"))
    );
    assert_eq!(instrument_debug(&runner, &hedge_symbol), before);
}
