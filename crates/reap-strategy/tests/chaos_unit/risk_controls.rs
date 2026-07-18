use super::*;

#[test]
fn java_parity_halts_derivative_during_utc_interval() {
    let mut cfg = config();
    cfg.instruments
        .iter_mut()
        .find(|instrument| instrument.symbol == "BTC-PERP")
        .unwrap()
        .halt_intervals = vec![HaltIntervalConfig {
        start_sec_utc: 10,
        end_sec_utc: 20,
    }];
    let mut strategy = ChaosStrategy::new(cfg).unwrap();
    strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(50_000.0, 10.0),
        Level::new(50_001.0, 10.0),
    ));
    strategy.entities.get_mut("BTC-PERP").unwrap().book = Some(OrderBook::one_level(
        "BTC-PERP",
        1,
        Level::new(50_000.0, 10_000.0),
        Level::new(50_001.0, 10_000.0),
    ));

    strategy.now_ms = 15_000;
    let halted = legacy_intents(strategy.refresh_quotes());
    assert!(strategy.entity("BTC-PERP").unwrap().interval_halted);
    assert!(
        halted
            .iter()
            .all(|intent| !matches!(intent, OrderIntent::NewOrder(_)))
    );

    strategy.now_ms = 21_000;
    let resumed = legacy_intents(strategy.refresh_quotes());
    assert!(!strategy.entity("BTC-PERP").unwrap().interval_halted);
    assert!(
        resumed
            .iter()
            .any(|intent| matches!(intent, OrderIntent::NewOrder(_)))
    );
}

#[test]
fn java_parity_account_balances_drive_spot_group_delta() {
    let mut cfg = config();
    cfg.coin_offset = 30.0;
    cfg.risk_groups[0].coin_offset = 30.0;
    cfg.risk_groups[0].kind = RiskGroupKindConfig::PortfolioAccount;
    cfg.risk_groups[0].coins = vec![
        CoinConfig {
            currency: "BTC".to_string(),
            min_balance: 20.0,
            max_balance: 40.0,
            borrow_limit_usd: 50_000.0,
            borrow_limit_coin: 1.0,
            ..CoinConfig::default()
        },
        CoinConfig {
            currency: "USDT".to_string(),
            min_balance: 0.0,
            max_balance: 2_000_000.0,
            ..CoinConfig::default()
        },
    ];
    let spot = cfg
        .instruments
        .iter_mut()
        .find(|instrument| instrument.symbol == "BTC-USDT")
        .unwrap();
    spot.base_currency = "BTC".to_string();
    spot.quote_currency = "USDT".to_string();
    let mut strategy = ChaosStrategy::new(cfg).unwrap();
    strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(49_999.0, 1.0),
        Level::new(50_001.0, 1.0),
    ));

    strategy.on_account_update(&AccountUpdate {
        ts_ms: 2,
        balances: vec![
            Balance {
                account_id: None,
                currency: "BTC".to_string(),
                total: 31.0,
                available: 30.5,
                equity: 31.0,
                liability: 0.5,
                max_loan: 1.0,
                forced_repayment_indicator: None,
            },
            Balance {
                account_id: None,
                currency: "USDT".to_string(),
                total: 1_000_000.0,
                available: 1_000_000.0,
                equity: 1_000_000.0,
                liability: 0.0,
                max_loan: 0.0,
                forced_repayment_indicator: None,
            },
        ],
        positions: Vec::new(),
        margins: Vec::new(),
    });

    let spot = strategy.entity("BTC-USDT").unwrap();
    assert!(approx_eq(spot.base_balance, 31.0));
    assert!(approx_eq(spot.base_liability, 0.5));
    assert!(approx_eq(strategy.delta_usd(), 50_000.0));
}

#[test]
fn java_parity_latches_delta_limit_breach_and_stops_new_quotes() {
    let mut cfg = config();
    cfg.delta_limit_usd = 10_000.0;
    let mut strategy = ChaosStrategy::new(cfg).unwrap();
    let spot = strategy.entities.get_mut("BTC-USDT").unwrap();
    spot.book = Some(OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(49_999.0, 10.0),
        Level::new(50_001.0, 10.0),
    ));
    spot.position_qty = 1.0;
    strategy.entities.get_mut("BTC-PERP").unwrap().book = Some(OrderBook::one_level(
        "BTC-PERP",
        1,
        Level::new(49_999.0, 10_000.0),
        Level::new(50_001.0, 10_000.0),
    ));

    let intents = legacy_intents(strategy.refresh_quotes());

    assert!(strategy.halt_reason().unwrap().contains("strategy delta"));
    assert!(
        intents
            .iter()
            .all(|intent| !matches!(intent, OrderIntent::NewOrder(_)))
    );
}

