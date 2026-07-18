use super::*;

#[test]
fn repeated_submit_rejections_persist_risk_latch_and_cancel_live_orders() {
    let mut coordinator = coordinator_with_risk(
        true,
        RiskLimits {
            require_feed_health: false,
            require_private_health: false,
            order_reject_count_limit: 2,
            order_reject_count_per_symbol_limit: 2,
            order_reject_window_ms: 60_000,
            ..RiskLimits::default()
        },
    );
    ready(&mut coordinator);
    restore_test_owned_order(
        &mut coordinator,
        "main",
        OrderUpdate {
            ts_ms: 2,
            order_id: "live-order".to_string(),
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
        },
    )
    .unwrap();
    coordinator
        .register_local_order("main", "reject-1", order(), 3)
        .unwrap();
    let first = coordinator
        .on_submit_error("main", "reject-1", 4, false, "exchange rejected")
        .unwrap();
    assert!(!coordinator.kill_switch_active());
    assert!(
        !first
            .records
            .iter()
            .any(|record| matches!(record, StorageRecord::SafetyLatch(_)))
    );

    coordinator
        .register_local_order("main", "reject-2", order(), 5)
        .unwrap();
    let second = coordinator
        .on_submit_error("main", "reject-2", 6, false, "exchange rejected")
        .unwrap();

    assert!(coordinator.kill_switch_active());
    assert!(second.records.iter().any(|record| matches!(
        record,
        StorageRecord::SafetyLatch(latch)
            if latch.active
                && latch.scope == SafetyLatchScope::Global
                && latch.source == SafetyLatchSource::Risk
                && latch.reason.contains("order rejection count 2 reached limit 2")
    )));
    assert!(second.actions.iter().any(|action| matches!(
        action,
        LiveAction::Cancel(cancel) if cancel.client_order_id() == "live-order"
    )));
}

#[test]
fn repeated_unfilled_ioc_cancels_persist_risk_latch_and_cancel_live_orders() {
    let mut coordinator = coordinator_with_risk(
        true,
        RiskLimits {
            require_feed_health: false,
            require_private_health: false,
            unfilled_ioc_cancel_count_per_symbol_limit: 2,
            unfilled_ioc_cancel_window_ms: 60_000,
            ..RiskLimits::default()
        },
    );
    ready(&mut coordinator);
    restore_test_owned_order(
        &mut coordinator,
        "main",
        OrderUpdate {
            ts_ms: 2,
            order_id: "live-order".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Sell,
            event: OrderEvent::New,
            status: OrderStatus::Live,
            price: 101.0,
            time_in_force: Some(TimeInForce::PostOnly),
            qty: 0.1,
            open_qty: 0.1,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            last_fill_fee: None,
            reason: "quote".to_string(),
        },
    )
    .unwrap();

    let mut ioc = order();
    ioc.time_in_force = TimeInForce::Ioc;
    ioc.reason = "hedge:BTC-USDT:100".to_string();
    coordinator
        .register_local_order("main", "ioc-1", ioc.clone(), 3)
        .unwrap();
    let first_cancel = cancelled_private_order("ioc-1", "exchange-ioc-1", 4);
    let first = coordinator
        .process_feed(FeedOutput::PrivateOrder {
            account_id: Some("main".to_string()),
            update: first_cancel.clone(),
        })
        .unwrap();
    assert!(!coordinator.kill_switch_active());
    assert!(first.records.iter().any(|record| matches!(
        record,
        StorageRecord::Order { update, .. }
            if update.order_id == "ioc-1"
                && update.status == OrderStatus::Cancelled
                && update.time_in_force == Some(TimeInForce::Ioc)
    )));
    let duplicate = coordinator
        .process_feed(FeedOutput::PrivateOrder {
            account_id: Some("main".to_string()),
            update: first_cancel,
        })
        .unwrap();
    assert!(!coordinator.kill_switch_active());
    assert!(
        !duplicate
            .records
            .iter()
            .any(|record| matches!(record, StorageRecord::SafetyLatch(_)))
    );

    coordinator
        .register_local_order("main", "ioc-2", ioc, 5)
        .unwrap();
    let second = coordinator
        .process_feed(FeedOutput::PrivateOrder {
            account_id: Some("main".to_string()),
            update: cancelled_private_order("ioc-2", "exchange-ioc-2", 6),
        })
        .unwrap();

    assert!(coordinator.kill_switch_active());
    assert!(second.records.iter().any(|record| matches!(
        record,
        StorageRecord::SafetyLatch(latch)
            if latch.active
                && latch.scope == SafetyLatchScope::Global
                && latch.source == SafetyLatchSource::Risk
                && latch.reason.contains(
                    "symbol BTC-USDT unfilled IOC cancellation count 2 reached limit 2"
                )
    )));
    assert!(second.actions.iter().any(|action| matches!(
        action,
        LiveAction::Cancel(cancel) if cancel.client_order_id() == "live-order"
    )));
}

