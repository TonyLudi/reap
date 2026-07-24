use std::io::Cursor;

use reap_pm_core::{
    ConnectionEpoch, IngressSequence, PmConnectionId, PmFillId, PmOrderSalt, PmOrderSide, PmPrice,
    PmQuantity, PmVenueOrderId, PmVenueOrderKey, SnapshotRevision, exact_order_amounts,
};

use super::*;
use crate::journal::schema::{
    MAX_PM_ACKNOWLEDGEMENT_FILL_LEGS, PmJournalCancelIntentV1, PmJournalCancelReasonV1,
    PmJournalCancelResultV1, PmJournalFillDeliveryV1, PmJournalFillFeeV1,
    PmJournalFillOccurrenceV1, PmJournalFillRoleV1, PmJournalFillSettlementV1,
    PmJournalFillSourceV1, PmJournalFillV1, PmJournalFillWatermarkV1, PmJournalFingerprintV1,
    PmJournalHeaderV1, PmJournalPlaceRejectReasonV1, PmJournalPlaceResultV1,
    PmJournalQuoteProfileV1, derive_pm_journal_client_order, test_scope,
};

mod negative;

fn units(value: &str) -> U256 {
    PmQuantity::parse_decimal(value)
        .expect("quantity")
        .protocol_units()
}

fn quote(
    scope: &PmJournalScopeV1,
    intent_id: u64,
    side: PmOrderSide,
    quantity: &str,
) -> PmJournalQuoteIntentV1 {
    let price = PmPrice::from_units(500_000).expect("price");
    let quantity = PmQuantity::parse_decimal(quantity).expect("quantity");
    let amounts = exact_order_amounts(side, price, quantity).expect("amounts");
    let (reserved_collateral, reserved_outcome) = match side {
        PmOrderSide::Buy => (amounts.maker(), U256::ZERO),
        PmOrderSide::Sell => (U256::ZERO, quantity.protocol_units()),
    };
    let account_scope = scope.account_scope();
    PmJournalQuoteIntentV1 {
        intent_id,
        client_order: derive_pm_journal_client_order(scope, intent_id).expect("client order"),
        instrument: scope.instrument(),
        side: side.into(),
        price_units: price.units(),
        quantity,
        reserved_collateral,
        reserved_outcome,
        profile: PmJournalQuoteProfileV1::PassiveGtcPostOnlyEoa,
        metadata_revision: 1,
        book_revision: 2,
        model_revision: 3,
        book_readiness_revision: 4,
        private_readiness_revision: 5,
        expires_at_monotonic_ns: 10_000 + intent_id,
        salt: PmOrderSalt::from_u64(intent_id).expect("salt"),
        timestamp_ms: 1_000 + intent_id,
        maker: account_scope.funder().address(),
        signer: account_scope.signer().address(),
        maker_amount: amounts.maker(),
        taker_amount: amounts.taker(),
    }
}

fn venue_order(scope: &PmJournalScopeV1, id: &str) -> PmVenueOrderKey {
    PmVenueOrderKey::new(
        scope.account(),
        PmVenueOrderId::new(id).expect("venue order"),
    )
}

fn fill_key(venue_order: PmVenueOrderKey, id: &str) -> PmJournalFillKeyV1 {
    PmJournalFillKeyV1 {
        venue_order,
        fill_id: PmFillId::new(id).expect("fill id"),
    }
}

fn rejected(intent: PmJournalQuoteIntentV1) -> PmJournalPlaceResultV1 {
    PmJournalPlaceResultV1 {
        client_order: intent.client_order,
        outcome: PmJournalPlaceOutcomeV1::Rejected,
        reject_reason: Some(PmJournalPlaceRejectReasonV1::FixtureRejected),
        venue_order: None,
        immediate_fills: PmJournalImmediateFillsV1::empty(),
    }
}

fn resting(intent: PmJournalQuoteIntentV1, venue_order: PmVenueOrderKey) -> PmJournalPlaceResultV1 {
    PmJournalPlaceResultV1 {
        client_order: intent.client_order,
        outcome: PmJournalPlaceOutcomeV1::AcceptedResting,
        reject_reason: None,
        venue_order: Some(venue_order),
        immediate_fills: PmJournalImmediateFillsV1::empty(),
    }
}

#[derive(Clone, Copy)]
struct TestFillProgress {
    delta: PmQuantity,
    authoritative_cumulative: Option<U256>,
    cumulative: U256,
    remaining: U256,
}

fn fill_progress(
    delta: &str,
    authoritative_cumulative: Option<U256>,
    cumulative: U256,
    remaining: U256,
) -> TestFillProgress {
    TestFillProgress {
        delta: PmQuantity::parse_decimal(delta).expect("fill delta"),
        authoritative_cumulative,
        cumulative,
        remaining,
    }
}

