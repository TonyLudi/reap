use super::*;
use reap_pm_core::{
    ConnectionEpoch, EventClock, EventOrdering, EvmAddress, IngressSequence, MAX_REQUIRED_SPENDERS,
    PmAccountScope, PmAssetId, PmChainId, PmClientOrderId, PmClientOrderKey, PmConditionId,
    PmConnectionId, PmEnvironmentId, PmFunderId, PmMarketHandle, PmMarketId, PmMarketLifecycle,
    PmMarketMetadata, PmOrderIdentity, PmOrderProgress, PmOrderSide, PmOrderStatus, PmOutcomeLabel,
    PmOutcomeMetadata, PmPositionAvailability, PmPositionEvent, PmPrice, PmProductSource,
    PmQuantity, PmReconciliationRequestBoundary, PmSignerId, PmSnapshotEvidence, PmSourceHandle,
    PmSpenderDomain, PmSpenderRequirement, PmTick, PmTokenHandle, PmTokenId, PmVenueOrderId,
    SnapshotRevision, U256,
};

use crate::{
    PmCardinalityRiskLimits, PmExactReservation, PmExposureRiskLimits, PmFreshnessRiskLimits,
    PmOrderRiskLimits, PmOwnedIntentId, PmOwnedQuoteIntent, PmOwnedQuoteSlotKey,
    PmOwnedSubmitApply, PmOwnedSubmitResult, PmPrivateReadinessReason,
};

fn address(byte: u8) -> EvmAddress {
    EvmAddress::from_bytes([byte; 20]).unwrap()
}

fn account() -> PmAccountHandle {
    PmAccountHandle::from_ordinal(7)
}

fn scope() -> PmAccountScope {
    PmAccountScope::new(
        PmEnvironmentId::new("post-apply-refresh-test").unwrap(),
        PmChainId::new(137).unwrap(),
        PmSignerId::new(address(1)),
        PmFunderId::new(address(2)),
        account(),
    )
}

fn instrument() -> PmInstrumentHandle {
    PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(20),
        PmTokenHandle::from_ordinal(30),
    )
}

fn source() -> PmProductSource {
    PmProductSource::polymarket_account(PmSourceHandle::from_ordinal(4), account())
}

fn state() -> PmPrivateState {
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
    let config = PmPrivateStateConfig::new(source(), scope(), instrument(), metadata).unwrap();
    let limits = PmRiskLimits::new(
        PmOrderRiskLimits::new(
            PmQuantity::parse_decimal("100").unwrap(),
            U256::from_u64(100_000_000),
        )
        .unwrap(),
        PmExposureRiskLimits::new(
            U256::from_u64(1_000_000_000),
            U256::from_u64(1_000_000_000),
            U256::from_u64(1_000_000_000),
            U256::from_u64(1_000_000_000),
        )
        .unwrap(),
        PmCardinalityRiskLimits::new(8_192, 8_192, 8_192).unwrap(),
        PmFreshnessRiskLimits::new(1_000, 1_000, 1_000, 1_000, 1_000, 1_000).unwrap(),
    );
    PmPrivateState::new(config, limits).unwrap()
}

fn reservation() -> PmExactReservation {
    PmExactReservation::policy_approved(U256::from_u64(400_000), U256::ZERO).unwrap()
}

fn quote_intent() -> PmOwnedQuoteIntent {
    let client = PmClientOrderKey::new(account(), PmClientOrderId::from_bytes([9; 16]).unwrap());
    PmOwnedQuoteIntent::new(
        PmOwnedIntentId::new(1).unwrap(),
        PmOwnedQuoteSlotKey::new(scope(), instrument(), PmOrderSide::Buy),
        client,
        PmPrice::parse_decimal("0.40").unwrap(),
        PmQuantity::parse_decimal("1").unwrap(),
        reservation(),
    )
    .unwrap()
}

fn quote_request() -> PmPrivateQuoteRequest {
    PmPrivateQuoteRequest::new(
        10,
        PmOrderSide::Buy,
        PmPrice::parse_decimal("0.40").unwrap(),
        PmQuantity::parse_decimal("1").unwrap(),
        reservation(),
    )
}