#[test]
fn java_parity_latches_trading_pnl_breach() {
    let mut cfg = config();
    cfg.pnl_limit_usd = 10.0;
    cfg.pnl_breach_debounce_ms = 0;
    let mut strategy = ChaosStrategy::new(cfg).unwrap();
    strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(79.0, 10.0),
        Level::new(81.0, 10.0),
    ));
    strategy.entities.get_mut("BTC-PERP").unwrap().book = Some(OrderBook::one_level(
        "BTC-PERP",
        1,
        Level::new(79.0, 10_000.0),
        Level::new(81.0, 10_000.0),
    ));
    strategy.on_order_update(&OrderUpdate {
        ts_ms: 2,
        order_id: "q1".to_string(),
        symbol: "BTC-USDT".to_string(),
        side: Side::Buy,
        event: OrderEvent::FullyFilled,
        status: OrderStatus::Filled,
        price: 100.0,
        time_in_force: Some(TimeInForce::PostOnly),
        qty: 1.0,
        open_qty: 0.0,
        filled_qty: 1.0,
        avg_fill_price: 100.0,
        last_fill_qty: 1.0,
        last_fill_price: 100.0,
        last_fill_liquidity: None,
        last_fill_fee: None,
        reason: "quote".to_string(),
    });

    let intents = legacy_intents(strategy.refresh_quotes());

    assert!(approx_eq(strategy.trading_pnl_usd(), -20.02));
    assert!(strategy.halt_reason().unwrap().contains("trading pnl"));
    assert!(
        intents
            .iter()
            .all(|intent| !matches!(intent, OrderIntent::NewOrder(_)))
    );
}

#[test]
fn java_parity_debounces_margin_ratio_breach() {
    let mut cfg = config();
    cfg.margin_breach_debounce_ms = 100;
    cfg.risk_groups[0].min_margin_level = 0.3;
    let mut strategy = ChaosStrategy::new(cfg).unwrap();
    strategy.on_account_update(&AccountUpdate {
        ts_ms: 10,
        balances: Vec::new(),
        positions: Vec::new(),
        margins: vec![MarginSnapshot {
            account_id: None,
            ratio: Some(0.4),
            exchange_ratio: None,
            adjusted_equity_usd: Some(40_000.0),
            notional_usd: Some(100_000.0),
        }],
    });
    strategy.on_account_update(&AccountUpdate {
        ts_ms: 11,
        balances: Vec::new(),
        positions: Vec::new(),
        margins: vec![MarginSnapshot {
            account_id: None,
            ratio: Some(0.2),
            exchange_ratio: None,
            adjusted_equity_usd: Some(20_000.0),
            notional_usd: Some(100_000.0),
        }],
    });
    assert!(strategy.halt_reason().is_none());

    strategy.on_event(&StrategyEvent::Timer(reap_core::TimerEvent {
        ts_ms: 110,
        name: "risk".to_string(),
    }));
    assert!(strategy.halt_reason().is_none());
    strategy.on_event(&StrategyEvent::Timer(reap_core::TimerEvent {
        ts_ms: 111,
        name: "risk".to_string(),
    }));

    assert!(strategy.halt_reason().unwrap().contains("margin ratio"));
}

#[test]
fn zero_notional_account_does_not_create_infinite_margin_breach() {
    let mut strategy = ChaosStrategy::new(config()).unwrap();
    strategy.on_account_update(&AccountUpdate {
        ts_ms: 10,
        balances: Vec::new(),
        positions: Vec::new(),
        margins: vec![MarginSnapshot {
            account_id: None,
            ratio: None,
            exchange_ratio: None,
            adjusted_equity_usd: Some(10_000.0),
            notional_usd: Some(0.0),
        }],
    });
    strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
        "BTC-USDT",
        10,
        Level::new(99.0, 10.0),
        Level::new(101.0, 10.0),
    ));
    strategy.entities.get_mut("BTC-PERP").unwrap().book = Some(OrderBook::one_level(
        "BTC-PERP",
        10,
        Level::new(99.0, 10_000.0),
        Level::new(101.0, 10_000.0),
    ));

    strategy.refresh_quotes();

    assert!(strategy.halt_reason().is_none());
    assert!(strategy.risk_groups["main"].margin_ratio.is_none());
}