fn applied_fill(
    intent: PmJournalQuoteIntentV1,
    key: PmJournalFillKeyV1,
    progress: TestFillProgress,
    source: PmJournalFillSourceV1,
    occurrence: PmJournalFillOccurrenceV1,
) -> PmJournalFillAppliedV1 {
    PmJournalFillAppliedV1 {
        fill: PmJournalFillV1 {
            key,
            client_order: intent.client_order,
            instrument: intent.instrument,
            side: intent.side,
            price_units: intent.price_units,
            role: PmJournalFillRoleV1::Maker,
            settlement: PmJournalFillSettlementV1::Matched,
            fee: PmJournalFillFeeV1::Unknown,
            delta: progress.delta,
            authoritative_cumulative: progress.authoritative_cumulative,
            cumulative: progress.cumulative,
            remaining: progress.remaining,
        },
        source,
        occurrence,
        delivery: PmJournalFillDeliveryV1::Live,
    }
}

fn acknowledgement_occurrence(owner_sequence: u64, service_ns: u64) -> PmJournalFillOccurrenceV1 {
    PmJournalFillOccurrenceV1 {
        owner_sequence: IngressSequence::new(owner_sequence),
        connection: None,
        connection_epoch: None,
        ingress_sequence: None,
        snapshot_revision: None,
        monotonic_service_ns: service_ns,
    }
}

fn websocket_occurrence(
    owner_sequence: u64,
    epoch: u64,
    ingress: u64,
    service_ns: u64,
) -> PmJournalFillOccurrenceV1 {
    PmJournalFillOccurrenceV1 {
        owner_sequence: IngressSequence::new(owner_sequence),
        connection: Some(PmConnectionId::new("pm-private").expect("connection")),
        connection_epoch: Some(ConnectionEpoch::new(epoch)),
        ingress_sequence: Some(IngressSequence::new(ingress)),
        snapshot_revision: None,
        monotonic_service_ns: service_ns,
    }
}

fn rest_occurrence(
    owner_sequence: u64,
    snapshot: u64,
    epoch: u64,
    ingress: u64,
    service_ns: u64,
) -> PmJournalFillOccurrenceV1 {
    PmJournalFillOccurrenceV1 {
        owner_sequence: IngressSequence::new(owner_sequence),
        connection: Some(PmConnectionId::new("pm-private").expect("connection")),
        connection_epoch: Some(ConnectionEpoch::new(epoch)),
        ingress_sequence: Some(IngressSequence::new(ingress)),
        snapshot_revision: Some(SnapshotRevision::new(snapshot)),
        monotonic_service_ns: service_ns,
    }
}

fn apply(
    recovery: &mut PmJournalRecovery,
    record: PmJournalRecordV1,
) -> Result<(), PmJournalRecoveryError> {
    apply_record(recovery, &record, 1)
}

fn append_line(
    bytes: &mut Vec<u8>,
    scope: &PmJournalScopeV1,
    sequence: u64,
    record: PmJournalRecordV1,
) {
    serde_json::to_writer(
        &mut *bytes,
        &PmJournalLineV1::new(scope.fingerprint(), sequence, record),
    )
    .expect("encode journal line");
    bytes.push(b'\n');
}

#[test]
fn acknowledgement_fill_bound_matches_the_adapter_contract() {
    assert_eq!(
        MAX_PM_ACKNOWLEDGEMENT_FILL_LEGS,
        reap_polymarket_adapter::MAX_PM_FAKE_ACK_FILL_LEGS
    );
}

#[test]
fn tail_intent_and_pending_cancel_recover_as_unresolved() {
    let scope = test_scope();
    let buy = quote(&scope, 1, PmOrderSide::Buy, "1");
    let sell = quote(&scope, 2, PmOrderSide::Sell, "1");
    let sell_venue = venue_order(&scope, "sell-order");
    let mut bytes = Vec::new();
    append_line(
        &mut bytes,
        &scope,
        0,
        PmJournalRecordV1::Header(PmJournalHeaderV1::new(scope.clone())),
    );
    append_line(&mut bytes, &scope, 1, PmJournalRecordV1::QuoteIntent(buy));
    append_line(&mut bytes, &scope, 2, PmJournalRecordV1::QuoteIntent(sell));
    append_line(
        &mut bytes,
        &scope,
        3,
        PmJournalRecordV1::PlaceResult(resting(sell, sell_venue)),
    );
    append_line(
        &mut bytes,
        &scope,
        4,
        PmJournalRecordV1::CancelIntent(PmJournalCancelIntentV1 {
            client_order: sell.client_order,
            venue_order: sell_venue,
            reason: PmJournalCancelReasonV1::Replacement,
        }),
    );

    let recovery =
        recover_lines(&mut Cursor::new(bytes), &scope).expect("recover incomplete effects");
    assert_eq!(recovery.record_count(), 5);
    assert_eq!(recovery.last_sequence(), 4);
    assert_eq!(recovery.owned_order_count(), 2);
    assert_eq!(recovery.unresolved_order_count(), 2);
    assert!(recovery.requires_reconciliation());
}

