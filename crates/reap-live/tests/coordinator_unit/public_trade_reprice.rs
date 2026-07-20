use super::*;

fn feed_market(
    coordinator: &mut LiveCoordinator,
    event: MarketEvent,
    observed_now_ms: TimeMs,
    arrival_ns: u64,
) -> CoordinatorOutput {
    coordinator
        .process_feed_arrived_at(
            FeedOutput::Event(NormalizedEvent::Market(event)),
            observed_now_ms,
            arrival_ns,
        )
        .unwrap()
}

fn hedge_depth(ts_ms: TimeMs) -> MarketEvent {
    MarketEvent::Depth(OrderBook {
        symbol: "BTC-PERP".to_string(),
        ts_ms,
        bids: vec![
            Level::new(99.0, 10.0),
            Level::new(98.0, 10.0),
            Level::new(97.0, 10.0),
        ],
        asks: vec![
            Level::new(101.0, 10.0),
            Level::new(102.0, 10.0),
            Level::new(103.0, 10.0),
        ],
    })
}

fn crossing_trade(ts_ms: TimeMs) -> MarketEvent {
    MarketEvent::Trade {
        ts_ms,
        symbol: "BTC-PERP".to_string(),
        price: 102.0,
        qty: 5.0,
        taker_side: Side::Buy,
    }
}

fn hedge_commit_debug(coordinator: &LiveCoordinator, symbol: &str) -> String {
    let entity = coordinator
        .engine
        .strategy()
        .entity(symbol)
        .expect("hedge instrument must remain configured");
    let debug = format!("{entity:?}");
    debug
        .split("implied_depth: ")
        .nth(1)
        .expect("instrument debug must expose its private implied-depth state")
        .split(", trade: ")
        .next()
        .expect("instrument debug must delimit implied-depth state")
        .to_string()
}

fn seed_hedge_inputs(coordinator: &mut LiveCoordinator) {
    coordinator.set_order_entry_enabled(false);
    for event in [
        MarketEvent::Depth(OrderBook::one_level(
            "BTC-USDT",
            3,
            Level::new(50_000.0, 10.0),
            Level::new(50_001.0, 10.0),
        )),
        MarketEvent::Depth(OrderBook::one_level(
            "BTC-PERP",
            3,
            Level::new(50_003.0, 10_000.0),
            Level::new(50_004.0, 10_000.0),
        )),
    ] {
        let output = feed_market(coordinator, event, 3, 900_000_000);
        assert!(
            output.actions.is_empty(),
            "seed depth must not reserve local orders while entry is disabled"
        );
    }
    coordinator.set_order_entry_enabled(true);
}

fn positive_perp_delta(ts_ms: TimeMs) -> AccountUpdate {
    let mut update = account_update("main", ts_ms);
    update.positions.push(Position {
        symbol: "BTC-PERP".to_string(),
        qty: 100.0,
        avg_price: 50_000.0,
        margin_mode: Some(PositionMarginMode::Cross),
    });
    update
}

#[test]
fn live_coordinator_uses_exact_private_deadline_and_inclusive_service() {
    let mut coordinator = coordinator();
    ready(&mut coordinator);
    let _ = feed_market(&mut coordinator, hedge_depth(3), 1_000, 900_000_000);

    let immediate = feed_market(&mut coordinator, crossing_trade(4), 1_001, 1_000_000_000);
    assert!(
        immediate.actions.is_empty(),
        "trade reaction must remain deferred"
    );
    assert_eq!(coordinator.next_trade_reprice_due_ns(), Some(1_000_100_000));

    let before = coordinator.service_one_due_trade_reprice(1_000_099_999, 1_001);
    assert!(before.actions.is_empty());
    assert_eq!(coordinator.next_trade_reprice_due_ns(), Some(1_000_100_000));

    let callback = coordinator.service_one_due_trade_reprice(1_000_100_000, 1_001);
    assert!(callback.actions.is_empty());
    assert_eq!(
        coordinator.next_trade_reprice_due_ns(),
        Some(1_004_100_000),
        "the callback must join the shared worker at local millisecond 1005"
    );

    let trailing_before = coordinator.service_one_due_trade_reprice(1_004_099_999, 1_005);
    assert!(trailing_before.actions.is_empty());
    assert_eq!(coordinator.next_trade_reprice_due_ns(), Some(1_004_100_000));
    let _ = coordinator.service_one_due_trade_reprice(1_004_100_000, 1_005);
    assert_eq!(coordinator.next_trade_reprice_due_ns(), None);
}

