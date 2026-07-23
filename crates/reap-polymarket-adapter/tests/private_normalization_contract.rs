mod support;

use reap_pm_core::{
    ConnectionEpoch, EventOrdering, IngressSequence, PmFillFee, PmFillRole, PmFillSettlementStatus,
    PmOrderStatus, ReceivedEventClock,
};
use reap_polymarket_adapter::{
    PmFixtureCompletionOccurrence, PmFixtureFeeEvidence, PmPrivateLifecycleObservation,
    PmPrivateNormalizationError, PmUnresolvedTradeReason,
};
use serde_json::{Value, json};

use support::{FUNDER, MARKET, MIXED_CASE_FUNDER, completion, private_role};

fn order(status: &str, event_kind: &str, price: &str, original_size: &str, funder: &str) -> Value {
    json!({
        "event_type": "order",
        "id": "order-1",
        "market": MARKET,
        "asset_id": "123",
        "side": "BUY",
        "original_size": original_size,
        "size_matched": "0",
        "price": price,
        "status": status,
        "maker_address": funder,
        "type": event_kind
    })
}

fn direct_trade(
    id: &str,
    order_id: &str,
    role: &str,
    status: &str,
    price: &str,
    quantity: &str,
    funder: &str,
) -> Value {
    json!({
        "event_type": "trade",
        "id": id,
        "market": MARKET,
        "asset_id": "123",
        "side": "BUY",
        "size": quantity,
        "price": price,
        "status": status,
        "maker_address": funder,
        "transaction_hash": "0xfeed",
        "order_id": order_id,
        "trader_side": role
    })
}

fn maker_trade(legs: Value) -> Value {
    json!({
        "event_type": "trade",
        "id": "trade-maker",
        "market": MARKET,
        "asset_id": "123",
        "side": "BUY",
        "size": "12.5",
        "price": "0.40",
        "status": "MATCHED",
        "maker_address": "0xcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd",
        "transaction_hash": "0xbeef",
        "trader_side": "MAKER",
        "maker_orders": legs
    })
}

fn receive(
    role: &mut reap_polymarket_adapter::PmFixturePrivateLifecycle,
    sequence: u64,
    value: &Value,
    fee: PmFixtureFeeEvidence,
) -> Result<reap_polymarket_adapter::PmFixturePrivateDelivery, PmPrivateNormalizationError> {
    role.receive_user_fixture(
        completion(1, sequence, None),
        serde_json::to_string(value).unwrap().as_bytes(),
        fee,
    )
}

#[test]
fn lifecycle_preserves_frame_order_and_accepts_canonical_mixed_case_funder() {
    let mut role = private_role();
    role.reconnect(ConnectionEpoch::new(1)).unwrap();
    let frame = json!([
        order("LIVE", "PLACEMENT", "0.40", "5", MIXED_CASE_FUNDER),
        direct_trade(
            "trade-1",
            "order-1",
            "TAKER",
            "MATCHED",
            "0.40",
            "5",
            MIXED_CASE_FUNDER
        )
    ]);
    let delivery = receive(&mut role, 1, &frame, PmFixtureFeeEvidence::Unknown)
        .unwrap()
        .service_at(30_000)
        .unwrap();

    role.reduce_private_delivery(delivery, |scope, envelope| {
        assert_eq!(scope.account_scope(), support::account_scope());
        let observations = envelope.payload().observations();
        assert_eq!(observations.len(), 2);
        let PmPrivateLifecycleObservation::Order(order) = observations[0] else {
            panic!("first wire row must stay first");
        };
        assert_eq!(order.progress().status(), PmOrderStatus::Open);
        let PmPrivateLifecycleObservation::Fill(fill) = observations[1] else {
            panic!("second wire row must stay second");
        };
        assert_eq!(fill.execution().role(), PmFillRole::Taker);
        assert_eq!(fill.execution().fee(), PmFillFee::Unknown);
        assert_eq!(envelope.ordering().venue_sequence(), None);
    })
    .unwrap();
}

