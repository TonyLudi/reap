mod support;

use reap_pm_core::{
    ConnectionEpoch, IngressSequence, MAX_PM_RECONCILIATION_FILLS, MAX_PM_RECONCILIATION_ORDERS,
    PmAggregateError, PmFillQueryCursor, PmVenueOrderId, PmVenueOrderKey,
};
use reap_polymarket_adapter::{
    MAX_PM_FIXTURE_QUERY_PAGES, PmFixtureDeliveryError, PmFixtureFeeEvidence,
    PmPrivateNormalizationError, PmReconciliationContractError,
};
use serde_json::json;

use support::{
    FUNDER, MARKET, account_scope, completion, instrument_scope, reconciliation_role, snapshot,
};

fn open_order(id: &str, status: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "id": id,
        "market": MARKET,
        "asset_id": "123",
        "side": "BUY",
        "original_size": "5",
        "size_matched": "0",
        "price": "0.40",
        "status": status,
        "maker_address": FUNDER
    }))
    .unwrap()
}

fn fill(id: &str, order: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "event_type": "trade",
        "id": id,
        "market": MARKET,
        "asset_id": "123",
        "side": "BUY",
        "size": "5",
        "price": "0.40",
        "status": "MATCHED",
        "maker_address": FUNDER,
        "transaction_hash": "0xfeed",
        "order_id": order,
        "trader_side": "TAKER"
    }))
    .unwrap()
}

fn unlinked_fill() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "event_type": "trade",
        "id": "unlinked",
        "market": MARKET,
        "asset_id": "123",
        "side": "BUY",
        "size": "5",
        "price": "0.40",
        "status": "MATCHED",
        "maker_address": FUNDER,
        "transaction_hash": "0xdead"
    }))
    .unwrap()
}

fn order_key(id: &str) -> PmVenueOrderKey {
    PmVenueOrderKey::new(account_scope().handle(), PmVenueOrderId::new(id).unwrap())
}

fn page_cursor(index: usize) -> [u8; 32] {
    let mut cursor = [0_u8; 32];
    cursor[..8].copy_from_slice(&(index as u64).to_le_bytes());
    cursor
}

#[test]
fn open_orders_emit_only_after_terminal_chain_and_exact_completion_boundary() {
    let mut role = reconciliation_role();
    let mut assembly = role
        .request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(10))
        .unwrap()
        .begin(snapshot(1));
    let first = open_order("order-1", "LIVE");
    let second = open_order("order-2", "LIVE");
    assembly
        .push_json_page(None, Some([1; 32]), &[first.as_slice()])
        .unwrap();
    assembly
        .push_json_page(Some([1; 32]), None, &[second.as_slice()])
        .unwrap();
    let delivery = assembly
        .finish(completion(1, 11, Some(1)))
        .unwrap()
        .service_at(30_000)
        .unwrap();

    role.reduce_open_orders_delivery(delivery, |scope, envelope| {
        assert_eq!(scope.instrument_scope(), instrument_scope());
        assert_eq!(envelope.payload().orders().len(), 2);
        assert_eq!(
            envelope.payload().boundary().request_sequence(),
            IngressSequence::new(10)
        );
        assert_eq!(
            envelope.payload().boundary().completion_sequence(),
            envelope.ordering().local_ingress_sequence()
        );
    })
    .unwrap();
}

#[test]
fn missing_nonterminal_broken_and_post_terminal_pages_never_emit() {
    let mut role = reconciliation_role();

    let missing = role
        .request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(10))
        .unwrap()
        .begin(snapshot(1));
    assert!(matches!(
        missing.finish(completion(1, 11, Some(1))),
        Err(PmReconciliationContractError::MissingPage)
    ));

    let mut nonterminal = role
        .request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(20))
        .unwrap()
        .begin(snapshot(2));
    nonterminal.push_page(None, Some([1; 32]), &[]).unwrap();
    assert!(matches!(
        nonterminal.finish(completion(1, 21, Some(2))),
        Err(PmReconciliationContractError::MissingTerminalPage)
    ));

    let mut broken = role
        .request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(30))
        .unwrap()
        .begin(snapshot(3));
    broken.push_page(None, Some([1; 32]), &[]).unwrap();
    assert_eq!(
        broken.push_page(Some([2; 32]), None, &[]).unwrap_err(),
        PmReconciliationContractError::BrokenCursorChain
    );

    let mut terminal = role
        .request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(40))
        .unwrap()
        .begin(snapshot(4));
    terminal.push_page(None, None, &[]).unwrap();
    assert_eq!(
        terminal.push_page(None, None, &[]).unwrap_err(),
        PmReconciliationContractError::PageAfterTerminal
    );
}

