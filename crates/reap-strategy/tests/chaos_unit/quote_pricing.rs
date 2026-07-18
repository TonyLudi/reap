use super::*;

fn spot_skew_state(
    base_coin_config: CoinConfig,
    quote_coin_config: CoinConfig,
    base_balance: f64,
    quote_balance: f64,
) -> InstrumentState {
    let mut state = InstrumentState::new(InstrumentConfig {
        symbol: "BTC-USD".to_string(),
        kind: InstrumentKindConfig::Spot,
        base_currency: "BTC".to_string(),
        quote_currency: "USD".to_string(),
        hedge_profit_margin: 0.0005,
        min_trade_size: 0.0001,
        ..InstrumentConfig::default()
    });
    state.book = Some(OrderBook::one_level(
        "BTC-USD",
        1,
        Level::new(49_999.0, 10.0),
        Level::new(50_001.0, 10.0),
    ));
    state.trade.base_coin_config = Some(base_coin_config);
    state.trade.quote_coin_config = Some(quote_coin_config);
    state.base_balance = base_balance;
    state.base_available = base_balance.max(0.0);
    state.quote_balance = quote_balance;
    state.quote_available = quote_balance.max(0.0);
    state.balances_initialized = true;
    state
}

fn fixed_base_skew() -> CoinConfig {
    CoinConfig {
        currency: "BTC".to_string(),
        min_balance: 28.0,
        max_balance: 32.0,
        skew_offset: 30.0,
        skew_type: Some(SkewTypeConfig::Fix),
        buy_skew: 0.005,
        sell_skew: 0.005,
        buy_activation: 32.0,
        sell_activation: 28.0,
        ..CoinConfig::default()
    }
}

fn fixed_quote_skew() -> CoinConfig {
    CoinConfig {
        currency: "USD".to_string(),
        min_balance: 0.0,
        max_balance: 50_000.0,
        skew_offset: 25_000.0,
        skew_type: Some(SkewTypeConfig::Fix),
        buy_skew: 0.0000001,
        sell_skew: 0.0000001,
        ..CoinConfig::default()
    }
}

#[test]
fn computes_quotes_from_opposite_hedge_ladder() {
    let mut strategy = ChaosStrategy::new(config()).unwrap();
    let spot = OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(50_000.0, 1.0),
        Level::new(50_001.0, 1.0),
    );
    let perp = OrderBook::one_level(
        "BTC-PERP",
        1,
        Level::new(50_003.0, 200.0),
        Level::new(50_004.0, 200.0),
    );

    strategy.on_event(&StrategyEvent::Market(MarketEvent::Depth(spot)));
    let commands = strategy.on_event(&StrategyEvent::Market(MarketEvent::Depth(perp)));

    assert!(
        commands
            .iter()
            .any(|cmd| matches!(cmd, OrderIntent::NewOrder(o) if o.reason == "quote"))
    );
    let spot_state = strategy.entity("BTC-USDT").unwrap();
    assert!(spot_state.theo(Side::Buy).unwrap().price < 50_001.0);
    assert!(spot_state.theo(Side::Sell).unwrap().price > 50_000.0);
}

#[test]
fn java_parity_prices_spot_buy_from_inverse_hedge() {
    let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
    seed_java_calculator_books(&mut strategy);

    strategy.update_best_hedges();
    strategy.update_theo_quotes();

    let quote = strategy
        .entity("BTC-USDT.OK")
        .unwrap()
        .theo(Side::Buy)
        .unwrap();
    assert!(
        approx_eq(quote.price, 44_947.09722222222),
        "{}",
        quote.price
    );
    assert!(approx_eq(quote.qty, 0.044444444444444446), "{}", quote.qty);
    assert!(quote.hedge_symbol.starts_with("BTC-USD-"));
}