#[test]
fn taker_role_selects_local_taker_order_despite_maker_counterparty_evidence() {
    let mut role = private_role();
    role.reconnect(ConnectionEpoch::new(1)).unwrap();
    let trade = json!({
        "event_type": "trade",
        "id": "trade-taker",
        "market": MARKET,
        "asset_id": "123",
        "side": "BUY",
        "size": "5",
        "price": "0.40",
        "status": "MINED",
        "maker_address": "0xcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd",
        "transaction_hash": "0xbeef",
        "trader_side": "TAKER",
        "taker_order_id": "local-taker-order",
        "maker_orders": [{
            "order_id": "external-maker-order",
            "asset_id": "123",
            "side": "SELL",
            "price": "0.40",
            "matched_amount": "5",
            "maker_address": "0xefefefefefefefefefefefefefefefefefefefef"
        }]
    });
    let delivery = receive(&mut role, 1, &trade, PmFixtureFeeEvidence::Incomplete)
        .unwrap()
        .service_at(30_000)
        .unwrap();

    role.reduce_private_delivery(delivery, |_, envelope| {
        let [PmPrivateLifecycleObservation::Fill(fill)] = envelope.payload().observations() else {
            panic!("exact local taker reference must produce one fill");
        };
        assert_eq!(fill.execution().role(), PmFillRole::Taker);
        assert_eq!(fill.execution().settlement(), PmFillSettlementStatus::Mined);
        assert_eq!(fill.execution().fee(), PmFillFee::Incomplete);
        assert_eq!(
            fill.order().venue_order_key().unwrap().id().as_str(),
            "local-taker-order"
        );
    })
    .unwrap();
}

#[test]
fn maker_legs_without_local_order_registry_are_quarantined() {
    let mut role = private_role();
    role.reconnect(ConnectionEpoch::new(1)).unwrap();
    let trade = maker_trade(json!([
        {
            "order_id": "apparently-local",
            "asset_id": "123",
            "side": "SELL",
            "price": "0.40",
            "matched_amount": "5",
            "maker_address": FUNDER
        },
        {
            "order_id": "external-counterparty",
            "asset_id": "123",
            "side": "BUY",
            "price": "0.60",
            "matched_amount": "5",
            "maker_address": "0xcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd"
        }
    ]));
    let delivery = receive(&mut role, 1, &trade, PmFixtureFeeEvidence::Unknown)
        .unwrap()
        .service_at(30_000)
        .unwrap();
    role.reduce_private_delivery(delivery, |_, envelope| {
        let [
            PmPrivateLifecycleObservation::Fill(local),
            PmPrivateLifecycleObservation::UnresolvedTrade(trade),
        ] = envelope.payload().observations()
        else {
            panic!("only the exact funder leg may become a local maker fill");
        };
        assert_eq!(local.execution().role(), PmFillRole::Maker);
        assert_eq!(
            local.order().venue_order_key().unwrap().id().as_str(),
            "apparently-local"
        );
        assert_eq!(trade.reason(), PmUnresolvedTradeReason::ExternalMakerOrder);
        assert_eq!(
            trade.candidate_order().unwrap().as_str(),
            "external-counterparty"
        );
        assert_eq!(trade.settlement(), PmFillSettlementStatus::Matched);
    })
    .unwrap();
}

#[test]
fn unlinked_and_ambiguous_trades_remain_typed_unresolved_evidence() {
    let unlinked = json!({
        "event_type": "trade",
        "id": "trade-unlinked",
        "market": MARKET,
        "asset_id": "123",
        "side": "BUY",
        "size": "5",
        "price": "0.40",
        "status": "MATCHED",
        "maker_address": FUNDER,
        "transaction_hash": "0xdead"
    });
    let ambiguous = json!({
        "event_type": "trade",
        "id": "trade-ambiguous",
        "market": MARKET,
        "asset_id": "123",
        "side": "BUY",
        "size": "5",
        "price": "0.40",
        "status": "MATCHED",
        "maker_address": FUNDER,
        "transaction_hash": "0xdead",
        "order_id": "direct",
        "taker_order_id": "taker",
        "trader_side": "TAKER"
    });
    for (sequence, raw, expected) in [
        (
            1,
            unlinked,
            PmUnresolvedTradeReason::MissingExactOrderLinkage,
        ),
        (
            2,
            ambiguous,
            PmUnresolvedTradeReason::MultipleOrderReferenceKinds,
        ),
    ] {
        let mut role = private_role();
        role.reconnect(ConnectionEpoch::new(1)).unwrap();
        let delivery = receive(&mut role, sequence, &raw, PmFixtureFeeEvidence::Unknown)
            .unwrap()
            .service_at(30_100)
            .unwrap();
        role.reduce_private_delivery(delivery, |_, envelope| {
            let [PmPrivateLifecycleObservation::UnresolvedTrade(trade)] =
                envelope.payload().observations()
            else {
                panic!("unlinked trade must not invent an order identity");
            };
            assert_eq!(trade.reason(), expected);
        })
        .unwrap();
    }
}

