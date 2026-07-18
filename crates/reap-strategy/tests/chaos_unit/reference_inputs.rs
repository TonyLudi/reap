use super::*;

fn seed_strict_reference_data(strategy: &mut ChaosStrategy, ts_ms: TimeMs) {
    let events = [
        MarketEvent::IndexPrice {
            ts_ms,
            symbol: "BTC-USDT-INDEX".to_string(),
            price: 50_000.5,
        },
        MarketEvent::PriceLimits {
            ts_ms,
            symbol: "BTC-USDT".to_string(),
            mark_price: 0.0,
            limit_down: 40_000.0,
            limit_up: 60_000.0,
        },
        MarketEvent::PriceLimits {
            ts_ms,
            symbol: "BTC-PERP".to_string(),
            mark_price: 0.0,
            limit_down: 40_000.0,
            limit_up: 60_000.0,
        },
        MarketEvent::PriceLimits {
            ts_ms,
            symbol: "BTC-PERP".to_string(),
            mark_price: 50_003.5,
            limit_down: 0.0,
            limit_up: 0.0,
        },
        MarketEvent::FundingRate {
            ts_ms,
            symbol: "BTC-PERP".to_string(),
            rate: 0.0001,
            funding_time_ms: ts_ms + 28_800_000,
            settlement: None,
        },
    ];
    for event in events {
        strategy.on_event(&StrategyEvent::Market(event));
    }
}

#[test]
fn strict_reference_contract_is_derived_once_for_strategy_and_live() {
    let mut config = config();
    config.reference_data_stale_threshold_ms = Some(1_000);
    config.instruments[0].index_symbol = Some("BTC-USDT-INDEX".to_string());
    config.instruments[1].kind = InstrumentKindConfig::LinearSwap;

    let requirements = config.reference_data_requirements();
    assert_eq!(requirements.len(), 5);
    assert!(requirements.iter().all(|item| item.max_age_ms == 1_000));
    assert!(requirements.iter().any(|item| {
        item.kind == ReferenceDataKind::IndexPrice && item.symbol == "BTC-USDT-INDEX"
    }));
    assert!(
        requirements.iter().any(|item| {
            item.kind == ReferenceDataKind::FundingRate && item.symbol == "BTC-PERP"
        })
    );
    assert_eq!(
        requirements
            .iter()
            .filter(|item| item.kind == ReferenceDataKind::PriceLimits)
            .count(),
        2
    );
}

#[test]
fn strict_reference_staleness_withdraws_quotes_without_cross_channel_masking() {
    let mut config = config();
    config.reference_data_stale_threshold_ms = Some(1_000);
    config.instruments[0].index_symbol = Some("BTC-USDT-INDEX".to_string());
    config.instruments[1].kind = InstrumentKindConfig::LinearSwap;
    let mut strategy = ChaosStrategy::new(config).unwrap();
    seed_strict_reference_data(&mut strategy, 1_000);

    strategy.on_event(&StrategyEvent::Market(MarketEvent::Depth(
        OrderBook::one_level(
            "BTC-USDT",
            1_000,
            Level::new(50_000.0, 1.0),
            Level::new(50_001.0, 1.0),
        ),
    )));
    let quotes = strategy.on_event(&StrategyEvent::Market(MarketEvent::Depth(
        OrderBook::one_level(
            "BTC-PERP",
            1_000,
            Level::new(50_003.0, 200.0),
            Level::new(50_004.0, 200.0),
        ),
    )));
    let quote = quotes
        .iter()
        .find_map(|intent| match intent {
            OrderIntent::NewOrder(order) if order.reason == "quote" => Some(order.clone()),
            _ => None,
        })
        .expect("fresh strict references should permit quoting");
    strategy.on_event(&StrategyEvent::Order(OrderUpdate {
        ts_ms: 1_001,
        order_id: "strict-q1".to_string(),
        symbol: quote.symbol,
        side: quote.side,
        event: OrderEvent::PendingNew,
        status: OrderStatus::PendingNew,
        price: quote.price,
        time_in_force: Some(quote.time_in_force),
        qty: quote.qty,
        open_qty: quote.qty,
        filled_qty: 0.0,
        avg_fill_price: 0.0,
        last_fill_qty: 0.0,
        last_fill_price: 0.0,
        last_fill_liquidity: None,
        last_fill_fee: None,
        reason: quote.reason,
    }));

    let intents = strategy.on_event(&StrategyEvent::Market(MarketEvent::PriceLimits {
        ts_ms: 2_001,
        symbol: "BTC-PERP".to_string(),
        mark_price: 50_003.5,
        limit_down: 0.0,
        limit_up: 0.0,
    }));

    let swap = strategy.entity("BTC-PERP").unwrap();
    assert_eq!(swap.mark_price_updated_ms, Some(2_001));
    assert_eq!(swap.price_limits_updated_ms, Some(1_000));
    assert_eq!(swap.funding_rate_updated_ms, Some(1_000));
    assert!(intents.iter().any(|intent| {
        matches!(intent, OrderIntent::CancelOrder { order_id, .. } if order_id == "strict-q1")
    }));

    strategy.on_event(&StrategyEvent::Market(MarketEvent::PriceLimits {
        ts_ms: 1_500,
        symbol: "BTC-PERP".to_string(),
        mark_price: 49_000.0,
        limit_down: 0.0,
        limit_up: 0.0,
    }));
    let swap = strategy.entity("BTC-PERP").unwrap();
    assert_eq!(swap.mark_price, Some(50_003.5));
    assert_eq!(swap.mark_price_updated_ms, Some(2_001));
    assert_eq!(strategy.now_ms, 2_001);
}

