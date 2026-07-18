use super::*;

#[test]
fn config_validation_catches_duplicate_symbols_and_invalid_ticks() {
    let valid = config().effective();
    assert!(valid.validate().valid);

    let mut invalid = valid;
    invalid.instruments[1].symbol = invalid.instruments[0].symbol.clone();
    invalid.instruments[0].tick_size = 0.0;
    let report = invalid.validate();
    assert!(!report.valid);
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("duplicate instrument symbol"))
    );
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("tick_size"))
    );
}

#[test]
fn config_validation_rejects_single_instrument_iarb() {
    let mut single = config();
    single.instruments.truncate(1);
    single.risk_groups[0].symbols.truncate(1);

    let report = single.validate();

    assert!(!report.valid);
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("requires at least two instruments"))
    );
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("no distinct hedge-enabled instrument"))
    );
}

#[test]
fn config_validation_requires_spot_reference() {
    let mut invalid = config();
    invalid.ref_symbol = "BTC-PERP".to_string();

    let report = invalid.validate();

    assert!(!report.valid);
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("must be a spot instrument"))
    );
}

#[test]
fn java_parity_rejects_taker_fee_below_maker_fee() {
    let mut invalid = config();
    invalid.instruments[0].maker_fee = 0.001;
    invalid.instruments[0].taker_fee = 0.0005;

    let report = invalid.validate();

    assert!(!report.valid);
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("taker_fee must not be lower than maker_fee"))
    );
}

#[test]
fn java_parity_applies_risk_multiplier_only_to_java_limits() {
    let mut cfg = config();
    cfg.risk_multiplier = 2.0;
    cfg.coin_offset = 30.0;
    cfg.balance_sheet_limit_usd = 10_000_000.0;
    cfg.delta_limit_usd = 30_000.0;
    cfg.pnl_limit_usd = 5_000.0;
    cfg.index_deviation_limit = 0.05;
    cfg.active_hedge_threshold_usd = 800.0;
    cfg.risk_groups[0].coin_offset = 30.0;
    cfg.risk_groups[0].soft_delta_limit_usd = 10_000.0;
    cfg.risk_groups[0].hard_delta_limit_usd = 20_000.0;
    cfg.risk_groups[0].delta_stop_limit_usd = 40_000.0;
    cfg.risk_groups[0].live_order_limit_usd = 100_000.0;
    cfg.risk_groups[0].turnover_limit_usd = 10_000_000.0;
    cfg.risk_groups[0].basis_limit = 0.05;
    cfg.risk_groups[0].min_margin_level = 0.3;

    let effective = cfg.effective();

    assert!(approx_eq(effective.balance_sheet_limit_usd, 20_000_000.0));
    assert!(approx_eq(effective.delta_limit_usd, 60_000.0));
    assert!(approx_eq(effective.pnl_limit_usd, 10_000.0));
    assert!(approx_eq(effective.index_deviation_limit, 0.1));
    assert!(approx_eq(effective.coin_offset, 30.0));
    assert!(approx_eq(effective.active_hedge_threshold_usd, 800.0));
    let group = &effective.risk_groups[0];
    assert!(approx_eq(group.delta_stop_limit_usd, 80_000.0));
    assert!(approx_eq(group.live_order_limit_usd, 200_000.0));
    assert!(approx_eq(group.turnover_limit_usd, 20_000_000.0));
    assert!(approx_eq(group.basis_limit, 0.1));
    assert!(approx_eq(group.coin_offset, 30.0));
    assert!(approx_eq(group.soft_delta_limit_usd, 10_000.0));
    assert!(approx_eq(group.hard_delta_limit_usd, 20_000.0));
    assert!(approx_eq(group.min_margin_level, 0.3));

    let unlimited = ChaosConfig {
        risk_multiplier: 2.0,
        ..ChaosConfig::default()
    }
    .effective();
    assert_eq!(unlimited.balance_sheet_limit_usd, f64::MAX);
    assert_eq!(unlimited.pnl_limit_usd, f64::MAX);
}