fn unavailable_account_reconciliation_at(
    config: &PmPrivateStateConfig,
    revision: u64,
    request_sequence: u64,
    completion_sequence: u64,
) -> (
    EventEnvelope<PmCompleteAccountSnapshot>,
    EventEnvelope<PmCompleteFillQuery>,
) {
    let epoch = ConnectionEpoch::new(1);
    let revision = SnapshotRevision::new(revision);
    let snapshot = PmSnapshotEvidence::new(revision).unwrap();
    let boundary = PmReconciliationRequestBoundary::new(
        IngressSequence::new(request_sequence),
        IngressSequence::new(completion_sequence),
    )
    .unwrap();
    let position = PmPositionEvent::new(
        source(),
        account(),
        instrument(),
        U256::from_u64(5),
        PmPositionAvailability::Unavailable,
        snapshot,
    )
    .unwrap();
    let account_snapshot = PmCompleteAccountSnapshot::new(
        source(),
        scope(),
        snapshot,
        boundary,
        vec![config.collateral_asset(), config.outcome_asset()].into_boxed_slice(),
        config.required_spenders().to_vec().into_boxed_slice(),
        vec![instrument()].into_boxed_slice(),
        Vec::new().into_boxed_slice(),
        Vec::new().into_boxed_slice(),
        vec![position].into_boxed_slice(),
    )
    .unwrap();
    let fills = PmCompleteFillQuery::new(
        source(),
        scope(),
        snapshot,
        boundary,
        None,
        reap_pm_core::PmFillQueryCursor::new(scope(), [1; 32]),
        Vec::new().into_boxed_slice(),
    )
    .unwrap();
    let clock = EventClock::new(None, 1_100, 100, 101).unwrap();
    let ordering = EventOrdering::new(
        epoch,
        Some(revision),
        None,
        None,
        IngressSequence::new(completion_sequence),
    )
    .unwrap();
    let connection = PmConnectionId::new("post-apply-reconciliation").unwrap();
    (
        EventEnvelope::new(
            source().venue(),
            source(),
            connection,
            clock,
            ordering,
            account_snapshot,
        )
        .unwrap(),
        EventEnvelope::new(
            source().venue(),
            source(),
            connection,
            clock,
            ordering,
            fills,
        )
        .unwrap(),
    )
}

fn unavailable_account_reconciliation(
    config: &PmPrivateStateConfig,
) -> (
    EventEnvelope<PmCompleteAccountSnapshot>,
    EventEnvelope<PmCompleteFillQuery>,
) {
    unavailable_account_reconciliation_at(config, 1, 10, 11)
}

fn admitted_private_reconnect(state: &mut PmPrivateState) -> PmRefreshTicket {
    state.observe_reconnect(ConnectionEpoch::new(1), 1).unwrap();
    let ticket = state
        .pending_refreshes()
        .find(|ticket| ticket.key().reason() == PmRefreshReason::PrivateReconnect)
        .expect("reconnect creates one canonical refresh ticket");
    assert_eq!(
        state.mark_refresh_admitted(ticket).unwrap(),
        PmRefreshAdmission::Admitted(ticket)
    );
    ticket
}

#[test]
fn partial_duplicate_and_stale_reconciliation_retain_private_reconnect_ticket() {
    let mut state = state();
    let reconnect = admitted_private_reconnect(&mut state);
    let config = state.config.clone();
    let (account, fills) = unavailable_account_reconciliation_at(&config, 2, 20, 21);

    assert!(matches!(
        state.apply_account_snapshot(account.clone()).unwrap(),
        PmAccountSnapshotApply::Applied { .. }
    ));
    assert!(matches!(
        state.apply_reconciliation(account, fills).unwrap(),
        PmReconciliationApply::NotApplied(PmAccountSnapshotApply::Duplicate { .. })
    ));

    let (stale_account, stale_fills) = unavailable_account_reconciliation_at(&config, 1, 22, 23);
    assert!(matches!(
        state
            .apply_reconciliation(stale_account, stale_fills)
            .unwrap(),
        PmReconciliationApply::NotApplied(PmAccountSnapshotApply::IgnoredStale { .. })
    ));
    assert_eq!(
        state.complete_refresh(reconnect).unwrap(),
        PmRefreshCompletion::Cleared(reconnect),
        "non-applied complete cuts must leave the admitted reconnect ticket intact"
    );
}

#[test]
fn rejected_mismatched_reconciliation_retains_private_reconnect_ticket() {
    let mut state = state();
    let reconnect = admitted_private_reconnect(&mut state);
    let config = state.config.clone();
    let (account, fills) = unavailable_account_reconciliation(&config);
    let venue = fills.venue();
    let source = fills.source();
    let clock = fills.clock();
    let ordering = fills.ordering();
    let mismatched_fills = EventEnvelope::new(
        venue,
        source,
        PmConnectionId::new("mismatched-reconciliation").unwrap(),
        clock,
        ordering,
        fills.into_payload(),
    )
    .unwrap();

    assert!(matches!(
        state.apply_reconciliation(account, mismatched_fills),
        Err(PmPrivateStateError::ReconciliationPairMismatch)
    ));
    assert_eq!(
        state.complete_refresh(reconnect).unwrap(),
        PmRefreshCompletion::Cleared(reconnect),
        "rejected pairs must leave the admitted reconnect ticket intact"
    );
}

#[test]
fn saturated_refresh_retains_ambiguous_submit_and_latches_coarse_reconciliation() {
    let mut state = state();
    let intent = quote_intent();
    let client = intent.client_order();
    state.admit_owned_quote(intent).unwrap();
    state.phase6_fill_refresh_capacity();

    assert_eq!(
        state
            .apply_owned_submit_result(client, PmOwnedSubmitResult::Ambiguous)
            .unwrap(),
        PmOwnedSubmitApply::MarkedAmbiguous
    );
    let retained = state.owned_order(client).unwrap();
    assert!(retained.reconciliation_required());
    assert_eq!(retained.submit(), crate::PmOwnedSubmitState::Ambiguous);
    assert!(state.full_reconcile_required());
    assert_eq!(state.refresh_counters().saturations(), 1);
    assert_eq!(
        state.quote_readiness(quote_request()),
        PmPrivateReadiness::Blocked(PmPrivateReadinessReason::FullReconciliationRequired)
    );
}

