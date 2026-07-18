use super::*;

#[test]
fn quote_fill_triggers_ioc_hedge_excluding_fill_symbol() {
    let mut strategy = ChaosStrategy::new(config()).unwrap();
    strategy.on_event(&StrategyEvent::Market(MarketEvent::Depth(
        OrderBook::one_level(
            "BTC-USDT",
            1,
            Level::new(50_000.0, 1.0),
            Level::new(50_001.0, 1.0),
        ),
    )));
    strategy.on_event(&StrategyEvent::Market(MarketEvent::Depth(
        OrderBook::one_level(
            "BTC-PERP",
            1,
            Level::new(50_003.0, 2000.0),
            Level::new(50_004.0, 2000.0),
        ),
    )));

    let fill_intents = strategy.on_event(&StrategyEvent::Order(OrderUpdate {
        ts_ms: 2,
        order_id: "q1".to_string(),
        symbol: "BTC-USDT".to_string(),
        side: Side::Buy,
        event: OrderEvent::FullyFilled,
        status: OrderStatus::Filled,
        price: 50_000.0,
        time_in_force: Some(TimeInForce::PostOnly),
        qty: 0.1,
        open_qty: 0.0,
        filled_qty: 0.1,
        avg_fill_price: 50_000.0,
        last_fill_qty: 0.1,
        last_fill_price: 50_000.0,
        last_fill_liquidity: None,
        last_fill_fee: None,
        reason: "quote".to_string(),
    }));
    assert!(fill_intents.is_empty());
    let hedge = strategy.on_event(&StrategyEvent::Account(AccountUpdate {
        ts_ms: 2,
        balances: Vec::new(),
        positions: vec![reap_core::Position {
            symbol: "BTC-USDT".to_string(),
            qty: 0.1,
            avg_price: 50_000.0,
            margin_mode: None,
        }],
        margins: Vec::new(),
    }));

    assert!(
        hedge
            .iter()
            .any(|cmd| matches!(cmd, OrderIntent::NewOrder(o)
        if o.symbol == "BTC-PERP"
            && o.side == Side::Sell
            && o.time_in_force == TimeInForce::Ioc
            && o.self_trade_prevention == Some(SelfTradePrevention::CancelMaker)))
    );
}

#[test]
fn normalized_fixture_drives_quote_then_hedge_decisions() {
    let events = include_str!("../../../../fixtures/normalized/chaos_quote_hedge.jsonl")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<NormalizedEvent>(line).unwrap())
        .collect::<Vec<_>>();
    let mut strategy = ChaosStrategy::new(config()).unwrap();

    let mut all_intents = Vec::new();
    for event in events {
        let intents = strategy.on_event(&event.into_strategy_event());
        all_intents.push(intents);
    }

    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/normalized/chaos_quote_hedge_intents.json"
    ))
    .unwrap();
    assert_eq!(serde_json::to_value(&all_intents).unwrap(), expected);

    assert!(
        all_intents[1].iter().any(
            |intent| matches!(intent, OrderIntent::NewOrder(order) if order.reason == "quote")
        )
    );
    assert!(all_intents[2].is_empty());
    assert!(all_intents[3].iter().any(|intent| matches!(intent, OrderIntent::NewOrder(order)
        if order.symbol == "BTC-PERP" && order.side == Side::Sell && order.time_in_force == TimeInForce::Ioc)));
}

#[test]
fn quote_replacement_emits_typed_cancel_owned_without_changing_legacy_record() {
    let mut strategy = ChaosStrategy::new(config()).unwrap();
    strategy.execution.active_quotes.insert(
        ("BTC-USDT".to_string(), Side::Buy, 0),
        execution_state::ActiveQuote {
            order_id: "canonical-q1".to_string(),
            price: 49_900.0,
            qty: 0.1,
        },
    );
    let mut intents = Vec::new();
    strategy.sync_quotes("BTC-USDT", Side::Buy, &[], &mut intents);

    let [ChaosExecutionIntent::CancelOwned(cancel)] = intents.as_slice() else {
        panic!("expected one typed owned-order cancellation");
    };
    assert_eq!(cancel.order_id(), "canonical-q1");
    assert_eq!(cancel.reason(), "quote_disabled");
    assert_eq!(
        serde_json::to_value(intents.remove(0).into_order_intent()).unwrap(),
        serde_json::json!({
            "CancelOrder": {
                "order_id": "canonical-q1",
                "reason": "quote_disabled"
            }
        })
    );
}

