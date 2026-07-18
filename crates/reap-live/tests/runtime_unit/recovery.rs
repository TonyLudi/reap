use super::*;

#[test]
fn recovered_latches_are_validated_and_applied_fail_closed() {
    let config = config();
    let mut recovered = RecoveredStorage {
        global_safety_latch: Some(latch(SafetyLatchScope::Global, SafetyLatchSource::Risk)),
        ..RecoveredStorage::default()
    };
    recovered.account_safety_latches.insert(
        "main".to_string(),
        latch(
            SafetyLatchScope::Account {
                account_id: "main".to_string(),
            },
            SafetyLatchSource::Operator,
        ),
    );
    recovered.symbol_safety_latches.insert(
        "BTC-USDT".to_string(),
        latch(
            SafetyLatchScope::Symbol {
                symbol: "BTC-USDT".to_string(),
            },
            SafetyLatchSource::Operator,
        ),
    );
    validate_recovered_safety_latches(&config, &recovered).unwrap();
    assert_eq!(recovered_safety_latch_count(&recovered), 3);

    let mut coordinator = ready_coordinator(&config, unix_time_ms(), true);
    let outputs = restore_safety_latches(&mut coordinator, &recovered).unwrap();

    assert_eq!(outputs.len(), 3);
    assert!(coordinator.kill_switch_active());
    assert!(coordinator.halted_accounts().contains_key("main"));
    assert!(coordinator.is_symbol_halted("BTC-USDT"));
    assert!(!coordinator.readiness().is_ready());
    assert!(outputs.iter().all(|output| {
        output
            .records
            .iter()
            .all(|record| !matches!(record, StorageRecord::SafetyLatch(_)))
    }));
}

#[test]
fn recovered_latch_identity_must_match_live_config() {
    let config = config();
    let mut recovered = RecoveredStorage::default();
    recovered.account_safety_latches.insert(
        "removed".to_string(),
        latch(
            SafetyLatchScope::Account {
                account_id: "removed".to_string(),
            },
            SafetyLatchSource::Operator,
        ),
    );
    let error = validate_recovered_safety_latches(&config, &recovered).unwrap_err();
    assert!(error.to_string().contains("unknown account removed"));
}

#[test]
fn restart_restores_exchange_binding_for_active_order() {
    let config = config();
    let mut coordinator = ready_coordinator(&config, unix_time_ms(), true);
    coordinator
        .restore_owned_order(
            recovered_submit_proof("main", "BTC-USDT", "restored-live"),
            OrderUpdate {
                ts_ms: 2,
                order_id: "restored-live".to_string(),
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                event: OrderEvent::New,
                status: OrderStatus::Live,
                price: 100.0,
                time_in_force: Some(reap_core::TimeInForce::PostOnly),
                qty: 1.0,
                open_qty: 1.0,
                filled_qty: 0.0,
                avg_fill_price: 0.0,
                last_fill_qty: 0.0,
                last_fill_price: 0.0,
                last_fill_liquidity: None,
                last_fill_fee: None,
                reason: "restored quote".to_string(),
            },
        )
        .unwrap();
    let mut recovered = recover_storage_records([
        StorageRecord::OrderRequest(OrderRequestRecord {
            ts_ms: 1,
            account_id: "main".to_string(),
            operation: OrderOperation::Submit,
            idempotency_key: Some("decision-1".to_string()),
            client_order_id: Some("restored-live".to_string()),
            exchange_order_id: None,
            symbol: "BTC-USDT".to_string(),
        }),
        StorageRecord::OrderAck(reap_storage::OrderAckRecord {
            ts_ms: 2,
            account_id: "main".to_string(),
            operation: OrderOperation::Submit,
            client_order_id: "restored-live".to_string(),
            exchange_order_id: Some("exchange-1".to_string()),
            status: OrderAckStatus::Accepted,
            message: "accepted".to_string(),
        }),
    ]);

    restore_active_order_bindings(&mut coordinator, &mut recovered).unwrap();

    assert_eq!(
        coordinator
            .private_state("main")
            .unwrap()
            .canonical_order_id("exchange-1"),
        Some("restored-live")
    );
}