#[test]
fn acknowledgement_fill_stays_deduplicable_until_explicit_watermark() {
    let scope = test_scope();
    let intent = quote(&scope, 1, PmOrderSide::Buy, "1");
    let venue = venue_order(&scope, "ack-order");
    let key = fill_key(venue, "ack-fill");
    let immediate_fills = PmJournalImmediateFillsV1::from_slice(&[key]).expect("immediate fill");
    let mut recovery = PmJournalRecovery::empty(scope.clone());

    apply(&mut recovery, PmJournalRecordV1::QuoteIntent(intent)).expect("quote");
    apply(
        &mut recovery,
        PmJournalRecordV1::PlaceResult(PmJournalPlaceResultV1 {
            client_order: intent.client_order,
            outcome: PmJournalPlaceOutcomeV1::AcceptedWithImmediateFill,
            reject_reason: None,
            venue_order: Some(venue),
            immediate_fills,
        }),
    )
    .expect("place acknowledgement");
    assert_eq!(recovery.owned_order_count(), 1);
    assert_eq!(recovery.unresolved_order_count(), 1);
    let pending = recovery.recovered_orders().next().expect("pending row");
    assert_eq!(pending.intent(), intent);
    assert_eq!(pending.venue_order(), Some(venue));
    assert_eq!(pending.place(), PmJournalRecoveredPlaceV1::Bound);
    assert_eq!(pending.known_fill_total(), U256::ZERO);
    assert_eq!(pending.authoritative_cumulative(), None);
    assert_eq!(pending.effective_cumulative(), U256::ZERO);
    assert!(!pending.cancel_pending());
    assert!(!pending.cancel_unknown());
    assert_eq!(pending.pending_ack_fill_count(), 1);
    assert_eq!(pending.terminal(), None);

    let applied = PmJournalRecordV1::FillApplied(applied_fill(
        intent,
        key,
        fill_progress("1", Some(units("1")), units("1"), U256::ZERO),
        PmJournalFillSourceV1::PlaceAcknowledgement,
        acknowledgement_occurrence(1, 10),
    ));
    apply(&mut recovery, applied.clone()).expect("applied acknowledgement fill");
    assert_eq!(recovery.owned_order_count(), 1);
    assert_eq!(recovery.fill_key_count(), 1);
    assert_eq!(recovery.compacted_intent_id(), 0);
    assert_eq!(recovery.unresolved_order_count(), 0);
    assert!(matches!(
        apply(&mut recovery, applied),
        Err(PmJournalRecoveryError::DuplicateFill)
    ));

    apply(
        &mut recovery,
        PmJournalRecordV1::FillWatermarkAdvanced(PmJournalFillWatermarkV1 {
            cursor: PmJournalFillCursorV1 {
                account_scope: scope.account_scope(),
                opaque: PmJournalFingerprintV1::from_bytes([0x66; 32]),
            },
        }),
    )
    .expect("deduplication cut");
    assert_eq!(recovery.owned_order_count(), 0);
    assert_eq!(recovery.fill_key_count(), 0);
    assert_eq!(recovery.compacted_intent_id(), 1);
    assert!(!recovery.requires_reconciliation());
}

