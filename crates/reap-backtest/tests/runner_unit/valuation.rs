use super::*;

#[test]
fn runner_requires_explicit_rates_for_non_usd_accounting_currencies() {
    let error = BacktestRunner::new(usdt_config())
        .err()
        .unwrap()
        .to_string();

    assert!(error.contains("lacks direct USD valuation routes"));
    assert!(error.contains("USDT"));
}

#[test]
fn delivered_currency_index_values_portfolio_and_report_evidence() {
    let mut runner =
        BacktestRunner::with_execution_config(usdt_config(), usdt_execution(0, 1_000)).unwrap();
    runner
        .valuation
        .depth_marks
        .insert("BTC-USDT".to_string(), 110.0);
    let events = vec![
        NormalizedEvent::Market(MarketEvent::IndexPrice {
            ts_ms: 1,
            symbol: "USDT-USD".to_string(),
            price: 0.95,
        }),
        external_spot_fill(2, 100.0),
        NormalizedEvent::Timer(TimerEvent {
            ts_ms: 3,
            name: "finish".to_string(),
        }),
    ];

    let report = runner.run(events).unwrap();

    assert!((report.final_equity_usd - 9.5).abs() < 1e-12);
    assert!((report.cash_usd + 95.0).abs() < 1e-12);
    assert_eq!(report.cash_by_currency.get("USDT"), Some(&-100.0));
    assert_eq!(report.currency_rate_events, 1);
    assert_eq!(report.currency_conversion_failures, 0);
    assert!(report.currency_rate_coverage_complete);
    assert!(report.missing_currency_rates.is_empty());
    assert_eq!(report.currency_rates.len(), 1);
    assert_eq!(report.currency_rates[0].usd_per_unit, Some(0.95));
    assert_eq!(report.currency_rates[0].source_ts_ms, Some(1));
    assert_eq!(report.currency_rates[0].effective_at_ns, Some(NS_PER_MS));
    assert_eq!(report.currency_rates[0].age_ms, Some(2));
    assert!(report.currency_rates[0].usable);
    assert!(report.final_valuation_complete);
    assert!(report.accounting_complete);
}

#[test]
fn order_entry_waits_for_books_and_fresh_accounting_rates() {
    let mut runner =
        BacktestRunner::with_execution_config(usdt_config(), usdt_execution(0, 1_000)).unwrap();
    let mut events = initial_books();
    events.push(NormalizedEvent::Market(MarketEvent::IndexPrice {
        ts_ms: 2,
        symbol: "USDT-USD".to_string(),
        price: 1.0,
    }));
    events.push(NormalizedEvent::from(MarketEvent::Depth(
        OrderBook::one_level(
            "BTC-USDT",
            3,
            Level::new(50_001.0, 2.0),
            Level::new(50_002.0, 2.0),
        ),
    )));

    let report = runner.run(events).unwrap();

    assert!(report.new_orders_blocked_not_ready > 0);
    assert_eq!(report.order_entry_ready_at_ns, Some(2 * NS_PER_MS));
    assert!(report.order_entry_ready_at_end);
    assert!(report.orders_sent > 0);
    assert_eq!(report.invalid_risk_metric_samples, 0);
    assert!(report.accounting_complete);
}