#[test]
fn live_depth_worker_clamps_a_regressed_finish_to_its_separate_work_start_clock() {
    let mut coordinator = coordinator();
    ready(&mut coordinator);

    let mut worker_clock_calls = 0_u8;
    coordinator
        .process_feed_received_at(
            FeedOutput::Event(NormalizedEvent::Market(hedge_depth(3))),
            10,
            1_000_000_000,
            || (10_000_000_000, 2_000),
            || {
                worker_clock_calls += 1;
                if worker_clock_calls == 1 {
                    2_001
                } else {
                    2_000
                }
            },
        )
        .unwrap();
    assert!(
        worker_clock_calls >= 2,
        "immediate depth must sample actual work start before post-work finish"
    );
    assert_eq!(
        coordinator.next_trade_reprice_due_ns(),
        None,
        "the first depth must price immediately"
    );

    coordinator
        .process_feed_received_at(
            FeedOutput::Event(NormalizedEvent::Market(hedge_depth(4))),
            11,
            1_001_000_000,
            || (10_005_000_000, 2_005),
            || 2_005,
        )
        .unwrap();
    assert_eq!(
        coordinator.next_trade_reprice_due_ns(),
        Some(10_006_000_000),
        "Java clamps the regressed finish to work start 2001, leaving a one-millisecond delay"
    );
}

#[test]
fn deferred_trade_callback_clamps_a_regressed_finish_to_its_separate_work_start_clock() {
    let mut coordinator = coordinator();
    ready(&mut coordinator);
    coordinator
        .process_feed_received_at(
            FeedOutput::Event(NormalizedEvent::Market(hedge_depth(3))),
            1_000,
            900_000_000,
            || (1_000_000_000, 1_000),
            || 1_000,
        )
        .unwrap();

    let trade_receipt_ns = 2_000_000_000;
    coordinator
        .process_feed_received_at(
            FeedOutput::Event(NormalizedEvent::Market(crossing_trade(4))),
            1_004,
            trade_receipt_ns,
            || (trade_receipt_ns + 10_000, 1_004),
            || 1_004,
        )
        .unwrap();
    assert_eq!(
        coordinator.next_trade_reprice_due_ns(),
        Some(trade_receipt_ns + 100_000)
    );

    let mut worker_clock_calls = 0_u8;
    coordinator.service_one_due_trade_reprice_with_clocks(
        || (trade_receipt_ns + 100_000, 1_005),
        || {
            worker_clock_calls += 1;
            if worker_clock_calls == 1 {
                1_006
            } else {
                1_005
            }
        },
    );
    assert!(
        worker_clock_calls >= 2,
        "an immediate trade callback must sample work start before finish"
    );
    assert_eq!(coordinator.next_trade_reprice_due_ns(), None);

    coordinator
        .process_feed_received_at(
            FeedOutput::Event(NormalizedEvent::Market(hedge_depth(5))),
            1_010,
            3_000_000_000,
            || (3_000_000_000, 1_010),
            || 1_010,
        )
        .unwrap();
    assert_eq!(
        coordinator.next_trade_reprice_due_ns(),
        Some(3_001_000_000),
        "Java clamps the callback finish to start 1006, so decision 1010 is still throttled"
    );
}

#[test]
fn pending_depth_worker_orders_two_sub_100us_trade_callbacks_before_refresh() {
    let mut coordinator = coordinator();
    ready(&mut coordinator);

    for (book_ts_ms, receipt_ns, processing_ns, processing_ms) in [
        (3, 900_000_000, 1_000_000_000, 1_000),
        (4, 901_000_000, 1_001_000_000, 1_001),
    ] {
        coordinator
            .process_feed_received_at(
                FeedOutput::Event(NormalizedEvent::Market(hedge_depth(book_ts_ms))),
                processing_ms,
                receipt_ns,
                || (processing_ns, processing_ms),
                || processing_ms,
            )
            .unwrap();
    }
    assert_eq!(
        coordinator.next_trade_reprice_due_ns(),
        Some(1_005_000_000),
        "the second depth must leave one shared trailing worker"
    );

    for (trade_ts_ms, receipt_ns) in [(5, 1_001_050_000), (6, 1_001_100_000)] {
        coordinator
            .process_feed_received_at(
                FeedOutput::Event(NormalizedEvent::Market(crossing_trade(trade_ts_ms))),
                1_001,
                receipt_ns,
                || (receipt_ns + 10_000, 1_001),
                || 1_001,
            )
            .unwrap();
    }
    assert_eq!(
        coordinator.next_trade_reprice_due_ns(),
        Some(1_001_150_000),
        "the first retained callback must precede the pending depth worker"
    );

    let first =
        coordinator.service_one_due_trade_reprice_with_finish_clock(1_001_150_000, 1_001, || 1_001);
    assert!(first.actions.is_empty());
    assert_eq!(coordinator.next_trade_reprice_due_ns(), Some(1_001_200_000));

    let second =
        coordinator.service_one_due_trade_reprice_with_finish_clock(1_001_200_000, 1_001, || 1_001);
    assert!(second.actions.is_empty());
    assert_eq!(
        coordinator.next_trade_reprice_due_ns(),
        Some(1_005_000_000),
        "both callbacks must conflate into the already-pending depth refresh"
    );

    let trailing =
        coordinator.service_one_due_trade_reprice_with_finish_clock(1_005_000_000, 1_005, || 1_005);
    assert!(trailing.actions.is_empty());
    assert_eq!(coordinator.next_trade_reprice_due_ns(), None);
}