#[test]
fn java_parity_latches_spot_index_deviation_after_debounce() {
    let mut cfg = config();
    cfg.index_deviation_limit = 0.05;
    cfg.index_deviation_debounce_ms = 100;
    cfg.instruments
        .iter_mut()
        .find(|instrument| instrument.symbol == "BTC-USDT")
        .unwrap()
        .index_symbol = Some("BTC-INDEX".to_string());
    let mut strategy = ChaosStrategy::new(cfg).unwrap();
    strategy.on_depth(&OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(99.0, 10.0),
        Level::new(101.0, 10.0),
    ));
    strategy.on_depth(&OrderBook::one_level(
        "BTC-PERP",
        1,
        Level::new(99.0, 100_000.0),
        Level::new(101.0, 100_000.0),
    ));

    strategy.on_market_event(&MarketEvent::IndexPrice {
        ts_ms: 10,
        symbol: "BTC-INDEX".to_string(),
        price: 100.0,
    });
    strategy.on_market_event(&MarketEvent::IndexPrice {
        ts_ms: 11,
        symbol: "BTC-INDEX".to_string(),
        price: 80.0,
    });
    assert!(strategy.halt_reason().is_none());
    strategy.on_market_event(&MarketEvent::IndexPrice {
        ts_ms: 110,
        symbol: "BTC-INDEX".to_string(),
        price: 80.0,
    });
    assert!(strategy.halt_reason().is_none());
    let intents = legacy_intents(strategy.on_market_event(&MarketEvent::IndexPrice {
        ts_ms: 111,
        symbol: "BTC-INDEX".to_string(),
        price: 80.0,
    }));

    assert!(strategy.halt_reason().is_some());
    assert!(
        intents
            .iter()
            .all(|intent| !matches!(intent, OrderIntent::NewOrder(_)))
    );
}

#[test]
fn unrelated_index_prices_do_not_enter_strategy_pricing_state() {
    let mut strategy = ChaosStrategy::new(config()).unwrap();
    let intents = strategy.on_market_event(&MarketEvent::IndexPrice {
        ts_ms: 10,
        symbol: "USDT-USD".to_string(),
        price: 1.0,
    });

    assert!(intents.is_empty());
    assert!(
        !strategy
            .reference_health
            .index_prices
            .contains_key("USDT-USD")
    );
    assert_eq!(strategy.now_ms, 10);
}

#[test]
fn java_parity_stops_when_fewer_than_two_books_remain_valid() {
    let mut cfg = config();
    cfg.insufficient_valid_stop_ms = 100;
    for instrument in &mut cfg.instruments {
        instrument.depth_stale_threshold_ms = 10;
    }
    let mut strategy = ChaosStrategy::new(cfg).unwrap();
    strategy.on_depth(&OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(99.0, 10.0),
        Level::new(101.0, 10.0),
    ));
    strategy.on_depth(&OrderBook::one_level(
        "BTC-PERP",
        1,
        Level::new(99.0, 10_000.0),
        Level::new(101.0, 10_000.0),
    ));
    strategy.on_event(&StrategyEvent::Timer(reap_core::TimerEvent {
        ts_ms: 12,
        name: "risk".to_string(),
    }));
    assert!(strategy.halt_reason().is_none());
    strategy.on_event(&StrategyEvent::Timer(reap_core::TimerEvent {
        ts_ms: 113,
        name: "risk".to_string(),
    }));
    assert!(
        strategy
            .halt_reason()
            .unwrap()
            .contains("fewer than two instruments")
    );
}