#[test]
fn java_parity_disables_quote_when_group_can_only_self_hedge() {
    let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
    seed_java_calculator_books(&mut strategy);
    strategy.risk_groups.get_mut("OKEX-Spot").unwrap().delta_usd = 25_000.0;

    strategy.update_best_hedges();
    strategy.update_theo_quotes();

    assert!(
        strategy
            .entity("BTC-USDT.OK")
            .unwrap()
            .theo(Side::Buy)
            .is_none()
    );
}

#[test]
fn java_parity_uses_linear_hedge_when_inverse_group_cannot_sell() {
    let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
    seed_java_calculator_books(&mut strategy);
    strategy
        .risk_groups
        .get_mut("OKEX-Invert")
        .unwrap()
        .delta_usd = -40_000.0;

    strategy.update_best_hedges();
    strategy.update_theo_quotes();

    let quote = strategy
        .entity("BTC-USDT.OK")
        .unwrap()
        .theo(Side::Buy)
        .unwrap();
    assert!(
        approx_eq(quote.price, 44_943.01242236025),
        "{}",
        quote.price
    );
    assert!(approx_eq(quote.qty, 0.08), "{}", quote.qty);
    assert!(quote.hedge_symbol.starts_with("BTC-USDT-"));
}

#[test]
fn java_parity_applies_swap_funding_to_spot_quote() {
    let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
    seed_java_calculator_books(&mut strategy);
    strategy
        .risk_groups
        .get_mut("OKEX-Invert")
        .unwrap()
        .delta_usd = -40_000.0;
    strategy
        .entities
        .get_mut("BTC-USDT-SWAP.OK")
        .unwrap()
        .funding_rate = 0.001;

    strategy.update_best_hedges();
    strategy.update_theo_quotes();

    let quote = strategy
        .entity("BTC-USDT.OK")
        .unwrap()
        .theo(Side::Buy)
        .unwrap();
    assert!(
        approx_eq(quote.price, 44_993.01242236025),
        "{}",
        quote.price
    );
    assert_eq!(quote.hedge_symbol, "BTC-USDT-SWAP.OK");
}

#[test]
fn java_parity_prices_inverse_sell_from_spot_hedge() {
    let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
    seed_java_calculator_books(&mut strategy);

    strategy.update_best_hedges();
    strategy.update_theo_quotes();

    for symbol in ["BTC-USD-SWAP.OK", "BTC-USD-211231.OK"] {
        let quote = strategy.entity(symbol).unwrap().theo(Side::Sell).unwrap();
        assert!(approx_eq(quote.price, 55_055.125), "{}", quote.price);
        assert!(approx_eq(quote.qty, 40.0), "{}", quote.qty);
        assert_eq!(quote.hedge_symbol, "BTC-USDT.OK");
    }
}

#[test]
fn java_parity_spot_skew_uses_base_and_quote_balances() {
    let no_skew = spot_skew_state(
        CoinConfig::default(),
        CoinConfig::default(),
        30.0,
        1_000_000.0,
    );
    assert!(approx_eq(no_skew.average_skew_rate_to(0.0), 0.0));
    assert!(approx_eq(no_skew.posn_skew(), 0.0));
    assert!(no_skew.max_hedge_chunk_qty(50_000.0).is_infinite());

    let quote_skew = spot_skew_state(CoinConfig::default(), fixed_quote_skew(), 30.0, 10_000.0);
    assert!(approx_eq(quote_skew.average_skew_rate_to(0.0), 0.005));
    assert!(approx_eq(quote_skew.posn_skew(), -0.0015));
    assert!(approx_eq(quote_skew.max_hedge_chunk_qty(50_000.0), 0.1));

    let base_skew = spot_skew_state(fixed_base_skew(), CoinConfig::default(), 29.8, 1_000_000.0);
    assert!(approx_eq(base_skew.average_skew_rate_to(0.0), 0.005));
    assert!(approx_eq(base_skew.posn_skew(), 0.001));
    assert!(approx_eq(base_skew.max_hedge_chunk_qty(50_000.0), 0.1));

    let both_skew = spot_skew_state(fixed_base_skew(), fixed_quote_skew(), 29.8, 10_000.0);
    assert!(approx_eq(both_skew.average_skew_rate_to(0.0), 0.01));
    assert!(approx_eq(both_skew.posn_skew(), -0.0005));
    assert!(approx_eq(both_skew.max_hedge_chunk_qty(50_000.0), 0.05));
}