#[test]
fn closed_live_gate_updates_depth_without_running_the_pricing_worker() {
    let mut coordinator = coordinator();
    ready(&mut coordinator);
    coordinator.set_order_entry_enabled(false);
    coordinator.chaos_intent_trace.clear();

    for event in [
        MarketEvent::Depth(OrderBook::one_level(
            "BTC-USDT",
            3,
            Level::new(50_000.0, 10.0),
            Level::new(50_001.0, 10.0),
        )),
        MarketEvent::Depth(OrderBook::one_level(
            "BTC-PERP",
            3,
            Level::new(50_003.0, 10_000.0),
            Level::new(50_004.0, 10_000.0),
        )),
    ] {
        let output = feed_market(&mut coordinator, event, 3, 900_000_000);
        assert!(output.actions.is_empty());
    }

    assert!(
        coordinator.chaos_intent_trace.is_empty(),
        "Java's closed Live gate must not run quote pricing"
    );
    assert_eq!(coordinator.next_trade_reprice_due_ns(), None);
}

#[test]
fn live_gate_blocks_scheduling_and_is_rechecked_when_callback_fires() {
    let mut coordinator = coordinator();
    ready(&mut coordinator);
    let _ = feed_market(&mut coordinator, hedge_depth(3), 10, 900_000_000);

    coordinator.set_order_entry_enabled(false);
    let _ = feed_market(&mut coordinator, crossing_trade(4), 11, 1_000_000_000);
    assert_eq!(coordinator.next_trade_reprice_due_ns(), None);

    coordinator.set_order_entry_enabled(true);
    let _ = feed_market(&mut coordinator, crossing_trade(5), 12, 2_000_000_000);
    assert_eq!(coordinator.next_trade_reprice_due_ns(), Some(2_000_100_000));
    coordinator.set_order_entry_enabled(false);
    let output = coordinator.service_one_due_trade_reprice(2_000_100_000, 12);
    assert!(output.actions.is_empty());
    assert_eq!(
        coordinator.next_trade_reprice_due_ns(),
        Some(2_003_100_000),
        "a retained callback advances and schedules the shared worker even after the live gate closes"
    );
    let trailing =
        coordinator.service_one_due_trade_reprice_with_clocks(|| (2_003_100_000, 15), || 16);
    assert!(trailing.actions.is_empty());
    assert_eq!(coordinator.next_trade_reprice_due_ns(), None);

    coordinator.set_order_entry_enabled(true);
    let _ = feed_market(&mut coordinator, crossing_trade(6), 17, 2_004_000_000);
    assert_eq!(coordinator.next_trade_reprice_due_ns(), Some(2_004_100_000));
    let callback = coordinator.service_one_due_trade_reprice(2_004_100_000, 17);
    assert!(callback.actions.is_empty());
    assert_eq!(
        coordinator.next_trade_reprice_due_ns(),
        Some(2_008_100_000),
        "the closed-gate worker must retain its post-work finish clock"
    );
}

#[test]
fn live_hedge_commit_uses_post_reservation_clock_only_after_acceptance() {
    let mut accepted = coordinator_with_gateway_actions(true);
    let mut rejected = coordinator_with_gateway_actions(false);
    for coordinator in [&mut accepted, &mut rejected] {
        ready(coordinator);
        seed_hedge_inputs(coordinator);
    }
    assert_eq!(
        hedge_commit_debug(&accepted, "BTC-USDT"),
        hedge_commit_debug(&rejected, "BTC-USDT"),
        "fixtures must begin with identical implied-depth state"
    );

    let accepted_output = accepted
        .process_feed_received_at(
            FeedOutput::PrivateAccount {
                account_id: Some("main".to_string()),
                update: positive_perp_delta(7),
            },
            7,
            7_000_000,
            || panic!("private account reduction must not sample a market processing clock"),
            || 9,
        )
        .unwrap();
    let rejected_output = rejected
        .process_feed_received_at(
            FeedOutput::PrivateAccount {
                account_id: Some("main".to_string()),
                update: positive_perp_delta(7),
            },
            7,
            7_000_000,
            || panic!("private account reduction must not sample a market processing clock"),
            || panic!("rejected hedge must not sample the local-send commit clock"),
        )
        .unwrap();

    let hedge_symbol = accepted_output
        .actions
        .iter()
        .find_map(|action| match action {
            LiveAction::Submit(submit) if submit.order().time_in_force == TimeInForce::Ioc => {
                Some(submit.order().symbol.as_str())
            }
            _ => None,
        })
        .expect("accepted delta hedge must reserve one IOC order");
    assert!(!rejected_output.actions.iter().any(|action| {
        matches!(
            action,
            LiveAction::Submit(submit)
                if submit.order().time_in_force == TimeInForce::Ioc
        )
    }));
    let accepted_state = hedge_commit_debug(&accepted, hedge_symbol);
    let rejected_state = hedge_commit_debug(&rejected, hedge_symbol);
    assert_ne!(accepted_state, rejected_state);
    assert!(
        accepted_state.contains("updated_ms: 9"),
        "accepted local reservation must sample its commit clock after reservation, not reuse receipt time"
    );
    assert!(
        !rejected_state.contains("updated_ms: 9"),
        "policy rejection must consume no pending-hedge transition"
    );
}