#[test]
fn pending_new_quote_blocks_duplicate_intent_before_exchange_ack() {
    let mut strategy = ChaosStrategy::new(config()).unwrap();
    let levels = vec![TheoQuote {
        price: 49_900.0,
        qty: 0.1,
        hedge_px: 50_001.0,
        hedge_symbol: "BTC-PERP".to_string(),
    }];
    let mut first = Vec::new();
    strategy.sync_quotes("BTC-USDT", Side::Buy, &levels, &mut first);
    let first = legacy_intents(first);
    let OrderIntent::NewOrder(order) = &first[0] else {
        panic!("expected quote order");
    };

    strategy.on_order_update(&OrderUpdate {
        ts_ms: 2,
        order_id: "pending-q1".to_string(),
        symbol: order.symbol.clone(),
        side: order.side,
        event: OrderEvent::PendingNew,
        status: OrderStatus::PendingNew,
        price: order.price,
        time_in_force: Some(order.time_in_force),
        qty: order.qty,
        open_qty: order.qty,
        filled_qty: 0.0,
        avg_fill_price: 0.0,
        last_fill_qty: 0.0,
        last_fill_price: 0.0,
        last_fill_liquidity: None,
        last_fill_fee: None,
        reason: "quote:pending_new".to_string(),
    });

    let mut repeated = Vec::new();
    strategy.sync_quotes("BTC-USDT", Side::Buy, &levels, &mut repeated);

    assert!(repeated.is_empty());
}

#[test]
fn java_parity_pending_delta_includes_pending_and_live_hedges() {
    let mut strategy = ChaosStrategy::new(config()).unwrap();
    strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(50_000.0, 1.0),
        Level::new(50_002.0, 1.0),
    ));
    strategy.on_order_update(&OrderUpdate {
        ts_ms: 2,
        order_id: "h1".to_string(),
        symbol: "BTC-PERP".to_string(),
        side: Side::Sell,
        event: OrderEvent::PendingNew,
        status: OrderStatus::PendingNew,
        price: 49_990.0,
        time_in_force: Some(TimeInForce::Ioc),
        qty: 100.0,
        open_qty: 100.0,
        filled_qty: 0.0,
        avg_fill_price: 0.0,
        last_fill_qty: 0.0,
        last_fill_price: 0.0,
        last_fill_liquidity: None,
        last_fill_fee: None,
        reason: "hedge:BTC-USDT:50000".to_string(),
    });
    strategy.update_risk();

    assert!(approx_eq(strategy.delta_usd(), 0.0));
    assert!(approx_eq(strategy.pending_delta_usd(), -5_000.1));
}

#[test]
fn java_parity_does_not_reuse_pending_hedge_liquidity() {
    let mut strategy = ChaosStrategy::new(java_calculator_config()).unwrap();
    seed_java_calculator_books(&mut strategy);
    let entity = strategy.entity("BTC-USD-SWAP.OK").unwrap();
    let level = entity.hedge_levels(Side::Sell, 50_000.0, &[])[0].to_owned_level();
    strategy.execution.active_hedges.insert(
        "h1".to_string(),
        execution_state::ActiveHedge {
            symbol: level.symbol.clone(),
            signed_open_qty: -level.qty,
            price: 44_986.5,
            reference_price: level.px,
            updated_ms: 1,
        },
    );

    let targets = strategy.summarize_hedges(
        std::slice::from_ref(&level),
        Side::Sell,
        level.notional_usd,
        None,
    );

    assert!(targets.is_empty());
}

#[test]
fn java_parity_respects_top_quote_refill_interval() {
    let mut strategy = ChaosStrategy::new(config()).unwrap();
    strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(99.0, 10.0),
        Level::new(101.0, 10.0),
    ));
    let levels = vec![TheoQuote {
        price: 100.0,
        qty: 1.0,
        hedge_px: 101.0,
        hedge_symbol: "BTC-PERP".to_string(),
    }];
    strategy.on_order_update(&OrderUpdate {
        ts_ms: 10,
        order_id: "q1".to_string(),
        symbol: "BTC-USDT".to_string(),
        side: Side::Buy,
        event: OrderEvent::New,
        status: OrderStatus::Live,
        price: 100.0,
        time_in_force: Some(TimeInForce::PostOnly),
        qty: 1.0,
        open_qty: 1.0,
        filled_qty: 0.0,
        avg_fill_price: 0.0,
        last_fill_qty: 0.0,
        last_fill_price: 0.0,
        last_fill_liquidity: None,
        last_fill_fee: None,
        reason: "quote".to_string(),
    });
    strategy.on_order_update(&OrderUpdate {
        ts_ms: 100,
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
        last_fill_liquidity: Some(FillLiquidity::Maker),
        last_fill_fee: None,
        reason: "quote".to_string(),
    });

    strategy.now_ms = 399;
    let mut blocked = Vec::new();
    strategy.sync_quotes("BTC-USDT", Side::Buy, &levels, &mut blocked);
    assert!(blocked.is_empty());

    strategy.now_ms = 400;
    let mut refill = Vec::new();
    strategy.sync_quotes("BTC-USDT", Side::Buy, &levels, &mut refill);
    let refill = legacy_intents(refill);
    assert!(matches!(refill.as_slice(), [OrderIntent::NewOrder(_)]));
}