#[test]
fn chaos_internal_halt_persists_risk_latch_and_cancels_live_orders() {
    let mut coordinator = coordinator_with_risk(
        true,
        RiskLimits {
            max_abs_position_notional_usd: 1_000_000_000.0,
            require_feed_health: false,
            require_private_health: false,
            ..RiskLimits::default()
        },
    );
    ready(&mut coordinator);
    coordinator.set_order_entry_enabled(false);
    restore_test_owned_order(
        &mut coordinator,
        "main",
        OrderUpdate {
            ts_ms: 2,
            order_id: "live-order".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Sell,
            event: OrderEvent::New,
            status: OrderStatus::Live,
            price: 101.0,
            time_in_force: Some(TimeInForce::PostOnly),
            qty: 0.1,
            open_qty: 0.1,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            last_fill_fee: None,
            reason: "quote".to_string(),
        },
    )
    .unwrap();
    for symbol in ["BTC-USDT", "BTC-PERP"] {
        coordinator.process_event(NormalizedEvent::Market(MarketEvent::Depth(
            OrderBook::one_level(
                symbol,
                3,
                Level::new(99.0, 10_000.0),
                Level::new(101.0, 10_000.0),
            ),
        )));
    }
    assert!(!coordinator.kill_switch_active());

    let output = coordinator.process_event(NormalizedEvent::Account(AccountUpdate {
        ts_ms: 4,
        balances: Vec::new(),
        positions: vec![Position {
            symbol: "BTC-USDT".to_string(),
            qty: 1_000.0,
            avg_price: 100.0,
            margin_mode: None,
        }],
        margins: Vec::new(),
    }));

    assert!(coordinator.kill_switch_active());
    assert!(output.records.iter().any(|record| matches!(
        record,
        StorageRecord::SafetyLatch(latch)
            if latch.active
                && latch.scope == SafetyLatchScope::Global
                && latch.source == SafetyLatchSource::Risk
                && latch.reason.contains("strategy halted: strategy delta")
    )));
    assert!(output.actions.iter().any(|action| matches!(
        action,
        LiveAction::Cancel(cancel) if cancel.client_order_id() == "live-order"
    )));
}

#[test]
fn fill_convergence_fault_cancels_live_orders_and_reconciles_account() {
    let mut coordinator = coordinator();
    ready(&mut coordinator);
    coordinator
        .register_local_order("main", "client-1", order(), 3)
        .unwrap();
    coordinator
        .process_feed(FeedOutput::PrivateOrder {
            account_id: Some("main".to_string()),
            update: PrivateOrderUpdate {
                ts_ms: 4,
                exchange_order_id: "exchange-1".to_string(),
                client_order_id: "client-1".to_string(),
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                state: PrivateOrderState::Live,
                price: 100.0,
                qty: 0.1,
                cumulative_filled_qty: 0.0,
                average_fill_price: 0.0,
                last_fill_qty: 0.0,
                last_fill_price: 0.0,
                liquidity: None,
                last_fill_fee: None,
                fill_id: None,
                reject_reason: String::new(),
            },
        })
        .unwrap();

    let output = coordinator
        .reconciliation_fault(
            "main",
            5,
            Some("BTC-USDT".to_string()),
            "fill-to-account-state convergence exceeded 2000ms".to_string(),
        )
        .unwrap();

    assert!(!coordinator.readiness().is_ready());
    assert!(output.actions.iter().any(|action| matches!(
        action,
        LiveAction::Cancel(cancel) if cancel.client_order_id() == "client-1"
    )));
    assert!(output.actions.iter().any(|action| matches!(
        action,
        LiveAction::Reconcile(reconcile) if reconcile.account_id == "main"
    )));
    assert!(output.records.iter().any(|record| matches!(
        record,
        StorageRecord::System(event)
            if event.kind == SystemEventKind::ReconcileDrift
                && event.account_id.as_deref() == Some("main")
    )));
}

#[test]
fn forbidden_state_blocks_placement_cancels_owned_regular_and_requires_clean_reconcile() {
    let mut coordinator = coordinator();
    ready(&mut coordinator);
    restore_test_owned_order(
        &mut coordinator,
        "main",
        OrderUpdate {
            ts_ms: 2,
            order_id: "live-order".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Sell,
            event: OrderEvent::New,
            status: OrderStatus::Live,
            price: 101.0,
            time_in_force: Some(TimeInForce::PostOnly),
            qty: 0.1,
            open_qty: 0.1,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            last_fill_fee: None,
            reason: "quote".to_string(),
        },
    )
    .unwrap();

    let output = coordinator
        .on_forbidden_order_event(ForbiddenOrderEvent {
            account_id: "main".to_string(),
            observed_at_ms: 3,
            state: ForbiddenOrderState::NonZero {
                algo_orders_observed: Some(1),
                spread_orders_observed: Some(0),
            },
        })
        .unwrap();
    assert!(!coordinator.readiness().is_ready());
    assert!(output.actions.iter().any(|action| matches!(
        action,
        LiveAction::Cancel(cancel) if cancel.client_order_id() == "live-order"
    )));
    assert!(output.actions.iter().any(|action| matches!(
        action,
        LiveAction::Reconcile(reconcile) if reconcile.account_id == "main"
    )));

    coordinator
        .on_forbidden_order_event(ForbiddenOrderEvent {
            account_id: "main".to_string(),
            observed_at_ms: 4,
            state: ForbiddenOrderState::VerifiedZero {
                expires_at_ms: 30_004,
            },
        })
        .unwrap();
    assert!(
        !coordinator.readiness().is_ready(),
        "a fresh zero proof must not bypass the clean-reconciliation requirement"
    );
    coordinator
        .on_reconciliation(ReconciliationResult {
            account_id: "main".to_string(),
            ts_ms: 5,
            clean: true,
            local_live_orders: 1,
            remote_live_orders: 1,
            remote_recent_fills: 0,
            reason: "regular state is clean".to_string(),
        })
        .unwrap();
    assert!(coordinator.readiness().is_ready());
}
