use super::*;

#[test]
fn strategy_submit_is_registered_in_same_event_loop_turn() {
    let mut coordinator = coordinator();
    ready(&mut coordinator);
    let mut actions = Vec::new();
    for ts_ms in [1_000, 2_000] {
        for (symbol, bid, ask) in [("BTC-USDT", 99.9, 100.1), ("BTC-PERP", 99.8, 100.2)] {
            let output = coordinator.process_event(NormalizedEvent::Market(MarketEvent::Depth(
                OrderBook::one_level(
                    symbol,
                    ts_ms,
                    Level::new(bid, 100.0),
                    Level::new(ask, 100.0),
                ),
            )));
            actions.extend(output.actions);
        }
    }
    let submits = actions
        .iter()
        .filter_map(|action| match action {
            LiveAction::Submit(action) => Some(action),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(!submits.is_empty());
    for submit in submits {
        assert_eq!(
            coordinator
                .private_state(submit.account_id())
                .unwrap()
                .order_reducer()
                .get(submit.client_order_id())
                .unwrap()
                .status,
            OrderStatus::PendingNew
        );
    }
}

#[test]
fn restart_replays_missed_fill_and_terminal_order_before_reconciliation() {
    let mut coordinator = coordinator();
    let restored = OrderUpdate {
        ts_ms: 10,
        order_id: "client-1".to_string(),
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
    };
    restore_test_owned_order(&mut coordinator, "main", restored)
        .expect("checkpoint order should restore");
    let fill = RemoteFill {
        fill_id: "fill-1".to_string(),
        exchange_order_id: "exchange-1".to_string(),
        client_order_id: "client-1".to_string(),
        symbol: "BTC-USDT".to_string(),
        side: Side::Buy,
        price: 99.9,
        qty: 1.0,
        liquidity: FillLiquidity::Taker,
        fee: None,
        ts_ms: 11,
    };
    coordinator
        .process_feed(FeedOutput::PrivateFill {
            account_id: Some("main".to_string()),
            fill: fill.clone(),
        })
        .unwrap();
    coordinator
        .process_feed(FeedOutput::PrivateOrder {
            account_id: Some("main".to_string()),
            update: PrivateOrderUpdate {
                ts_ms: 12,
                exchange_order_id: "exchange-1".to_string(),
                client_order_id: "client-1".to_string(),
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                state: PrivateOrderState::Filled,
                price: 100.0,
                qty: 1.0,
                cumulative_filled_qty: 1.0,
                average_fill_price: 99.9,
                last_fill_qty: 0.0,
                last_fill_price: 0.0,
                liquidity: None,
                last_fill_fee: None,
                fill_id: None,
                reject_reason: String::new(),
            },
        })
        .unwrap();
    let remote = RemoteOrder {
        exchange_order_id: "exchange-1".to_string(),
        client_order_id: "client-1".to_string(),
        symbol: "BTC-USDT".to_string(),
        side: Side::Buy,
        state: PrivateOrderState::Filled,
        price: 100.0,
        qty: 1.0,
        cumulative_filled_qty: 1.0,
        average_fill_price: 99.9,
        update_time_ms: 12,
    };
    let state = coordinator.private_state("main").unwrap();
    let report = reconcile(
        state.order_reducer(),
        &HashSet::from([FillKey::new("BTC-USDT", "fill-1")]),
        &[remote],
        &[fill],
    );

    assert!(report.is_clean(), "{:?}", report.issues);
    assert_eq!(
        state.order_reducer().get("client-1").unwrap().status,
        OrderStatus::Filled
    );
}

#[test]
fn account_halt_cancels_and_blocks_only_the_target_account() {
    let mut coordinator = two_account_coordinator();
    ready_two_accounts(&mut coordinator);
    let mut main_order = order();
    main_order.reason = "external".to_string();
    let mut hedge_order = order();
    hedge_order.symbol = "BTC-PERP".to_string();
    hedge_order.qty = 1.0;
    hedge_order.reason = "external".to_string();
    coordinator
        .register_local_order("main", "client-main", main_order.clone(), 3)
        .unwrap();
    coordinator
        .register_local_order("hedge", "client-hedge", hedge_order.clone(), 3)
        .unwrap();

    let output = coordinator
        .halt_account(4, "main", "unexpected exposure")
        .unwrap();

    let cancels = output
        .actions
        .iter()
        .filter_map(|action| match action {
            LiveAction::Cancel(cancel) => Some((cancel.account_id(), cancel.client_order_id())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(cancels, vec![("main", "client-main")]);
    assert_eq!(
        coordinator
            .halted_accounts()
            .get("main")
            .map(String::as_str),
        Some("unexpected exposure")
    );
    assert!(output.records.iter().any(|record| matches!(
        record,
        StorageRecord::System(SystemEvent {
            kind: SystemEventKind::AccountHalted,
            account_id: Some(account_id),
            ..
        }) if account_id == "main"
    )));

    let mut blocked = CoordinatorOutput::default();
    coordinator.route_intent(5, OrderIntent::NewOrder(main_order), &mut blocked);
    assert!(blocked.actions.is_empty());
    assert!(blocked.records.iter().any(|record| matches!(
        record,
        StorageRecord::IntentRejected { reason, .. }
            if reason.contains("legacy serialized OrderIntent has no live execution authority")
    )));

    let mut healthy = CoordinatorOutput::default();
    coordinator.route_intent(5, OrderIntent::NewOrder(hedge_order), &mut healthy);
    assert!(healthy.actions.is_empty());
    assert!(healthy.records.iter().any(|record| matches!(
        record,
        StorageRecord::IntentRejected { reason, .. }
            if reason.contains("legacy serialized OrderIntent has no live execution authority")
    )));
}

#[test]
fn observe_mode_never_dispatches_gateway_cancels() {
    let mut coordinator = coordinator_with_gateway_actions(false);
    ready(&mut coordinator);
    coordinator
        .register_local_order("main", "client-1", order(), 3)
        .unwrap();
    let output = coordinator.process_event(NormalizedEvent::System(SystemEvent {
        ts_ms: 4,
        kind: SystemEventKind::KillSwitchActivated,
        venue: None,
        account_id: None,
        symbol: None,
        reason: "observe".to_string(),
    }));

    assert!(
        !output
            .actions
            .iter()
            .any(|action| matches!(action, LiveAction::Cancel(_)))
    );
    assert!(output.records.iter().any(|record| matches!(
        record,
        StorageRecord::IntentRejected { reason, .. }
            if reason.contains("observe mode")
    )));
}

#[test]
fn disabling_new_order_entry_preserves_kill_switch_cancels() {
    let mut coordinator = coordinator();
    ready(&mut coordinator);
    coordinator
        .register_local_order("main", "client-1", order(), 3)
        .unwrap();
    coordinator.set_order_entry_enabled(false);

    let output = coordinator.process_event(NormalizedEvent::System(SystemEvent {
        ts_ms: 4,
        kind: SystemEventKind::KillSwitchActivated,
        venue: None,
        account_id: None,
        symbol: None,
        reason: "shutdown".to_string(),
    }));

    assert!(output.actions.iter().any(|action| matches!(
        action,
        LiveAction::Cancel(cancel) if cancel.client_order_id() == "client-1"
    )));
    assert!(
        !output
            .actions
            .iter()
            .any(|action| matches!(action, LiveAction::Submit(_)))
    );
}
