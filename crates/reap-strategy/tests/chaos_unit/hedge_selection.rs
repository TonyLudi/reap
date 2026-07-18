use std::sync::Arc;

use super::*;

#[test]
fn java_parity_summarizes_global_sell_hedge() {
    let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
    seed_java_calculator_books(&mut strategy);
    strategy.update_best_hedges();

    let targets = strategy.summarize_hedges(
        strategy.hedging.best_hedges.get(&Side::Sell).unwrap(),
        Side::Sell,
        3_000.0,
        None,
    );

    assert_eq!(targets.len(), 1);
    let target = targets
        .iter()
        .find(|target| target.symbol == "BTC-USDT.OK")
        .unwrap();
    assert!(approx_eq(target.orig_px, 45_000.0));
    assert!(approx_eq(target.hedge_px, 44_986.5));
    assert!(approx_eq(target.qty, 0.06));
}

#[test]
fn java_parity_summarizes_inverse_hedges_when_spot_is_blocked() {
    let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
    seed_java_calculator_books(&mut strategy);
    strategy.risk_groups.get_mut("OKEX-Spot").unwrap().delta_usd = -35_000.0;
    strategy.update_best_hedges();

    let targets = strategy.summarize_hedges(
        strategy.hedging.best_hedges.get(&Side::Sell).unwrap(),
        Side::Sell,
        3_000.0,
        None,
    );

    assert_eq!(targets.len(), 2);
    let swap = targets
        .iter()
        .find(|target| target.symbol == "BTC-USD-SWAP.OK")
        .unwrap();
    let future = targets
        .iter()
        .find(|target| target.symbol == "BTC-USD-211231.OK")
        .unwrap();
    assert!(approx_eq(swap.orig_px, 45_000.0));
    assert!(approx_eq(swap.hedge_px, 44_986.5));
    assert!(approx_eq(swap.qty, 20.0));
    assert!(approx_eq(future.orig_px, 45_000.0));
    assert!(approx_eq(future.hedge_px, 44_986.5));
    assert!(approx_eq(future.qty, 7.0));
}

#[test]
fn java_parity_summarizes_linear_hedge_when_other_groups_are_blocked() {
    let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
    seed_java_calculator_books(&mut strategy);
    strategy.risk_groups.get_mut("OKEX-Spot").unwrap().delta_usd = -35_000.0;
    strategy
        .risk_groups
        .get_mut("OKEX-Invert")
        .unwrap()
        .delta_usd = -35_000.0;
    strategy.update_best_hedges();

    let targets = strategy.summarize_hedges(
        strategy.hedging.best_hedges.get(&Side::Sell).unwrap(),
        Side::Sell,
        3_000.0,
        None,
    );

    assert_eq!(targets.len(), 1);
    let target = &targets[0];
    assert_eq!(target.symbol, "BTC-USDT-211231.OK");
    assert!(approx_eq(target.orig_px, 45_000.0));
    assert!(approx_eq(target.hedge_px, 44_986.5));
    assert!(approx_eq(target.qty, 6.0));
}

#[test]
fn java_parity_summarizes_multi_level_inverse_buy_hedge() {
    let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
    seed_java_calculator_books(&mut strategy);
    strategy.update_best_hedges();
    let hedges = strategy
        .risk_groups
        .get("OKEX-Invert")
        .unwrap()
        .best_hedges_for(Side::Buy);

    let targets = strategy.summarize_hedges(hedges, Side::Buy, 9_000.0, None);

    assert_eq!(targets.len(), 2);
    let swap = targets
        .iter()
        .find(|target| target.symbol == "BTC-USD-SWAP.OK")
        .unwrap();
    let future = targets
        .iter()
        .find(|target| target.symbol == "BTC-USD-211231.OK")
        .unwrap();
    assert!(approx_eq(swap.orig_px, 65_000.0));
    assert!(approx_eq(swap.hedge_px, 65_019.5));
    assert!(approx_eq(swap.qty, 60.0));
    assert!(approx_eq(future.orig_px, 65_000.0));
    assert!(approx_eq(future.hedge_px, 65_019.5));
    assert!(approx_eq(future.qty, 46.0));
}

#[test]
fn java_parity_timer_hedges_strategy_delta_without_symbol_exclusion() {
    let mut strategy = ChaosStrategy::new(config()).unwrap();
    strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(50_000.0, 1.0),
        Level::new(50_001.0, 1.0),
    ));
    strategy.entities.get_mut("BTC-PERP").unwrap().book = Some(OrderBook::one_level(
        "BTC-PERP",
        1,
        Level::new(50_000.0, 10_000.0),
        Level::new(50_001.0, 10_000.0),
    ));
    strategy.entities.get_mut("BTC-USDT").unwrap().position_qty = 0.1;

    let intents = strategy.on_event(&StrategyEvent::Timer(reap_core::TimerEvent {
        ts_ms: 2_000,
        name: "risk".to_string(),
    }));

    assert!(
        intents
            .iter()
            .any(|intent| matches!(intent, OrderIntent::NewOrder(order)
        if order.time_in_force == TimeInForce::Ioc && order.reason.starts_with("hedge:timer:")))
    );
}

