use super::*;

#[test]
fn settled_carry_round_trips_portfolio_margin_and_raw_handoff() {
    let mut strategy = usdt_config();
    for instrument in &mut strategy.instruments {
        instrument.quote_profit_margin = 1.0;
        instrument.halted = true;
    }
    let execution = usdt_execution(0, 1_000);
    let initial = BacktestInitialPortfolioConfig {
        account_id: None,
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
        positions: vec![BacktestInitialPositionConfig {
            symbol: "BTC-PERP".to_string(),
            qty: 2.0,
            avg_price: 49_000.0,
            margin_mode: Some(reap_core::PositionMarginMode::Cross),
        }],
        margin: BacktestInitialMarginConfig::default(),
    };
    let mut runner =
        BacktestRunner::with_initial_portfolio_config(strategy.clone(), execution.clone(), initial)
            .unwrap();
    let mut events = initial_books();
    events.push(NormalizedEvent::Market(MarketEvent::IndexPrice {
        ts_ms: 2,
        symbol: "USDT-USD".to_string(),
        price: 1.0,
    }));
    events.push(NormalizedEvent::Timer(TimerEvent {
        ts_ms: 3,
        name: "finish".to_string(),
    }));

    let report = runner.run(events).unwrap();
    assert!(report.carry_state_failures.is_empty());
    let mut carry = report.settled_carry_state.unwrap();
    assert_eq!(carry.settled_at_ns, 3 * NS_PER_MS);
    assert_eq!(carry.portfolio.balances[0].available, Some(0.002));
    assert_eq!(carry.portfolio.positions[0].qty, 2.0);
    assert_eq!(carry.portfolio.positions[0].avg_price, 50_003.5);
    assert_eq!(
        carry.portfolio.positions[0].margin_mode,
        Some(reap_core::PositionMarginMode::Cross)
    );
    assert!(carry.terminal_exchange_marks.is_empty());
    assert_eq!(carry.terminal_depth_marks.get("BTC-PERP"), Some(&50_003.5));

    let mut bad_balance = carry.clone();
    bad_balance.portfolio.balances[1].total += 1.0;
    assert!(
        bad_balance
            .validate_for(&strategy, &execution)
            .unwrap_err()
            .to_string()
            .contains("settled carry")
    );
    let mut bad_average = carry.clone();
    bad_average.portfolio.positions[0].avg_price += 1.0;
    assert!(bad_average.validate_for(&strategy, &execution).is_err());
    let mut bad_margin = carry.clone();
    bad_margin.portfolio.margin.exchange_ratio = bad_margin
        .portfolio
        .margin
        .exchange_ratio
        .map(|ratio| ratio + 1.0);
    assert!(bad_margin.validate_for(&strategy, &execution).is_err());

    carry.source_raw_boundary = Some(RawReplayBoundary {
        capture_session_id: "session-a".to_string(),
        first_capture_record_seq: 1,
        last_capture_record_seq: 10,
        raw_records: 10,
        first_recv_ts_ns: 1,
        last_recv_ts_ns: 3 * NS_PER_MS,
        maximum_recv_ts_ns: 3 * NS_PER_MS,
    });
    let carried =
        BacktestRunner::with_carry_state(strategy.clone(), execution.clone(), carry).unwrap();
    assert_eq!(carried.opening_equity_usd, report.opening_equity_usd);
    carried
        .validate_carry_handoff(&RawReplayBoundary {
            capture_session_id: "session-a".to_string(),
            first_capture_record_seq: 11,
            last_capture_record_seq: 20,
            raw_records: 10,
            first_recv_ts_ns: 3 * NS_PER_MS + 1,
            last_recv_ts_ns: 4 * NS_PER_MS,
            maximum_recv_ts_ns: 4 * NS_PER_MS,
        })
        .unwrap();
    let error = carried
        .validate_carry_handoff(&RawReplayBoundary {
            capture_session_id: "session-a".to_string(),
            first_capture_record_seq: 12,
            last_capture_record_seq: 20,
            raw_records: 9,
            first_recv_ts_ns: 3 * NS_PER_MS + 1,
            last_recv_ts_ns: 4 * NS_PER_MS,
            maximum_recv_ts_ns: 4 * NS_PER_MS,
        })
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("next capture record sequence 11")
    );
    let error = carried
        .validate_carry_handoff(&RawReplayBoundary {
            capture_session_id: "session-a".to_string(),
            first_capture_record_seq: 11,
            last_capture_record_seq: 20,
            raw_records: 10,
            first_recv_ts_ns: 3 * NS_PER_MS - 1,
            last_recv_ts_ns: 4 * NS_PER_MS,
            maximum_recv_ts_ns: 4 * NS_PER_MS,
        })
        .unwrap_err();
    assert!(error.to_string().contains("receive time regresses"));
}