#[test]
fn explicit_empty_and_completion_epoch_or_snapshot_mismatch_are_distinct() {
    let mut role = reconciliation_role();
    let empty = role
        .request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(10))
        .unwrap()
        .complete(completion(1, 11, Some(1)), snapshot(1), &[])
        .unwrap()
        .service_at(30_000)
        .unwrap();
    role.reduce_open_orders_delivery(empty, |_, envelope| {
        assert!(envelope.payload().orders().is_empty());
    })
    .unwrap();

    let wrong_epoch = role
        .request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(20))
        .unwrap()
        .complete(completion(2, 21, Some(2)), snapshot(2), &[]);
    assert!(matches!(
        wrong_epoch,
        Err(PmReconciliationContractError::Delivery(
            PmFixtureDeliveryError::CompletionEpochMismatch
        ))
    ));

    let wrong_snapshot = role
        .request_open_orders(ConnectionEpoch::new(2), IngressSequence::new(1))
        .unwrap()
        .complete(completion(2, 2, Some(4)), snapshot(3), &[]);
    assert!(matches!(
        wrong_snapshot,
        Err(PmReconciliationContractError::Delivery(
            PmFixtureDeliveryError::CompletionSnapshotMismatch
        ))
    ));
}

#[test]
fn exact_order_detail_is_explicit_absence_and_rejects_another_order() {
    let mut role = reconciliation_role();
    let requested = order_key("requested");
    let absent = role
        .request_order_detail(ConnectionEpoch::new(1), IngressSequence::new(10), requested)
        .unwrap()
        .complete(completion(1, 11, Some(1)), snapshot(1), None)
        .unwrap()
        .service_at(30_000)
        .unwrap();
    role.reduce_order_detail_delivery(absent, |_, envelope| {
        assert_eq!(envelope.payload().requested_order(), requested);
        assert_eq!(envelope.payload().order(), None);
    })
    .unwrap();

    let another =
        reap_polymarket_wire::parse_open_order_fixture(&open_order("another-order", "LIVE"))
            .unwrap();
    assert!(matches!(
        role.request_order_detail(ConnectionEpoch::new(1), IngressSequence::new(20), requested,)
            .unwrap()
            .complete(completion(1, 21, Some(2)), snapshot(2), Some(&another)),
        Err(PmReconciliationContractError::Aggregate(
            PmAggregateError::OrderDetailVenueMismatch
        ))
    ));
}

#[test]
fn exact_order_detail_json_bridge_keeps_wire_dtos_inside_the_adapter() {
    let mut role = reconciliation_role();
    let requested = order_key("requested");
    let raw = open_order("requested", "LIVE");
    let present = role
        .request_order_detail(ConnectionEpoch::new(1), IngressSequence::new(10), requested)
        .unwrap()
        .complete_json_object(
            completion(1, 11, Some(1)),
            snapshot(1),
            Some(raw.as_slice()),
        )
        .unwrap()
        .service_at(30_000)
        .unwrap();
    role.reduce_order_detail_delivery(present, |_, envelope| {
        assert_eq!(
            envelope
                .payload()
                .order()
                .unwrap()
                .order()
                .venue_order_key(),
            Some(requested)
        );
    })
    .unwrap();

    let absent = role
        .request_order_detail(ConnectionEpoch::new(1), IngressSequence::new(20), requested)
        .unwrap()
        .complete_json_object(completion(1, 21, Some(2)), snapshot(2), None)
        .unwrap()
        .service_at(30_000)
        .unwrap();
    role.reduce_order_detail_delivery(absent, |_, envelope| {
        assert_eq!(envelope.payload().order(), None);
    })
    .unwrap();
}

