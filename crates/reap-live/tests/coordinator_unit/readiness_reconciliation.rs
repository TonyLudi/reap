use super::*;

#[test]
fn account_snapshot_is_ready_only_after_the_engine_consumes_it() {
    let mut coordinator = coordinator();
    assert_eq!(
        coordinator.readiness().missing_account_snapshots,
        vec!["main".to_string()]
    );
    assert!(
        coordinator
            .private_state("main")
            .is_some_and(|state| !state.balances().is_empty())
    );

    coordinator.mark_storage_ready(true, "open");
    coordinator.mark_public_connectivity(true, "connected");
    coordinator
        .on_reconciliation(ReconciliationResult {
            account_id: "main".to_string(),
            ts_ms: 2,
            clean: true,
            local_live_orders: 0,
            remote_live_orders: 0,
            remote_recent_fills: 0,
            reason: "clean".to_string(),
        })
        .unwrap();
    for symbol in ["BTC-USDT", "BTC-PERP"] {
        coordinator.process_event(NormalizedEvent::System(SystemEvent {
            ts_ms: 2,
            kind: SystemEventKind::FeedRecovered,
            venue: Some(Venue::Okx),
            account_id: None,
            symbol: Some(symbol.to_string()),
            reason: "snapshot".to_string(),
        }));
    }
    coordinator.process_event(NormalizedEvent::System(SystemEvent {
        ts_ms: 2,
        kind: SystemEventKind::PrivateStreamRecovered,
        venue: Some(Venue::Okx),
        account_id: Some("main".to_string()),
        symbol: None,
        reason: "connected".to_string(),
    }));
    coordinator.process_event(NormalizedEvent::System(SystemEvent {
        ts_ms: 2,
        kind: SystemEventKind::OrderTransportRecovered,
        venue: Some(Venue::Okx),
        account_id: Some("main".to_string()),
        symbol: None,
        reason: "all sessions authenticated".to_string(),
    }));
    coordinator
        .startup
        .mark_forbidden_order_proof("main", true, "complete zero proof")
        .unwrap();
    seed_strategy_references(&mut coordinator, 2);

    assert!(!coordinator.readiness().is_ready());
    assert_eq!(coordinator.readiness().phase, crate::LivePhase::Reconciling);

    coordinator
        .process_feed(FeedOutput::PrivateAccount {
            account_id: Some("main".to_string()),
            update: account_update("main", 3),
        })
        .unwrap();

    assert!(coordinator.readiness().is_ready());
    assert!(coordinator.readiness().missing_account_snapshots.is_empty());
}