#[test]
fn java_parity_reduces_spot_quote_levels_until_balance_recovers() {
    let mut entity = InstrumentState::new(InstrumentConfig {
        symbol: "BTC-USDT".to_string(),
        kind: InstrumentKindConfig::Spot,
        base_currency: "BTC".to_string(),
        quote_currency: "USDT".to_string(),
        num_quote_levels: 3,
        max_order_size: 1.0,
        min_trade_size: 0.01,
        ..InstrumentConfig::default()
    });
    entity.book = Some(OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(99.0, 10.0),
        Level::new(101.0, 10.0),
    ));
    entity.trade.base_coin_config = Some(CoinConfig {
        currency: "BTC".to_string(),
        min_balance: 0.0,
        max_balance: 100.0,
        safety_multiplier: 1.0,
        ..CoinConfig::default()
    });
    entity.trade.quote_coin_config = Some(CoinConfig {
        currency: "USDT".to_string(),
        min_balance: 0.0,
        max_balance: 10_000.0,
        safety_multiplier: 1.0,
        ..CoinConfig::default()
    });
    entity.base_balance = 10.0;
    entity.base_equity = 10.0;
    entity.quote_balance = 150.0;
    entity.quote_equity = 150.0;
    entity.balances_initialized = true;

    entity.refresh_trade_permissions(100);
    assert!(entity.can_quote(Side::Buy));
    assert_eq!(entity.quote_level_count(Side::Buy), 1);

    entity.quote_balance = 400.0;
    entity.quote_equity = 400.0;
    entity.refresh_trade_permissions(200);
    assert_eq!(entity.quote_level_count(Side::Buy), 1);
    entity.quote_balance = 1_000.0;
    entity.quote_equity = 1_000.0;
    entity.refresh_trade_permissions(10_200);
    assert_eq!(entity.quote_level_count(Side::Buy), 1);
    entity.refresh_trade_permissions(10_201);
    assert_eq!(entity.quote_level_count(Side::Buy), 3);
}

#[test]
fn java_parity_debounces_trade_permission_recovery() {
    let mut entity = InstrumentState::new(InstrumentConfig {
        symbol: "BTC-USDT-SWAP".to_string(),
        kind: InstrumentKindConfig::LinearSwap,
        max_order_size: 1.0,
        min_trade_size: 1.0,
        min_position: -100.0,
        max_position: 100.0,
        safety_multiplier: 1.0,
        ..InstrumentConfig::default()
    });
    entity.book = Some(OrderBook::one_level(
        "BTC-USDT-SWAP",
        1,
        Level::new(99.0, 10.0),
        Level::new(101.0, 10.0),
    ));
    entity.position_qty = 99.0;
    entity.refresh_trade_permissions(100);
    assert!(!entity.trade.can_trade[&Side::Buy]);

    entity.position_qty = 0.0;
    entity.refresh_trade_permissions(600);
    assert!(!entity.trade.can_trade[&Side::Buy]);
    entity.refresh_trade_permissions(601);
    assert!(entity.trade.can_trade[&Side::Buy]);
}

#[test]
fn java_parity_startup_basis_uses_one_third_limit() {
    let mut strategy = ChaosStrategy::new(config()).unwrap();
    strategy.on_depth(&OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(99.0, 10.0),
        Level::new(101.0, 10.0),
    ));
    strategy.on_depth(&OrderBook::one_level(
        "BTC-PERP",
        1,
        Level::new(110.0, 10_000.0),
        Level::new(112.0, 10_000.0),
    ));

    assert_eq!(strategy.halt_reason(), Some("startup basis limit breached"));
}

#[test]
fn java_parity_runtime_basis_breach_is_diagnostic_only() {
    let mut strategy = ChaosStrategy::new(config()).unwrap();
    strategy.on_depth(&OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(99.0, 10.0),
        Level::new(101.0, 10.0),
    ));
    strategy.on_depth(&OrderBook::one_level(
        "BTC-PERP",
        1,
        Level::new(100.0, 10_000.0),
        Level::new(102.0, 10_000.0),
    ));
    assert!(strategy.halt_reason().is_none());

    strategy.on_depth(&OrderBook::one_level(
        "BTC-PERP",
        6_002,
        Level::new(110.0, 10_000.0),
        Level::new(112.0, 10_000.0),
    ));

    assert!(strategy.halt_reason().is_none());
    assert!(strategy.basis_breaches().contains_key("main"));
}