#[test]
fn status_discriminator_event_kind_scope_tick_lot_and_amount_fail_closed() {
    let cases = [
        (
            order("MYSTERY", "PLACEMENT", "0.40", "5", FUNDER),
            PmPrivateNormalizationError::UnknownOrderStatus,
        ),
        (
            order("LIVE", "MYSTERY_EVENT", "0.40", "5", FUNDER),
            PmPrivateNormalizationError::UnknownOrderEventKind,
        ),
        (
            order("LIVE", "PLACEMENT", "0.401", "5", FUNDER),
            PmPrivateNormalizationError::PriceOffTick,
        ),
        (
            order("LIVE", "PLACEMENT", "0.40", "1", FUNDER),
            PmPrivateNormalizationError::InvalidOrderQuantityContract,
        ),
        (
            order("LIVE", "PLACEMENT", "0.40", "5.000001", FUNDER),
            PmPrivateNormalizationError::InvalidOrderQuantityContract,
        ),
        (
            direct_trade(
                "trade-status",
                "order-1",
                "TAKER",
                "UNKNOWN",
                "0.40",
                "5",
                FUNDER,
            ),
            PmPrivateNormalizationError::UnknownTradeStatus,
        ),
        (
            direct_trade(
                "trade-tick",
                "order-1",
                "TAKER",
                "MATCHED",
                "0.401",
                "5",
                FUNDER,
            ),
            PmPrivateNormalizationError::PriceOffTick,
        ),
        (
            direct_trade(
                "trade-rounding",
                "order-1",
                "TAKER",
                "MATCHED",
                "0.40",
                "0.000001",
                FUNDER,
            ),
            PmPrivateNormalizationError::NonIntegralProtocolAmounts,
        ),
    ];

    for (raw, expected) in cases {
        let mut role = private_role();
        role.reconnect(ConnectionEpoch::new(1)).unwrap();
        assert_eq!(
            receive(&mut role, 1, &raw, PmFixtureFeeEvidence::Unknown).unwrap_err(),
            expected
        );
    }

    let mut wrong_funder = private_role();
    wrong_funder.reconnect(ConnectionEpoch::new(1)).unwrap();
    let row = order(
        "LIVE",
        "PLACEMENT",
        "0.40",
        "5",
        "0xcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd",
    );
    assert_eq!(
        receive(&mut wrong_funder, 1, &row, PmFixtureFeeEvidence::Unknown).unwrap_err(),
        PmPrivateNormalizationError::FunderMismatch
    );
}