#[test]
fn fill_pages_require_terminal_watermark_and_preserve_rest_then_ws_order() {
    let mut role = reconciliation_role();
    let middle = PmFillQueryCursor::new(account_scope(), [1; 32]);
    let watermark = PmFillQueryCursor::new(account_scope(), [2; 32]);
    let first = fill("fill-1", "order-1");
    let second = fill("fill-2", "order-2");
    let mut assembly = role
        .request_fills(ConnectionEpoch::new(1), IngressSequence::new(10), None)
        .unwrap()
        .begin(snapshot(1));
    assembly
        .push_user_frame_page(
            None,
            Some(middle),
            None,
            &[first.as_slice()],
            PmFixtureFeeEvidence::Unknown,
        )
        .unwrap();
    assembly
        .push_user_frame_page(
            Some(middle),
            None,
            Some(watermark),
            &[second.as_slice()],
            PmFixtureFeeEvidence::Incomplete,
        )
        .unwrap();
    let delivery = assembly
        .finish(completion(1, 11, Some(1)))
        .unwrap()
        .service_at(30_000)
        .unwrap();
    role.reduce_fill_query_delivery(delivery, |_, envelope| {
        let fills = envelope.payload().fills();
        assert_eq!(fills.len(), 2);
        assert_eq!(
            fills[0].order().venue_order_key(),
            Some(order_key("order-1"))
        );
        assert_eq!(
            fills[1].order().venue_order_key(),
            Some(order_key("order-2"))
        );
        assert_eq!(envelope.payload().resulting_watermark(), watermark);
    })
    .unwrap();
}

#[test]
fn fill_query_rejects_missing_watermark_unresolved_and_duplicate_legs() {
    let mut role = reconciliation_role();
    let cursor = PmFillQueryCursor::new(account_scope(), [1; 32]);

    let mut missing = role
        .request_fills(ConnectionEpoch::new(1), IngressSequence::new(10), None)
        .unwrap()
        .begin(snapshot(1));
    assert_eq!(
        missing
            .push_user_frame_page(None, None, None, &[], PmFixtureFeeEvidence::Unknown)
            .unwrap_err(),
        PmReconciliationContractError::MissingResultingWatermark
    );

    let unlinked = unlinked_fill();
    let mut unresolved = role
        .request_fills(ConnectionEpoch::new(1), IngressSequence::new(20), None)
        .unwrap()
        .begin(snapshot(2));
    assert_eq!(
        unresolved
            .push_user_frame_page(
                None,
                None,
                Some(cursor),
                &[unlinked.as_slice()],
                PmFixtureFeeEvidence::Unknown,
            )
            .unwrap_err(),
        PmReconciliationContractError::UnresolvedTrade
    );

    let repeated = fill("same-fill", "same-order");
    let mut duplicate = role
        .request_fills(ConnectionEpoch::new(1), IngressSequence::new(30), None)
        .unwrap()
        .begin(snapshot(3));
    duplicate
        .push_user_frame_page(
            None,
            Some(cursor),
            None,
            &[repeated.as_slice()],
            PmFixtureFeeEvidence::Unknown,
        )
        .unwrap();
    duplicate
        .push_user_frame_page(
            Some(cursor),
            None,
            Some(PmFillQueryCursor::new(account_scope(), [2; 32])),
            &[repeated.as_slice()],
            PmFixtureFeeEvidence::Unknown,
        )
        .unwrap();
    assert!(matches!(
        duplicate.finish(completion(1, 31, Some(3))),
        Err(PmReconciliationContractError::Aggregate(
            PmAggregateError::DuplicateFillKey
        ))
    ));
}

#[test]
fn terminal_or_unknown_open_order_status_never_becomes_complete_authority() {
    let mut role = reconciliation_role();
    for (sequence, status, expected) in [
        (
            10,
            "MATCHED",
            PmPrivateNormalizationError::OpenOrderIsTerminal,
        ),
        (
            20,
            "MYSTERY",
            PmPrivateNormalizationError::UnknownOrderStatus,
        ),
    ] {
        let raw = open_order("order", status);
        let result = role
            .request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(sequence))
            .unwrap()
            .complete_json_objects(
                completion(1, sequence + 1, Some(sequence)),
                snapshot(sequence),
                &[raw.as_slice()],
            );
        assert!(matches!(
            result,
            Err(PmReconciliationContractError::Normalization(error)) if error == expected
        ));
    }
}