#[test]
fn java_parity_applies_default_safety_multipliers() {
    let mut cfg = config();
    cfg.risk_groups[0].coins = vec![
        CoinConfig {
            currency: "BTC".to_string(),
            ..CoinConfig::default()
        },
        CoinConfig {
            currency: "USDT".to_string(),
            ..CoinConfig::default()
        },
    ];

    let effective = cfg.effective();

    assert!(approx_eq(
        effective.risk_groups[0].coins[0].safety_multiplier,
        2.5
    ));
    assert!(approx_eq(
        effective.risk_groups[0].coins[1].safety_multiplier,
        4.0
    ));
    assert!(approx_eq(effective.instruments[0].safety_multiplier, 1.0));
    assert!(approx_eq(effective.instruments[1].safety_multiplier, 2.0));
}

#[test]
fn inverse_contract_uses_java_iarb_coin_and_usd_conversions() {
    let mut state = InstrumentState::new(InstrumentConfig {
        symbol: "BTC-USD-SWAP".to_string(),
        kind: InstrumentKindConfig::InverseSwap,
        contract_value: 100.0,
        funding_rate: 0.001,
        ..InstrumentConfig::default()
    });
    state.book = Some(OrderBook::one_level(
        "BTC-USD-SWAP",
        1,
        Level::new(44_999.0, 100.0),
        Level::new(45_001.0, 100.0),
    ));
    state.position_qty = 20.0;
    state.position_avg_price = 50_000.0;

    assert!(approx_eq(state.size_from_usd(2_000.0, 50_000.0), 20.0));
    assert!(approx_eq(
        state.notional_coin(20.0, 45_000.0),
        20.0 * 100.0 / 45_000.0
    ));
    assert!(approx_eq(
        state.notional_usd(20.0, 45_000.0, 50_000.0),
        2_222.222222222222
    ));
    assert!(approx_eq(state.delta_coin(), 0.04));
    assert!(approx_eq(state.effective_funding_rate(), 0.001));
}

#[test]
fn funding_override_matches_java_swap_precedence() {
    let state = InstrumentState::new(InstrumentConfig {
        kind: InstrumentKindConfig::LinearSwap,
        funding_rate: 0.001,
        funding_override: Some(-0.002),
        ..InstrumentConfig::default()
    });
    assert!(approx_eq(state.effective_funding_rate(), -0.002));

    let dated = InstrumentState::new(InstrumentConfig {
        kind: InstrumentKindConfig::LinearFuture,
        funding_rate: 0.001,
        funding_override: Some(-0.002),
        ..InstrumentConfig::default()
    });
    assert!(approx_eq(dated.effective_funding_rate(), 0.0));
}

#[test]
fn java_parity_funding_manager_uses_earliest_swap_window() {
    let mut cfg = java_calculator_config();
    cfg.use_funding_rate_manager = true;
    let mut strategy = ChaosStrategy::new(cfg).unwrap();
    {
        let early = strategy.entities.get_mut("BTC-USD-SWAP.OK").unwrap();
        early.funding_rate = 0.001;
        early.funding_time_ms = 1_000;
    }
    {
        let later = strategy.entities.get_mut("BTC-USDT-SWAP.OK").unwrap();
        later.funding_rate = 0.002;
        later.funding_time_ms = 2_000;
    }

    strategy.now_ms = 100;
    strategy.update_funding_window();
    assert!(approx_eq(
        strategy
            .entity("BTC-USD-SWAP.OK")
            .unwrap()
            .effective_funding_rate(),
        0.001
    ));
    assert!(approx_eq(
        strategy
            .entity("BTC-USDT-SWAP.OK")
            .unwrap()
            .effective_funding_rate(),
        0.0
    ));

    strategy.now_ms = 1_500;
    strategy.update_funding_window();
    assert!(approx_eq(
        strategy
            .entity("BTC-USD-SWAP.OK")
            .unwrap()
            .effective_funding_rate(),
        0.0
    ));
    assert!(approx_eq(
        strategy
            .entity("BTC-USDT-SWAP.OK")
            .unwrap()
            .effective_funding_rate(),
        0.002
    ));
}

#[test]
fn java_random_matches_seeded_backtest_provider() {
    let mut random = JavaRandom::new(1);
    assert!(approx_eq(random.next_f64(), 0.7308781907032909));
    assert!(approx_eq(random.next_f64(), 0.41008081149220166));
}
