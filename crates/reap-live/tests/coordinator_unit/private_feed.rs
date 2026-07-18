use super::*;

#[test]
fn registration_is_canonical_before_rest_result() {
    let mut coordinator = coordinator();
    ready(&mut coordinator);
    let output = coordinator
        .register_local_order("main", "client-1", order(), 3)
        .unwrap();

    assert_eq!(
        coordinator
            .private_state("main")
            .unwrap()
            .order_reducer()
            .get("client-1")
            .unwrap()
            .status,
        OrderStatus::PendingNew
    );
    assert!(output.records.iter().any(
            |record| matches!(record, StorageRecord::Order { update, .. } if update.order_id == "client-1")
        ));
}

#[test]
fn explicit_submit_failure_is_terminal_but_ambiguous_failure_degrades() {
    let mut explicit = coordinator();
    ready(&mut explicit);
    explicit
        .register_local_order("main", "client-1", order(), 3)
        .unwrap();
    explicit
        .on_submit_error("main", "client-1", 4, false, "rejected")
        .unwrap();
    assert_eq!(
        explicit
            .private_state("main")
            .unwrap()
            .order_reducer()
            .get("client-1")
            .unwrap()
            .status,
        OrderStatus::Rejected
    );

    let mut ambiguous = coordinator();
    ready(&mut ambiguous);
    ambiguous
        .register_local_order("main", "client-2", order(), 3)
        .unwrap();
    let output = ambiguous
        .on_submit_error("main", "client-2", 4, true, "timeout")
        .unwrap();
    assert!(!ambiguous.readiness().is_ready());
    assert!(output.actions.iter().any(
        |action| matches!(action, LiveAction::Reconcile(action) if action.account_id == "main")
    ));
}