#[test]
fn position_margin_mode_drift_is_rejected_before_state_application() {
    let mut coordinator = coordinator();
    let wrong_mode = AccountUpdate {
        ts_ms: 3,
        balances: Vec::new(),
        positions: vec![Position {
            symbol: "BTC-PERP".to_string(),
            qty: 2.0,
            avg_price: 50_000.0,
            margin_mode: Some(PositionMarginMode::Isolated),
        }],
        margins: Vec::new(),
    };

    let error = coordinator
        .process_feed(FeedOutput::PrivateAccount {
            account_id: Some("main".to_string()),
            update: wrong_mode.clone(),
        })
        .unwrap_err();

    assert!(matches!(
        error,
        CoordinatorError::AccountStatePolicy { ref account_id, ref message }
            if account_id == "main"
                && message.contains("BTC-PERP expected Cross, received Isolated")
    ));
    assert!(
        coordinator
            .private_state("main")
            .unwrap()
            .positions()
            .is_empty()
    );
    assert!(matches!(
        coordinator.apply_authoritative_account_snapshot("main", wrong_mode),
        Err(CoordinatorError::AccountStatePolicy { .. })
    ));

    coordinator
        .process_feed(FeedOutput::PrivateAccount {
            account_id: Some("main".to_string()),
            update: AccountUpdate {
                ts_ms: 4,
                balances: Vec::new(),
                positions: vec![Position {
                    symbol: "BTC-PERP".to_string(),
                    qty: 0.0,
                    avg_price: 0.0,
                    margin_mode: Some(PositionMarginMode::Isolated),
                }],
                margins: Vec::new(),
            },
        })
        .unwrap();

    let error = coordinator
        .process_feed(FeedOutput::PrivateAccount {
            account_id: Some("main".to_string()),
            update: AccountUpdate {
                ts_ms: 5,
                balances: Vec::new(),
                positions: vec![Position {
                    symbol: "ETH-USDT-SWAP".to_string(),
                    qty: 1.0,
                    avg_price: 3_000.0,
                    margin_mode: Some(PositionMarginMode::Cross),
                }],
                margins: Vec::new(),
            },
        })
        .unwrap_err();
    assert!(matches!(
        error,
        CoordinatorError::AccountStatePolicy { ref message, .. }
            if message.contains("unmanaged nonzero position ETH-USDT-SWAP qty=1")
    ));

    coordinator
        .process_feed(FeedOutput::PrivateAccount {
            account_id: Some("main".to_string()),
            update: AccountUpdate {
                ts_ms: 6,
                balances: Vec::new(),
                positions: vec![Position {
                    symbol: "ETH-USDT-SWAP".to_string(),
                    qty: 0.0,
                    avg_price: 0.0,
                    margin_mode: Some(PositionMarginMode::Isolated),
                }],
                margins: Vec::new(),
            },
        })
        .unwrap();

    let mut borrowed = account_update("main", 7);
    borrowed.balances[0].liability = 0.01;
    let error = coordinator
        .process_feed(FeedOutput::PrivateAccount {
            account_id: Some("main".to_string()),
            update: borrowed,
        })
        .unwrap_err();
    assert!(matches!(
        error,
        CoordinatorError::AccountStatePolicy { ref message, .. }
            if message.contains("liability 0.01 is nonzero")
    ));

    let mut forced_repayment = account_update("main", 8);
    forced_repayment.balances[0].total = 9_000.0;
    forced_repayment.balances[0].forced_repayment_indicator = Some(1);
    let error = coordinator
        .process_feed(FeedOutput::PrivateAccount {
            account_id: Some("main".to_string()),
            update: forced_repayment,
        })
        .unwrap_err();
    assert!(matches!(
        error,
        CoordinatorError::AccountStatePolicy { ref message, .. }
            if message.contains(
                "currency USDT forced repayment indicator 1 reached limit 1"
            )
    ));
    assert_eq!(
        coordinator.private_state("main").unwrap().balances()["USDT"].total,
        10_000.0
    );
}

#[test]
fn post_connect_reconciliation_blocks_readiness_until_clean() {
    let mut coordinator = coordinator();
    ready(&mut coordinator);

    let output = coordinator
        .require_reconciliation("main", 3, "private stream connected")
        .unwrap();

    assert!(!coordinator.readiness().is_ready());
    assert!(
        coordinator
            .readiness()
            .missing_reconciliation
            .contains(&"main".to_string())
    );
    assert!(output.actions.iter().any(
        |action| matches!(action, LiveAction::Reconcile(action) if action.account_id == "main")
    ));

    coordinator
        .on_reconciliation(ReconciliationResult {
            account_id: "main".to_string(),
            ts_ms: 4,
            clean: true,
            local_live_orders: 0,
            remote_live_orders: 0,
            remote_recent_fills: 0,
            reason: "post-connect REST state is clean".to_string(),
        })
        .unwrap();
    assert!(coordinator.readiness().is_ready());
}

#[test]
fn retained_reference_frame_is_aged_and_cancelled_while_already_degraded() {
    let mut coordinator = coordinator();
    ready(&mut coordinator);
    coordinator
        .register_local_order("main", "reference-q1", order(), 3)
        .unwrap();
    coordinator
        .startup
        .mark_runtime_health("test_fault", false, "already degraded");
    assert_eq!(coordinator.readiness().phase, crate::LivePhase::Degraded);
    assert!(coordinator.startup.strategy_references_ready());

    let output = coordinator
        .process_feed_at(
            FeedOutput::Event(NormalizedEvent::Market(MarketEvent::PriceLimits {
                ts_ms: 3,
                symbol: "BTC-USDT".to_string(),
                mark_price: 0.0,
                limit_down: 50.0,
                limit_up: 150.0,
            })),
            120_004,
        )
        .unwrap();

    let readiness = coordinator.readiness();
    assert_eq!(readiness.phase, crate::LivePhase::Degraded);
    assert!(
        readiness
            .missing_strategy_references
            .contains(&"price_limits:BTC-USDT".to_string())
    );
    assert!(
        readiness
            .faults
            .contains_key("strategy_reference:price_limits:BTC-USDT")
    );
    assert!(output.actions.iter().any(|action| {
        matches!(action, LiveAction::Cancel(cancel) if cancel.client_order_id() == "reference-q1")
    }));
}

