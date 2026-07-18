use super::*;

#[test]
fn realized_funding_rate_settles_signed_linear_swap_position() {
    let mut cfg = config();
    cfg.instruments[1].kind = InstrumentKindConfig::LinearSwap;
    cfg.instruments[1].taker_fee = 0.0;
    let mut runner = BacktestRunner::new(cfg).unwrap();
    runner
        .valuation
        .depth_marks
        .insert("BTC-PERP".to_string(), 50_000.0);
    runner.portfolio.apply_fill(
        &OrderUpdate {
            ts_ms: 0,
            order_id: "initial-fill".to_string(),
            symbol: "BTC-PERP".to_string(),
            side: Side::Buy,
            event: OrderEvent::FullyFilled,
            status: OrderStatus::Filled,
            price: 50_000.0,
            time_in_force: Some(TimeInForce::Ioc),
            qty: 100.0,
            open_qty: 0.0,
            filled_qty: 100.0,
            avg_fill_price: 50_000.0,
            last_fill_qty: 100.0,
            last_fill_price: 50_000.0,
            last_fill_liquidity: Some(FillLiquidity::Taker),
            last_fill_fee: None,
            reason: "initial".to_string(),
        },
        &HashMap::new(),
    );
    let events = vec![
        NormalizedEvent::from(MarketEvent::FundingRate {
            ts_ms: 1,
            symbol: "BTC-PERP".to_string(),
            rate: 0.001,
            funding_time_ms: 10,
            settlement: None,
        }),
        NormalizedEvent::from(MarketEvent::FundingRate {
            ts_ms: 5,
            symbol: "BTC-PERP".to_string(),
            rate: 0.002,
            funding_time_ms: 10,
            settlement: None,
        }),
        NormalizedEvent::from(MarketEvent::FundingRate {
            ts_ms: 11,
            symbol: "BTC-PERP".to_string(),
            rate: 0.003,
            funding_time_ms: 20,
            settlement: Some(FundingSettlement {
                funding_time_ms: 10,
                rate: 0.0015,
            }),
        }),
        NormalizedEvent::Timer(TimerEvent {
            ts_ms: 12,
            name: "funding".to_string(),
        }),
    ];

    let report = runner.run(events).unwrap();

    assert_eq!(report.funding_rate_events, 3);
    assert_eq!(report.funding_settlement_observations, 1);
    assert_eq!(report.funding_settlements, 1);
    assert_eq!(report.pending_funding_actions, 1);
    assert!((report.funding_pnl_usd + 7.5).abs() < 1e-9);
    assert!((report.final_equity_usd + 7.5).abs() < 1e-9);
    assert!(report.accounting_complete);
}

#[test]
fn funding_beyond_the_data_horizon_remains_explicitly_pending() {
    let mut cfg = config();
    cfg.instruments[1].kind = InstrumentKindConfig::LinearSwap;
    let mut runner = BacktestRunner::new(cfg).unwrap();

    let report = runner
        .run([NormalizedEvent::from(MarketEvent::FundingRate {
            ts_ms: 1,
            symbol: "BTC-PERP".to_string(),
            rate: 0.001,
            funding_time_ms: 100,
            settlement: None,
        })])
        .unwrap();

    assert_eq!(report.funding_settlements, 0);
    assert_eq!(report.pending_funding_actions, 1);
    assert_eq!(report.pending_scheduled_actions, 1);
    assert!(report.accounting_complete);
}

#[test]
fn due_funding_without_a_realized_rate_marks_accounting_incomplete() {
    let mut cfg = config();
    cfg.instruments[1].kind = InstrumentKindConfig::LinearSwap;
    let mut runner = BacktestRunner::new(cfg).unwrap();

    let report = runner
        .run([
            NormalizedEvent::from(MarketEvent::FundingRate {
                ts_ms: 1,
                symbol: "BTC-PERP".to_string(),
                rate: 0.001,
                funding_time_ms: 10,
                settlement: None,
            }),
            NormalizedEvent::Timer(TimerEvent {
                ts_ms: 10,
                name: "funding".to_string(),
            }),
        ])
        .unwrap();

    assert_eq!(report.funding_settlements, 0);
    assert_eq!(report.funding_settlement_failures, 1);
    assert!(!report.accounting_complete);
    assert!(
        report
            .carry_state_failures
            .iter()
            .any(|failure| failure.contains("requires complete accounting"))
    );
}

#[test]
fn conflicting_realized_funding_rates_are_rejected() {
    let mut cfg = config();
    cfg.instruments[1].kind = InstrumentKindConfig::LinearSwap;
    let mut runner = BacktestRunner::new(cfg).unwrap();
    let event = |ts_ms, settled_rate| {
        NormalizedEvent::from(MarketEvent::FundingRate {
            ts_ms,
            symbol: "BTC-PERP".to_string(),
            rate: 0.001,
            funding_time_ms: 20,
            settlement: Some(FundingSettlement {
                funding_time_ms: 10,
                rate: settled_rate,
            }),
        })
    };

    let error = runner
        .run([event(11, 0.001), event(12, 0.002)])
        .unwrap_err()
        .to_string();

    assert!(error.contains("conflicting realized funding rates"));
}

#[test]
fn stale_first_funding_forecast_marks_accounting_incomplete() {
    let mut cfg = config();
    cfg.instruments[1].kind = InstrumentKindConfig::LinearSwap;
    let mut runner = BacktestRunner::new(cfg).unwrap();

    let report = runner
        .run([NormalizedEvent::from(MarketEvent::FundingRate {
            ts_ms: 100_000,
            symbol: "BTC-PERP".to_string(),
            rate: 0.001,
            funding_time_ms: 1,
            settlement: None,
        })])
        .unwrap();

    assert_eq!(report.missed_funding_settlements, 1);
    assert!(!report.accounting_complete);
}