#[test]
fn java_parity_enforces_exchange_price_limits() {
    let mut entity = InstrumentState::new(InstrumentConfig {
        price_limit_buffer: 0.01,
        ..InstrumentConfig::default()
    });
    entity.book = Some(OrderBook {
        symbol: "BTC-USDT".to_string(),
        ts_ms: 1,
        bids: vec![Level::new(95.0, 1.0), Level::new(94.0, 1.0)],
        asks: vec![Level::new(105.0, 1.0), Level::new(106.0, 1.0)],
    });
    entity.limit_up = Some(100.0);
    entity.limit_down = Some(90.0);

    assert!(approx_eq(entity.px_within_limit(Side::Buy, 102.0), 99.0));
    assert!(approx_eq(entity.px_within_limit(Side::Sell, 88.0), 94.0));
    assert!(!entity.can_take_within_price_limit(Side::Buy));
    assert!(entity.can_take_within_price_limit(Side::Sell));
}

#[test]
fn java_parity_removes_halted_and_stale_symbols_from_hedges() {
    let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
    seed_java_calculator_books(&mut strategy);
    strategy.update_best_hedges();
    assert!(
        strategy
            .hedging
            .best_hedges
            .values()
            .flatten()
            .any(|level| { level.symbol == "BTC-USD-SWAP.OK" })
    );

    strategy.on_system_event(&SystemEvent {
        ts_ms: 2,
        kind: SystemEventKind::SymbolHalted,
        venue: None,
        account_id: None,
        symbol: Some("BTC-USD-SWAP.OK".to_string()),
        reason: "test".to_string(),
    });
    assert!(
        strategy
            .hedging
            .best_hedges
            .values()
            .flatten()
            .all(|level| { level.symbol != "BTC-USD-SWAP.OK" })
    );

    strategy.on_system_event(&SystemEvent {
        ts_ms: 3,
        kind: SystemEventKind::FeedStale,
        venue: None,
        account_id: None,
        symbol: Some("BTC-USDT-SWAP.OK".to_string()),
        reason: "test".to_string(),
    });
    assert!(
        strategy
            .hedging
            .best_hedges
            .values()
            .flatten()
            .all(|level| { level.symbol != "BTC-USDT-SWAP.OK" })
    );
}

#[test]
fn account_halt_disables_every_instrument_owned_by_the_account() {
    let mut config = config();
    config.risk_groups[0].account_id = Some("main".to_string());
    let mut strategy = ChaosStrategy::new(config).unwrap();

    let intents = strategy.on_system_event(&SystemEvent {
        ts_ms: 2,
        kind: SystemEventKind::AccountHalted,
        venue: None,
        account_id: Some("main".to_string()),
        symbol: None,
        reason: "operator".to_string(),
    });

    assert!(intents.is_empty());
    assert!(
        strategy
            .entities
            .values()
            .all(|entity| entity.system_halted)
    );
}

#[test]
fn healthy_feed_heartbeat_does_not_recalculate_quotes() {
    let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
    seed_java_calculator_books(&mut strategy);
    assert!(!strategy.reference_health.startup_basis_checked);

    let intents = strategy.on_system_event(&SystemEvent {
        ts_ms: 2,
        kind: SystemEventKind::FeedHeartbeat,
        venue: None,
        account_id: None,
        symbol: Some("BTC-USDT.OK".to_string()),
        reason: "accepted sequence".to_string(),
    });

    assert!(intents.is_empty());
    assert!(!strategy.reference_health.startup_basis_checked);
    assert_eq!(strategy.now_ms, 2);
}

#[test]
fn feed_recovery_waits_for_the_following_book_before_repricing() {
    let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
    seed_java_calculator_books(&mut strategy);
    let symbol = "BTC-USDT.OK";

    strategy.on_system_event(&SystemEvent {
        ts_ms: 2,
        kind: SystemEventKind::FeedStale,
        venue: None,
        account_id: None,
        symbol: Some(symbol.to_string()),
        reason: "gap".to_string(),
    });
    assert!(strategy.entities[symbol].feed_stale);

    let intents = strategy.on_system_event(&SystemEvent {
        ts_ms: 3,
        kind: SystemEventKind::FeedRecovered,
        venue: None,
        account_id: None,
        symbol: Some(symbol.to_string()),
        reason: "snapshot accepted".to_string(),
    });

    assert!(intents.is_empty());
    assert!(!strategy.entities[symbol].feed_stale);
    assert!(!strategy.reference_health.startup_basis_checked);
}