#[test]
fn java_parity_spot_skew_changes_sign_with_inventory() {
    let base_positive_quote_negative =
        spot_skew_state(fixed_base_skew(), fixed_quote_skew(), 30.2, 10_000.0);
    assert!(approx_eq(base_positive_quote_negative.posn_skew(), -0.0025));

    let base_negative_quote_positive =
        spot_skew_state(fixed_base_skew(), fixed_quote_skew(), 29.8, 40_000.0);
    assert!(approx_eq(base_negative_quote_positive.posn_skew(), 0.0025));

    let both_positive = spot_skew_state(fixed_base_skew(), fixed_quote_skew(), 30.2, 40_000.0);
    assert!(approx_eq(both_positive.posn_skew(), 0.0005));
}

#[test]
fn java_parity_builds_configured_mass_quote_levels() {
    let mut cfg = config();
    let spot = cfg
        .instruments
        .iter_mut()
        .find(|instrument| instrument.symbol == "BTC-USDT")
        .unwrap();
    spot.num_quote_levels = 3;
    spot.min_level_spread = 0.001;
    spot.max_level_spread = 0.002;
    spot.min_order_size_usd = 100.0;
    spot.max_order_size_usd = 200.0;
    spot.tick_size = 0.1;
    spot.lot_size = 0.01;

    let mut strategy = ChaosStrategy::new(cfg).unwrap();
    strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(99.0, 10.0),
        Level::new(101.0, 10.0),
    ));
    strategy.now_ms = 10_000;
    let levels = strategy.desired_quote_levels(
        "BTC-USDT",
        Side::Buy,
        Some(TheoQuote {
            price: 100.0,
            qty: 1.5,
            hedge_px: 101.0,
            hedge_symbol: "BTC-PERP".to_string(),
        }),
    );

    assert_eq!(levels.len(), 3);
    assert!(approx_eq(levels[0].price, 100.0));
    assert!(levels[1].price < levels[0].price);
    assert!(levels[2].price < levels[1].price);
    assert!(levels[1].qty >= 100.0 / 100.0);
    assert!(levels[1].qty <= 200.0 / 100.0);

    let mut intents = Vec::new();
    strategy.sync_quotes("BTC-USDT", Side::Buy, &levels, &mut intents);
    let intents = legacy_intents(intents);
    assert_eq!(intents.len(), 3);
    assert!(matches!(&intents[0], OrderIntent::NewOrder(order) if order.reason == "quote"));
    assert!(matches!(&intents[1], OrderIntent::NewOrder(order) if order.reason == "quote:1"));
    assert!(matches!(&intents[2], OrderIntent::NewOrder(order) if order.reason == "quote:2"));
}