#[test]
fn open_order_cap_is_preflighted_before_parsing_and_failed_page_is_retryable() {
    let mut role = reconciliation_role();
    let mut assembly = role
        .request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(10))
        .unwrap()
        .begin(snapshot(1));
    let first = open_order("order-1", "LIVE");
    assembly
        .push_json_page(None, Some(page_cursor(1)), &[first.as_slice()])
        .unwrap();

    let malformed = b"not-json".as_slice();
    let cap_plus_one_for_remaining = vec![malformed; MAX_PM_RECONCILIATION_ORDERS];
    assert!(matches!(
        assembly.push_json_page(Some(page_cursor(1)), None, &cap_plus_one_for_remaining,),
        Err(PmReconciliationContractError::Aggregate(
            PmAggregateError::TooManyOrders
        ))
    ));

    assembly
        .push_json_page(Some(page_cursor(1)), None, &[])
        .unwrap();
    let delivery = assembly
        .finish(completion(1, 11, Some(1)))
        .unwrap()
        .service_at(30_000)
        .unwrap();
    role.reduce_open_orders_delivery(delivery, |_, envelope| {
        assert_eq!(envelope.payload().orders().len(), 1);
    })
    .unwrap();
    role.request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(20))
        .expect("a rejected oversized page must not corrupt role request ordering");
}

#[test]
fn fill_cap_is_preflighted_against_remaining_rows_and_failed_page_is_retryable() {
    let mut role = reconciliation_role();
    let middle = PmFillQueryCursor::new(account_scope(), page_cursor(1));
    let watermark = PmFillQueryCursor::new(account_scope(), page_cursor(2));
    let first = fill("fill-1", "order-1");
    let mut assembly = role
        .request_fills(ConnectionEpoch::new(1), IngressSequence::new(10), None)
        .unwrap()
        .begin(snapshot(1));
    assembly
        .push_user_frame_page(
            None,
            Some(middle),
            None,
            &[first.as_slice()],
            PmFixtureFeeEvidence::Unknown,
        )
        .unwrap();

    let malformed = b"not-json".as_slice();
    let cap_plus_one_for_remaining = vec![malformed; MAX_PM_RECONCILIATION_FILLS];
    assert!(matches!(
        assembly.push_user_frame_page(
            Some(middle),
            None,
            Some(watermark),
            &cap_plus_one_for_remaining,
            PmFixtureFeeEvidence::Unknown,
        ),
        Err(PmReconciliationContractError::Aggregate(
            PmAggregateError::TooManyFills
        ))
    ));

    assembly
        .push_user_frame_page(
            Some(middle),
            None,
            Some(watermark),
            &[],
            PmFixtureFeeEvidence::Unknown,
        )
        .unwrap();
    let delivery = assembly
        .finish(completion(1, 11, Some(1)))
        .unwrap()
        .service_at(30_000)
        .unwrap();
    role.reduce_fill_query_delivery(delivery, |_, envelope| {
        assert_eq!(envelope.payload().fills().len(), 1);
    })
    .unwrap();
}

#[test]
fn page_cap_is_preflighted_before_parsing_and_does_not_corrupt_role_ordering() {
    let mut role = reconciliation_role();
    let mut assembly = role
        .request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(10))
        .unwrap()
        .begin(snapshot(1));
    for page in 0..MAX_PM_FIXTURE_QUERY_PAGES {
        let requested = (page != 0).then(|| page_cursor(page));
        assembly
            .push_json_page(requested, Some(page_cursor(page + 1)), &[])
            .unwrap();
    }

    let malformed = b"not-json".as_slice();
    assert_eq!(
        assembly
            .push_json_page(
                Some(page_cursor(MAX_PM_FIXTURE_QUERY_PAGES)),
                None,
                &[malformed],
            )
            .unwrap_err(),
        PmReconciliationContractError::TooManyPages
    );
    assert_eq!(
        assembly
            .push_json_page(Some(page_cursor(MAX_PM_FIXTURE_QUERY_PAGES)), None, &[],)
            .unwrap_err(),
        PmReconciliationContractError::TooManyPages
    );
    role.request_open_orders(ConnectionEpoch::new(1), IngressSequence::new(20))
        .expect("page saturation must not corrupt role request ordering");
}