#[test]
fn java_parity_derivative_size_is_limited_by_margin_capacity() {
    let mut entity = InstrumentState::new(InstrumentConfig {
        symbol: "BTC-USDT-SWAP".to_string(),
        kind: InstrumentKindConfig::LinearSwap,
        contract_value: 0.01,
        max_order_size: 100.0,
        safety_multiplier: 1.0,
        min_position: -1_000.0,
        max_position: 1_000.0,
        ..InstrumentConfig::default()
    });
    entity.book = Some(OrderBook::one_level(
        "BTC-USDT-SWAP",
        1,
        Level::new(49_999.0, 1_000.0),
        Level::new(50_001.0, 1_000.0),
    ));
    entity.margin_initialized = true;
    entity.margin_balance = 30_000.0;
    entity.trade.margin_coin_config = Some(CoinConfig {
        currency: "USDT".to_string(),
        ..CoinConfig::default()
    });

    assert!(approx_eq(entity.max_trade_size(Side::Buy, false), 20.0));
    entity.position_qty = -50.0;
    assert!(approx_eq(entity.max_trade_size(Side::Buy, false), 100.0));
}

#[test]
fn java_parity_checks_exchange_margin_ratio_separately() {
    let mut cfg = config();
    cfg.margin_breach_debounce_ms = 100;
    let mut strategy = ChaosStrategy::new(cfg).unwrap();
    strategy.on_account_update(&AccountUpdate {
        ts_ms: 10,
        balances: Vec::new(),
        positions: Vec::new(),
        margins: vec![MarginSnapshot {
            account_id: None,
            ratio: None,
            exchange_ratio: Some(10.0),
            adjusted_equity_usd: None,
            notional_usd: None,
        }],
    });
    strategy.on_account_update(&AccountUpdate {
        ts_ms: 11,
        balances: Vec::new(),
        positions: Vec::new(),
        margins: vec![MarginSnapshot {
            account_id: None,
            ratio: None,
            exchange_ratio: Some(4.0),
            adjusted_equity_usd: None,
            notional_usd: None,
        }],
    });
    assert!(strategy.halt_reason().is_none());

    strategy.on_event(&StrategyEvent::Timer(reap_core::TimerEvent {
        ts_ms: 112,
        name: "risk".to_string(),
    }));

    assert!(
        strategy
            .halt_reason()
            .unwrap()
            .contains("exchange margin ratio")
    );
}

#[test]
fn java_parity_stops_on_zombie_hedge_order() {
    let mut strategy = ChaosStrategy::new(config()).unwrap();
    strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(99.0, 10.0),
        Level::new(101.0, 10.0),
    ));
    strategy.entities.get_mut("BTC-PERP").unwrap().book = Some(OrderBook::one_level(
        "BTC-PERP",
        1,
        Level::new(99.0, 10_000.0),
        Level::new(101.0, 10_000.0),
    ));
    strategy.on_order_update(&OrderUpdate {
        ts_ms: 1,
        order_id: "stuck-hedge".to_string(),
        symbol: "BTC-PERP".to_string(),
        side: Side::Sell,
        event: OrderEvent::New,
        status: OrderStatus::Live,
        price: 100.0,
        time_in_force: Some(TimeInForce::Ioc),
        qty: 1.0,
        open_qty: 1.0,
        filled_qty: 0.0,
        avg_fill_price: 0.0,
        last_fill_qty: 0.0,
        last_fill_price: 0.0,
        last_fill_liquidity: None,
        last_fill_fee: None,
        reason: "hedge:BTC-USDT:100".to_string(),
    });

    strategy.on_event(&StrategyEvent::Timer(reap_core::TimerEvent {
        ts_ms: 30_001,
        name: "risk".to_string(),
    }));
    assert!(strategy.halt_reason().is_none());
    strategy.on_event(&StrategyEvent::Timer(reap_core::TimerEvent {
        ts_ms: 30_002,
        name: "risk".to_string(),
    }));

    assert!(strategy.halt_reason().unwrap().contains("stuck-hedge"));
}

#[test]
fn risk_checks_fail_closed_on_non_finite_state() {
    let mut strategy = ChaosStrategy::new(config()).unwrap();
    strategy.risk_groups.get_mut("main").unwrap().delta_usd = f64::NAN;

    assert!(!strategy.check_risk_limits());
    assert!(strategy.halt_reason().unwrap().contains("non-finite"));
}