#[test]
fn java_parity_burst_adjusts_one_quote_side_and_hedge_aggression() {
    let mut cfg = java_calculator_config();
    cfg.act_on_burst = true;
    let mut strategy = ChaosStrategy::new(cfg).unwrap();
    seed_java_calculator_books(&mut strategy);
    strategy.update_best_hedges();
    strategy.update_theo_quotes();
    let baseline_buy = strategy
        .entity("BTC-USDT.OK")
        .unwrap()
        .theo(Side::Buy)
        .unwrap()
        .price;
    let baseline_sell = strategy
        .entity("BTC-USDT.OK")
        .unwrap()
        .theo(Side::Sell)
        .unwrap()
        .price;

    strategy.on_market_event(&MarketEvent::BurstSignal {
        ts_ms: 2,
        symbol: "BTC-USDT.OK".to_string(),
        value: 0.001,
    });

    let burst_buy = strategy
        .entity("BTC-USDT.OK")
        .unwrap()
        .theo(Side::Buy)
        .unwrap()
        .price;
    let burst_sell = strategy
        .entity("BTC-USDT.OK")
        .unwrap()
        .theo(Side::Sell)
        .unwrap()
        .price;
    assert!(approx_eq(burst_buy, baseline_buy));
    assert!(approx_eq(burst_sell, baseline_sell + 50.0));

    let targets = strategy.summarize_hedges(
        strategy.hedging.best_hedges.get(&Side::Buy).unwrap(),
        Side::Buy,
        9_000.0,
        None,
    );
    let spot = targets
        .iter()
        .find(|target| target.symbol == "BTC-USDT.OK")
        .unwrap();
    assert!(approx_eq(spot.hedge_px, 55_055.0));

    strategy.on_market_event(&MarketEvent::BurstSignal {
        ts_ms: 3,
        symbol: "BTC-USDT-SWAP.OK".to_string(),
        value: -0.002,
    });
    assert!(approx_eq(strategy.pricing.burst, 0.0));
    assert!(strategy.pricing.burst_symbol.is_none());
}

#[test]
fn java_parity_ignore_best_level_uses_second_raw_level() {
    let mut cfg = config();
    cfg.ignore_best_level = true;
    let mut strategy = ChaosStrategy::new(cfg).unwrap();
    let entity = strategy.entities.get_mut("BTC-USDT").unwrap();
    entity.book = Some(OrderBook {
        symbol: "BTC-USDT".to_string(),
        ts_ms: 1,
        bids: vec![Level::new(46_000.0, 1.0), Level::new(45_000.0, 1.0)],
        asks: vec![Level::new(54_000.0, 1.0), Level::new(55_000.0, 1.0)],
    });

    assert!(approx_eq(entity.mid().unwrap(), 50_000.0));
    assert!(approx_eq(
        entity.effective_levels(Side::Buy)[0].px,
        45_000.0
    ));
    assert!(approx_eq(
        entity.effective_levels(Side::Sell)[0].px,
        55_000.0
    ));
}

#[test]
fn java_parity_quote_only_stays_at_top_of_book() {
    let mut entity = InstrumentState::new(InstrumentConfig {
        tick_size: 1.0,
        ..InstrumentConfig::default()
    });
    entity.book = Some(OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(100.0, 1.0),
        Level::new(105.0, 1.0),
    ));

    assert!(approx_eq(entity.quote_only_price(Side::Buy, 103.0), 101.0));
    assert!(approx_eq(entity.quote_only_price(Side::Sell, 102.0), 104.0));
    assert!(approx_eq(entity.quote_only_price(Side::Buy, 99.0), 99.0));
    assert!(approx_eq(entity.quote_only_price(Side::Sell, 106.0), 106.0));
}

#[test]
fn java_parity_separates_self_crossing_theoretical_quotes() {
    let mut entity = InstrumentState::new(InstrumentConfig {
        quote_profit_margin: 0.001,
        ..InstrumentConfig::default()
    });
    entity.book = Some(OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(99.0, 1.0),
        Level::new(101.0, 1.0),
    ));
    entity.buy_theo = Some(TheoQuote {
        price: 101.0,
        qty: 1.0,
        hedge_px: 100.0,
        hedge_symbol: "hedge".to_string(),
    });
    entity.sell_theo = Some(TheoQuote {
        price: 99.0,
        qty: 1.0,
        hedge_px: 100.0,
        hedge_symbol: "hedge".to_string(),
    });

    entity.prevent_self_cross();

    assert!(approx_eq(entity.buy_theo.unwrap().price, 99.9));
    assert!(approx_eq(entity.sell_theo.unwrap().price, 100.1));
}