#[test]
fn java_parity_local_account_hedge_ignores_strategy_interval() {
    let mut cfg = config();
    cfg.min_hedge_interval_ms = 100_000;
    let mut strategy = ChaosStrategy::new(cfg).unwrap();
    strategy.on_depth(&OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(50_000.0, 10.0),
        Level::new(50_001.0, 10.0),
    ));
    strategy.on_depth(&OrderBook::one_level(
        "BTC-PERP",
        1,
        Level::new(50_000.0, 10_000.0),
        Level::new(50_001.0, 10_000.0),
    ));
    strategy.hedging.last_hedge_ms = 10;

    let intents = legacy_intents(strategy.on_account_update(&AccountUpdate {
        ts_ms: 20,
        balances: Vec::new(),
        positions: vec![reap_core::Position {
            symbol: "BTC-USDT".to_string(),
            qty: 0.1,
            avg_price: 50_000.0,
            margin_mode: None,
        }],
        margins: Vec::new(),
    }));

    assert!(intents.iter().any(|intent| matches!(intent, OrderIntent::NewOrder(order)
        if order.symbol == "BTC-PERP" && order.side == Side::Sell && order.time_in_force == TimeInForce::Ioc)));
    assert_eq!(strategy.hedging.last_hedge_ms, 10);
}

#[test]
fn java_parity_excludes_own_quotes_from_hedge_depth() {
    let mut entity = InstrumentState::new(InstrumentConfig {
        symbol: "BTC-USDT".to_string(),
        kind: InstrumentKindConfig::Spot,
        min_trade_size: 0.01,
        ..InstrumentConfig::default()
    });
    entity.book = Some(OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(99.0, 2.0),
        Level::new(101.0, 2.0),
    ));

    let levels = entity.hedge_levels(Side::Buy, 100.0, &[(Side::Sell, 101.0, 1.5)]);

    assert!(approx_eq(levels[0].qty, 0.5));
}

#[test]
fn hedge_candidates_stop_after_covering_notional_target() {
    let mut entity = InstrumentState::new(InstrumentConfig {
        symbol: "BTC-USDT".to_string(),
        kind: InstrumentKindConfig::Spot,
        lot_size: 0.01,
        min_trade_size: 0.01,
        ..InstrumentConfig::default()
    });
    entity.book = Some(OrderBook {
        symbol: "BTC-USDT".to_string(),
        ts_ms: 1,
        bids: (0..100)
            .map(|level| Level::new(99.0 - level as f64 * 0.01, 1.0))
            .collect(),
        asks: (0..100)
            .map(|level| Level::new(101.0 + level as f64 * 0.01, 1.0))
            .collect(),
    });
    let mut candidates = Vec::new();

    entity.append_hedge_candidates(Side::Buy, 100.0, &[], 150.0, &mut candidates);

    assert_eq!(candidates.len(), 2);
    assert!(
        candidates
            .iter()
            .map(|level| level.notional_usd)
            .sum::<f64>()
            >= 150.0
    );
}

#[test]
fn hedge_selection_preserves_rate_order_until_alternative_coverage() {
    let strategy = ChaosStrategy::new(config()).unwrap();
    let candidate = |symbol, level| HedgeCandidate {
        symbol: Arc::from(symbol),
        priority: 0,
        level,
        px: 100.0,
        qty: 1_000.0,
        hedge_rate: 1.0,
        notional_usd: 100_000.0,
        acc_qty: 1_000.0,
    };
    let levels = vec![
        candidate("BTC-USDT", 0),
        candidate("BTC-USDT", 1),
        candidate("BTC-PERP", 0),
    ];

    let selected = strategy.select_required_hedges("main", Side::Buy, &levels);

    assert_eq!(selected.len(), 3);
    assert_eq!(selected[0].symbol, "BTC-USDT");
    assert_eq!(selected[1].symbol, "BTC-USDT");
    assert_eq!(selected[2].symbol, "BTC-PERP");
}

#[test]
fn java_parity_master_strategy_suppresses_automatic_hedges() {
    let mut cfg = config();
    cfg.master_strategy = Some("leader".to_string());
    let mut strategy = ChaosStrategy::new(cfg).unwrap();
    strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(49_900.0, 10.0),
        Level::new(50_100.0, 10.0),
    ));
    strategy.entities.get_mut("BTC-PERP").unwrap().book = Some(OrderBook::one_level(
        "BTC-PERP",
        1,
        Level::new(49_900.0, 10_000.0),
        Level::new(50_100.0, 10_000.0),
    ));

    let account_intents = legacy_intents(strategy.on_account_update(&AccountUpdate {
        ts_ms: 10,
        balances: Vec::new(),
        positions: vec![reap_core::Position {
            symbol: "BTC-USDT".to_string(),
            qty: 0.1,
            avg_price: 50_000.0,
            margin_mode: None,
        }],
        margins: Vec::new(),
    }));
    let timer_intents = strategy.on_event(&StrategyEvent::Timer(reap_core::TimerEvent {
        ts_ms: 11,
        name: "risk".to_string(),
    }));

    assert!(account_intents.iter().chain(&timer_intents).all(|intent| {
        !matches!(intent, OrderIntent::NewOrder(order) if order.time_in_force == TimeInForce::Ioc)
    }));
}
