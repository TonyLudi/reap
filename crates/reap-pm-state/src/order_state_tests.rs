use reap_pm_core::{
    ConnectionEpoch, EventClock, EventEnvelope, EventOrdering, EvmAddress, IngressSequence,
    MAX_REQUIRED_SPENDERS, PmAccountHandle, PmAccountScope, PmAssetId, PmChainId, PmClientOrderId,
    PmClientOrderKey, PmConditionId, PmConnectionId, PmEnvironmentId, PmExactOrderDetail,
    PmFunderId, PmInstrumentHandle, PmMarketHandle, PmMarketId, PmMarketLifecycle,
    PmMarketMetadata, PmOrderEvent, PmOrderIdentity, PmOrderProgress, PmOrderSide, PmOrderStatus,
    PmOutcomeLabel, PmOutcomeMetadata, PmPrice, PmProductSource, PmQuantity,
    PmReconciliationRequestBoundary, PmSignerId, PmSnapshotEvidence, PmSourceHandle,
    PmSpenderDomain, PmSpenderRequirement, PmTick, PmTokenHandle, PmTokenId, PmVenueOrderId,
    PmVenueOrderKey, SnapshotRevision, U256,
};

use super::{
    MAX_PM_PRIVATE_ORDERS, OrderEntry, OrderOverlap, OwnershipState, PmOrderApply, PmOrderState,
    PmOrderStateError, PmReservationKnowledge, PmReservationTotalsError, compare_entries,
};
use crate::private_config::PmPrivateStateConfig;

const ACCOUNT: PmAccountHandle = PmAccountHandle::from_ordinal(7);
const INSTRUMENT: PmInstrumentHandle = PmInstrumentHandle::new(
    PmMarketHandle::from_ordinal(11),
    PmTokenHandle::from_ordinal(13),
);

fn detail_test_address(byte: u8) -> EvmAddress {
    EvmAddress::from_bytes([byte; 20]).unwrap()
}

fn detail_test_scope() -> PmAccountScope {
    PmAccountScope::new(
        PmEnvironmentId::new("order-state-test").unwrap(),
        PmChainId::new(137).unwrap(),
        PmSignerId::new(detail_test_address(1)),
        PmFunderId::new(detail_test_address(2)),
        ACCOUNT,
    )
}

fn detail_test_source() -> PmProductSource {
    PmProductSource::polymarket_account(PmSourceHandle::from_ordinal(4), ACCOUNT)
}