#[test]
fn order_transport_loss_blocks_entry_cancels_and_requires_reconciliation() {
    let mut coordinator = coordinator();
    ready(&mut coordinator);
    coordinator
        .register_local_order("main", "client-1", order(), 3)
        .unwrap();

    let output = coordinator.process_event(NormalizedEvent::System(SystemEvent {
        ts_ms: 4,
        kind: SystemEventKind::OrderTransportStale,
        venue: Some(Venue::Okx),
        account_id: Some("main".to_string()),
        symbol: None,
        reason: "session 3 disconnected".to_string(),
    }));

    let readiness = coordinator.readiness();
    assert_eq!(readiness.phase, crate::LivePhase::Degraded);
    assert_eq!(readiness.missing_order_transports, vec!["main"]);
    assert_eq!(readiness.missing_reconciliation, vec!["main"]);
    assert!(output.actions.iter().any(|action| matches!(
        action,
        LiveAction::Cancel(cancel) if cancel.client_order_id() == "client-1"
    )));
    assert!(output.actions.iter().any(|action| matches!(
        action,
        LiveAction::Reconcile(reconcile) if reconcile.account_id == "main"
    )));

    let mut blocked = CoordinatorOutput::default();
    coordinator.route_intent(5, OrderIntent::NewOrder(order()), &mut blocked);
    assert!(blocked.actions.is_empty());
}

#[test]
fn authoritative_reconciliation_clears_closed_position_before_clean_retry() {
    let mut coordinator = coordinator();
    ready(&mut coordinator);
    coordinator
        .process_feed(FeedOutput::PrivateAccount {
            account_id: Some("main".to_string()),
            update: AccountUpdate {
                ts_ms: 3,
                balances: account_update("main", 3).balances,
                positions: vec![Position {
                    symbol: "BTC-PERP".to_string(),
                    qty: 4.0,
                    avg_price: 50_000.0,
                    margin_mode: Some(PositionMarginMode::Cross),
                }],
                margins: Vec::new(),
            },
        })
        .unwrap();
    assert_eq!(
        coordinator
            .engine
            .strategy()
            .entity("BTC-PERP")
            .unwrap()
            .position_qty,
        4.0
    );

    coordinator
        .require_reconciliation("main", 4, "private stream recovered")
        .unwrap();
    let remote_account = account_update("main", 5);
    let first = reconcile_full_state(
        coordinator.private_state("main").unwrap(),
        &[],
        &[],
        &remote_account,
    );
    assert!(first.issues.iter().any(|issue| matches!(
        issue,
        ReconcileIssue::PositionMissingRemote { symbol, .. } if symbol == "BTC-PERP"
    )));

    let snapshot_output = coordinator
        .apply_authoritative_account_snapshot("main", remote_account.clone())
        .unwrap();
    assert!(matches!(
        snapshot_output.records.first(),
        Some(StorageRecord::AccountSnapshot(snapshot))
            if snapshot.account_id == "main"
                && snapshot.ts_ms == 5
                && snapshot.update.positions.iter().any(|position|
                    position.symbol == "BTC-PERP" && position.qty == 0.0)
    ));
    assert!(
        coordinator
            .private_state("main")
            .unwrap()
            .positions()
            .is_empty()
    );
    assert_eq!(
        coordinator
            .engine
            .strategy()
            .entity("BTC-PERP")
            .unwrap()
            .position_qty,
        0.0
    );
    let stale = coordinator
        .process_feed(FeedOutput::PrivateAccount {
            account_id: Some("main".to_string()),
            update: AccountUpdate {
                ts_ms: 4,
                balances: Vec::new(),
                positions: vec![Position {
                    symbol: "BTC-PERP".to_string(),
                    qty: 8.0,
                    avg_price: 49_000.0,
                    margin_mode: Some(PositionMarginMode::Cross),
                }],
                margins: Vec::new(),
            },
        })
        .unwrap();
    assert!(stale.records.is_empty());
    assert_eq!(
        coordinator
            .engine
            .strategy()
            .entity("BTC-PERP")
            .unwrap()
            .position_qty,
        0.0
    );
    coordinator
        .on_reconciliation(ReconciliationResult {
            account_id: "main".to_string(),
            ts_ms: 5,
            clean: first.is_clean(),
            local_live_orders: first.local_live_orders,
            remote_live_orders: first.remote_live_orders,
            remote_recent_fills: first.remote_fills,
            reason: format!("{:?}", first.issues),
        })
        .unwrap();
    assert_eq!(
        coordinator.readiness().missing_reconciliation,
        vec!["main".to_string()]
    );

    let second = reconcile_full_state(
        coordinator.private_state("main").unwrap(),
        &[],
        &[],
        &remote_account,
    );
    assert!(second.is_clean());
    coordinator
        .on_reconciliation(ReconciliationResult {
            account_id: "main".to_string(),
            ts_ms: 6,
            clean: true,
            local_live_orders: 0,
            remote_live_orders: 0,
            remote_recent_fills: 0,
            reason: "second authoritative pass is clean".to_string(),
        })
        .unwrap();
    assert!(coordinator.readiness().missing_reconciliation.is_empty());
}