#[test]
fn recovered_order_binding_account_must_match_live_config() {
    let config = config();
    let mut coordinator = ready_coordinator(&config, unix_time_ms(), true);
    let mut recovered = recover_storage_records([
        StorageRecord::OrderRequest(OrderRequestRecord {
            ts_ms: 1,
            account_id: "removed".to_string(),
            operation: OrderOperation::Submit,
            idempotency_key: Some("decision-1".to_string()),
            client_order_id: Some("order-1".to_string()),
            exchange_order_id: None,
            symbol: "BTC-USDT".to_string(),
        }),
        StorageRecord::OrderAck(reap_storage::OrderAckRecord {
            ts_ms: 2,
            account_id: "removed".to_string(),
            operation: OrderOperation::Submit,
            client_order_id: "order-1".to_string(),
            exchange_order_id: Some("exchange-1".to_string()),
            status: OrderAckStatus::Accepted,
            message: "accepted".to_string(),
        }),
    ]);

    let error = restore_active_order_bindings(&mut coordinator, &mut recovered).unwrap_err();

    assert!(error.to_string().contains("unknown account removed"));
}

#[test]
fn restart_restores_only_orders_with_durable_regular_submit_proof() {
    let config = config();
    let update = OrderUpdate {
        ts_ms: 2,
        order_id: "foreign-or-legacy".to_string(),
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
        reason: "private observation".to_string(),
    };
    let mut unproven = recover_storage_records([StorageRecord::Order {
        account_id: Some("main".to_string()),
        update: update.clone(),
    }]);
    assert!(proven_active_recovered_orders(&config, &mut unproven).is_empty());

    let mut proven = recover_storage_records([
        StorageRecord::OrderRequest(OrderRequestRecord {
            ts_ms: 1,
            account_id: "main".to_string(),
            operation: OrderOperation::Submit,
            idempotency_key: Some("decision-owned".to_string()),
            client_order_id: Some(update.order_id.clone()),
            exchange_order_id: None,
            symbol: update.symbol.clone(),
        }),
        StorageRecord::Order {
            account_id: Some("main".to_string()),
            update: update.clone(),
        },
    ]);
    let restored = proven_active_recovered_orders(&config, &mut proven);
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].0.order_id, update.order_id);
    assert_eq!(restored[0].0.symbol, update.symbol);
    assert_eq!(restored[0].0.status, update.status);
    assert_eq!(restored[0].1.account_id(), "main");
    assert_eq!(restored[0].1.client_order_id(), update.order_id);
}

#[test]
fn recovered_account_latch_blocks_replay_and_cancels_restored_orders() {
    let config = config();
    let mut recovered = RecoveredStorage::default();
    recovered.account_safety_latches.insert(
        "main".to_string(),
        latch(
            SafetyLatchScope::Account {
                account_id: "main".to_string(),
            },
            SafetyLatchSource::Operator,
        ),
    );
    let mut coordinator = ready_coordinator(&config, unix_time_ms(), true);
    let _ = restore_safety_latches(&mut coordinator, &recovered).unwrap();
    let replay = coordinator
        .restore_owned_order(
            recovered_submit_proof("main", "BTC-USDT", "restored-live"),
            OrderUpdate {
                ts_ms: 2,
                order_id: "restored-live".to_string(),
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                event: OrderEvent::New,
                status: OrderStatus::Live,
                price: 100.0,
                time_in_force: Some(reap_core::TimeInForce::PostOnly),
                qty: 1.0,
                open_qty: 1.0,
                filled_qty: 0.0,
                avg_fill_price: 0.0,
                last_fill_qty: 0.0,
                last_fill_price: 0.0,
                last_fill_liquidity: None,
                last_fill_fee: None,
                reason: "restored quote".to_string(),
            },
        )
        .unwrap();

    assert!(
        replay
            .actions
            .iter()
            .all(|action| !matches!(action, LiveAction::Submit(_)))
    );
    let reapplied = restore_safety_latches(&mut coordinator, &recovered).unwrap();
    assert!(
        reapplied
            .iter()
            .flat_map(|output| &output.actions)
            .any(|action| matches!(
                action,
                LiveAction::Cancel(cancel) if cancel.client_order_id() == "restored-live"
            ))
    );
}