fn detail_test_config() -> PmPrivateStateConfig {
    let chain = PmChainId::new(137).unwrap();
    let exchange = EvmAddress::parse("0xE111180000d2663C0091e4f400237545B87B996B").unwrap();
    let token = PmTokenId::new(U256::from_u64(123)).unwrap();
    let collateral = PmAssetId::collateral(
        EvmAddress::parse("0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB").unwrap(),
    );
    let outcome = PmAssetId::outcome(
        EvmAddress::parse("0x4D97DCd97eC945f40cF65F87097ACe5EA0476045").unwrap(),
        token,
    );
    let mut spenders = [None; MAX_REQUIRED_SPENDERS];
    spenders[0] = Some(PmSpenderRequirement::new(
        chain,
        exchange,
        PmSpenderDomain::Standard,
        collateral,
    ));
    spenders[1] = Some(PmSpenderRequirement::new(
        chain,
        exchange,
        PmSpenderDomain::Standard,
        outcome,
    ));
    let metadata = PmMarketMetadata::new(
        PmConditionId::parse("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .unwrap(),
        PmMarketId::parse("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
            .unwrap(),
        PmOutcomeMetadata::new(token, PmOutcomeLabel::new("YES").unwrap()),
        PmMarketLifecycle::new(true, false, false, true, true),
        PmTick::parse_decimal("0.01").unwrap(),
        PmQuantity::parse_decimal("1").unwrap(),
        false,
        chain,
        exchange,
        spenders,
        2,
    )
    .unwrap();
    PmPrivateStateConfig::new(
        detail_test_source(),
        detail_test_scope(),
        INSTRUMENT,
        metadata,
    )
    .unwrap()
}

fn detail_test_boundary(request: u64, completion: u64) -> PmReconciliationRequestBoundary {
    PmReconciliationRequestBoundary::new(
        IngressSequence::new(request),
        IngressSequence::new(completion),
    )
    .unwrap()
}

fn detail_test_envelope(
    revision: u64,
    request: u64,
    completion: u64,
    requested_order: PmVenueOrderKey,
    order: Option<PmOrderEvent>,
) -> EventEnvelope<PmExactOrderDetail> {
    let detail = PmExactOrderDetail::new(
        detail_test_source(),
        detail_test_scope(),
        PmSnapshotEvidence::new(SnapshotRevision::new(revision)).unwrap(),
        detail_test_boundary(request, completion),
        requested_order,
        order,
    )
    .unwrap();
    EventEnvelope::new(
        detail_test_source().venue(),
        detail_test_source(),
        PmConnectionId::new("order-state-detail-test").unwrap(),
        EventClock::new(None, 1_000 + completion, completion, completion).unwrap(),
        EventOrdering::new(
            ConnectionEpoch::new(1),
            Some(SnapshotRevision::new(revision)),
            None,
            None,
            IngressSequence::new(completion),
        )
        .unwrap(),
        detail,
    )
    .unwrap()
}

fn detail_test_open_order(venue_order: PmVenueOrderKey) -> PmOrderEvent {
    PmOrderEvent::new(
        detail_test_source(),
        INSTRUMENT,
        PmOrderIdentity::new(None, Some(venue_order)).unwrap(),
        PmOrderSide::Buy,
        PmPrice::parse_decimal("0.40").unwrap(),
        PmOrderProgress::new(
            PmQuantity::parse_decimal("1").unwrap(),
            U256::ZERO,
            PmOrderStatus::Open,
        )
        .unwrap(),
    )
    .unwrap()
}

fn client(ordinal: usize) -> PmClientOrderKey {
    let mut bytes = [0_u8; 16];
    bytes[8..].copy_from_slice(
        &u64::try_from(ordinal.saturating_add(1))
            .expect("test ordinal fits u64")
            .to_be_bytes(),
    );
    PmClientOrderKey::new(ACCOUNT, PmClientOrderId::from_bytes(bytes).unwrap())
}

fn venue(ordinal: usize) -> PmVenueOrderKey {
    PmVenueOrderKey::new(
        ACCOUNT,
        PmVenueOrderId::new(&format!("venue-{ordinal:04}")).unwrap(),
    )
}

fn client_only(ordinal: usize) -> PmOrderIdentity {
    PmOrderIdentity::new(Some(client(ordinal)), None).unwrap()
}

fn venue_only(ordinal: usize) -> PmOrderIdentity {
    PmOrderIdentity::new(None, Some(venue(ordinal))).unwrap()
}

fn complete(client_ordinal: usize, venue_ordinal: usize) -> PmOrderIdentity {
    PmOrderIdentity::new(Some(client(client_ordinal)), Some(venue(venue_ordinal))).unwrap()
}

fn entry(identity: PmOrderIdentity) -> OrderEntry {
    OrderEntry {
        identity,
        instrument: INSTRUMENT,
        event: None,
        ownership: OwnershipState::Unmanaged,
        registered_terms: None,
        reservation: None,
        missing_from_complete_open_snapshot: false,
        terminal_by_detail_absence: false,
        last_occurrence: None,
    }
}

fn proven_owned_terminal_entry(ordinal: usize) -> OrderEntry {
    let price = PmPrice::parse_decimal("0.40").unwrap();
    let quantity = PmQuantity::parse_decimal("1").unwrap();
    OrderEntry {
        identity: client_only(ordinal),
        instrument: INSTRUMENT,
        event: None,
        ownership: OwnershipState::ProvenOwned,
        registered_terms: Some((PmOrderSide::Buy, price, quantity)),
        reservation: Some(PmReservationKnowledge::Known(
            super::PmExactReservation::policy_approved(U256::from_u64(400_000), U256::ZERO)
                .unwrap(),
        )),
        missing_from_complete_open_snapshot: false,
        terminal_by_detail_absence: true,
        last_occurrence: None,
    }
}

fn assert_strictly_ordered(state: &PmOrderState) {
    let canonical = state.canonical_entries().copied().collect::<Vec<_>>();
    assert!(
        canonical
            .windows(2)
            .all(|pair| compare_entries(&pair[0], &pair[1]).is_lt())
    );
    assert_eq!(state.entries.len(), state.canonical_index.len());
    let mut referenced = vec![false; state.entries.len()];
    for dense_slot in state.canonical_index.iter().copied().map(usize::from) {
        assert!(dense_slot < state.entries.len());
        assert!(!referenced[dense_slot]);
        referenced[dense_slot] = true;
    }
    assert!(referenced.into_iter().all(|present| present));

    let mut client_referenced = vec![false; state.entries.len()];
    let mut prior_client = None;
    for dense_slot in state.client_index.iter().copied().map(usize::from) {
        assert!(dense_slot < state.entries.len());
        assert!(!client_referenced[dense_slot]);
        client_referenced[dense_slot] = true;
        let client = state.entries[dense_slot]
            .identity
            .client_order_key()
            .expect("client index references only client-bearing rows");
        assert!(prior_client.is_none_or(|prior| prior < client));
        prior_client = Some(client);
    }
    for (dense_slot, entry) in state.entries.iter().enumerate() {
        assert_eq!(
            client_referenced[dense_slot],
            entry.identity.client_order_key().is_some()
        );
    }
    assert_eq!(
        usize::from(state.live_count),
        state.entries.iter().filter(|entry| entry.is_live()).count(),
        "the exact live-row scalar must match the dense rows"
    );
}

fn canonical_entries(state: &PmOrderState) -> Vec<OrderEntry> {
    state.canonical_entries().copied().collect()
}

type CanonicalDecisionProjection = (
    bool,
    Option<PmOrderIdentity>,
    Result<(U256, U256), PmReservationTotalsError>,
    u16,
    u16,
);

fn canonical_decision_oracle(state: &PmOrderState) -> CanonicalDecisionProjection {
    let mut unmanaged_ambiguity = false;
    let mut first_unknown = None;
    let mut totals = Ok((U256::ZERO, U256::ZERO));
    let mut live_count = 0_u16;
    let mut unresolved_count = 0_u16;
    for entry in state.canonical_entries().filter(|entry| entry.is_live()) {
        live_count += 1;
        let ambiguous = entry.ownership == OwnershipState::Ambiguous;
        unmanaged_ambiguity |= ambiguous;
        if ambiguous || entry.reservation == Some(PmReservationKnowledge::Unknown) {
            first_unknown.get_or_insert(entry.identity);
        }
        if ambiguous || entry.event.is_none() || entry.missing_from_complete_open_snapshot {
            unresolved_count += 1;
        }
        let Ok((collateral, outcome)) = totals else {
            continue;
        };
        totals = match entry.reservation {
            None => Ok((collateral, outcome)),
            Some(PmReservationKnowledge::Unknown) => {
                Err(PmReservationTotalsError::Unknown(entry.identity))
            }
            Some(PmReservationKnowledge::Known(reservation)) => collateral
                .checked_add(reservation.collateral())
                .and_then(|collateral| {
                    outcome
                        .checked_add(reservation.outcome())
                        .map(|outcome| (collateral, outcome))
                })
                .map_err(|_| PmReservationTotalsError::Overflow),
        };
    }
    (
        unmanaged_ambiguity,
        first_unknown,
        totals,
        live_count,
        unresolved_count,
    )
}

fn assert_decision_summary_matches_canonical_oracle(state: &PmOrderState) {
    let summary = state.decision_summary();
    let (ambiguity, first_unknown, totals, live_count, unresolved_count) =
        canonical_decision_oracle(state);
    assert_eq!(summary.has_unmanaged_ambiguity(), ambiguity);
    assert_eq!(summary.first_unknown_reservation(), first_unknown);
    assert_eq!(summary.reservation_totals(), totals);
    assert_eq!(summary.live_count(), live_count);
    assert_eq!(summary.unresolved_count(), unresolved_count);
}

#[test]
fn exact_live_count_tracks_insert_replace_and_remove_transitions() {
    let mut state = PmOrderState::new();
    let live = entry(client_only(1));
    state.insert(live).unwrap();
    assert_eq!(state.live_count, 1);
    assert_decision_summary_matches_canonical_oracle(&state);

    let live_slot = state.find(live.identity).unwrap();
    let mut terminal = live;
    terminal.terminal_by_detail_absence = true;
    state.replace_ordered(live_slot, terminal).unwrap();
    assert_eq!(state.live_count, 0);
    let zero = state.decision_summary();
    assert!(!zero.has_unmanaged_ambiguity());
    assert_eq!(zero.first_unknown_reservation(), None);
    assert_eq!(zero.reservation_totals(), Ok((U256::ZERO, U256::ZERO)));
    assert_eq!(zero.live_count(), 0);
    assert_eq!(zero.unresolved_count(), 0);
    assert_decision_summary_matches_canonical_oracle(&state);

    state.replace_ordered(live_slot, live).unwrap();
    assert_eq!(state.live_count, 1);

    let already_terminal = proven_owned_terminal_entry(2);
    state.insert(already_terminal).unwrap();
    assert_eq!(state.live_count, 1);
    assert_decision_summary_matches_canonical_oracle(&state);

    let live_slot = state.find(live.identity).unwrap();
    assert_eq!(state.remove_dense(live_slot), live);
    assert_eq!(state.live_count, 0);
    assert_decision_summary_matches_canonical_oracle(&state);

    let terminal_slot = state.find(already_terminal.identity).unwrap();
    assert_eq!(state.remove_dense(terminal_slot), already_terminal);
    assert_eq!(state.live_count, 0);
    assert_strictly_ordered(&state);
    assert_decision_summary_matches_canonical_oracle(&state);
}

#[test]
fn detail_absence_duplicate_and_reactivation_preserve_exact_live_count() {
    let config = detail_test_config();
    let venue_order = venue(700);
    let identity = PmOrderIdentity::new(None, Some(venue_order)).unwrap();
    let mut state = PmOrderState::new();
    state.insert(entry(identity)).unwrap();
    assert_eq!(state.live_count, 1);

    assert_eq!(
        state
            .apply_detail(
                detail_test_envelope(1, 10, 11, venue_order, None),
                PmReservationKnowledge::Unknown,
                &config,
            )
            .unwrap(),
        PmOrderApply::DetailAbsenceTerminalized
    );
    assert_eq!(state.live_count, 0);
    assert_decision_summary_matches_canonical_oracle(&state);

    assert_eq!(
        state
            .apply_detail(
                detail_test_envelope(1, 10, 11, venue_order, None),
                PmReservationKnowledge::Unknown,
                &config,
            )
            .unwrap(),
        PmOrderApply::DetailAbsenceIgnoredAfterLaterEvent
    );
    assert_eq!(state.live_count, 0);

    assert_eq!(
        state
            .apply_detail(
                detail_test_envelope(2, 12, 13, venue_order, None),
                PmReservationKnowledge::Unknown,
                &config,
            )
            .unwrap(),
        PmOrderApply::Duplicate
    );
    assert_eq!(state.live_count, 0);

    let open = detail_test_open_order(venue_order);
    assert_eq!(
        state
            .apply_detail(
                detail_test_envelope(3, 14, 15, venue_order, Some(open)),
                PmReservationKnowledge::Unknown,
                &config,
            )
            .unwrap(),
        PmOrderApply::Updated
    );
    assert_eq!(state.live_count, 1);
    assert_strictly_ordered(&state);
    assert_decision_summary_matches_canonical_oracle(&state);
}

fn shuffle(ordinals: &mut [usize]) {
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    for upper in (1..ordinals.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let index = usize::try_from(seed % u64::try_from(upper + 1).unwrap()).unwrap();
        ordinals.swap(upper, index);
    }
}

#[test]
fn ordered_insert_matches_full_sort_at_fixed_capacity_without_growing() {
    let mut state = PmOrderState::new();
    let reserved_capacity = state.entries.capacity();
    let reserved_pointer = state.entries.as_ptr();
    let index_capacity = state.canonical_index.capacity();
    let index_pointer = state.canonical_index.as_ptr();
    let client_capacity = state.client_index.capacity();
    let client_pointer = state.client_index.as_ptr();
    let mut insertion_order = (0..MAX_PM_PRIVATE_ORDERS).collect::<Vec<_>>();
    shuffle(&mut insertion_order);

    let mut expected = Vec::with_capacity(MAX_PM_PRIVATE_ORDERS);
    for ordinal in insertion_order {
        let identity = if ordinal % 2 == 0 {
            client_only(ordinal)
        } else {
            venue_only(ordinal)
        };
        let next = entry(identity);
        expected.push(next);
        state.insert(next).unwrap();
        assert_strictly_ordered(&state);
        assert_eq!(state.entries.capacity(), reserved_capacity);
        assert_eq!(state.entries.as_ptr(), reserved_pointer);
        assert_eq!(state.canonical_index.capacity(), index_capacity);
        assert_eq!(state.canonical_index.as_ptr(), index_pointer);
        assert_eq!(state.client_index.capacity(), client_capacity);
        assert_eq!(state.client_index.as_ptr(), client_pointer);
    }

    expected.sort_unstable_by(compare_entries);
    assert_eq!(canonical_entries(&state), expected);
    for (client_position, dense_slot) in state.client_index.iter().copied().enumerate() {
        let dense_slot = usize::from(dense_slot);
        let client = state.entries[dense_slot]
            .identity
            .client_order_key()
            .unwrap();
        assert_eq!(
            state.client_search(client),
            Ok((client_position, dense_slot))
        );
    }
    assert_eq!(
        state.insert(entry(client_only(MAX_PM_PRIVATE_ORDERS))),
        Err(PmOrderStateError::Capacity)
    );
    assert_eq!(state.entries.capacity(), reserved_capacity);
    assert_eq!(state.entries.as_ptr(), reserved_pointer);
    assert_eq!(state.canonical_index.capacity(), index_capacity);
    assert_eq!(state.canonical_index.as_ptr(), index_pointer);
    assert_eq!(state.client_index.capacity(), client_capacity);
    assert_eq!(state.client_index.as_ptr(), client_pointer);
}

#[test]
fn adversarial_identity_enrichment_repositions_only_the_changed_entry() {
    const ENTRIES: usize = 257;
    let mut state = PmOrderState::new();
    for ordinal in 0..ENTRIES {
        state.insert(entry(client_only(ordinal))).unwrap();
    }
    let reserved_capacity = state.entries.capacity();
    let reserved_pointer = state.entries.as_ptr();
    let index_capacity = state.canonical_index.capacity();
    let index_pointer = state.canonical_index.as_ptr();
    let client_capacity = state.client_index.capacity();
    let client_pointer = state.client_index.as_ptr();
    let mut enrichment_order = (0..ENTRIES).collect::<Vec<_>>();
    shuffle(&mut enrichment_order);

    for client_ordinal in enrichment_order {
        let venue_ordinal = ENTRIES - client_ordinal;
        let lookup = client_only(client_ordinal);
        let index = state.find(lookup).expect("client identity remains indexed");
        let mut replacement = state.entries[index];
        replacement.identity = complete(client_ordinal, venue_ordinal);
        state.replace_ordered(index, replacement).unwrap();

        assert_eq!(
            state.find(replacement.identity),
            Some(
                state
                    .entries
                    .iter()
                    .position(|candidate| candidate.identity == replacement.identity)
                    .unwrap()
            )
        );
        assert_strictly_ordered(&state);
        assert_eq!(state.entries.capacity(), reserved_capacity);
        assert_eq!(state.entries.as_ptr(), reserved_pointer);
        assert_eq!(state.canonical_index.capacity(), index_capacity);
        assert_eq!(state.canonical_index.as_ptr(), index_pointer);
        assert_eq!(state.client_index.capacity(), client_capacity);
        assert_eq!(state.client_index.as_ptr(), client_pointer);
    }

    let mut expected = state.entries.clone();
    expected.sort_unstable_by(compare_entries);
    assert_eq!(canonical_entries(&state), expected);
}

#[test]
fn venue_only_enrichment_adds_clients_without_reallocating_indexes() {
    const ENTRIES: usize = 129;
    let mut state = PmOrderState::new();
    let mut insertion_order = (0..ENTRIES).collect::<Vec<_>>();
    shuffle(&mut insertion_order);
    for ordinal in insertion_order {
        state.insert(entry(venue_only(ordinal))).unwrap();
    }
    let entries_pointer = state.entries.as_ptr();
    let canonical_pointer = state.canonical_index.as_ptr();
    let client_pointer = state.client_index.as_ptr();
    let mut enrichment_order = (0..ENTRIES).collect::<Vec<_>>();
    shuffle(&mut enrichment_order);

    for venue_ordinal in enrichment_order {
        let dense_slot = state.find(venue_only(venue_ordinal)).unwrap();
        let mut replacement = state.entries[dense_slot];
        replacement.identity = complete(ENTRIES - venue_ordinal, venue_ordinal);
        state.replace_ordered(dense_slot, replacement).unwrap();
        assert_eq!(state.find(replacement.identity), Some(dense_slot));
        assert_strictly_ordered(&state);
        assert_eq!(state.entries.as_ptr(), entries_pointer);
        assert_eq!(state.canonical_index.as_ptr(), canonical_pointer);
        assert_eq!(state.client_index.as_ptr(), client_pointer);
    }

    let mut expected = state.entries.clone();
    expected.sort_unstable_by(compare_entries);
    assert_eq!(canonical_entries(&state), expected);
    assert_eq!(state.client_index.len(), ENTRIES);
}

#[test]
fn bridge_coalesce_preserves_total_order_and_canonical_identity() {
    let mut state = PmOrderState::new();
    for ordinal in [4, 2, 8, 6] {
        state.insert(entry(client_only(ordinal))).unwrap();
    }
    state.insert(entry(client_only(31))).unwrap();
    state.insert(entry(venue_only(17))).unwrap();
    for ordinal in [3, 19, 1, 9] {
        state.insert(entry(venue_only(ordinal))).unwrap();
    }
    let reserved_capacity = state.entries.capacity();
    let reserved_pointer = state.entries.as_ptr();
    let index_pointer = state.canonical_index.as_ptr();
    let client_pointer = state.client_index.as_ptr();
    let bridge = complete(31, 17);
    let client_half = state.find(client_only(31)).unwrap();

    let OrderOverlap::Bridge(first, second) = state.overlaps(bridge).unwrap() else {
        panic!("client and venue halves must form a bridge");
    };
    assert_eq!(state.find(bridge), Some(client_half));
    assert_eq!(first, client_half);
    state.coalesce_entries(first, second).unwrap();

    assert_eq!(state.entries.len(), 9);
    let dense_slot = state.find(bridge).expect("merged bridge remains indexed");
    assert_eq!(state.entries[dense_slot].identity, bridge);
    assert_eq!(
        canonical_entries(&state)
            .iter()
            .position(|candidate| candidate.identity == bridge),
        Some(7)
    );
    assert_strictly_ordered(&state);
    assert_eq!(state.entries.capacity(), reserved_capacity);
    assert_eq!(state.entries.as_ptr(), reserved_pointer);
    assert_eq!(state.canonical_index.as_ptr(), index_pointer);
    assert_eq!(state.client_index.as_ptr(), client_pointer);
}

#[test]
fn unchanged_sort_key_updates_in_place() {
    let mut state = PmOrderState::new();
    for ordinal in [9, 3, 7, 1] {
        state.insert(entry(complete(ordinal, ordinal))).unwrap();
    }
    let target = complete(7, 7);
    let index = state.find(target).unwrap();
    let reserved_pointer = state.entries.as_ptr();
    let mut replacement = state.entries[index];
    replacement.missing_from_complete_open_snapshot = true;

    state.replace_ordered(index, replacement).unwrap();

    assert_eq!(state.entries.as_ptr(), reserved_pointer);
    assert_eq!(state.find(target), Some(index));
    assert!(state.entries[index].missing_from_complete_open_snapshot);
    assert_strictly_ordered(&state);
}

#[test]
fn canonical_and_client_collisions_fail_without_mutating_indexes() {
    let mut state = PmOrderState::new();
    for ordinal in [1, 2, 3] {
        state.insert(entry(client_only(ordinal))).unwrap();
    }
    state.insert(entry(venue_only(50))).unwrap();
    let original_entries = state.entries.clone();
    let original_canonical_index = state.canonical_index.clone();
    let original_client_index = state.client_index.clone();

    assert_eq!(
        state.insert(entry(complete(2, 99))),
        Err(PmOrderStateError::IdentityConflict)
    );
    assert_eq!(
        state.insert(entry(venue_only(50))),
        Err(PmOrderStateError::IdentityConflict)
    );
    assert_eq!(state.entries, original_entries);
    assert_eq!(state.canonical_index, original_canonical_index);
    assert_eq!(state.client_index, original_client_index);

    let index = state.find(client_only(3)).unwrap();
    let mut replacement = state.entries[index];
    replacement.identity = complete(1, 101);
    assert_eq!(
        state.replace_ordered(index, replacement),
        Err(PmOrderStateError::IdentityConflict)
    );
    assert_eq!(state.entries, original_entries);
    assert_eq!(state.canonical_index, original_canonical_index);
    assert_eq!(state.client_index, original_client_index);
    assert_strictly_ordered(&state);
}

#[test]
fn coalesce_merge_conflict_fails_before_removing_bridge_halves() {
    let mut state = PmOrderState::new();
    state.insert(entry(client_only(5))).unwrap();
    state.insert(entry(venue_only(7))).unwrap();
    let venue_index = state.find(venue_only(7)).unwrap();
    state.entries[venue_index].instrument = PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(17),
        PmTokenHandle::from_ordinal(19),
    );
    let original_entries = state.entries.clone();
    let original_canonical_index = state.canonical_index.clone();
    let original_client_index = state.client_index.clone();
    let client_index = state.find(client_only(5)).unwrap();

    assert_eq!(
        state.coalesce_entries(client_index, venue_index),
        Err(PmOrderStateError::IdentityConflict)
    );
    assert_eq!(state.entries, original_entries);
    assert_eq!(state.canonical_index, original_canonical_index);
    assert_eq!(state.client_index, original_client_index);
    assert_strictly_ordered(&state);
}

#[test]
fn arbitrary_dense_removal_repairs_swap_references_and_matches_sort_oracle() {
    const ENTRIES: usize = 257;
    let mut state = PmOrderState::new();
    for ordinal in 0..ENTRIES {
        let identity = if ordinal % 3 == 0 {
            complete(ordinal, ENTRIES + ordinal)
        } else if ordinal % 3 == 1 {
            client_only(ordinal)
        } else {
            venue_only(ordinal)
        };
        state.insert(entry(identity)).unwrap();
    }
    let entries_pointer = state.entries.as_ptr();
    let index_pointer = state.canonical_index.as_ptr();
    let client_pointer = state.client_index.as_ptr();
    let mut removal_order = (0..ENTRIES).collect::<Vec<_>>();
    shuffle(&mut removal_order);

    for ordinal in removal_order {
        let identity = if ordinal % 3 == 0 {
            complete(ordinal, ENTRIES + ordinal)
        } else if ordinal % 3 == 1 {
            client_only(ordinal)
        } else {
            venue_only(ordinal)
        };
        let dense_slot = state
            .find(identity)
            .expect("retained identity remains indexed");
        assert_eq!(state.remove_dense(dense_slot).identity, identity);
        let mut expected = state.entries.clone();
        expected.sort_unstable_by(compare_entries);
        assert_eq!(canonical_entries(&state), expected);
        assert_strictly_ordered(&state);
        assert_eq!(state.entries.as_ptr(), entries_pointer);
        assert_eq!(state.canonical_index.as_ptr(), index_pointer);
        assert_eq!(state.client_index.as_ptr(), client_pointer);
    }
}

#[test]
fn proven_owned_terminal_compaction_preserves_index_and_fixed_storage() {
    const ENTRIES: usize = 97;
    let mut state = PmOrderState::new();
    let mut insertion_order = (0..ENTRIES).collect::<Vec<_>>();
    shuffle(&mut insertion_order);
    for ordinal in insertion_order {
        state.insert(proven_owned_terminal_entry(ordinal)).unwrap();
    }
    let entries_pointer = state.entries.as_ptr();
    let index_pointer = state.canonical_index.as_ptr();
    let client_pointer = state.client_index.as_ptr();
    let entries_capacity = state.entries.capacity();
    let index_capacity = state.canonical_index.capacity();
    let client_capacity = state.client_index.capacity();
    let mut compaction_order = (0..ENTRIES).collect::<Vec<_>>();
    shuffle(&mut compaction_order);

    for ordinal in compaction_order {
        state.compact_proven_owned(client(ordinal)).unwrap();
        let mut expected = state.entries.clone();
        expected.sort_unstable_by(compare_entries);
        assert_eq!(canonical_entries(&state), expected);
        assert_strictly_ordered(&state);
        assert_eq!(state.entries.as_ptr(), entries_pointer);
        assert_eq!(state.canonical_index.as_ptr(), index_pointer);
        assert_eq!(state.client_index.as_ptr(), client_pointer);
        assert_eq!(state.entries.capacity(), entries_capacity);
        assert_eq!(state.canonical_index.capacity(), index_capacity);
        assert_eq!(state.client_index.capacity(), client_capacity);
    }
}

#[test]
fn decision_summary_preserves_canonical_unknown_and_global_ambiguity_precedence() {
    let mut state = PmOrderState::new();
    let mut terminal_ambiguity = entry(client_only(0));
    terminal_ambiguity.ownership = OwnershipState::Ambiguous;
    terminal_ambiguity.reservation = Some(PmReservationKnowledge::Unknown);
    terminal_ambiguity.terminal_by_detail_absence = true;
    state.insert(terminal_ambiguity).unwrap();

    let mut known = entry(client_only(1));
    known.ownership = OwnershipState::ProvenOwned;
    known.reservation = Some(PmReservationKnowledge::Known(
        super::PmExactReservation::policy_approved(U256::from_u64(10), U256::ZERO).unwrap(),
    ));
    state.insert(known).unwrap();

    let mut later_ambiguity = entry(client_only(3));
    later_ambiguity.ownership = OwnershipState::Ambiguous;
    later_ambiguity.reservation = Some(PmReservationKnowledge::Unknown);
    state.insert(later_ambiguity).unwrap();

    let mut first_unknown = entry(client_only(2));
    first_unknown.reservation = Some(PmReservationKnowledge::Unknown);
    state.insert(first_unknown).unwrap();

    let summary = state.decision_summary();
    assert!(summary.has_unmanaged_ambiguity());
    assert_eq!(
        summary.first_unknown_reservation(),
        Some(first_unknown.identity)
    );
    assert_eq!(
        summary.reservation_totals(),
        Err(PmReservationTotalsError::Unknown(first_unknown.identity))
    );
    assert_eq!(summary.live_count(), 3);
    assert_eq!(summary.unresolved_count(), 3);
}

#[test]
fn decision_summary_accumulates_exact_totals_and_retains_first_arithmetic_error() {
    let mut exact = PmOrderState::new();
    let mut collateral = entry(client_only(1));
    collateral.reservation = Some(PmReservationKnowledge::Known(
        super::PmExactReservation::policy_approved(U256::from_u64(10), U256::ZERO).unwrap(),
    ));
    exact.insert(collateral).unwrap();
    let mut outcome = entry(client_only(2));
    outcome.reservation = Some(PmReservationKnowledge::Known(
        super::PmExactReservation::policy_approved(U256::ZERO, U256::from_u64(7)).unwrap(),
    ));
    exact.insert(outcome).unwrap();

    let summary = exact.decision_summary();
    assert_eq!(
        summary.reservation_totals(),
        Ok((U256::from_u64(10), U256::from_u64(7)))
    );
    assert_eq!(summary.live_count(), 2);
    assert_eq!(summary.unresolved_count(), 2);

    let mut overflow = PmOrderState::new();
    let mut maximum = entry(client_only(1));
    maximum.reservation = Some(PmReservationKnowledge::Known(
        super::PmExactReservation::policy_approved(U256::MAX, U256::ZERO).unwrap(),
    ));
    overflow.insert(maximum).unwrap();
    let mut one = entry(client_only(2));
    one.reservation = Some(PmReservationKnowledge::Known(
        super::PmExactReservation::policy_approved(U256::ONE, U256::ZERO).unwrap(),
    ));
    overflow.insert(one).unwrap();

    assert_eq!(
        overflow.decision_summary().reservation_totals(),
        Err(PmReservationTotalsError::Overflow)
    );
}

#[test]
fn dense_decision_summary_matches_canonical_oracle_for_random_exact_rows() {
    const ENTRIES: usize = 257;
    let mut state = PmOrderState::new();
    let mut insertion_order = (0..ENTRIES).collect::<Vec<_>>();
    shuffle(&mut insertion_order);
    for ordinal in insertion_order {
        let mut next = entry(client_only(ordinal));
        next.ownership = if ordinal % 3 == 0 {
            OwnershipState::ProvenOwned
        } else {
            OwnershipState::Unmanaged
        };
        next.reservation = Some(PmReservationKnowledge::Known(
            super::PmExactReservation::policy_approved(
                U256::from_u64(u64::try_from(ordinal + 1).unwrap()),
                U256::from_u64(u64::try_from(ordinal % 5).unwrap()),
            )
            .unwrap(),
        ));
        next.missing_from_complete_open_snapshot = ordinal % 5 == 0;
        next.terminal_by_detail_absence = ordinal % 7 == 0;
        state.insert(next).unwrap();
    }

    assert_ne!(
        state
            .entries
            .iter()
            .map(|entry| entry.identity)
            .collect::<Vec<_>>(),
        canonical_entries(&state)
            .into_iter()
            .map(|entry| entry.identity)
            .collect::<Vec<_>>()
    );
    assert!(state.decision_summary().reservation_totals().is_ok());
    assert_decision_summary_matches_canonical_oracle(&state);
}

#[test]
fn canonical_fallback_preserves_unknown_overflow_precedence_after_dense_error() {
    fn precedence_state(unknown_ordinal: usize, dense_prefix: &[usize]) -> PmOrderState {
        const ENTRIES: usize = 31;
        let mut remainder = (0..ENTRIES)
            .filter(|ordinal| !dense_prefix.contains(ordinal))
            .collect::<Vec<_>>();
        shuffle(&mut remainder);
        let mut insertion_order = dense_prefix.to_vec();
        insertion_order.extend(remainder);
        let mut state = PmOrderState::new();
        for ordinal in insertion_order {
            let mut next = entry(client_only(ordinal));
            next.missing_from_complete_open_snapshot = ordinal % 4 == 0;
            next.terminal_by_detail_absence = matches!(ordinal, 0 | 1 | 19);
            next.reservation = match ordinal {
                1 => {
                    next.ownership = OwnershipState::Ambiguous;
                    Some(PmReservationKnowledge::Unknown)
                }
                5 => Some(PmReservationKnowledge::Known(
                    super::PmExactReservation::policy_approved(U256::MAX, U256::ZERO).unwrap(),
                )),
                6 => Some(PmReservationKnowledge::Known(
                    super::PmExactReservation::policy_approved(U256::ONE, U256::ZERO).unwrap(),
                )),
                17 => {
                    next.ownership = OwnershipState::Ambiguous;
                    Some(PmReservationKnowledge::Known(
                        super::PmExactReservation::policy_approved(U256::ONE, U256::ZERO).unwrap(),
                    ))
                }
                ordinal if ordinal == unknown_ordinal => Some(PmReservationKnowledge::Unknown),
                _ => None,
            };
            state.insert(next).unwrap();
        }
        state
    }

    let unknown_after_overflow = precedence_state(11, &[5, 6, 11]);
    assert_eq!(
        unknown_after_overflow
            .decision_summary()
            .reservation_totals(),
        Err(PmReservationTotalsError::Overflow)
    );
    assert_eq!(
        unknown_after_overflow
            .decision_summary()
            .first_unknown_reservation(),
        Some(client_only(11))
    );
    assert_decision_summary_matches_canonical_oracle(&unknown_after_overflow);

    let overflow_after_unknown = precedence_state(2, &[2, 5, 6]);
    assert_eq!(
        overflow_after_unknown
            .decision_summary()
            .reservation_totals(),
        Err(PmReservationTotalsError::Unknown(client_only(2)))
    );
    assert_eq!(
        overflow_after_unknown
            .decision_summary()
            .first_unknown_reservation(),
        Some(client_only(2))
    );
    assert_decision_summary_matches_canonical_oracle(&overflow_after_unknown);
}