#[test]
fn multi_order_immediate_fills_share_one_global_bound_and_resolve_once() {
    let scope = test_scope();
    let buy = quote(&scope, 1, PmOrderSide::Buy, "1");
    let sell = quote(&scope, 2, PmOrderSide::Sell, "1");
    let buy_venue = venue_order(&scope, "multi-ack-buy");
    let sell_venue = venue_order(&scope, "multi-ack-sell");
    let buy_key = fill_key(buy_venue, "multi-buy-fill");
    let sell_key = fill_key(sell_venue, "multi-sell-fill");
    let mut recovery = PmJournalRecovery::empty(scope);

    apply(&mut recovery, PmJournalRecordV1::QuoteIntent(buy)).expect("buy quote");
    apply(&mut recovery, PmJournalRecordV1::QuoteIntent(sell)).expect("sell quote");
    for (intent, venue, key) in [(buy, buy_venue, buy_key), (sell, sell_venue, sell_key)] {
        apply(
            &mut recovery,
            PmJournalRecordV1::PlaceResult(PmJournalPlaceResultV1 {
                client_order: intent.client_order,
                outcome: PmJournalPlaceOutcomeV1::AcceptedWithImmediateFill,
                reject_reason: None,
                venue_order: Some(venue),
                immediate_fills: PmJournalImmediateFillsV1::from_slice(&[key])
                    .expect("one immediate fill"),
            }),
        )
        .expect("place acknowledgement");
    }

    assert_eq!(recovery.pending_ack_fill_keys, 2);
    assert_eq!(
        recovery
            .recovered_orders()
            .map(PmJournalRecoveredOrderV1::pending_ack_fill_count)
            .sum::<u8>(),
        2
    );
    assert!(
        ensure_fill_key_capacity(&recovery, MAX_PM_JOURNAL_FILL_KEYS - 2).is_ok(),
        "the shared applied-plus-pending limit admits exactly its remaining capacity"
    );
    assert!(matches!(
        ensure_fill_key_capacity(&recovery, MAX_PM_JOURNAL_FILL_KEYS - 1),
        Err(PmJournalRecoveryError::TooManyFillKeys)
    ));

    let buy_applied = PmJournalRecordV1::FillApplied(applied_fill(
        buy,
        buy_key,
        fill_progress("1", Some(units("1")), units("1"), U256::ZERO),
        PmJournalFillSourceV1::PlaceAcknowledgement,
        acknowledgement_occurrence(1, 10),
    ));
    apply(&mut recovery, buy_applied.clone()).expect("resolve buy acknowledgement fill");
    assert_eq!(recovery.pending_ack_fill_keys, 1);
    assert_eq!(recovery.fill_key_count(), 1);
    assert!(matches!(
        apply(&mut recovery, buy_applied),
        Err(PmJournalRecoveryError::DuplicateFill)
    ));
    assert_eq!(recovery.pending_ack_fill_keys, 1);

    apply(
        &mut recovery,
        PmJournalRecordV1::FillApplied(applied_fill(
            sell,
            sell_key,
            fill_progress("1", Some(units("1")), units("1"), U256::ZERO),
            PmJournalFillSourceV1::PlaceAcknowledgement,
            acknowledgement_occurrence(2, 20),
        )),
    )
    .expect("resolve sell acknowledgement fill");
    assert_eq!(recovery.pending_ack_fill_keys, 0);
    assert_eq!(recovery.fill_key_count(), 2);
    assert_eq!(
        recovery
            .recovered_orders()
            .map(PmJournalRecoveredOrderV1::pending_ack_fill_count)
            .sum::<u8>(),
        0
    );
}

#[test]
fn older_authoritative_jump_does_not_move_canonical_progress_backwards() {
    let scope = test_scope();
    let intent = quote(&scope, 1, PmOrderSide::Buy, "3");
    let venue = venue_order(&scope, "out-of-order");
    let cursor = PmJournalFillCursorV1 {
        account_scope: scope.account_scope(),
        opaque: PmJournalFingerprintV1::from_bytes([0x67; 32]),
    };
    let mut recovery = PmJournalRecovery::empty(scope);
    apply(&mut recovery, PmJournalRecordV1::QuoteIntent(intent)).expect("quote");
    apply(
        &mut recovery,
        PmJournalRecordV1::PlaceResult(resting(intent, venue)),
    )
    .expect("resting");

    apply(
        &mut recovery,
        PmJournalRecordV1::FillApplied(applied_fill(
            intent,
            fill_key(venue, "newer-fill"),
            fill_progress("1", Some(units("2")), units("2"), units("1")),
            PmJournalFillSourceV1::PrivateWebsocket,
            websocket_occurrence(1, 3, 20, 200),
        )),
    )
    .expect("newer authoritative progress");
    apply(
        &mut recovery,
        PmJournalRecordV1::FillApplied(applied_fill(
            intent,
            fill_key(venue, "older-fill"),
            fill_progress("1", Some(units("1")), units("2"), units("1")),
            PmJournalFillSourceV1::PrivateWebsocket,
            websocket_occurrence(2, 3, 10, 100),
        )),
    )
    .expect("older authoritative progress");

    let recovered = recovery.recovered_orders().next().expect("active order");
    assert_eq!(recovered.known_fill_total(), units("2"));
    assert_eq!(recovered.authoritative_cumulative(), Some(units("2")));
    assert_eq!(recovered.effective_cumulative(), units("2"));
    assert_eq!(recovered.terminal(), None);
    let fill_owners = recovery.recovered_fills().collect::<Vec<_>>();
    assert_eq!(fill_owners.len(), 2);
    assert_eq!(fill_owners[0].fill.client_order, intent.client_order);
    assert_eq!(fill_owners[1].fill.client_order, intent.client_order);
    assert_ne!(fill_owners[0].fill.key, fill_owners[1].fill.key);

    apply(
        &mut recovery,
        PmJournalRecordV1::FillApplied(applied_fill(
            intent,
            fill_key(venue, "final-fill"),
            fill_progress("1", None, units("3"), U256::ZERO),
            PmJournalFillSourceV1::PrivateWebsocket,
            websocket_occurrence(3, 3, 30, 300),
        )),
    )
    .expect("final known fill");
    assert_eq!(recovery.owned_order_count(), 1);
    assert_eq!(recovery.fill_key_count(), 3);
    assert_eq!(recovery.compacted_intent_id(), 0);
    apply(
        &mut recovery,
        PmJournalRecordV1::FillWatermarkAdvanced(PmJournalFillWatermarkV1 { cursor }),
    )
    .expect("deduplication cut");
    assert_eq!(recovery.owned_order_count(), 0);
    assert_eq!(recovery.compacted_intent_id(), 1);
    assert!(!recovery.requires_reconciliation());
}