#[test]
fn proven_order_kinds_and_trade_settlement_statuses_are_exact() {
    let mut placement = order("LIVE", "PLACEMENT", "0.40", "5", FUNDER);
    let mut update = order("LIVE", "UPDATE", "0.40", "5", FUNDER);
    update["size_matched"] = json!("1");
    let cancellation = order("CANCELED", "CANCELLATION", "0.40", "5", FUNDER);

    for (sequence, row, expected) in [
        (1, &mut placement, PmOrderStatus::Open),
        (2, &mut update, PmOrderStatus::PartiallyFilled),
    ] {
        let mut role = private_role();
        role.reconnect(ConnectionEpoch::new(1)).unwrap();
        let delivery = receive(&mut role, sequence, row, PmFixtureFeeEvidence::Unknown)
            .unwrap()
            .service_at(30_000)
            .unwrap();
        role.reduce_private_delivery(delivery, |_, envelope| {
            let [PmPrivateLifecycleObservation::Order(order)] = envelope.payload().observations()
            else {
                panic!("order kind must remain an order");
            };
            assert_eq!(order.progress().status(), expected);
        })
        .unwrap();
    }
    let mut role = private_role();
    role.reconnect(ConnectionEpoch::new(1)).unwrap();
    let delivery = receive(&mut role, 1, &cancellation, PmFixtureFeeEvidence::Unknown)
        .unwrap()
        .service_at(30_000)
        .unwrap();
    role.reduce_private_delivery(delivery, |_, envelope| {
        let [PmPrivateLifecycleObservation::Order(order)] = envelope.payload().observations()
        else {
            panic!("cancellation must remain an order lifecycle event");
        };
        assert_eq!(order.progress().status(), PmOrderStatus::Cancelled);
    })
    .unwrap();

    for mut mismatch in [
        order("LIVE", "UPDATE", "0.40", "5", FUNDER),
        order("LIVE", "CANCELLATION", "0.40", "5", FUNDER),
    ] {
        let mut role = private_role();
        role.reconnect(ConnectionEpoch::new(1)).unwrap();
        assert_eq!(
            receive(&mut role, 1, &mismatch, PmFixtureFeeEvidence::Unknown).unwrap_err(),
            PmPrivateNormalizationError::OrderEventKindStatusMismatch
        );
        mismatch["status"] = json!("MYSTERY");
    }

    for (status, settlement) in [
        ("MATCHED", PmFillSettlementStatus::Matched),
        ("MINED", PmFillSettlementStatus::Mined),
        ("CONFIRMED", PmFillSettlementStatus::Confirmed),
        ("RETRYING", PmFillSettlementStatus::Retrying),
        ("FAILED", PmFillSettlementStatus::Failed),
    ] {
        let mut role = private_role();
        role.reconnect(ConnectionEpoch::new(1)).unwrap();
        let trade = direct_trade(
            "settlement",
            "local-order",
            "TAKER",
            status,
            "0.40",
            "5",
            FUNDER,
        );
        let delivery = receive(&mut role, 1, &trade, PmFixtureFeeEvidence::Unknown)
            .unwrap()
            .service_at(30_000)
            .unwrap();
        role.reduce_private_delivery(delivery, |_, envelope| {
            let [PmPrivateLifecycleObservation::Fill(fill)] = envelope.payload().observations()
            else {
                panic!("exact local trade must remain a fill lifecycle event");
            };
            assert_eq!(fill.execution().settlement(), settlement);
        })
        .unwrap();
    }
}

#[test]
fn duplicate_maker_refs_and_reconnect_ordering_fail_closed() {
    let duplicate = maker_trade(json!([
        {
            "order_id": "same-order",
            "asset_id": "123",
            "side": "SELL",
            "price": "0.40",
            "matched_amount": "5"
        },
        {
            "order_id": "same-order",
            "asset_id": "123",
            "side": "SELL",
            "price": "0.40",
            "matched_amount": "5"
        }
    ]));
    let mut role = private_role();
    assert_eq!(
        receive(&mut role, 1, &duplicate, PmFixtureFeeEvidence::Unknown).unwrap_err(),
        PmPrivateNormalizationError::NoActiveEpoch
    );
    role.reconnect(ConnectionEpoch::new(1)).unwrap();
    assert_eq!(
        receive(&mut role, 1, &duplicate, PmFixtureFeeEvidence::Unknown).unwrap_err(),
        PmPrivateNormalizationError::DuplicateOrderReference
    );

    let valid = order("LIVE", "PLACEMENT", "0.40", "5", FUNDER);
    receive(&mut role, 100, &valid, PmFixtureFeeEvidence::Unknown).unwrap();
    assert_eq!(
        receive(&mut role, 100, &valid, PmFixtureFeeEvidence::Unknown).unwrap_err(),
        PmPrivateNormalizationError::IngressSequenceDidNotAdvance
    );
    role.reconnect(ConnectionEpoch::new(2)).unwrap();
    role.receive_user_fixture(
        completion(2, 1, None),
        serde_json::to_string(&valid).unwrap().as_bytes(),
        PmFixtureFeeEvidence::Unknown,
    )
    .expect("later epoch resets ingress ordering");
    assert_eq!(
        role.reconnect(ConnectionEpoch::new(1)).unwrap_err(),
        PmPrivateNormalizationError::ConnectionEpochDidNotAdvance
    );

    let old_epoch = PmFixtureCompletionOccurrence::new(
        ReceivedEventClock::new(None, 40_000, 50_000).unwrap(),
        EventOrdering::new(
            ConnectionEpoch::new(1),
            None,
            None,
            None,
            IngressSequence::new(101),
        )
        .unwrap(),
    );
    assert_eq!(
        role.receive_user_fixture(
            old_epoch,
            serde_json::to_string(&valid).unwrap().as_bytes(),
            PmFixtureFeeEvidence::Unknown,
        )
        .unwrap_err(),
        PmPrivateNormalizationError::ConnectionEpochMismatch
    );
}