#[test]
fn submit_ack_binding_resolves_missing_private_client_ids() {
    let mut coordinator = coordinator();
    ready(&mut coordinator);
    coordinator
        .register_local_order("main", "client-1", order(), 3)
        .unwrap();
    coordinator
        .on_submit_outcome(
            "main",
            SubmitOutcome::Submitted {
                client_order_id: "client-1".to_string(),
                exchange_order_id: "exchange-1".to_string(),
            },
            4,
        )
        .unwrap();
    let private_order = coordinator
        .process_feed(FeedOutput::PrivateOrder {
            account_id: Some("main".to_string()),
            update: PrivateOrderUpdate {
                ts_ms: 5,
                exchange_order_id: "exchange-1".to_string(),
                client_order_id: String::new(),
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

    assert!(private_order.records.iter().any(|record| matches!(
        record,
        StorageRecord::Order { update, .. } if update.order_id == "client-1"
    )));
    assert!(
        !private_order
            .actions
            .iter()
            .any(|action| matches!(action, LiveAction::Reconcile(_)))
    );
    let fill = coordinator
        .process_feed(FeedOutput::PrivateFill {
            account_id: Some("main".to_string()),
            fill: RemoteFill {
                fill_id: "fill-1".to_string(),
                exchange_order_id: "exchange-1".to_string(),
                client_order_id: "0".to_string(),
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                price: 100.0,
                qty: 0.05,
                liquidity: FillLiquidity::Taker,
                fee: Some(FillFee {
                    amount: -0.005,
                    currency: "USDT".to_string(),
                }),
                ts_ms: 6,
            },
        })
        .unwrap();

    assert!(fill.records.iter().any(|record| matches!(
        record,
        StorageRecord::Fill(fill)
            if fill.order_id == "client-1"
                && fill.fee.as_ref().is_some_and(|fee|
                    fee.amount == -0.005 && fee.currency == "USDT")
    )));
    assert!(
        !fill
            .actions
            .iter()
            .any(|action| matches!(action, LiveAction::Reconcile(_)))
    );
    let state = coordinator.private_state("main").unwrap();
    assert_eq!(state.order_reducer().orders().count(), 1);
    assert!(state.order_reducer().get("exchange-1").is_none());
}

#[test]
fn order_channel_fill_is_persisted_once_across_private_channels() {
    let mut coordinator = coordinator();
    ready(&mut coordinator);
    coordinator
        .register_local_order("main", "client-1", order(), 3)
        .unwrap();
    coordinator
        .on_submit_outcome(
            "main",
            SubmitOutcome::Submitted {
                client_order_id: "client-1".to_string(),
                exchange_order_id: "exchange-1".to_string(),
            },
            4,
        )
        .unwrap();

    let order_fill = coordinator
        .process_feed(FeedOutput::PrivateOrder {
            account_id: Some("main".to_string()),
            update: PrivateOrderUpdate {
                ts_ms: 5,
                exchange_order_id: "exchange-1".to_string(),
                client_order_id: "client-1".to_string(),
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                state: PrivateOrderState::PartiallyFilled,
                price: 100.0,
                qty: 0.1,
                cumulative_filled_qty: 0.05,
                average_fill_price: 100.0,
                last_fill_qty: 0.05,
                last_fill_price: 100.0,
                liquidity: None,
                last_fill_fee: Some(FillFee {
                    amount: -0.005,
                    currency: "USDT".to_string(),
                }),
                fill_id: Some("fill-1".to_string()),
                reject_reason: String::new(),
            },
        })
        .unwrap();

    let persisted = order_fill
        .records
        .iter()
        .filter_map(|record| match record {
            StorageRecord::Fill(fill) => Some(fill),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].fill_id, "fill-1");
    assert_eq!(persisted[0].order_id, "client-1");
    assert_eq!(persisted[0].liquidity, None);
    assert_eq!(
        persisted[0].fee,
        Some(FillFee {
            amount: -0.005,
            currency: "USDT".to_string(),
        })
    );

    let duplicate = coordinator
        .process_feed(FeedOutput::PrivateFill {
            account_id: Some("main".to_string()),
            fill: RemoteFill {
                fill_id: "fill-1".to_string(),
                exchange_order_id: "exchange-1".to_string(),
                client_order_id: "client-1".to_string(),
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                price: 100.0,
                qty: 0.05,
                liquidity: FillLiquidity::Maker,
                fee: Some(FillFee {
                    amount: -0.005,
                    currency: "USDT".to_string(),
                }),
                ts_ms: 6,
            },
        })
        .unwrap();

    assert!(
        duplicate
            .records
            .iter()
            .all(|record| !matches!(record, StorageRecord::Fill(_)))
    );
    assert_eq!(
        coordinator
            .private_state("main")
            .unwrap()
            .order_reducer()
            .get("client-1")
            .unwrap()
            .filled_qty,
        0.05
    );
}

#[test]
fn fee_less_fill_channel_does_not_hide_later_exact_order_fill() {
    let mut coordinator = coordinator();
    ready(&mut coordinator);
    coordinator
        .register_local_order("main", "client-1", order(), 3)
        .unwrap();
    coordinator
        .on_submit_outcome(
            "main",
            SubmitOutcome::Submitted {
                client_order_id: "client-1".to_string(),
                exchange_order_id: "exchange-1".to_string(),
            },
            4,
        )
        .unwrap();

    let early_fill = coordinator
        .process_feed(FeedOutput::PrivateFill {
            account_id: Some("main".to_string()),
            fill: RemoteFill {
                fill_id: "fill-1".to_string(),
                exchange_order_id: "exchange-1".to_string(),
                client_order_id: "client-1".to_string(),
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                price: 100.0,
                qty: 0.05,
                liquidity: FillLiquidity::Maker,
                fee: None,
                ts_ms: 5,
            },
        })
        .unwrap();
    assert!(
        early_fill
            .records
            .iter()
            .all(|record| !matches!(record, StorageRecord::Fill(_)))
    );

    let exact_fill = coordinator
        .process_feed(FeedOutput::PrivateOrder {
            account_id: Some("main".to_string()),
            update: PrivateOrderUpdate {
                ts_ms: 6,
                exchange_order_id: "exchange-1".to_string(),
                client_order_id: "client-1".to_string(),
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                state: PrivateOrderState::PartiallyFilled,
                price: 100.0,
                qty: 0.1,
                cumulative_filled_qty: 0.05,
                average_fill_price: 100.0,
                last_fill_qty: 0.05,
                last_fill_price: 100.0,
                liquidity: Some(FillLiquidity::Maker),
                last_fill_fee: Some(FillFee {
                    amount: -0.005,
                    currency: "USDT".to_string(),
                }),
                fill_id: Some("fill-1".to_string()),
                reject_reason: String::new(),
            },
        })
        .unwrap();

    let persisted = exact_fill
        .records
        .iter()
        .filter_map(|record| match record {
            StorageRecord::Fill(fill) => Some(fill),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].fill_id, "fill-1");
    assert_eq!(persisted[0].order_id, "client-1");
    assert_eq!(persisted[0].liquidity, Some(FillLiquidity::Maker));
    assert_eq!(
        persisted[0].fee,
        Some(FillFee {
            amount: -0.005,
            currency: "USDT".to_string(),
        })
    );
    assert_eq!(
        coordinator
            .private_state("main")
            .unwrap()
            .order_reducer()
            .get("client-1")
            .unwrap()
            .filled_qty,
        0.05
    );
}

#[test]
fn wrong_account_private_order_and_fill_fail_before_state_mutation() {
    let mut coordinator = two_account_coordinator();
    ready_two_accounts(&mut coordinator);
    let order_error = coordinator
        .process_feed(FeedOutput::PrivateOrder {
            account_id: Some("hedge".to_string()),
            update: cancelled_private_order("wrong-order", "wrong-exchange", 3),
        })
        .unwrap_err();
    assert!(matches!(
        order_error,
        CoordinatorError::WrongOrderAccount {
            actual,
            expected,
            ..
        } if actual == "hedge" && expected == "main"
    ));
    assert!(
        !coordinator
            .private_state("hedge")
            .unwrap()
            .order_reducer()
            .contains_order("wrong-order")
    );

    let fill_error = coordinator
        .process_feed(FeedOutput::PrivateFill {
            account_id: Some("hedge".to_string()),
            fill: RemoteFill {
                fill_id: "wrong-fill".to_string(),
                exchange_order_id: "wrong-exchange".to_string(),
                client_order_id: "wrong-order".to_string(),
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                price: 100.0,
                qty: 0.1,
                liquidity: FillLiquidity::Taker,
                fee: None,
                ts_ms: 4,
            },
        })
        .unwrap_err();
    assert!(matches!(
        fill_error,
        CoordinatorError::WrongOrderAccount { actual, .. } if actual == "hedge"
    ));
    assert!(
        !coordinator
            .private_state("hedge")
            .unwrap()
            .has_seen_fill("BTC-USDT", "wrong-fill")
    );
}

#[test]
fn unproven_private_orders_are_observed_but_never_become_cancel_authority() {
    for foreign_id in ["reap-prefix-foreign", "algo-order-7", "spread-order-9"] {
        let mut coordinator = coordinator();
        ready(&mut coordinator);

        let output = coordinator
            .process_feed(FeedOutput::PrivateOrder {
                account_id: Some("main".to_string()),
                update: PrivateOrderUpdate {
                    ts_ms: 4,
                    exchange_order_id: format!("exchange-{foreign_id}"),
                    client_order_id: foreign_id.to_string(),
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

        assert!(
            coordinator
                .private_state("main")
                .unwrap()
                .order_reducer()
                .contains_order(foreign_id),
            "foreign exposure must remain observable"
        );
        assert!(!coordinator.readiness().is_ready());
        assert!(output.actions.iter().all(|action| {
            !matches!(action, LiveAction::Cancel(cancel) if cancel.client_order_id() == foreign_id)
        }));
        assert!(output.actions.iter().any(|action| {
            matches!(action, LiveAction::Reconcile(reconcile) if reconcile.account_id == "main")
        }));
        assert!(
            coordinator.readiness().faults.keys().any(|fault| {
                fault == &format!("runtime:foreign_regular_order:main:{foreign_id}")
            })
        );

        let stale_terminal = coordinator
            .process_feed(FeedOutput::PrivateOrder {
                account_id: Some("main".to_string()),
                update: cancelled_private_order(foreign_id, &format!("exchange-{foreign_id}"), 3),
            })
            .unwrap();
        assert!(stale_terminal.actions.is_empty());
        assert_eq!(
            coordinator
                .private_state("main")
                .unwrap()
                .order_reducer()
                .get(foreign_id)
                .unwrap()
                .status,
            OrderStatus::Live
        );
        coordinator
            .on_reconciliation(ReconciliationResult {
                account_id: "main".to_string(),
                ts_ms: 5,
                clean: true,
                local_live_orders: 1,
                remote_live_orders: 1,
                remote_recent_fills: 0,
                reason: "foreign order still requires operator handling".to_string(),
            })
            .unwrap();
        assert!(!coordinator.readiness().is_ready());
        assert!(
            coordinator.readiness().faults.keys().any(|fault| {
                fault == &format!("runtime:foreign_regular_order:main:{foreign_id}")
            })
        );

        let safety = coordinator.process_event(NormalizedEvent::System(SystemEvent {
            ts_ms: 6,
            kind: SystemEventKind::KillSwitchActivated,
            venue: None,
            account_id: None,
            symbol: None,
            reason: "test fail-closed cancellation".to_string(),
        }));
        assert!(safety.actions.iter().all(|action| {
            !matches!(action, LiveAction::Cancel(cancel) if cancel.client_order_id() == foreign_id)
        }));
        assert!(safety.records.iter().any(|record| matches!(
            record,
            StorageRecord::IntentRejected { reason, .. }
                if reason.contains("not a proven owned regular order")
        )));
    }
}

#[test]
fn known_order_identity_mismatch_fails_before_mapping_or_fill_mutation() {
    let mut coordinator = coordinator();
    coordinator
        .register_local_order("main", "client-1", order(), 3)
        .unwrap();
    let order_error = coordinator
        .process_feed(FeedOutput::PrivateOrder {
            account_id: Some("main".to_string()),
            update: PrivateOrderUpdate {
                ts_ms: 4,
                exchange_order_id: "exchange-1".to_string(),
                client_order_id: "client-1".to_string(),
                symbol: "BTC-PERP".to_string(),
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
        .unwrap_err();
    assert!(matches!(
        order_error,
        CoordinatorError::PrivateOrderIdentity {
            source: PrivateOrderIdentityError::SymbolMismatch { .. },
            ..
        }
    ));
    assert!(
        coordinator
            .private_state("main")
            .unwrap()
            .canonical_order_id("exchange-1")
            .is_none()
    );

    let fill_error = coordinator
        .process_feed(FeedOutput::PrivateFill {
            account_id: Some("main".to_string()),
            fill: RemoteFill {
                fill_id: "fill-wrong-side".to_string(),
                exchange_order_id: String::new(),
                client_order_id: "client-1".to_string(),
                symbol: "BTC-USDT".to_string(),
                side: Side::Sell,
                price: 100.0,
                qty: 0.05,
                liquidity: FillLiquidity::Taker,
                fee: None,
                ts_ms: 5,
            },
        })
        .unwrap_err();
    assert!(matches!(
        fill_error,
        CoordinatorError::PrivateOrderIdentity {
            source: PrivateOrderIdentityError::SideMismatch { .. },
            ..
        }
    ));
    assert!(
        !coordinator
            .private_state("main")
            .unwrap()
            .has_seen_fill("BTC-USDT", "fill-wrong-side")
    );
}