#[test]
fn authenticated_instrument_order_limits_seed_live_pre_trade_risk() {
    let coordinator = coordinator();
    let mut oversized = order();
    oversized.qty = 101.0;

    assert!(matches!(
        coordinator
            .engine
            .risk()
            .pre_trade(1, OrderIntent::NewOrder(oversized)),
        RiskDecision::Rejected {
            reason: RiskRejectReason::InstrumentOrderQuantity {
                symbol,
                value: 101.0,
                limit: 100.0,
            },
            ..
        } if symbol == "BTC-USDT"
    ));

    let mut oversized_amount = order();
    oversized_amount.qty = 1.0;
    oversized_amount.price = 1_000_001.0;
    assert!(matches!(
        coordinator
            .engine
            .risk()
            .pre_trade(2, OrderIntent::NewOrder(oversized_amount)),
        RiskDecision::Rejected {
            reason: RiskRejectReason::InstrumentOrderNotional {
                value: 1_000_001.0,
                limit: 1_000_000.0,
                ..
            },
            ..
        }
    ));
}

#[test]
fn stablecoin_breach_blocks_entry_then_latches_and_cancels() {
    let mut coordinator = coordinator_with_risk(
        true,
        RiskLimits {
            require_feed_health: false,
            require_private_health: false,
            stablecoin_guards: vec![StablecoinGuardConfig {
                symbol: "USDT-USD".to_string(),
                max_downside_deviation: 0.01,
            }],
            stablecoin_max_age_ms: 10_000,
            stablecoin_breach_debounce_ms: 5_000,
            ..RiskLimits::default()
        },
    );
    bootstrap_readiness(&mut coordinator);
    assert_eq!(
        coordinator.readiness().phase,
        crate::LivePhase::AwaitingStreams
    );
    assert_eq!(
        coordinator.readiness().missing_stablecoin_rates,
        vec!["USDT-USD".to_string()]
    );

    coordinator.process_event(NormalizedEvent::Market(MarketEvent::IndexPrice {
        ts_ms: 2,
        symbol: "USDT-USD".to_string(),
        price: 1.0,
    }));
    assert!(coordinator.readiness().is_ready());
    coordinator
        .register_local_order("main", "live-1", order(), 3)
        .unwrap();

    let transient = coordinator.process_event(NormalizedEvent::Market(MarketEvent::IndexPrice {
        ts_ms: 10,
        symbol: "USDT-USD".to_string(),
        price: 0.98,
    }));
    assert_eq!(coordinator.readiness().phase, crate::LivePhase::Degraded);
    assert!(!coordinator.kill_switch_active());
    assert!(
        !transient
            .records
            .iter()
            .any(|record| { matches!(record, StorageRecord::SafetyLatch(_)) })
    );
    assert!(matches!(
        coordinator
            .engine
            .risk()
            .pre_trade(10, OrderIntent::NewOrder(order())),
        RiskDecision::Rejected {
            reason: RiskRejectReason::StablecoinDepeg { .. },
            ..
        }
    ));
    let mut blocked = CoordinatorOutput::default();
    coordinator.route_intent(10, OrderIntent::NewOrder(order()), &mut blocked);
    assert!(blocked.actions.is_empty());

    let latched = coordinator.process_event(NormalizedEvent::Market(MarketEvent::IndexPrice {
        ts_ms: 5_010,
        symbol: "USDT-USD".to_string(),
        price: 0.98,
    }));
    assert!(coordinator.kill_switch_active());
    assert!(latched.records.iter().any(|record| {
        matches!(
            record,
            StorageRecord::SafetyLatch(latch)
                if latch.active
                    && latch.scope == SafetyLatchScope::Global
                    && latch.source == SafetyLatchSource::Risk
        )
    }));
    assert!(latched.actions.iter().any(|action| {
        matches!(
            action,
            LiveAction::Cancel(cancel) if cancel.client_order_id() == "live-1"
        )
    }));
}