#[test]
fn configured_opening_portfolio_reports_true_net_pnl_and_strategy_balances() {
    let initial = BacktestInitialPortfolioConfig {
        balances: vec![
            BacktestInitialBalanceConfig {
                currency: "BTC".to_string(),
                total: 0.002,
                valuation_symbol: Some("BTC-USDT".to_string()),
                ..Default::default()
            },
            BacktestInitialBalanceConfig {
                currency: "USDT".to_string(),
                total: 1_000.0,
                valuation_symbol: None,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let mut runner = BacktestRunner::with_initial_portfolio_config(
        usdt_config(),
        usdt_execution(0, 1_000),
        initial.clone(),
    )
    .unwrap();
    let mut events = initial_books();
    events.push(NormalizedEvent::Market(MarketEvent::IndexPrice {
        ts_ms: 2,
        symbol: "USDT-USD".to_string(),
        price: 1.0,
    }));
    events.push(NormalizedEvent::from(MarketEvent::Depth(
        OrderBook::one_level(
            "BTC-USDT",
            3,
            Level::new(50_000.0, 2.0),
            Level::new(50_001.0, 2.0),
        ),
    )));

    let report = runner.run(events).unwrap();

    let expected_opening = 1_000.0 + 0.002 * 50_000.5;
    assert_eq!(report.initial_portfolio, initial);
    assert_eq!(report.opening_valuation_at_ns, Some(2 * NS_PER_MS));
    assert!(report.opening_valuation_complete);
    assert!((report.opening_equity_usd.unwrap() - expected_opening).abs() < 1e-9);
    assert!((report.final_equity_usd - expected_opening).abs() < 1e-9);
    assert!(report.net_pnl_usd.unwrap().abs() < 1e-9);
    assert_eq!(report.account_balances.get("BTC"), Some(&0.002));
    assert_eq!(report.account_balances.get("USDT"), Some(&1_000.0));
    assert_eq!(report.positions.get("BTC-USDT"), Some(&0.002));
    assert!(report.orders_sent > 0);
    assert!(report.accounting_complete);
}

#[test]
fn periodic_account_refreshes_do_not_bypass_pending_fill_latency() {
    let mut strategy = usdt_config();
    for instrument in &mut strategy.instruments {
        instrument.quote_profit_margin = 1.0;
        instrument.halted = true;
    }
    let initial = BacktestInitialPortfolioConfig {
        balances: vec![
            BacktestInitialBalanceConfig {
                currency: "BTC".to_string(),
                total: 0.0,
                valuation_symbol: Some("BTC-USDT".to_string()),
                ..Default::default()
            },
            BacktestInitialBalanceConfig {
                currency: "USDT".to_string(),
                total: 100_000.0,
                ..Default::default()
            },
        ],
        positions: vec![BacktestInitialPositionConfig {
            symbol: "BTC-PERP".to_string(),
            qty: 0.0,
            avg_price: 0.0,
            margin_mode: Some(reap_core::PositionMarginMode::Cross),
        }],
        ..Default::default()
    };
    let mut execution = usdt_execution(0, 30_000);
    let mut refresh_runner = BacktestRunner::with_initial_portfolio_config(
        strategy.clone(),
        execution.clone(),
        initial.clone(),
    )
    .unwrap();
    let mut refresh_events = initial_books();
    refresh_events.push(NormalizedEvent::Market(MarketEvent::IndexPrice {
        ts_ms: 2,
        symbol: "USDT-USD".to_string(),
        price: 1.0,
    }));
    refresh_events.push(NormalizedEvent::Timer(TimerEvent {
        ts_ms: 10_002,
        name: "refresh".to_string(),
    }));
    let refreshed = refresh_runner.run(refresh_events).unwrap();
    assert_eq!(refreshed.periodic_account_refreshes, 1);

    execution.fill_account_latency_ms = 20_000;
    let mut delayed_runner =
        BacktestRunner::with_initial_portfolio_config(strategy, execution, initial).unwrap();
    let mut delayed_events = initial_books();
    delayed_events.push(NormalizedEvent::Market(MarketEvent::IndexPrice {
        ts_ms: 2,
        symbol: "USDT-USD".to_string(),
        price: 1.0,
    }));
    delayed_events.push(external_spot_fill(3, 50_000.0));
    delayed_events.push(NormalizedEvent::Timer(TimerEvent {
        ts_ms: 12_000,
        name: "refresh".to_string(),
    }));
    let delayed = delayed_runner.run(delayed_events).unwrap();
    assert_eq!(delayed.periodic_account_refreshes, 0);
    assert!(delayed.pending_strategy_event_actions >= 2);
}

#[test]
fn configured_opening_portfolio_keeps_order_entry_blocked_without_valuation() {
    let initial = BacktestInitialPortfolioConfig {
        balances: vec![
            BacktestInitialBalanceConfig {
                currency: "BTC".to_string(),
                total: 0.01,
                valuation_symbol: Some("BTC-USDT".to_string()),
                ..Default::default()
            },
            BacktestInitialBalanceConfig {
                currency: "USDT".to_string(),
                total: 1_000.0,
                valuation_symbol: None,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let mut runner = BacktestRunner::with_initial_portfolio_config(
        usdt_config(),
        usdt_execution(0, 1_000),
        initial,
    )
    .unwrap();

    let report = runner.run(initial_books()).unwrap();

    assert_eq!(report.opening_equity_usd, None);
    assert_eq!(report.net_pnl_usd, None);
    assert!(!report.opening_valuation_complete);
    assert!(!report.order_entry_ready_at_end);
    assert_eq!(report.orders_sent, 0);
    assert!(!report.accounting_complete);
}

#[test]
fn stale_currency_index_makes_final_accounting_incomplete() {
    let mut runner =
        BacktestRunner::with_execution_config(usdt_config(), usdt_execution(0, 1)).unwrap();
    runner
        .valuation
        .depth_marks
        .insert("BTC-USDT".to_string(), 110.0);
    let events = vec![
        NormalizedEvent::Market(MarketEvent::IndexPrice {
            ts_ms: 1,
            symbol: "USDT-USD".to_string(),
            price: 0.95,
        }),
        external_spot_fill(2, 100.0),
        NormalizedEvent::Timer(TimerEvent {
            ts_ms: 4,
            name: "stale".to_string(),
        }),
    ];

    let report = runner.run(events).unwrap();

    assert!(!report.currency_rate_coverage_complete);
    assert_eq!(report.missing_currency_rates, vec!["USDT".to_string()]);
    assert!((report.final_equity_usd - 9.5).abs() < 1e-12);
    assert!((report.final_gross_exposure_usd - 104.5).abs() < 1e-12);
    assert!(!report.final_valuation_complete);
    assert!(!report.accounting_complete);
    assert!(report.invalid_risk_metric_samples > 0);
}

#[test]
fn fill_before_delayed_currency_index_records_conversion_failure() {
    let mut runner =
        BacktestRunner::with_execution_config(usdt_config(), usdt_execution(10, 1_000)).unwrap();
    runner
        .valuation
        .depth_marks
        .insert("BTC-USDT".to_string(), 110.0);
    let events = vec![
        NormalizedEvent::Market(MarketEvent::IndexPrice {
            ts_ms: 1,
            symbol: "USDT-USD".to_string(),
            price: 0.95,
        }),
        external_spot_fill(2, 100.0),
        NormalizedEvent::Timer(TimerEvent {
            ts_ms: 11,
            name: "deliver-reference".to_string(),
        }),
    ];

    let report = runner.run(events).unwrap();

    assert_eq!(report.currency_rate_events, 1);
    assert!(report.currency_rate_coverage_complete);
    assert_eq!(report.currency_conversion_failures, 1);
    assert_eq!(report.currency_rates[0].source_ts_ms, Some(1));
    assert_eq!(
        report.currency_rates[0].effective_at_ns,
        Some(11 * NS_PER_MS)
    );
    assert_eq!(report.currency_rates[0].age_ms, Some(10));
    assert_eq!(report.turnover_usd, 100.0);
    assert!(!report.accounting_complete);
}

#[test]
fn source_age_can_make_a_currency_index_stale_at_delivery() {
    let mut runner =
        BacktestRunner::with_execution_config(usdt_config(), usdt_execution(10, 5)).unwrap();
    let events = vec![
        NormalizedEvent::Market(MarketEvent::IndexPrice {
            ts_ms: 1,
            symbol: "USDT-USD".to_string(),
            price: 0.95,
        }),
        NormalizedEvent::Timer(TimerEvent {
            ts_ms: 11,
            name: "deliver-stale-reference".to_string(),
        }),
    ];

    let report = runner.run(events).unwrap();

    assert_eq!(report.currency_rate_events, 1);
    assert_eq!(
        report.currency_rates[0].effective_at_ns,
        Some(11 * NS_PER_MS)
    );
    assert_eq!(report.currency_rates[0].age_ms, Some(10));
    assert!(!report.currency_rates[0].usable);
    assert!(!report.currency_rate_coverage_complete);
    assert!(!report.accounting_complete);
}

#[test]
fn report_tracks_drawdown_delta_and_inventory_duration_on_the_event_clock() {
    let mut cfg = config();
    cfg.active_hedge_threshold_usd = 1_000_000_000.0;
    for instrument in &mut cfg.instruments {
        instrument.maker_fee = 0.0;
        instrument.taker_fee = 0.0;
        instrument.quote_profit_margin = 0.5;
        instrument.hedge_profit_margin = 0.5;
    }
    let mut runner = BacktestRunner::new(cfg).unwrap();
    let events = vec![
        NormalizedEvent::from(MarketEvent::Depth(OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(49_999.0, 2.0),
            Level::new(50_001.0, 2.0),
        ))),
        NormalizedEvent::from(MarketEvent::Depth(OrderBook::one_level(
            "BTC-PERP",
            1,
            Level::new(49_999.0, 10_000.0),
            Level::new(50_001.0, 10_000.0),
        ))),
        NormalizedEvent::Order(OrderUpdate {
            ts_ms: 2,
            order_id: "external-fill".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            event: OrderEvent::FullyFilled,
            status: OrderStatus::Filled,
            price: 50_000.0,
            time_in_force: Some(TimeInForce::Ioc),
            qty: 1.0,
            open_qty: 0.0,
            filled_qty: 1.0,
            avg_fill_price: 50_000.0,
            last_fill_qty: 1.0,
            last_fill_price: 50_000.0,
            last_fill_liquidity: Some(FillLiquidity::Taker),
            last_fill_fee: None,
            reason: "fixture".to_string(),
        }),
        NormalizedEvent::from(MarketEvent::Depth(OrderBook::one_level(
            "BTC-USDT",
            12,
            Level::new(44_999.0, 2.0),
            Level::new(45_001.0, 2.0),
        ))),
    ];

    let report = runner.run(events).unwrap();

    assert_eq!(report.observed_duration_ns, 11_000_000);
    assert_eq!(report.final_equity_usd, -5_000.0);
    assert_eq!(report.max_drawdown_usd, 5_000.0);
    assert_eq!(report.max_abs_delta_usd, 50_000.0);
    assert_eq!(report.inventory_open_duration_ns, 10_000_000);
    assert!((report.inventory_open_fraction - 10.0 / 11.0).abs() < 1e-12);
    assert!(report.final_valuation_complete);
    assert_eq!(report.invalid_risk_metric_samples, 0);
    assert!(report.accounting_complete);
}