#[test]
fn incomparable_cross_source_cumulative_keeps_the_maximum_without_false_ordering() {
    let scope = test_scope();
    let intent = quote(&scope, 1, PmOrderSide::Buy, "3");
    let venue = venue_order(&scope, "cross-source");
    let mut recovery = PmJournalRecovery::empty(scope);
    apply(&mut recovery, PmJournalRecordV1::QuoteIntent(intent)).expect("quote");
    apply(
        &mut recovery,
        PmJournalRecordV1::PlaceResult(resting(intent, venue)),
    )
    .expect("resting");
    apply(
        &mut recovery,
        PmJournalRecordV1::FillApplied(applied_fill(
            intent,
            fill_key(venue, "websocket-newer"),
            fill_progress("1", Some(units("2")), units("2"), units("1")),
            PmJournalFillSourceV1::PrivateWebsocket,
            websocket_occurrence(1, 4, 20, 200),
        )),
    )
    .expect("websocket progress");
    apply(
        &mut recovery,
        PmJournalRecordV1::FillApplied(applied_fill(
            intent,
            fill_key(venue, "rest-older"),
            fill_progress("1", Some(units("1")), units("2"), units("1")),
            PmJournalFillSourceV1::RestReconciliation,
            rest_occurrence(2, 3, 4, 30, 300),
        )),
    )
    .expect("incomparable older REST progress");
    let recovered = recovery.recovered_orders().next().expect("active order");
    assert_eq!(recovered.known_fill_total(), units("2"));
    assert_eq!(recovered.authoritative_cumulative(), Some(units("2")));
    assert_eq!(recovered.effective_cumulative(), units("2"));
}

#[test]
fn authoritative_full_progress_is_not_compacted_until_known_legs_catch_up() {
    let scope = test_scope();
    let intent = quote(&scope, 1, PmOrderSide::Buy, "3");
    let venue = venue_order(&scope, "authoritative-gap");
    let cursor = PmJournalFillCursorV1 {
        account_scope: scope.account_scope(),
        opaque: PmJournalFingerprintV1::from_bytes([0x55; 32]),
    };
    let mut recovery = PmJournalRecovery::empty(scope);
    apply(&mut recovery, PmJournalRecordV1::QuoteIntent(intent)).expect("quote");
    apply(
        &mut recovery,
        PmJournalRecordV1::PlaceResult(resting(intent, venue)),
    )
    .expect("resting");
    apply(
        &mut recovery,
        PmJournalRecordV1::FillApplied(applied_fill(
            intent,
            fill_key(venue, "observed-leg-1"),
            fill_progress("1", Some(units("3")), units("3"), U256::ZERO),
            PmJournalFillSourceV1::RestReconciliation,
            rest_occurrence(1, 9, 4, 1, 400),
        )),
    )
    .expect("authoritative jump");
    assert_eq!(recovery.owned_order_count(), 1);
    assert_eq!(recovery.unresolved_order_count(), 1);
    assert_eq!(recovery.compacted_intent_id(), 0);
    assert!(recovery.requires_reconciliation());

    apply(
        &mut recovery,
        PmJournalRecordV1::FillWatermarkAdvanced(PmJournalFillWatermarkV1 { cursor }),
    )
    .expect("complete-read watermark");
    assert_eq!(recovery.owned_order_count(), 1);
    assert_eq!(recovery.compacted_intent_id(), 0);

    for (id, ingress, known) in [("observed-leg-2", 2, "2"), ("observed-leg-3", 3, "3")] {
        apply(
            &mut recovery,
            PmJournalRecordV1::FillApplied(applied_fill(
                intent,
                fill_key(venue, id),
                fill_progress("1", None, units("3"), U256::ZERO),
                PmJournalFillSourceV1::PrivateWebsocket,
                websocket_occurrence(ingress, 4, ingress, 400 + ingress),
            )),
        )
        .unwrap_or_else(|error| panic!("known total {known} failed: {error}"));
    }
    assert_eq!(recovery.owned_order_count(), 1);
    assert_eq!(recovery.fill_key_count(), 3);
    assert_eq!(recovery.compacted_intent_id(), 0);
    apply(
        &mut recovery,
        PmJournalRecordV1::FillWatermarkAdvanced(PmJournalFillWatermarkV1 {
            cursor: PmJournalFillCursorV1 {
                account_scope: cursor.account_scope,
                opaque: PmJournalFingerprintV1::from_bytes([0x56; 32]),
            },
        }),
    )
    .expect("post-fill deduplication cut");
    assert_eq!(recovery.owned_order_count(), 0);
    assert_eq!(recovery.fill_key_count(), 0);
    assert_eq!(recovery.compacted_intent_id(), 1);
    assert!(!recovery.requires_reconciliation());
}