#[test]
fn converged_reconciliation_preserves_post_cut_account_requirement_saturation() {
    let mut state = state();
    state.observe_reconnect(ConnectionEpoch::new(1), 1).unwrap();
    state.phase6_fill_refresh_capacity();
    assert!(matches!(
        state
            .require_refresh(PmRefreshReason::AmbiguousOrder)
            .unwrap(),
        PmRefreshRequired::Saturated { .. }
    ));
    assert!(state.full_reconcile_required());
    let prior_saturations = state.refresh_counters().saturations();
    let config = state.config.clone();
    let (account, fills) = unavailable_account_reconciliation(&config);

    assert!(matches!(
        state.apply_reconciliation(account, fills).unwrap(),
        PmReconciliationApply::Applied {
            uncovered_fills: 0,
            ..
        }
    ));

    assert!(state.full_reconcile_required());
    assert_eq!(
        state.refresh_counters().saturations(),
        prior_saturations + 2
    );
    assert!(
        !state
            .pending_refresh_keys()
            .any(|key| key.reason() == PmRefreshReason::PositionUnavailable)
    );
    assert!(
        !state
            .pending_refresh_keys()
            .any(|key| key.reason() == PmRefreshReason::AllowanceUnavailable)
    );
}

#[test]
fn saturated_refresh_retains_applied_ambiguous_order_detail_fact() {
    let mut state = state();
    let epoch = ConnectionEpoch::new(1);
    state.observe_reconnect(epoch, 1).unwrap();
    let intent = quote_intent();
    let client = intent.client_order();
    state.admit_owned_quote(intent).unwrap();

    let venue = PmVenueOrderKey::new(account(), PmVenueOrderId::new("saturated-detail").unwrap());
    let venue_only_event = PmOrderEvent::new(
        source(),
        instrument(),
        PmOrderIdentity::new(None, Some(venue)).unwrap(),
        PmOrderSide::Buy,
        PmPrice::parse_decimal("0.40").unwrap(),
        PmOrderProgress::new(
            PmQuantity::parse_decimal("1").unwrap(),
            U256::ZERO,
            PmOrderStatus::Open,
        )
        .unwrap(),
    )
    .unwrap();
    let venue_only_envelope = EventEnvelope::new(
        source().venue(),
        source(),
        PmConnectionId::new("post-apply-refresh").unwrap(),
        EventClock::new(None, 1_050, 50, 51).unwrap(),
        EventOrdering::new(epoch, None, None, None, IngressSequence::new(5)).unwrap(),
        venue_only_event,
    )
    .unwrap();
    state
        .observe_order(
            venue_only_envelope,
            PmRemoteOrderKnowledge::Unmanaged(PmReservationKnowledge::Known(reservation())),
        )
        .unwrap();
    state.phase6_fill_refresh_capacity();

    let event = PmOrderEvent::new(
        source(),
        instrument(),
        PmOrderIdentity::new(Some(client), Some(venue)).unwrap(),
        PmOrderSide::Buy,
        PmPrice::parse_decimal("0.40").unwrap(),
        PmOrderProgress::new(
            PmQuantity::parse_decimal("1").unwrap(),
            U256::ZERO,
            PmOrderStatus::Open,
        )
        .unwrap(),
    )
    .unwrap();
    let boundary =
        PmReconciliationRequestBoundary::new(IngressSequence::new(10), IngressSequence::new(11))
            .unwrap();
    let detail = PmExactOrderDetail::new(
        source(),
        scope(),
        PmSnapshotEvidence::new(SnapshotRevision::new(1)).unwrap(),
        boundary,
        venue,
        Some(event),
    )
    .unwrap();
    let envelope = EventEnvelope::new(
        source().venue(),
        source(),
        PmConnectionId::new("post-apply-refresh").unwrap(),
        EventClock::new(None, 1_100, 100, 101).unwrap(),
        EventOrdering::new(
            epoch,
            Some(SnapshotRevision::new(1)),
            None,
            None,
            IngressSequence::new(11),
        )
        .unwrap(),
        detail,
    )
    .unwrap();

    state
        .apply_order_detail(envelope, PmReservationKnowledge::Known(reservation()))
        .unwrap();
    let retained = state
        .orders()
        .find(|order| order.identity().venue_order_key() == Some(venue))
        .unwrap();
    assert_eq!(retained.ownership(), PmOrderOwnership::ProvenOwned);
    assert_eq!(retained.status(), Some(PmOrderStatus::Open));
    assert!(state.full_reconcile_required());
    assert_eq!(state.refresh_counters().saturations(), 1);
    assert!(
        !state
            .pending_refresh_keys()
            .any(|key| key.reason() == PmRefreshReason::AmbiguousOrder)
    );
    assert_eq!(
        state.quote_readiness(quote_request()),
        PmPrivateReadiness::Blocked(PmPrivateReadinessReason::FullReconciliationRequired)
    );
}