#[test]
fn java_parity_conflates_quote_changes_within_debounce_interval() {
    let mut cfg = config();
    cfg.instruments
        .iter_mut()
        .find(|instrument| instrument.symbol == "BTC-USDT")
        .unwrap()
        .tick_size = 0.01;
    let mut strategy = ChaosStrategy::new(cfg).unwrap();
    strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(99.0, 10.0),
        Level::new(101.0, 10.0),
    ));
    let quote = |price| TheoQuote {
        price,
        qty: 1.0,
        hedge_px: 101.0,
        hedge_symbol: "BTC-PERP".to_string(),
    };

    strategy.now_ms = 100;
    let initial = strategy.desired_quote_levels("BTC-USDT", Side::Buy, Some(quote(100.0)));
    strategy.now_ms = 110;
    let conflated = strategy.desired_quote_levels("BTC-USDT", Side::Buy, Some(quote(100.05)));
    strategy.now_ms = 130;
    let updated = strategy.desired_quote_levels("BTC-USDT", Side::Buy, Some(quote(100.05)));

    assert!(approx_eq(initial[0].price, 100.0));
    assert!(approx_eq(conflated[0].price, 100.0));
    assert!(approx_eq(updated[0].price, 100.05));
}

#[test]
fn java_parity_stops_after_six_consecutive_anomalous_fills() {
    let mut strategy = ChaosStrategy::new(config()).unwrap();
    strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(99.0, 10.0),
        Level::new(101.0, 10.0),
    ));
    for index in 0..6 {
        strategy.on_order_update(&OrderUpdate {
            ts_ms: 3_000 + index * 100,
            order_id: format!("q{index}"),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            event: OrderEvent::FullyFilled,
            status: OrderStatus::Filled,
            price: 100.0,
            time_in_force: Some(TimeInForce::PostOnly),
            qty: 0.01,
            open_qty: 0.0,
            filled_qty: 0.01,
            avg_fill_price: 98.0,
            last_fill_qty: 0.01,
            last_fill_price: 98.0,
            last_fill_liquidity: Some(FillLiquidity::Maker),
            last_fill_fee: None,
            reason: "quote".to_string(),
        });
        if index < 5 {
            assert!(strategy.halt_reason().is_none());
        }
    }
    assert!(
        strategy
            .halt_reason()
            .unwrap()
            .contains("consecutive anomalous fills")
    );
}

#[test]
fn java_parity_records_unfilled_ioc_hedge_delta() {
    let mut strategy = ChaosStrategy::new(config()).unwrap();
    strategy.entities.get_mut("BTC-USDT").unwrap().book = Some(OrderBook::one_level(
        "BTC-USDT",
        1,
        Level::new(99.0, 10.0),
        Level::new(101.0, 10.0),
    ));

    strategy.on_order_update(&OrderUpdate {
        ts_ms: 20,
        order_id: "hedge-1".to_string(),
        symbol: "BTC-PERP".to_string(),
        side: Side::Sell,
        event: OrderEvent::Cancelled,
        status: OrderStatus::Cancelled,
        price: 100.0,
        time_in_force: Some(TimeInForce::Ioc),
        qty: 1_000.0,
        open_qty: 0.0,
        filled_qty: 200.0,
        avg_fill_price: 100.0,
        last_fill_qty: 0.0,
        last_fill_price: 0.0,
        last_fill_liquidity: None,
        last_fill_fee: None,
        reason: "hedge:BTC-USDT:100".to_string(),
    });

    let missed = strategy.missed_hedges().last().unwrap();
    assert_eq!(missed.order_id, "hedge-1");
    assert_eq!(missed.missed_qty, 800.0);
    assert_eq!(missed.reference_symbol.as_deref(), Some("BTC-USDT"));
    assert!(missed.missed_delta_usd.is_finite() && missed.missed_delta_usd < 0.0);
}