#[test]
fn older_occurrence_cannot_claim_more_progress_than_newer_evidence() {
    let scope = test_scope();
    let intent = quote(&scope, 1, PmOrderSide::Buy, "4");
    let venue = venue_order(&scope, "contradictory-order");
    let mut recovery = PmJournalRecovery::empty(scope);
    apply(&mut recovery, PmJournalRecordV1::QuoteIntent(intent)).expect("quote");
    apply(
        &mut recovery,
        PmJournalRecordV1::PlaceResult(resting(intent, venue)),
    )
    .expect("resting");
    apply(
        &mut recovery,
        PmJournalRecordV1::FillApplied(applied_fill(
            intent,
            fill_key(venue, "newer"),
            fill_progress("1", Some(units("2")), units("2"), units("2")),
            PmJournalFillSourceV1::PrivateWebsocket,
            websocket_occurrence(1, 8, 20, 200),
        )),
    )
    .expect("newer evidence");

    assert!(matches!(
        apply(
            &mut recovery,
            PmJournalRecordV1::FillApplied(applied_fill(
                intent,
                fill_key(venue, "older-contradiction"),
                fill_progress("1", Some(units("3")), units("3"), units("1")),
                PmJournalFillSourceV1::PrivateWebsocket,
                websocket_occurrence(2, 8, 10, 100),
            )),
        ),
        Err(PmJournalRecoveryError::ContradictoryAuthoritativeCumulative)
    ));
    let recovered = recovery.recovered_orders().next().expect("active order");
    assert_eq!(recovered.known_fill_total(), units("1"));
    assert_eq!(recovered.authoritative_cumulative(), Some(units("2")));
    assert_eq!(recovery.fill_key_count(), 1);
}

#[test]
fn coordinator_fill_owner_sequence_never_regresses() {
    let scope = test_scope();
    let intent = quote(&scope, 1, PmOrderSide::Buy, "2");
    let venue = venue_order(&scope, "owner-sequence");
    let mut recovery = PmJournalRecovery::empty(scope);
    apply(&mut recovery, PmJournalRecordV1::QuoteIntent(intent)).expect("quote");
    apply(
        &mut recovery,
        PmJournalRecordV1::PlaceResult(resting(intent, venue)),
    )
    .expect("resting");
    apply(
        &mut recovery,
        PmJournalRecordV1::FillApplied(applied_fill(
            intent,
            fill_key(venue, "owner-first"),
            fill_progress("1", Some(units("1")), units("1"), units("1")),
            PmJournalFillSourceV1::PrivateWebsocket,
            websocket_occurrence(2, 1, 10, 100),
        )),
    )
    .expect("first owner occurrence");

    assert!(matches!(
        apply(
            &mut recovery,
            PmJournalRecordV1::FillApplied(applied_fill(
                intent,
                fill_key(venue, "owner-regression"),
                fill_progress("1", Some(units("2")), units("2"), U256::ZERO),
                PmJournalFillSourceV1::PrivateWebsocket,
                websocket_occurrence(1, 1, 20, 200),
            )),
        ),
        Err(
            PmJournalRecoveryError::NonMonotonicOwnedObservationSequence {
                prior: 2,
                actual: 1
            }
        )
    ));
    assert_eq!(recovery.last_owned_observation_sequence(), 2);
    assert_eq!(recovery.fill_key_count(), 1);
}