#[test]
fn settled_carry_preserves_pending_funding_and_settlement_watermark() {
    let mut strategy = config();
    strategy.instruments[1].kind = InstrumentKindConfig::LinearSwap;
    for instrument in &mut strategy.instruments {
        instrument.base_currency = "BTC".to_string();
        instrument.quote_currency = "USD".to_string();
        if instrument.kind.is_derivative() {
            instrument.settle_currency = "USD".to_string();
        }
        instrument.quote_profit_margin = 1.0;
        instrument.halted = true;
    }
    let execution = BacktestExecutionConfig::default();
    let initial = BacktestInitialPortfolioConfig {
        balances: vec![
            BacktestInitialBalanceConfig {
                currency: "BTC".to_string(),
                total: 0.0,
                valuation_symbol: Some("BTC-USDT".to_string()),
                ..Default::default()
            },
            BacktestInitialBalanceConfig {
                currency: "USD".to_string(),
                total: 10_000.0,
                ..Default::default()
            },
        ],
        positions: vec![BacktestInitialPositionConfig {
            symbol: "BTC-PERP".to_string(),
            qty: 10.0,
            avg_price: 50_000.0,
            margin_mode: Some(reap_core::PositionMarginMode::Cross),
        }],
        ..Default::default()
    };
    let mut first =
        BacktestRunner::with_initial_portfolio_config(strategy.clone(), execution.clone(), initial)
            .unwrap();
    let mut first_events = initial_books();
    first_events.push(NormalizedEvent::from(MarketEvent::FundingRate {
        ts_ms: 2,
        symbol: "BTC-PERP".to_string(),
        rate: 0.001,
        funding_time_ms: 100,
        settlement: None,
    }));
    let first_report = first.run(first_events).unwrap();
    let carry = first_report.settled_carry_state.unwrap();
    assert_eq!(carry.pending_funding.len(), 1);
    assert_eq!(carry.pending_funding[0].funding_time_ms, 100);
    assert_eq!(carry.pending_funding[0].realized_rate, None);
    let mut overlapping = carry.clone();
    overlapping
        .last_settled_funding_time_ms
        .insert("BTC-PERP".to_string(), 100);
    assert!(
        overlapping
            .validate_for(&strategy, &execution)
            .unwrap_err()
            .to_string()
            .contains("overlaps its settlement watermark")
    );

    let carry_settled_at_ns = carry.settled_at_ns;
    let mut second = BacktestRunner::with_carry_state(strategy, execution, carry).unwrap();
    assert!(second.initial_account_snapshot_delivered);
    assert_eq!(second.last_account_publish_ns, Some(carry_settled_at_ns));
    let second_report = second
        .run([NormalizedEvent::from(MarketEvent::FundingRate {
            ts_ms: 101,
            symbol: "BTC-PERP".to_string(),
            rate: 0.002,
            funding_time_ms: 200,
            settlement: Some(FundingSettlement {
                funding_time_ms: 100,
                rate: 0.001,
            }),
        })])
        .unwrap();

    assert_eq!(second_report.funding_settlements, 1);
    assert!((second_report.funding_pnl_usd + 0.500_035).abs() < 1e-9);
    assert_eq!(second_report.observed_duration_ns, 0);
    assert_eq!(second_report.inventory_open_duration_ns, 0);
    assert_eq!(second_report.inventory_open_fraction, 0.0);
    assert!(
        second_report.settled_carry_state.is_some(),
        "{:?}",
        second_report.carry_state_failures
    );
    let second_carry = second_report.settled_carry_state.unwrap();
    assert_eq!(
        second_carry.last_settled_funding_time_ms.get("BTC-PERP"),
        Some(&100)
    );
    assert_eq!(second_carry.pending_funding.len(), 1);
    assert_eq!(second_carry.pending_funding[0].funding_time_ms, 200);
}