#[test]
fn bootstrap_rows_preserve_intent_and_fill_owner_order() {
    let scope = test_scope();
    let buy = quote(&scope, 1, PmOrderSide::Buy, "3");
    let sell = quote(&scope, 2, PmOrderSide::Sell, "3");
    let buy_venue = venue_order(&scope, "ordered-buy");
    let sell_venue = venue_order(&scope, "ordered-sell");
    let mut recovery = PmJournalRecovery::empty(scope);

    apply(&mut recovery, PmJournalRecordV1::QuoteIntent(buy)).expect("buy quote");
    apply(
        &mut recovery,
        PmJournalRecordV1::PlaceResult(resting(buy, buy_venue)),
    )
    .expect("buy resting");
    apply(&mut recovery, PmJournalRecordV1::QuoteIntent(sell)).expect("sell quote");
    apply(
        &mut recovery,
        PmJournalRecordV1::PlaceResult(resting(sell, sell_venue)),
    )
    .expect("sell resting");
    apply(
        &mut recovery,
        PmJournalRecordV1::FillApplied(applied_fill(
            buy,
            fill_key(buy_venue, "z-first-by-owner"),
            fill_progress("1", Some(units("1")), units("1"), units("2")),
            PmJournalFillSourceV1::PrivateWebsocket,
            websocket_occurrence(2, 1, 10, 100),
        )),
    )
    .expect("first fill");
    apply(
        &mut recovery,
        PmJournalRecordV1::FillApplied(applied_fill(
            sell,
            fill_key(sell_venue, "a-second-by-owner"),
            fill_progress("1", Some(units("1")), units("1"), units("2")),
            PmJournalFillSourceV1::PrivateWebsocket,
            websocket_occurrence(7, 1, 20, 200),
        )),
    )
    .expect("second fill");

    assert_eq!(
        recovery
            .recovered_orders()
            .map(|row| row.intent().intent_id)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(
        recovery
            .recovered_fills()
            .map(|row| row.occurrence.owner_sequence.value())
            .collect::<Vec<_>>(),
        vec![2, 7]
    );
}

#[test]
fn terminal_cancel_and_replacement_remain_retained_until_the_fill_watermark_cut() {
    let scope = test_scope();
    let account_scope = scope.account_scope();
    let buy = quote(&scope, 1, PmOrderSide::Buy, "1");
    let sell = quote(&scope, 2, PmOrderSide::Sell, "1");
    let buy_venue = venue_order(&scope, "live-buy");
    let mut recovery = PmJournalRecovery::empty(scope);
    apply(&mut recovery, PmJournalRecordV1::QuoteIntent(buy)).expect("buy quote");
    apply(
        &mut recovery,
        PmJournalRecordV1::PlaceResult(resting(buy, buy_venue)),
    )
    .expect("buy resting");
    apply(&mut recovery, PmJournalRecordV1::QuoteIntent(sell)).expect("sell quote");
    apply(
        &mut recovery,
        PmJournalRecordV1::PlaceResult(rejected(sell)),
    )
    .expect("sell rejected");
    assert_eq!(recovery.compacted_intent_id(), 0);

    apply(
        &mut recovery,
        PmJournalRecordV1::CancelIntent(PmJournalCancelIntentV1 {
            client_order: buy.client_order,
            venue_order: buy_venue,
            reason: PmJournalCancelReasonV1::Replacement,
        }),
    )
    .expect("cancel intent");
    apply(
        &mut recovery,
        PmJournalRecordV1::CancelResult(PmJournalCancelResultV1 {
            client_order: buy.client_order,
            venue_order: buy_venue,
            outcome: PmJournalCancelOutcomeV1::Accepted,
            reject_reason: None,
        }),
    )
    .expect("cancel accepted");
    assert_eq!(recovery.owned_order_count(), 2);

    let replacement = quote(&recovery.scope, 3, PmOrderSide::Buy, "1");
    apply(&mut recovery, PmJournalRecordV1::QuoteIntent(replacement))
        .expect("same-slot replacement");
    assert_eq!(recovery.compacted_intent_id(), 0);
    assert_eq!(recovery.owned_order_count(), 3);
    assert_eq!(
        recovery
            .recovered_orders()
            .map(|order| order.intent().intent_id)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );

    apply(
        &mut recovery,
        PmJournalRecordV1::FillWatermarkAdvanced(PmJournalFillWatermarkV1 {
            cursor: PmJournalFillCursorV1 {
                account_scope,
                opaque: PmJournalFingerprintV1::from_bytes([0x43; 32]),
            },
        }),
    )
    .expect("authoritative compaction cut");
    assert_eq!(recovery.compacted_intent_id(), 2);
    assert_eq!(recovery.owned_order_count(), 1);
    assert_eq!(
        recovery
            .recovered_orders()
            .next()
            .expect("replacement remains live")
            .intent(),
        replacement
    );
}

#[test]
fn full_account_fill_watermark_is_retained_and_is_an_explicit_cancel_cut() {
    let scope = test_scope();
    let intent = quote(&scope, 1, PmOrderSide::Buy, "1");
    let venue = venue_order(&scope, "watermark-order");
    let cursor = PmJournalFillCursorV1 {
        account_scope: scope.account_scope(),
        opaque: PmJournalFingerprintV1::from_bytes([0x44; 32]),
    };
    let mut recovery = PmJournalRecovery::empty(scope);
    apply(&mut recovery, PmJournalRecordV1::QuoteIntent(intent)).expect("quote");
    apply(
        &mut recovery,
        PmJournalRecordV1::PlaceResult(resting(intent, venue)),
    )
    .expect("resting");
    apply(
        &mut recovery,
        PmJournalRecordV1::CancelIntent(PmJournalCancelIntentV1 {
            client_order: intent.client_order,
            venue_order: venue,
            reason: PmJournalCancelReasonV1::SafetyHalt,
        }),
    )
    .expect("cancel intent");
    apply(
        &mut recovery,
        PmJournalRecordV1::CancelResult(PmJournalCancelResultV1 {
            client_order: intent.client_order,
            venue_order: venue,
            outcome: PmJournalCancelOutcomeV1::Accepted,
            reject_reason: None,
        }),
    )
    .expect("cancel accepted");
    assert_eq!(recovery.owned_order_count(), 1);

    apply(
        &mut recovery,
        PmJournalRecordV1::FillWatermarkAdvanced(PmJournalFillWatermarkV1 { cursor }),
    )
    .expect("watermark");
    assert_eq!(recovery.fill_watermark(), Some(cursor));
    assert_eq!(recovery.owned_order_count(), 0);
    assert_eq!(recovery.compacted_intent_id(), 1);
    assert!(matches!(
        apply(
            &mut recovery,
            PmJournalRecordV1::FillWatermarkAdvanced(PmJournalFillWatermarkV1 { cursor }),
        ),
        Err(PmJournalRecoveryError::DuplicateWatermark)
    ));
}

#[test]
fn more_than_ten_thousand_terminal_cycles_stay_bounded() {
    let scope = test_scope();
    let account_scope = scope.account_scope();
    let mut recovery = PmJournalRecovery::empty(scope);
    for intent_id in 1..=10_001 {
        let intent = quote(&recovery.scope, intent_id, PmOrderSide::Buy, "1");
        let venue = venue_order(&recovery.scope, &format!("venue-{intent_id}"));
        apply(&mut recovery, PmJournalRecordV1::QuoteIntent(intent)).expect("quote");
        apply(
            &mut recovery,
            PmJournalRecordV1::PlaceResult(resting(intent, venue)),
        )
        .expect("resting");
        apply(
            &mut recovery,
            PmJournalRecordV1::FillApplied(applied_fill(
                intent,
                fill_key(venue, &format!("fill-{intent_id}")),
                fill_progress("1", Some(units("1")), units("1"), U256::ZERO),
                PmJournalFillSourceV1::PrivateWebsocket,
                websocket_occurrence(intent_id, 1, intent_id, intent_id),
            )),
        )
        .expect("filled");
        assert!(recovery.owned_order_count() <= 500);
        assert!(recovery.fill_key_count() <= 500);

        if intent_id % 500 == 0 || intent_id == 10_001 {
            let mut opaque = [0_u8; 32];
            opaque[24..].copy_from_slice(&intent_id.to_be_bytes());
            apply(
                &mut recovery,
                PmJournalRecordV1::FillWatermarkAdvanced(PmJournalFillWatermarkV1 {
                    cursor: PmJournalFillCursorV1 {
                        account_scope,
                        opaque: PmJournalFingerprintV1::from_bytes(opaque),
                    },
                }),
            )
            .expect("bounded generation watermark");
            assert_eq!(recovery.owned_order_count(), 0);
            assert_eq!(recovery.fill_key_count(), 0);
        }
    }
    assert_eq!(recovery.last_intent_id(), 10_001);
    assert_eq!(recovery.last_owned_observation_sequence(), 10_001);
    assert_eq!(recovery.compacted_intent_id(), 10_001);
    assert_eq!(recovery.owned_order_count(), 0);
    assert_eq!(recovery.fill_key_count(), 0);
    assert_eq!(recovery.reserved_capacity_bytes(), 0);
    assert!(!recovery.requires_reconciliation());
}

#[test]
fn intent_high_water_rejects_identity_reuse_after_compaction() {
    let scope = test_scope();
    let intent = quote(&scope, 1, PmOrderSide::Buy, "1");
    let mut recovery = PmJournalRecovery::empty(scope);
    apply(&mut recovery, PmJournalRecordV1::QuoteIntent(intent)).expect("quote");
    apply(
        &mut recovery,
        PmJournalRecordV1::PlaceResult(rejected(intent)),
    )
    .expect("rejected");
    assert!(matches!(
        apply(&mut recovery, PmJournalRecordV1::QuoteIntent(intent),),
        Err(PmJournalRecoveryError::NonMonotonicIntentId {
            prior: 1,
            actual: 1
        })
    ));
}
