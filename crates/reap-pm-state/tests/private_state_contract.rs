use reap_pm_core::{
    ConnectionEpoch, EventClock, EventEnvelope, EventOrdering, EvmAddress, IngressSequence,
    PmAccountHandle, PmAccountScope, PmAllowanceEvent, PmAllowanceValue, PmAssetId, PmBalanceEvent,
    PmChainId, PmClientOrderId, PmClientOrderKey, PmCompleteAccountSnapshot, PmCompleteFillQuery,
    PmCompleteOpenOrdersSnapshot, PmConditionId, PmConnectionId, PmEnvironmentId,
    PmErc1155OperatorApproval, PmExactOrderDetail, PmFillEvent, PmFillExecution, PmFillFee,
    PmFillId, PmFillKey, PmFillQueryCursor, PmFillRole, PmFillSettlementStatus, PmFunderId,
    PmGoalFTradingDomain, PmInstrumentHandle, PmMarketHandle, PmMarketId, PmMarketLifecycle,
    PmMarketMetadata, PmOrderEvent, PmOrderIdentity, PmOrderProgress, PmOrderSide, PmOrderStatus,
    PmOutcomeLabel, PmOutcomeMetadata, PmPositionAvailability, PmPositionEvent, PmPrice,
    PmProductSource, PmQuantity, PmReconciliationRequestBoundary, PmSign, PmSignedUnits,
    PmSignerId, PmSnapshotEvidence, PmSourceBound, PmSourceHandle, PmSpenderDomain, PmSpenderId,
    PmSpenderRequirement, PmTick, PmTokenHandle, PmTokenId, PmVenueOrderId, PmVenueOrderKey,
    SnapshotRevision, U256,
};
use reap_pm_state::{
    PmAccountSnapshotApply, PmCardinalityRiskLimits, PmExactReservation, PmExposureRiskLimits,
    PmFillApply, PmFillFeeState, PmFreshnessRiskLimits, PmOpenOrderReservation, PmOpenOrdersApply,
    PmOrderApply, PmOrderOwnership, PmOrderRiskLimits, PmOwnedCancelOutcome,
    PmOwnedCancelRequestApply, PmOwnedCancelState, PmOwnedFillApply, PmOwnedIntentId,
    PmOwnedObservationOccurrence, PmOwnedObservationSource, PmOwnedOrderLifecycleError,
    PmOwnedOrderRegistration, PmOwnedProgressApply, PmOwnedQuoteAdmission, PmOwnedQuoteIntent,
    PmOwnedQuoteSlotKey, PmOwnedRecoveryFill, PmOwnedReductionSequence, PmOwnedRemoteOrderApply,
    PmOwnedSubmitApply, PmOwnedSubmitResult, PmPositionKnowledge, PmPrivateConvergence,
    PmPrivateDependency, PmPrivateExternalIngressFailure, PmPrivateExternalIngressFault,
    PmPrivateExternalIngressLane, PmPrivateHaltReason, PmPrivateOccurrence,
    PmPrivateQuoteEvaluation, PmPrivateQuoteRequest, PmPrivateReadiness, PmPrivateReadinessReason,
    PmPrivateState, PmPrivateStateConfig, PmPrivateStateError, PmReconciliationFillDisposition,
    PmReconciliationReductions, PmRefreshAdmission, PmRefreshReason, PmRefreshRequired,
    PmRemoteOrderKnowledge, PmReservationKnowledge, PmRiskDecision, PmRiskDependency, PmRiskLimits,
    PmRiskReason, PmUnresolvedFillApply, PmUnresolvedFillObservation, PmUnresolvedFillReason,
};

const CONDITION: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MARKET: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const PUSD: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
const CTF: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
const EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";

fn address(byte: u8) -> EvmAddress {
    EvmAddress::from_bytes([byte; 20]).unwrap()
}

fn account() -> PmAccountHandle {
    PmAccountHandle::from_ordinal(7)
}

fn scope() -> PmAccountScope {
    PmAccountScope::new(
        PmEnvironmentId::new("fixture").unwrap(),
        PmChainId::new(137).unwrap(),
        PmSignerId::new(address(1)),
        PmFunderId::new(address(2)),
        account(),
    )
}

fn source() -> PmProductSource {
    PmProductSource::polymarket_account(PmSourceHandle::from_ordinal(4), account())
}

fn instrument() -> PmInstrumentHandle {
    PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(20),
        PmTokenHandle::from_ordinal(30),
    )
}

fn metadata() -> PmMarketMetadata {
    let chain = PmChainId::new(137).unwrap();
    let exchange = EvmAddress::parse(EXCHANGE).unwrap();
    let token = PmTokenId::new(U256::from_u64(123)).unwrap();
    let collateral = PmAssetId::collateral(EvmAddress::parse(PUSD).unwrap());
    let outcome = PmAssetId::outcome(EvmAddress::parse(CTF).unwrap(), token);
    let mut spenders = [None; 8];
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
    PmMarketMetadata::new(
        PmConditionId::parse(CONDITION).unwrap(),
        PmMarketId::parse(MARKET).unwrap(),
        PmOutcomeMetadata::new(token, PmOutcomeLabel::new("YES").unwrap()),
        PmMarketLifecycle::new(true, false, false, true, true),
        PmTick::parse_decimal("0.0001").unwrap(),
        PmQuantity::parse_decimal("0.01").unwrap(),
        false,
        chain,
        exchange,
        spenders,
        2,
    )
    .unwrap()
}

fn domain() -> PmGoalFTradingDomain {
    PmGoalFTradingDomain::from_metadata(metadata()).unwrap()
}

fn spenders() -> [PmSpenderId; 2] {
    domain()
        .required_spenders()
        .map(|requirement| PmSpenderId::new(account(), requirement))
}

fn limits() -> PmRiskLimits {
    PmRiskLimits::new(
        PmOrderRiskLimits::new(
            PmQuantity::parse_decimal("10").unwrap(),
            U256::from_u64(10_000_000),
        )
        .unwrap(),
        PmExposureRiskLimits::new(
            U256::from_u64(100_000_000),
            U256::from_u64(100_000_000),
            U256::from_u64(100_000_000),
            U256::from_u64(100_000_000),
        )
        .unwrap(),
        PmCardinalityRiskLimits::new(1_024, 1_024, 10_000).unwrap(),
        PmFreshnessRiskLimits::new(1_000, 1_000, 1_000, 1_000, 1_000, 1_000).unwrap(),
    )
}

fn config() -> PmPrivateStateConfig {
    PmPrivateStateConfig::new(source(), scope(), instrument(), metadata()).unwrap()
}

fn unstarted_state() -> PmPrivateState {
    PmPrivateState::new(config(), limits()).unwrap()
}

fn new_state() -> PmPrivateState {
    let mut state = unstarted_state();
    state.observe_reconnect(ConnectionEpoch::new(1), 1).unwrap();
    state
}

fn snapshot(revision: u64) -> PmSnapshotEvidence {
    PmSnapshotEvidence::new(SnapshotRevision::new(revision)).unwrap()
}

fn boundary(request: u64, completion: u64) -> PmReconciliationRequestBoundary {
    PmReconciliationRequestBoundary::new(
        IngressSequence::new(request),
        IngressSequence::new(completion),
    )
    .unwrap()
}

fn envelope<P: PmSourceBound>(
    payload: P,
    epoch: u64,
    ingress: u64,
    revision: Option<u64>,
    monotonic_ns: u64,
) -> EventEnvelope<P> {
    envelope_on(
        payload,
        "fixture-private",
        epoch,
        ingress,
        revision,
        monotonic_ns,
    )
}

fn envelope_on<P: PmSourceBound>(
    payload: P,
    connection: &str,
    epoch: u64,
    ingress: u64,
    revision: Option<u64>,
    monotonic_ns: u64,
) -> EventEnvelope<P> {
    let event_source = payload.source();
    EventEnvelope::new(
        event_source.venue(),
        event_source,
        PmConnectionId::new(connection).unwrap(),
        EventClock::new(None, 1_000 + monotonic_ns, monotonic_ns, monotonic_ns).unwrap(),
        EventOrdering::new(
            ConnectionEpoch::new(epoch),
            revision.map(SnapshotRevision::new),
            None,
            None,
            IngressSequence::new(ingress),
        )
        .unwrap(),
        payload,
    )
    .unwrap()
}

#[derive(Clone, Copy)]
struct AccountFacts {
    collateral: Option<u64>,
    outcome: Option<u64>,
    position: Option<(u64, PmPositionAvailability)>,
    collateral_allowance: Option<u64>,
    outcome_approval: Option<bool>,
    unknown_extra: bool,
}

impl AccountFacts {
    const fn ready() -> Self {
        Self {
            collateral: Some(10_000_000),
            outcome: Some(5_000_000),
            position: Some((5_000_000, PmPositionAvailability::Tradable)),
            collateral_allowance: Some(10_000_000),
            outcome_approval: Some(true),
            unknown_extra: false,
        }
    }
}

fn account_snapshot(
    revision: u64,
    cut: PmReconciliationRequestBoundary,
    facts: AccountFacts,
) -> PmCompleteAccountSnapshot {
    let domain = domain();
    let [collateral_spender, outcome_spender] = spenders();
    let mut balances = Vec::new();
    if let Some(value) = facts.collateral {
        balances.push(
            PmBalanceEvent::new(
                source(),
                account(),
                domain.collateral(),
                U256::from_u64(value),
                snapshot(revision),
            )
            .unwrap(),
        );
    }
    if let Some(value) = facts.outcome {
        balances.push(
            PmBalanceEvent::new(
                source(),
                account(),
                domain.outcome(),
                U256::from_u64(value),
                snapshot(revision),
            )
            .unwrap(),
        );
    }
    if facts.unknown_extra {
        balances.push(
            PmBalanceEvent::new(
                source(),
                account(),
                PmAssetId::collateral(address(99)),
                U256::MAX,
                snapshot(revision),
            )
            .unwrap(),
        );
    }
    let mut allowances = Vec::new();
    if let Some(value) = facts.collateral_allowance {
        allowances.push(
            PmAllowanceEvent::new(
                source(),
                collateral_spender,
                PmAllowanceValue::Erc20(U256::from_u64(value)),
                snapshot(revision),
            )
            .unwrap(),
        );
    }
    if let Some(approved) = facts.outcome_approval {
        allowances.push(
            PmAllowanceEvent::new(
                source(),
                outcome_spender,
                PmAllowanceValue::Erc1155Operator(PmErc1155OperatorApproval::from_bool(approved)),
                snapshot(revision),
            )
            .unwrap(),
        );
    }
    let positions = facts
        .position
        .map_or_else(Vec::new, |(quantity, availability)| {
            vec![
                PmPositionEvent::new(
                    source(),
                    account(),
                    instrument(),
                    U256::from_u64(quantity),
                    availability,
                    snapshot(revision),
                )
                .unwrap(),
            ]
        });
    PmCompleteAccountSnapshot::new(
        source(),
        scope(),
        snapshot(revision),
        cut,
        vec![domain.collateral(), domain.outcome()].into_boxed_slice(),
        vec![collateral_spender, outcome_spender].into_boxed_slice(),
        vec![instrument()].into_boxed_slice(),
        balances.into_boxed_slice(),
        allowances.into_boxed_slice(),
        positions.into_boxed_slice(),
    )
    .unwrap()
}

fn open_orders(
    revision: u64,
    cut: PmReconciliationRequestBoundary,
    orders: Vec<PmOrderEvent>,
) -> PmCompleteOpenOrdersSnapshot {
    PmCompleteOpenOrdersSnapshot::new(
        source(),
        scope(),
        snapshot(revision),
        cut,
        orders.into_boxed_slice(),
    )
    .unwrap()
}

fn fill_query(
    revision: u64,
    cut: PmReconciliationRequestBoundary,
    requested_after: Option<PmFillQueryCursor>,
    resulting: u8,
    fills: Vec<PmFillEvent>,
) -> PmCompleteFillQuery {
    PmCompleteFillQuery::new(
        source(),
        scope(),
        snapshot(revision),
        cut,
        requested_after,
        PmFillQueryCursor::new(scope(), [resulting; 32]),
        fills.into_boxed_slice(),
    )
    .unwrap()
}

fn venue_order(id: &str) -> PmVenueOrderKey {
    PmVenueOrderKey::new(account(), PmVenueOrderId::new(id).unwrap())
}

fn client_order(id: u8) -> PmClientOrderKey {
    PmClientOrderKey::new(account(), PmClientOrderId::from_bytes([id; 16]).unwrap())
}

fn owned_quote_intent(intent: u64, client: u8, side: PmOrderSide) -> PmOwnedQuoteIntent {
    let price = PmPrice::parse_decimal("0.40").unwrap();
    let quantity = PmQuantity::parse_decimal("1").unwrap();
    let reservation = match side {
        PmOrderSide::Buy => {
            PmExactReservation::policy_approved(U256::from_u64(400_000), U256::ZERO)
        }
        PmOrderSide::Sell => {
            PmExactReservation::policy_approved(U256::ZERO, quantity.protocol_units())
        }
    }
    .unwrap();
    PmOwnedQuoteIntent::new(
        PmOwnedIntentId::new(intent).unwrap(),
        PmOwnedQuoteSlotKey::new(scope(), instrument(), side),
        client_order(client),
        price,
        quantity,
        reservation,
    )
    .unwrap()
}

fn order(
    client: u8,
    venue: &str,
    side: PmOrderSide,
    status: PmOrderStatus,
    cumulative: u64,
) -> PmOrderEvent {
    let identity = PmOrderIdentity::new(
        Some(PmClientOrderKey::new(
            account(),
            PmClientOrderId::from_bytes([client; 16]).unwrap(),
        )),
        Some(venue_order(venue)),
    )
    .unwrap();
    PmOrderEvent::new(
        source(),
        instrument(),
        identity,
        side,
        PmPrice::parse_decimal("0.40").unwrap(),
        PmOrderProgress::new(
            PmQuantity::parse_decimal("1").unwrap(),
            U256::from_u64(cumulative),
            status,
        )
        .unwrap(),
    )
    .unwrap()
}

fn venue_only_order(venue: &str) -> PmOrderEvent {
    PmOrderEvent::new(
        source(),
        instrument(),
        PmOrderIdentity::new(None, Some(venue_order(venue))).unwrap(),
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

fn client_only_order(client: u8) -> PmOrderEvent {
    PmOrderEvent::new(
        source(),
        instrument(),
        PmOrderIdentity::new(
            Some(PmClientOrderKey::new(
                account(),
                PmClientOrderId::from_bytes([client; 16]).unwrap(),
            )),
            None,
        )
        .unwrap(),
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

fn fill(venue: &str, id: &str, side: PmOrderSide, quantity: &str, fee: PmFillFee) -> PmFillEvent {
    fill_with_settlement(
        venue,
        id,
        side,
        quantity,
        PmFillSettlementStatus::Matched,
        fee,
    )
}

fn fill_with_settlement(
    venue: &str,
    id: &str,
    side: PmOrderSide,
    quantity: &str,
    settlement: PmFillSettlementStatus,
    fee: PmFillFee,
) -> PmFillEvent {
    let venue_order = venue_order(venue);
    PmFillEvent::new(
        source(),
        instrument(),
        PmFillKey::new(venue_order, PmFillId::new(id).unwrap()),
        PmOrderIdentity::new(None, Some(venue_order)).unwrap(),
        PmFillExecution::new(
            side,
            PmFillRole::Maker,
            settlement,
            PmPrice::parse_decimal("0.40").unwrap(),
            PmQuantity::parse_decimal(quantity).unwrap(),
            fee,
        ),
    )
    .unwrap()
}

fn unresolved_fill(
    id: &str,
    exact_order: Option<&str>,
    candidate_order: Option<&str>,
    reason: PmUnresolvedFillReason,
    settlement: PmFillSettlementStatus,
) -> PmUnresolvedFillObservation {
    PmUnresolvedFillObservation::new(
        source(),
        account(),
        instrument(),
        PmFillId::new(id).unwrap(),
        exact_order.map(venue_order),
        candidate_order.map(|order| PmVenueOrderId::new(order).unwrap()),
        reason,
        settlement,
    )
    .unwrap()
}

fn quote(now: u64) -> PmPrivateQuoteRequest {
    PmPrivateQuoteRequest::new(
        now,
        PmOrderSide::Buy,
        PmPrice::parse_decimal("0.40").unwrap(),
        PmQuantity::parse_decimal("1").unwrap(),
        PmExactReservation::policy_approved(U256::from_u64(400_000), U256::ZERO).unwrap(),
    )
}

fn make_ready(state: &mut PmPrivateState) {
    let orders_cut = boundary(10, 11);
    assert!(matches!(
        state
            .apply_open_orders_snapshot(
                envelope(open_orders(1, orders_cut, Vec::new()), 1, 11, Some(1), 100),
                &[],
            )
            .unwrap(),
        PmOpenOrdersApply::Applied { .. }
    ));
    let cut = boundary(20, 21);
    let account = envelope(
        account_snapshot(2, cut, AccountFacts::ready()),
        1,
        21,
        Some(2),
        110,
    );
    let fills = envelope(fill_query(2, cut, None, 1, Vec::new()), 1, 21, Some(2), 110);
    state.apply_reconciliation(account, fills).unwrap();
    assert!(matches!(
        state.quote_readiness(quote(120)),
        PmPrivateReadiness::Ready(_)
    ));
}

#[test]
fn combined_quote_evaluation_matches_legacy_ready_and_risk_effects() {
    let reference = PmRiskDependency::available(120);
    let book = PmRiskDependency::available(120);
    let mut legacy = new_state();
    make_ready(&mut legacy);
    let legacy_ready = match legacy.quote_readiness(quote(120)) {
        PmPrivateReadiness::Ready(ready) => ready,
        PmPrivateReadiness::Blocked(reason) => panic!("ready fixture blocked: {reason:?}"),
    };
    let legacy_risk = legacy
        .evaluate_risk_candidate(quote(120), reference, book)
        .unwrap();
    let legacy_counters = legacy.risk_counters();

    let mut combined = new_state();
    make_ready(&mut combined);
    assert_eq!(
        combined
            .evaluate_quote_candidate(quote(120), reference, book)
            .unwrap(),
        PmPrivateQuoteEvaluation::Evaluated {
            ready: legacy_ready,
            risk: legacy_risk,
        }
    );
    assert_eq!(combined.risk_counters(), legacy_counters);

    let mut legacy_rejection = new_state();
    make_ready(&mut legacy_rejection);
    let rejected_ready = match legacy_rejection.quote_readiness(quote(120)) {
        PmPrivateReadiness::Ready(ready) => ready,
        PmPrivateReadiness::Blocked(reason) => panic!("ready fixture blocked: {reason:?}"),
    };
    let rejected_risk = legacy_rejection
        .evaluate_risk_candidate(quote(120), PmRiskDependency::unavailable(), book)
        .unwrap();
    let rejected_counters = legacy_rejection.risk_counters();

    let mut combined_rejection = new_state();
    make_ready(&mut combined_rejection);
    assert_eq!(
        combined_rejection
            .evaluate_quote_candidate(quote(120), PmRiskDependency::unavailable(), book)
            .unwrap(),
        PmPrivateQuoteEvaluation::Evaluated {
            ready: rejected_ready,
            risk: rejected_risk,
        }
    );
    assert_eq!(combined_rejection.risk_counters(), rejected_counters);
}

#[test]
fn combined_quote_evaluation_preserves_readiness_precedence_without_risk_effects() {
    let mut state = new_state();
    make_ready(&mut state);
    let unknown = order(
        1,
        "unknown-before-ambiguity",
        PmOrderSide::Buy,
        PmOrderStatus::Open,
        0,
    );
    state
        .observe_order(
            envelope(unknown, 1, 30, None, 120),
            PmRemoteOrderKnowledge::Unmanaged(PmReservationKnowledge::Unknown),
        )
        .unwrap();
    let ambiguous = order(
        2,
        "later-global-ambiguity",
        PmOrderSide::Buy,
        PmOrderStatus::Open,
        0,
    );
    state
        .observe_order(
            envelope(ambiguous, 1, 31, None, 121),
            PmRemoteOrderKnowledge::Ambiguous,
        )
        .unwrap();
    let before = state.risk_counters();

    assert_eq!(
        state
            .evaluate_quote_candidate(
                quote(122),
                PmRiskDependency::available(121),
                PmRiskDependency::available(121),
            )
            .unwrap(),
        PmPrivateQuoteEvaluation::Blocked(PmPrivateReadinessReason::UnmanagedOwnershipAmbiguous)
    );
    assert_eq!(state.risk_counters(), before);

    let mut unknown_only = new_state();
    make_ready(&mut unknown_only);
    unknown_only
        .observe_order(
            envelope(unknown, 1, 30, None, 120),
            PmRemoteOrderKnowledge::Unmanaged(PmReservationKnowledge::Unknown),
        )
        .unwrap();
    let PmPrivateQuoteEvaluation::Blocked(PmPrivateReadinessReason::UnknownReservation(identity)) =
        unknown_only
            .evaluate_quote_candidate(
                quote(121),
                PmRiskDependency::available(120),
                PmRiskDependency::available(120),
            )
            .unwrap()
    else {
        panic!("canonical unknown reservation must block before risk");
    };
    assert_eq!(identity, unknown.order());
    assert!(matches!(
        unknown_only.evaluate_risk_candidate(
            quote(121),
            PmRiskDependency::available(120),
            PmRiskDependency::available(120),
        ),
        Err(PmPrivateStateError::CanonicalExposureUnavailable)
    ));
}

#[test]
fn combined_quote_evaluation_preserves_invalid_and_overflow_error_mapping() {
    let mut invalid = new_state();
    make_ready(&mut invalid);
    let invalid_request = PmPrivateQuoteRequest::new(
        120,
        PmOrderSide::Buy,
        PmPrice::parse_decimal("0.40").unwrap(),
        PmQuantity::parse_decimal("1").unwrap(),
        PmExactReservation::policy_approved(U256::ONE, U256::ZERO).unwrap(),
    );
    assert_eq!(
        invalid
            .evaluate_quote_candidate(
                invalid_request,
                PmRiskDependency::available(120),
                PmRiskDependency::available(120),
            )
            .unwrap(),
        PmPrivateQuoteEvaluation::Blocked(PmPrivateReadinessReason::ArithmeticInvalid)
    );
    assert_eq!(invalid.risk_counters().evaluations(), 0);

    let mut overflow = new_state();
    make_ready(&mut overflow);
    for client in [10_u8, 11_u8] {
        overflow
            .register_owned_order(
                PmOwnedOrderRegistration::new(
                    client_order(client),
                    instrument(),
                    PmOrderSide::Buy,
                    PmPrice::parse_decimal("0.40").unwrap(),
                    PmQuantity::parse_decimal("1").unwrap(),
                    PmExactReservation::policy_approved(U256::MAX, U256::ZERO).unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
    }
    assert_eq!(
        overflow
            .evaluate_quote_candidate(
                quote(121),
                PmRiskDependency::available(120),
                PmRiskDependency::available(120),
            )
            .unwrap(),
        PmPrivateQuoteEvaluation::Blocked(PmPrivateReadinessReason::ArithmeticInvalid)
    );
    assert_eq!(overflow.risk_counters().evaluations(), 0);
    assert!(matches!(
        overflow.evaluate_risk_candidate(
            quote(121),
            PmRiskDependency::available(120),
            PmRiskDependency::available(120),
        ),
        Err(PmPrivateStateError::ArithmeticOverflow)
    ));
}

#[test]
fn atomic_complete_snapshot_proves_absence_but_unknown_extras_grant_no_authority() {
    let mut state = new_state();
    state
        .apply_open_orders_snapshot(
            envelope(
                open_orders(1, boundary(10, 11), Vec::new()),
                1,
                11,
                Some(1),
                100,
            ),
            &[],
        )
        .unwrap();
    let facts = AccountFacts {
        collateral: None,
        outcome: None,
        position: None,
        collateral_allowance: Some(u64::MAX),
        outcome_approval: None,
        unknown_extra: true,
    };
    let cut = boundary(20, 21);
    state
        .apply_reconciliation(
            envelope(account_snapshot(2, cut, facts), 1, 21, Some(2), 110),
            envelope(fill_query(2, cut, None, 1, Vec::new()), 1, 21, Some(2), 110),
        )
        .unwrap();

    let projection = state.account_projection();
    assert_eq!(projection.collateral().value(), Some(U256::ZERO));
    assert_eq!(projection.outcome_balance().value(), Some(U256::ZERO));
    assert_eq!(projection.position(), PmPositionKnowledge::ExplicitAbsent);
    assert_eq!(projection.unknown_balance_rows(), 1);
    let diagnostic = state.diagnostic_balance_rows().next().unwrap();
    assert_eq!(diagnostic.asset(), PmAssetId::collateral(address(99)));
    assert_eq!(diagnostic.balance(), U256::MAX);
    let [_, outcome_spender] = spenders();
    assert_eq!(
        state.quote_readiness(quote(120)),
        PmPrivateReadiness::Blocked(PmPrivateReadinessReason::ExactAllowanceAbsent(
            outcome_spender
        ))
    );
    assert!(
        state
            .pending_refresh_keys()
            .any(|key| key.reason() == PmRefreshReason::AllowanceUnavailable)
    );
}

#[test]
fn fill_principal_is_once_fee_enrichment_is_fee_only_and_maker_legs_are_distinct() {
    let mut state = new_state();
    make_ready(&mut state);
    let unknown = fill(
        "maker-a",
        "trade-1",
        PmOrderSide::Buy,
        "0.25",
        PmFillFee::Unknown,
    );
    assert!(matches!(
        state
            .observe_fill(envelope(unknown, 1, 30, None, 120))
            .unwrap(),
        PmFillApply::PrincipalApplied {
            fee: PmFillFeeState::Unknown,
            ..
        }
    ));
    assert_eq!(
        state.quote_readiness(quote(121)),
        PmPrivateReadiness::Blocked(PmPrivateReadinessReason::FillFeeUnknown(unknown.fill_key()))
    );
    let charge = fill(
        "maker-a",
        "trade-1",
        PmOrderSide::Buy,
        "0.25",
        PmFillFee::Known {
            asset: domain().collateral(),
            delta: PmSignedUnits::from_parts(PmSign::Negative, U256::from_u64(10)).unwrap(),
        },
    );
    assert!(matches!(
        state
            .observe_fill(envelope(charge, 1, 31, None, 121))
            .unwrap(),
        PmFillApply::Enriched { .. }
    ));
    assert_eq!(state.fill_counters().principal_applications(), 1);
    assert_eq!(
        state.provisional_deltas().collateral(),
        PmSignedUnits::from_parts(PmSign::Negative, U256::from_u64(100_010)).unwrap()
    );
    assert_eq!(
        state
            .observe_fill(envelope(charge, 1, 32, None, 122))
            .unwrap(),
        PmFillApply::Duplicate
    );
    let second_leg = fill(
        "maker-b",
        "trade-1",
        PmOrderSide::Buy,
        "0.25",
        PmFillFee::Known {
            asset: domain().outcome(),
            delta: PmSignedUnits::from_parts(PmSign::Positive, U256::from_u64(5)).unwrap(),
        },
    );
    state
        .observe_fill(envelope(second_leg, 1, 33, None, 123))
        .unwrap();
    assert_eq!(state.fill_counters().principal_applications(), 2);
    assert_eq!(state.fills().count(), 2);
    assert_eq!(
        state.provisional_deltas().outcome(),
        PmSignedUnits::from_parts(PmSign::Positive, U256::from_u64(500_005)).unwrap()
    );
    let incomplete = fill(
        "maker-c",
        "trade-2",
        PmOrderSide::Buy,
        "0.25",
        PmFillFee::Incomplete,
    );
    state
        .observe_fill(envelope(incomplete, 1, 34, None, 124))
        .unwrap();
    let rebate = fill(
        "maker-c",
        "trade-2",
        PmOrderSide::Buy,
        "0.25",
        PmFillFee::Known {
            asset: domain().collateral(),
            delta: PmSignedUnits::from_parts(PmSign::Positive, U256::from_u64(7)).unwrap(),
        },
    );
    assert!(matches!(
        state
            .observe_fill(envelope(rebate, 1, 35, None, 125))
            .unwrap(),
        PmFillApply::Enriched { .. }
    ));
    assert_eq!(state.fill_counters().principal_applications(), 3);
    let conflicting_fee = fill(
        "maker-c",
        "trade-2",
        PmOrderSide::Buy,
        "0.25",
        PmFillFee::Known {
            asset: domain().collateral(),
            delta: PmSignedUnits::from_parts(PmSign::Positive, U256::from_u64(8)).unwrap(),
        },
    );
    assert!(matches!(
        state.observe_fill(envelope(conflicting_fee, 1, 36, None, 126)),
        Err(PmPrivateStateError::Fill(_))
    ));
    assert_eq!(state.fill_counters().principal_applications(), 3);
}

#[test]
fn wrong_fee_asset_blocks_until_explicit_cut_and_convergence_is_not_numeric_coincidence() {
    let mut state = new_state();
    make_ready(&mut state);
    let wrong_asset = PmAssetId::collateral(address(88));
    let wrong = fill(
        "maker-a",
        "trade-wrong",
        PmOrderSide::Buy,
        "0.25",
        PmFillFee::Known {
            asset: wrong_asset,
            delta: PmSignedUnits::from_parts(PmSign::Negative, U256::from_u64(1)).unwrap(),
        },
    );
    state
        .observe_fill(envelope(wrong, 1, 30, None, 120))
        .unwrap();
    assert_eq!(
        state.quote_readiness(quote(121)),
        PmPrivateReadiness::Blocked(PmPrivateReadinessReason::FillFeeAssetUnmapped {
            fill: wrong.fill_key(),
            asset: wrong_asset,
        })
    );
    assert!(matches!(
        state.convergence(),
        PmPrivateConvergence::Divergent { uncovered_fills: 1 }
    ));

    let cut = boundary(40, 41);
    let prior = state.fill_watermark();
    state
        .apply_reconciliation(
            envelope(
                account_snapshot(3, cut, AccountFacts::ready()),
                1,
                41,
                Some(3),
                130,
            ),
            envelope(
                fill_query(3, cut, prior, 2, vec![wrong]),
                1,
                41,
                Some(3),
                130,
            ),
        )
        .unwrap();
    assert!(matches!(
        state.convergence(),
        PmPrivateConvergence::Converged { boundary, .. } if boundary == cut
    ));
    assert_eq!(state.provisional_deltas().uncovered_fills(), 0);
}

#[test]
fn a_reconciliation_cut_preserves_later_ws_fills_and_never_claims_convergence() {
    let mut state = new_state();
    make_ready(&mut state);
    let later = fill(
        "maker-later",
        "trade-later",
        PmOrderSide::Buy,
        "0.25",
        PmFillFee::Known {
            asset: domain().collateral(),
            delta: PmSignedUnits::ZERO,
        },
    );
    state
        .observe_fill(envelope(later, 1, 30, None, 120))
        .unwrap();
    let cut = boundary(25, 35);
    state
        .apply_reconciliation(
            envelope(
                account_snapshot(3, cut, AccountFacts::ready()),
                1,
                35,
                Some(3),
                130,
            ),
            envelope(
                fill_query(3, cut, state.fill_watermark(), 2, Vec::new()),
                1,
                35,
                Some(3),
                130,
            ),
        )
        .unwrap();
    assert_eq!(state.provisional_deltas().uncovered_fills(), 1);
    assert_eq!(
        state.quote_readiness(quote(131)),
        PmPrivateReadiness::Blocked(PmPrivateReadinessReason::Divergent { uncovered_fills: 1 })
    );
}

#[test]
fn a_reconciliation_row_cannot_cover_or_overwrite_a_later_ws_fill() {
    let mut state = new_state();
    make_ready(&mut state);
    let fee = PmFillFee::Known {
        asset: domain().collateral(),
        delta: PmSignedUnits::ZERO,
    };
    let later = fill_with_settlement(
        "maker-cut",
        "trade-cut",
        PmOrderSide::Buy,
        "0.25",
        PmFillSettlementStatus::Mined,
        fee,
    );
    state
        .observe_fill(envelope(later, 1, 45, None, 120))
        .unwrap();

    let historical = fill_with_settlement(
        "maker-cut",
        "trade-cut",
        PmOrderSide::Buy,
        "0.25",
        PmFillSettlementStatus::Matched,
        fee,
    );
    let cut = boundary(40, 50);
    state
        .apply_reconciliation(
            envelope(
                account_snapshot(3, cut, AccountFacts::ready()),
                1,
                50,
                Some(3),
                130,
            ),
            envelope(
                fill_query(3, cut, state.fill_watermark(), 2, vec![historical]),
                1,
                50,
                Some(3),
                130,
            ),
        )
        .unwrap();

    let projection = state
        .fills()
        .find(|projection| projection.key() == later.fill_key())
        .unwrap();
    assert_eq!(projection.settlement(), PmFillSettlementStatus::Mined);
    assert_eq!(
        projection
            .last_occurrence()
            .private_occurrence()
            .unwrap()
            .ingress(),
        IngressSequence::new(45)
    );
    assert_eq!(projection.covered_by_reconciliation(), None);
    assert_eq!(state.provisional_deltas().uncovered_fills(), 1);
    assert_eq!(
        state.convergence(),
        PmPrivateConvergence::Divergent { uncovered_fills: 1 }
    );
}

#[test]
fn conflicting_fill_at_the_same_occurrence_is_rejected_without_double_principal() {
    let mut state = new_state();
    let fee = PmFillFee::Known {
        asset: domain().collateral(),
        delta: PmSignedUnits::ZERO,
    };
    let matched = fill_with_settlement(
        "same-occurrence",
        "trade-same-occurrence",
        PmOrderSide::Buy,
        "0.25",
        PmFillSettlementStatus::Matched,
        fee,
    );
    state
        .observe_fill(envelope(matched, 1, 30, None, 120))
        .unwrap();
    let mined = fill_with_settlement(
        "same-occurrence",
        "trade-same-occurrence",
        PmOrderSide::Buy,
        "0.25",
        PmFillSettlementStatus::Mined,
        fee,
    );
    assert!(matches!(
        state.observe_fill(envelope(mined, 1, 30, None, 121)),
        Err(PmPrivateStateError::Fill(_))
    ));
    assert_eq!(state.fill_counters().principal_applications(), 1);
    assert_eq!(
        state.fills().next().unwrap().settlement(),
        PmFillSettlementStatus::Matched
    );
}

#[test]
fn open_snapshot_absence_retains_reservation_until_exact_detail_proves_terminal() {
    let mut state = new_state();
    make_ready(&mut state);
    let unmanaged = order(1, "external-1", PmOrderSide::Buy, PmOrderStatus::Open, 0);
    let reserve = PmExactReservation::policy_approved(U256::from_u64(400_010), U256::ZERO).unwrap();
    state
        .observe_order(
            envelope(unmanaged, 1, 30, None, 120),
            PmRemoteOrderKnowledge::Unmanaged(PmReservationKnowledge::Known(reserve)),
        )
        .unwrap();
    state
        .apply_open_orders_snapshot(
            envelope(
                open_orders(3, boundary(40, 41), Vec::new()),
                1,
                41,
                Some(3),
                130,
            ),
            &[],
        )
        .unwrap();
    let projection = state
        .orders()
        .find(|row| row.identity().venue_order_key() == Some(venue_order("external-1")))
        .unwrap();
    assert!(projection.missing_from_complete_open_snapshot());
    assert_eq!(
        projection.reservation(),
        Some(PmReservationKnowledge::Known(reserve))
    );
    let partial = order(
        1,
        "external-1",
        PmOrderSide::Buy,
        PmOrderStatus::PartiallyFilled,
        250_000,
    );
    let partial_reserve =
        PmExactReservation::policy_approved(U256::from_u64(300_010), U256::ZERO).unwrap();
    state
        .observe_order(
            envelope(partial, 1, 45, None, 135),
            PmRemoteOrderKnowledge::Unmanaged(PmReservationKnowledge::Known(partial_reserve)),
        )
        .unwrap();
    state
        .apply_open_orders_snapshot(
            envelope(
                open_orders(4, boundary(40, 50), Vec::new()),
                1,
                50,
                Some(4),
                140,
            ),
            &[],
        )
        .unwrap();
    let projection = state
        .orders()
        .find(|row| row.identity().venue_order_key() == Some(venue_order("external-1")))
        .unwrap();
    assert!(!projection.missing_from_complete_open_snapshot());
    assert_eq!(
        projection.reservation(),
        Some(PmReservationKnowledge::Known(partial_reserve))
    );

    let cut = boundary(55, 56);
    let detail = PmExactOrderDetail::new(
        source(),
        scope(),
        snapshot(5),
        cut,
        venue_order("external-1"),
        None,
    )
    .unwrap();
    assert_eq!(
        state
            .apply_order_detail(
                envelope(detail, 1, 56, Some(5), 150),
                PmReservationKnowledge::Unknown,
            )
            .unwrap(),
        PmOrderApply::DetailAbsenceTerminalized
    );
    let projection = state
        .orders()
        .find(|row| row.identity().venue_order_key() == Some(venue_order("external-1")))
        .unwrap();
    assert!(projection.terminal_by_detail_absence());
    assert_eq!(projection.reservation(), None);
    assert!(
        state
            .pending_refresh_keys()
            .any(|key| key.reason() == PmRefreshReason::MissingOrderDetail)
    );
}

#[test]
fn client_and_venue_identity_bridge_coalesces_to_one_owned_order() {
    let mut state = new_state();
    make_ready(&mut state);
    let registered = client_only_order(42);
    let reservation =
        PmExactReservation::policy_approved(U256::from_u64(4_900_000), U256::ZERO).unwrap();
    state
        .register_owned_order(
            PmOwnedOrderRegistration::new(
                registered.order().client_order_key().unwrap(),
                instrument(),
                registered.side(),
                registered.price(),
                registered.progress().original_quantity(),
                reservation,
            )
            .unwrap(),
        )
        .unwrap();
    state
        .observe_order(
            envelope(venue_only_order("bridge-order"), 1, 30, None, 120),
            PmRemoteOrderKnowledge::Unmanaged(PmReservationKnowledge::Known(reservation)),
        )
        .unwrap();
    assert_eq!(state.orders().count(), 2);

    let combined = order(42, "bridge-order", PmOrderSide::Buy, PmOrderStatus::Open, 0);
    state
        .observe_order(
            envelope(combined, 1, 31, None, 121),
            PmRemoteOrderKnowledge::Unmanaged(PmReservationKnowledge::Known(reservation)),
        )
        .unwrap();

    let projections = state.orders().collect::<Vec<_>>();
    assert_eq!(projections.len(), 1);
    assert_eq!(
        projections[0].identity().client_order_key(),
        registered.order().client_order_key()
    );
    assert_eq!(
        projections[0].identity().venue_order_key(),
        Some(venue_order("bridge-order"))
    );
    assert_eq!(projections[0].ownership(), PmOrderOwnership::ProvenOwned);
    assert_eq!(
        projections[0].reservation(),
        Some(PmReservationKnowledge::Known(reservation))
    );
}

#[test]
fn canonical_owned_order_does_not_emit_an_unmanaged_refresh_requirement() {
    let mut state = new_state();
    let registered = client_only_order(43);
    let reservation =
        PmExactReservation::policy_approved(U256::from_u64(400_000), U256::ZERO).unwrap();
    state
        .register_owned_order(
            PmOwnedOrderRegistration::new(
                registered.order().client_order_key().unwrap(),
                instrument(),
                registered.side(),
                registered.price(),
                registered.progress().original_quantity(),
                reservation,
            )
            .unwrap(),
        )
        .unwrap();
    let observed = order(
        43,
        "owned-classification",
        PmOrderSide::Buy,
        PmOrderStatus::Open,
        0,
    );
    state
        .observe_order(
            envelope(observed, 1, 30, None, 120),
            PmRemoteOrderKnowledge::Unmanaged(PmReservationKnowledge::Known(reservation)),
        )
        .unwrap();
    assert!(state.pending_refresh_keys().all(|key| !matches!(
        key.reason(),
        PmRefreshReason::UnmanagedOrder | PmRefreshReason::AmbiguousOrder
    )));
}

#[test]
fn complete_open_snapshot_is_atomic_when_a_later_row_conflicts() {
    let mut state = new_state();
    make_ready(&mut state);
    let existing = order(
        50,
        "atomic-existing",
        PmOrderSide::Buy,
        PmOrderStatus::Open,
        0,
    );
    state
        .observe_order(
            envelope(existing, 1, 30, None, 120),
            PmRemoteOrderKnowledge::Ambiguous,
        )
        .unwrap();
    let before = state.orders().collect::<Vec<_>>();
    let inserted = order(
        51,
        "atomic-inserted",
        PmOrderSide::Buy,
        PmOrderStatus::Open,
        0,
    );
    let conflicting = PmOrderEvent::new(
        source(),
        instrument(),
        existing.order(),
        existing.side(),
        PmPrice::parse_decimal("0.41").unwrap(),
        existing.progress(),
    )
    .unwrap();
    let cut = boundary(40, 41);
    assert!(matches!(
        state.apply_open_orders_snapshot(
            envelope(
                open_orders(3, cut, vec![inserted, conflicting]),
                1,
                41,
                Some(3),
                130,
            ),
            &[],
        ),
        Err(PmPrivateStateError::Order(_))
    ));
    assert_eq!(state.orders().collect::<Vec<_>>(), before);

    assert!(matches!(
        state
            .apply_open_orders_snapshot(
                envelope(
                    open_orders(3, cut, vec![inserted, existing]),
                    1,
                    41,
                    Some(3),
                    130,
                ),
                &[],
            )
            .unwrap(),
        PmOpenOrdersApply::Applied { inserted: 1, .. }
    ));
    assert_eq!(state.orders().count(), 2);
}

#[test]
fn reconnect_orders_by_epoch_and_old_epoch_delivery_cannot_create_state() {
    let mut state = new_state();
    let first = order(1, "epoch-order", PmOrderSide::Buy, PmOrderStatus::Open, 0);
    state
        .observe_order(
            envelope(first, 1, 100, None, 100),
            PmRemoteOrderKnowledge::Unmanaged(PmReservationKnowledge::Unknown),
        )
        .unwrap();
    state
        .observe_reconnect(ConnectionEpoch::new(2), 200)
        .unwrap();
    let terminal = order(
        1,
        "epoch-order",
        PmOrderSide::Buy,
        PmOrderStatus::Cancelled,
        0,
    );
    assert_eq!(
        state
            .observe_order(
                envelope(terminal, 2, 1, None, 201),
                PmRemoteOrderKnowledge::Unmanaged(PmReservationKnowledge::Unknown),
            )
            .unwrap(),
        PmOrderApply::TerminalReservationReleased
    );
    let old_new_order = order(2, "old-epoch-new", PmOrderSide::Buy, PmOrderStatus::Open, 0);
    assert!(matches!(
        state.observe_order(
            envelope(old_new_order, 1, 101, None, 202),
            PmRemoteOrderKnowledge::Ambiguous,
        ),
        Err(PmPrivateStateError::OldConnectionEpoch)
    ));
    assert_eq!(state.orders().count(), 1);
    let old_unresolved = unresolved_fill(
        "old-epoch-unresolved",
        None,
        Some("old-candidate"),
        PmUnresolvedFillReason::MissingExactOrderLinkage,
        PmFillSettlementStatus::Matched,
    );
    assert!(matches!(
        state.observe_unresolved_fill(envelope(old_unresolved, 1, 102, None, 203)),
        Err(PmPrivateStateError::OldConnectionEpoch)
    ));
    assert_eq!(state.unresolved_fills().count(), 0);
}

#[test]
fn read_only_inputs_cannot_create_or_advance_private_epoch_evidence() {
    let cut = boundary(10, 11);
    let mut unstarted = unstarted_state();
    let account = envelope(
        account_snapshot(1, cut, AccountFacts::ready()),
        1,
        11,
        Some(1),
        100,
    );
    assert!(matches!(
        unstarted.apply_account_snapshot(account),
        Err(PmPrivateStateError::MissingPrivateEpochEvidence)
    ));
    let account = envelope(
        account_snapshot(1, cut, AccountFacts::ready()),
        1,
        11,
        Some(1),
        100,
    );
    let fills = envelope(fill_query(1, cut, None, 1, Vec::new()), 1, 11, Some(1), 100);
    assert!(matches!(
        unstarted.apply_reconciliation(account, fills),
        Err(PmPrivateStateError::MissingPrivateEpochEvidence)
    ));
    assert_eq!(
        unstarted.account_projection().collateral(),
        reap_pm_state::PmObservedAmount::Unavailable
    );

    let mut state = new_state();
    let ahead = boundary(20, 21);
    assert!(matches!(
        state.apply_account_snapshot(envelope(
            account_snapshot(2, ahead, AccountFacts::ready()),
            2,
            21,
            Some(2),
            110,
        )),
        Err(PmPrivateStateError::PrivateEpochMismatch)
    ));
    assert_eq!(
        state.account_projection().collateral(),
        reap_pm_state::PmObservedAmount::Unavailable
    );
}

#[test]
fn reconciliation_requires_the_same_private_connection_identity() {
    let mut state = new_state();
    let cut = boundary(20, 21);
    let account = envelope_on(
        account_snapshot(2, cut, AccountFacts::ready()),
        "fixture-private-a",
        1,
        21,
        Some(2),
        110,
    );
    let fills = envelope_on(
        fill_query(2, cut, None, 1, Vec::new()),
        "fixture-private-b",
        1,
        21,
        Some(2),
        110,
    );
    assert!(matches!(
        state.apply_reconciliation(account, fills),
        Err(PmPrivateStateError::ReconciliationPairMismatch)
    ));
    assert_eq!(
        state.account_projection().collateral(),
        reap_pm_state::PmObservedAmount::Unavailable
    );
}

#[test]
fn reconnect_invalidates_old_order_freshness_until_new_epoch_evidence() {
    let mut state = new_state();
    make_ready(&mut state);
    state
        .observe_reconnect(ConnectionEpoch::new(2), 200)
        .unwrap();
    let cut = boundary(10, 11);
    state
        .apply_reconciliation(
            envelope(
                account_snapshot(1, cut, AccountFacts::ready()),
                2,
                11,
                Some(1),
                210,
            ),
            envelope(
                fill_query(1, cut, state.fill_watermark(), 2, Vec::new()),
                2,
                11,
                Some(1),
                210,
            ),
        )
        .unwrap();
    assert_eq!(
        state.quote_readiness(quote(211)),
        PmPrivateReadiness::Blocked(PmPrivateReadinessReason::DependencyUnavailable(
            PmPrivateDependency::OrderLifecycle
        ))
    );
}

#[test]
fn startup_unknown_and_later_ambiguous_orders_fail_closed_without_owned_cancel() {
    let mut startup = new_state();
    let discovered = order(
        9,
        "startup-external",
        PmOrderSide::Buy,
        PmOrderStatus::Open,
        0,
    );
    startup
        .apply_open_orders_snapshot(
            envelope(
                open_orders(1, boundary(10, 11), vec![discovered]),
                1,
                11,
                Some(1),
                100,
            ),
            &[],
        )
        .unwrap();
    let cut = boundary(20, 21);
    startup
        .apply_reconciliation(
            envelope(
                account_snapshot(2, cut, AccountFacts::ready()),
                1,
                21,
                Some(2),
                110,
            ),
            envelope(fill_query(2, cut, None, 1, Vec::new()), 1, 21, Some(2), 110),
        )
        .unwrap();
    assert!(matches!(
        startup.quote_readiness(quote(120)),
        PmPrivateReadiness::Blocked(PmPrivateReadinessReason::UnknownReservation(_))
    ));
    assert!(startup.owned_cancel_intents().next().is_none());

    let mut later = new_state();
    make_ready(&mut later);
    let ambiguous = order(
        10,
        "later-ambiguous",
        PmOrderSide::Buy,
        PmOrderStatus::Open,
        0,
    );
    later
        .observe_order(
            envelope(ambiguous, 1, 30, None, 120),
            PmRemoteOrderKnowledge::Ambiguous,
        )
        .unwrap();
    assert_eq!(
        later.quote_readiness(quote(121)),
        PmPrivateReadiness::Blocked(PmPrivateReadinessReason::UnmanagedOwnershipAmbiguous)
    );
    assert!(later.owned_cancel_intents().next().is_none());
}

#[test]
fn owned_cancel_intents_exclude_unmanaged_orders_and_are_canonically_ordered() {
    let mut state = new_state();
    make_ready(&mut state);
    for (client, venue) in [(2, "owned-b"), (1, "owned-a")] {
        let event = order(client, venue, PmOrderSide::Buy, PmOrderStatus::Open, 0);
        let reservation =
            PmExactReservation::policy_approved(U256::from_u64(400_000), U256::ZERO).unwrap();
        state
            .register_owned_order(
                PmOwnedOrderRegistration::new(
                    event.order().client_order_key().unwrap(),
                    instrument(),
                    event.side(),
                    event.price(),
                    event.progress().original_quantity(),
                    reservation,
                )
                .unwrap(),
            )
            .unwrap();
        state
            .observe_order(
                envelope(
                    event,
                    1,
                    30 + u64::from(client),
                    None,
                    120 + u64::from(client),
                ),
                PmRemoteOrderKnowledge::Unmanaged(PmReservationKnowledge::Known(reservation)),
            )
            .unwrap();
    }
    let external = order(3, "external-z", PmOrderSide::Buy, PmOrderStatus::Open, 0);
    state
        .observe_order(
            envelope(external, 1, 40, None, 130),
            PmRemoteOrderKnowledge::Unmanaged(PmReservationKnowledge::Known(
                PmExactReservation::policy_approved(U256::from_u64(400_000), U256::ZERO).unwrap(),
            )),
        )
        .unwrap();
    let changed_terms = PmOrderEvent::new(
        source(),
        instrument(),
        order(1, "owned-a", PmOrderSide::Buy, PmOrderStatus::Open, 0).order(),
        PmOrderSide::Buy,
        PmPrice::parse_decimal("0.41").unwrap(),
        PmOrderProgress::new(
            PmQuantity::parse_decimal("1").unwrap(),
            U256::ZERO,
            PmOrderStatus::Open,
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        state
            .observe_order(
                envelope(changed_terms, 1, 50, None, 140),
                PmRemoteOrderKnowledge::Unmanaged(PmReservationKnowledge::Known(
                    PmExactReservation::policy_approved(U256::from_u64(410_000), U256::ZERO)
                        .unwrap(),
                )),
            )
            .is_err()
    );
    assert_eq!(
        state
            .owned_cancel_intents()
            .map(|intent| intent.venue_order())
            .collect::<Vec<_>>(),
        vec![venue_order("owned-a"), venue_order("owned-b")]
    );

    let tight_limits = PmRiskLimits::new(
        PmOrderRiskLimits::new(
            PmQuantity::parse_decimal("10").unwrap(),
            U256::from_u64(10_000_000),
        )
        .unwrap(),
        PmExposureRiskLimits::new(
            U256::from_u64(100_000_000),
            U256::from_u64(1),
            U256::from_u64(1),
            U256::from_u64(100_000_000),
        )
        .unwrap(),
        PmCardinalityRiskLimits::new(10, 10, 10).unwrap(),
        PmFreshnessRiskLimits::new(1_000, 1_000, 1_000, 1_000, 1_000, 1_000).unwrap(),
    );
    let mut risk_state = PmPrivateState::new(config(), tight_limits).unwrap();
    risk_state
        .observe_reconnect(ConnectionEpoch::new(1), 1)
        .unwrap();
    make_ready(&mut risk_state);
    let owned = order(1, "owned-risk", PmOrderSide::Buy, PmOrderStatus::Open, 0);
    let reservation =
        PmExactReservation::policy_approved(U256::from_u64(400_000), U256::ZERO).unwrap();
    risk_state
        .register_owned_order(
            PmOwnedOrderRegistration::new(
                owned.order().client_order_key().unwrap(),
                instrument(),
                owned.side(),
                owned.price(),
                owned.progress().original_quantity(),
                reservation,
            )
            .unwrap(),
        )
        .unwrap();
    risk_state
        .observe_order(
            envelope(owned, 1, 30, None, 120),
            PmRemoteOrderKnowledge::Unmanaged(PmReservationKnowledge::Known(reservation)),
        )
        .unwrap();
    risk_state
        .evaluate_risk_candidate(
            quote(121),
            PmRiskDependency::available(120),
            PmRiskDependency::available(120),
        )
        .unwrap();
    assert_eq!(
        risk_state
            .owned_cancel_intents()
            .map(|intent| intent.venue_order())
            .collect::<Vec<_>>(),
        vec![venue_order("owned-risk")]
    );
}

#[test]
fn refresh_tickets_are_bound_to_one_opaque_state_owner() {
    let mut first = new_state();
    let mut sibling = new_state();
    let PmRefreshRequired::Inserted { ticket } = first
        .require_refresh(PmRefreshReason::AccountDivergence)
        .unwrap()
    else {
        panic!("first requirement inserts");
    };
    assert!(matches!(
        sibling.mark_refresh_admitted(ticket),
        Err(PmPrivateStateError::RefreshScopeMismatch)
    ));
    assert_eq!(
        first.mark_refresh_admitted(ticket).unwrap(),
        PmRefreshAdmission::Admitted(ticket)
    );
}

#[test]
fn external_ingress_fault_is_typed_bounded_and_fail_closed() {
    let mut state = new_state();
    let first = PmPrivateExternalIngressFault::new(
        PmPrivateExternalIngressLane::PrivateLifecycle,
        PmPrivateExternalIngressFailure::Normalization,
    );
    state.record_external_ingress_fault(first);
    state.record_external_ingress_fault(PmPrivateExternalIngressFault::new(
        PmPrivateExternalIngressLane::OpenOrders,
        PmPrivateExternalIngressFailure::Contract,
    ));

    assert_eq!(
        state.halt(),
        Some(PmPrivateHaltReason::ExternalIngressFault(first))
    );
    let counters = state.external_ingress_counters();
    assert_eq!(counters.total(), 2);
    assert_eq!(
        counters.for_lane(PmPrivateExternalIngressLane::PrivateLifecycle),
        1
    );
    assert_eq!(
        counters.for_lane(PmPrivateExternalIngressLane::OpenOrders),
        1
    );
    assert_eq!(
        counters.for_failure(PmPrivateExternalIngressFailure::Normalization),
        1
    );
    assert_eq!(
        counters.for_failure(PmPrivateExternalIngressFailure::Contract),
        1
    );
    assert_eq!(
        state.quote_readiness(quote(120)),
        PmPrivateReadiness::Blocked(PmPrivateReadinessReason::Halted(
            PmPrivateHaltReason::ExternalIngressFault(first)
        ))
    );
    assert!(
        state
            .pending_refresh_keys()
            .any(|key| key.reason() == PmRefreshReason::ExternalIngressFault)
    );
}

#[test]
fn private_events_are_checked_against_local_tick_minimum_and_partial_fill_integrality() {
    let mut state = new_state();
    let off_tick = PmOrderEvent::new(
        source(),
        instrument(),
        order(1, "off-tick", PmOrderSide::Buy, PmOrderStatus::Open, 0).order(),
        PmOrderSide::Buy,
        PmPrice::parse_decimal("0.40005").unwrap(),
        PmOrderProgress::new(
            PmQuantity::parse_decimal("1").unwrap(),
            U256::ZERO,
            PmOrderStatus::Open,
        )
        .unwrap(),
    )
    .unwrap();
    assert!(matches!(
        state.observe_order(
            envelope(off_tick, 1, 1, None, 100),
            PmRemoteOrderKnowledge::Ambiguous,
        ),
        Err(PmPrivateStateError::Order(_))
    ));

    let mut state = new_state();
    let below_min = PmOrderEvent::new(
        source(),
        instrument(),
        order(2, "below-min", PmOrderSide::Buy, PmOrderStatus::Open, 0).order(),
        PmOrderSide::Buy,
        PmPrice::parse_decimal("0.40").unwrap(),
        PmOrderProgress::new(
            PmQuantity::parse_decimal("0.001").unwrap(),
            U256::ZERO,
            PmOrderStatus::Open,
        )
        .unwrap(),
    )
    .unwrap();
    assert!(matches!(
        state.observe_order(
            envelope(below_min, 1, 1, None, 100),
            PmRemoteOrderKnowledge::Ambiguous,
        ),
        Err(PmPrivateStateError::Order(_))
    ));

    let mut state = new_state();
    let partial = fill(
        "partial",
        "partial-fill",
        PmOrderSide::Buy,
        "0.0001",
        PmFillFee::Unknown,
    );
    assert!(matches!(
        state
            .observe_fill(envelope(partial, 1, 1, None, 100))
            .unwrap(),
        PmFillApply::PrincipalApplied { .. }
    ));
}

#[test]
fn stale_dependency_is_typed_after_a_valid_complete_state() {
    let mut state = new_state();
    make_ready(&mut state);
    assert_eq!(
        state.quote_readiness(quote(1_111)),
        PmPrivateReadiness::Blocked(PmPrivateReadinessReason::DependencyStale {
            dependency: PmPrivateDependency::PrivateLifecycle,
            age_ns: 1_110,
            limit_ns: 1_000,
        })
    );
}

#[test]
fn published_balance_position_disagreement_and_resolved_inventory_are_distinct() {
    let mut mismatch = new_state();
    mismatch
        .apply_open_orders_snapshot(
            envelope(
                open_orders(1, boundary(10, 11), Vec::new()),
                1,
                11,
                Some(1),
                100,
            ),
            &[],
        )
        .unwrap();
    let mut facts = AccountFacts::ready();
    facts.position = Some((4_000_000, PmPositionAvailability::Tradable));
    let cut = boundary(20, 21);
    mismatch
        .apply_reconciliation(
            envelope(account_snapshot(2, cut, facts), 1, 21, Some(2), 110),
            envelope(fill_query(2, cut, None, 1, Vec::new()), 1, 21, Some(2), 110),
        )
        .unwrap();
    assert_eq!(
        mismatch.quote_readiness(quote(120)),
        PmPrivateReadiness::Blocked(PmPrivateReadinessReason::PublishedInventoryMismatch {
            balance: U256::from_u64(5_000_000),
            position: U256::from_u64(4_000_000),
        })
    );
    assert!(
        mismatch
            .pending_refresh_keys()
            .any(|key| key.reason() == PmRefreshReason::AccountDivergence)
    );

    let mut resolved = new_state();
    resolved
        .apply_open_orders_snapshot(
            envelope(
                open_orders(1, boundary(10, 11), Vec::new()),
                1,
                11,
                Some(1),
                100,
            ),
            &[],
        )
        .unwrap();
    let mut facts = AccountFacts::ready();
    facts.position = Some((5_000_000, PmPositionAvailability::ResolvedUnredeemed));
    let cut = boundary(20, 21);
    resolved
        .apply_reconciliation(
            envelope(account_snapshot(2, cut, facts), 1, 21, Some(2), 110),
            envelope(fill_query(2, cut, None, 1, Vec::new()), 1, 21, Some(2), 110),
        )
        .unwrap();
    assert_eq!(
        resolved.quote_readiness(quote(120)),
        PmPrivateReadiness::Blocked(PmPrivateReadinessReason::PositionResolvedUnredeemed(
            U256::from_u64(5_000_000)
        ))
    );
    assert!(
        resolved
            .pending_refresh_keys()
            .any(|key| key.reason() == PmRefreshReason::PositionUnavailable)
    );
}

#[test]
fn exact_allowances_and_live_reservations_constrain_resources_independently() {
    let mut low_allowance = new_state();
    low_allowance
        .apply_open_orders_snapshot(
            envelope(
                open_orders(1, boundary(10, 11), Vec::new()),
                1,
                11,
                Some(1),
                100,
            ),
            &[],
        )
        .unwrap();
    let mut facts = AccountFacts::ready();
    facts.collateral_allowance = Some(399_999);
    let cut = boundary(20, 21);
    low_allowance
        .apply_reconciliation(
            envelope(account_snapshot(2, cut, facts), 1, 21, Some(2), 110),
            envelope(fill_query(2, cut, None, 1, Vec::new()), 1, 21, Some(2), 110),
        )
        .unwrap();
    let [collateral_spender, _] = spenders();
    assert_eq!(
        low_allowance.quote_readiness(quote(120)),
        PmPrivateReadiness::Blocked(PmPrivateReadinessReason::ExactAllowanceInsufficient {
            spender: collateral_spender,
            required: U256::from_u64(400_000),
            available: U256::from_u64(399_999),
        })
    );

    let mut pressure = new_state();
    make_ready(&mut pressure);
    let external = order(
        3,
        "reservation-pressure",
        PmOrderSide::Buy,
        PmOrderStatus::Open,
        0,
    );
    pressure
        .observe_order(
            envelope(external, 1, 30, None, 120),
            PmRemoteOrderKnowledge::Unmanaged(PmReservationKnowledge::Known(
                PmExactReservation::policy_approved(U256::from_u64(9_800_001), U256::ZERO).unwrap(),
            )),
        )
        .unwrap();
    assert_eq!(
        pressure.quote_readiness(quote(121)),
        PmPrivateReadiness::Blocked(PmPrivateReadinessReason::ExactAllowanceInsufficient {
            spender: collateral_spender,
            required: U256::from_u64(10_200_001),
            available: U256::from_u64(10_000_000),
        })
    );
}

#[test]
fn risk_requires_canonical_inventory_and_counts_both_candidate_resources() {
    let mut unavailable = new_state();
    assert!(matches!(
        unavailable.evaluate_risk_candidate(
            quote(120),
            PmRiskDependency::available(120),
            PmRiskDependency::available(120),
        ),
        Err(PmPrivateStateError::CanonicalInventoryUnavailable)
    ));

    let token_tight_limits = PmRiskLimits::new(
        PmOrderRiskLimits::new(
            PmQuantity::parse_decimal("10").unwrap(),
            U256::from_u64(10_000_000),
        )
        .unwrap(),
        PmExposureRiskLimits::new(
            U256::from_u64(100_000_000),
            U256::from_u64(100_000_000),
            U256::from_u64(100_000_000),
            U256::from_u64(1),
        )
        .unwrap(),
        PmCardinalityRiskLimits::new(1_024, 1_024, 10_000).unwrap(),
        PmFreshnessRiskLimits::new(1_000, 1_000, 1_000, 1_000, 1_000, 1_000).unwrap(),
    );
    let mut state = PmPrivateState::new(config(), token_tight_limits).unwrap();
    state.observe_reconnect(ConnectionEpoch::new(1), 1).unwrap();
    make_ready(&mut state);
    let candidate = PmPrivateQuoteRequest::new(
        120,
        PmOrderSide::Buy,
        PmPrice::parse_decimal("0.40").unwrap(),
        PmQuantity::parse_decimal("1").unwrap(),
        PmExactReservation::policy_approved(U256::from_u64(400_000), U256::from_u64(2)).unwrap(),
    );
    assert_eq!(
        state
            .evaluate_risk_candidate(
                candidate,
                PmRiskDependency::available(120),
                PmRiskDependency::available(120),
            )
            .unwrap(),
        PmRiskDecision::Rejected {
            reason: PmRiskReason::ReservedToken {
                observed: U256::from_u64(2),
                limit: U256::from_u64(1),
            },
            halt: reap_pm_state::PmRiskHaltScope::Market,
        }
    );
}

#[test]
fn account_snapshot_duplicate_is_idempotent_and_does_not_zero_later_state() {
    let mut state = new_state();
    let cut = boundary(10, 11);
    let event = envelope(
        account_snapshot(1, cut, AccountFacts::ready()),
        1,
        11,
        Some(1),
        100,
    );
    assert!(matches!(
        state.apply_account_snapshot(event).unwrap(),
        PmAccountSnapshotApply::Applied { .. }
    ));
    let duplicate = envelope(
        account_snapshot(1, cut, AccountFacts::ready()),
        1,
        11,
        Some(1),
        100,
    );
    assert!(matches!(
        state.apply_account_snapshot(duplicate).unwrap(),
        PmAccountSnapshotApply::Duplicate { .. }
    ));
    assert_eq!(
        state.account_projection().collateral().value(),
        Some(U256::from_u64(10_000_000))
    );
}

#[test]
fn fill_settlement_progression_is_idempotent_and_retry_failure_needs_a_new_cut() {
    let mut state = new_state();
    make_ready(&mut state);
    let known_fee = PmFillFee::Known {
        asset: domain().collateral(),
        delta: PmSignedUnits::ZERO,
    };
    let matched = fill_with_settlement(
        "settlement-a",
        "trade-settlement-a",
        PmOrderSide::Buy,
        "0.25",
        PmFillSettlementStatus::Matched,
        known_fee,
    );
    state
        .observe_fill(envelope(matched, 1, 30, None, 120))
        .unwrap();
    let mined = fill_with_settlement(
        "settlement-a",
        "trade-settlement-a",
        PmOrderSide::Buy,
        "0.25",
        PmFillSettlementStatus::Mined,
        known_fee,
    );
    assert!(matches!(
        state
            .observe_fill(envelope(mined, 1, 31, None, 121))
            .unwrap(),
        PmFillApply::Enriched {
            settlement: PmFillSettlementStatus::Mined,
            ..
        }
    ));
    let confirmed = fill_with_settlement(
        "settlement-a",
        "trade-settlement-a",
        PmOrderSide::Buy,
        "0.25",
        PmFillSettlementStatus::Confirmed,
        known_fee,
    );
    assert!(matches!(
        state
            .observe_fill(envelope(confirmed, 1, 32, None, 122))
            .unwrap(),
        PmFillApply::Enriched {
            settlement: PmFillSettlementStatus::Confirmed,
            ..
        }
    ));
    assert_eq!(
        state
            .observe_fill(envelope(confirmed, 1, 33, None, 123))
            .unwrap(),
        PmFillApply::Duplicate
    );
    let retryable = fill_with_settlement(
        "settlement-b",
        "trade-settlement-b",
        PmOrderSide::Buy,
        "0.25",
        PmFillSettlementStatus::Matched,
        known_fee,
    );
    state
        .observe_fill(envelope(retryable, 1, 34, None, 124))
        .unwrap();
    assert_eq!(state.fill_counters().principal_applications(), 2);

    let first_cut = boundary(40, 41);
    state
        .apply_reconciliation(
            envelope(
                account_snapshot(3, first_cut, AccountFacts::ready()),
                1,
                41,
                Some(3),
                130,
            ),
            envelope(
                fill_query(
                    3,
                    first_cut,
                    state.fill_watermark(),
                    2,
                    vec![confirmed, retryable],
                ),
                1,
                41,
                Some(3),
                130,
            ),
        )
        .unwrap();
    assert!(matches!(
        state.quote_readiness(quote(131)),
        PmPrivateReadiness::Ready(_)
    ));

    let retrying = fill_with_settlement(
        "settlement-b",
        "trade-settlement-b",
        PmOrderSide::Buy,
        "0.25",
        PmFillSettlementStatus::Retrying,
        known_fee,
    );
    state
        .observe_fill(envelope(retrying, 1, 50, None, 140))
        .unwrap();
    assert_eq!(state.provisional_deltas().uncovered_fills(), 1);
    assert_eq!(
        state.quote_readiness(quote(141)),
        PmPrivateReadiness::Blocked(PmPrivateReadinessReason::FillSettlementRetrying(
            retrying.fill_key()
        ))
    );
    let failed = fill_with_settlement(
        "settlement-b",
        "trade-settlement-b",
        PmOrderSide::Buy,
        "0.25",
        PmFillSettlementStatus::Failed,
        known_fee,
    );
    state
        .observe_fill(envelope(failed, 1, 51, None, 141))
        .unwrap();
    assert_eq!(state.fill_counters().principal_applications(), 2);
    assert_eq!(
        state.quote_readiness(quote(142)),
        PmPrivateReadiness::Blocked(PmPrivateReadinessReason::FillSettlementFailed(
            failed.fill_key()
        ))
    );

    let second_cut = boundary(60, 61);
    state
        .apply_reconciliation(
            envelope(
                account_snapshot(4, second_cut, AccountFacts::ready()),
                1,
                61,
                Some(4),
                150,
            ),
            envelope(
                fill_query(
                    4,
                    second_cut,
                    state.fill_watermark(),
                    3,
                    vec![confirmed, failed],
                ),
                1,
                61,
                Some(4),
                150,
            ),
        )
        .unwrap();
    assert_eq!(state.provisional_deltas().uncovered_fills(), 0);
    assert!(matches!(
        state.quote_readiness(quote(151)),
        PmPrivateReadiness::Ready(_)
    ));
}

#[test]
fn failed_fill_cannot_invent_a_recovery_or_rollback_principal() {
    let mut state = new_state();
    make_ready(&mut state);
    let known_fee = PmFillFee::Known {
        asset: domain().collateral(),
        delta: PmSignedUnits::ZERO,
    };
    let failed = fill_with_settlement(
        "failed-terminal",
        "trade-failed-terminal",
        PmOrderSide::Buy,
        "0.25",
        PmFillSettlementStatus::Failed,
        known_fee,
    );
    state
        .observe_fill(envelope(failed, 1, 30, None, 120))
        .unwrap();
    let invented_recovery = fill_with_settlement(
        "failed-terminal",
        "trade-failed-terminal",
        PmOrderSide::Buy,
        "0.25",
        PmFillSettlementStatus::Confirmed,
        known_fee,
    );
    assert!(matches!(
        state.observe_fill(envelope(invented_recovery, 1, 31, None, 121)),
        Err(PmPrivateStateError::Fill(_))
    ));
    assert_eq!(state.fill_counters().principal_applications(), 1);
    assert_eq!(state.provisional_deltas().uncovered_fills(), 1);
}

#[test]
fn unresolved_fill_is_bounded_idempotent_and_cut_only_authoritative() {
    let mut state = new_state();
    make_ready(&mut state);
    let unresolved = unresolved_fill(
        "unresolved-trade",
        None,
        Some("candidate-maker"),
        PmUnresolvedFillReason::MissingLocalMakerOrderProof,
        PmFillSettlementStatus::Matched,
    );
    let key = unresolved.key();
    assert_eq!(
        state
            .observe_unresolved_fill(envelope(unresolved, 1, 30, None, 120))
            .unwrap(),
        PmUnresolvedFillApply::Inserted(key)
    );
    assert_eq!(
        state.quote_readiness(quote(121)),
        PmPrivateReadiness::Blocked(PmPrivateReadinessReason::UnresolvedFill {
            fill: key,
            reason: PmUnresolvedFillReason::MissingLocalMakerOrderProof,
        })
    );
    assert!(
        state
            .pending_refreshes()
            .any(|ticket| ticket.key().reason() == PmRefreshReason::UnresolvedFill)
    );
    assert_eq!(
        state
            .observe_unresolved_fill(envelope(unresolved, 1, 31, None, 121))
            .unwrap(),
        PmUnresolvedFillApply::Duplicate(key)
    );
    assert_eq!(state.unresolved_fill_counters().insertions(), 1);
    assert_eq!(state.unresolved_fills().count(), 1);

    let retrying = unresolved_fill(
        "unresolved-trade",
        None,
        Some("candidate-maker"),
        PmUnresolvedFillReason::MissingLocalMakerOrderProof,
        PmFillSettlementStatus::Retrying,
    );
    assert!(matches!(
        state
            .observe_unresolved_fill(envelope(retrying, 1, 32, None, 122))
            .unwrap(),
        PmUnresolvedFillApply::SettlementAdvanced {
            settlement: PmFillSettlementStatus::Retrying,
            ..
        }
    ));
    assert!(state.unresolved_fills().next().is_some_and(
        |fill| fill.is_active() && fill.settlement() == PmFillSettlementStatus::Retrying
    ));

    let cut = boundary(40, 41);
    state
        .apply_reconciliation(
            envelope(
                account_snapshot(3, cut, AccountFacts::ready()),
                1,
                41,
                Some(3),
                130,
            ),
            envelope(
                fill_query(3, cut, state.fill_watermark(), 2, Vec::new()),
                1,
                41,
                Some(3),
                130,
            ),
        )
        .unwrap();
    assert!(
        state
            .unresolved_fills()
            .next()
            .is_some_and(|fill| !fill.is_active())
    );
    assert!(matches!(
        state.quote_readiness(quote(131)),
        PmPrivateReadiness::Ready(_)
    ));
    assert_eq!(
        state
            .observe_unresolved_fill(envelope(retrying, 1, 50, None, 140))
            .unwrap(),
        PmUnresolvedFillApply::Duplicate(key)
    );
    assert!(matches!(
        state.quote_readiness(quote(141)),
        PmPrivateReadiness::Ready(_)
    ));

    let failed = unresolved_fill(
        "unresolved-trade",
        None,
        Some("candidate-maker"),
        PmUnresolvedFillReason::MissingLocalMakerOrderProof,
        PmFillSettlementStatus::Failed,
    );
    state
        .observe_unresolved_fill(envelope(failed, 1, 51, None, 141))
        .unwrap();
    assert_eq!(
        state.quote_readiness(quote(142)),
        PmPrivateReadiness::Blocked(PmPrivateReadinessReason::UnresolvedFill {
            fill: key,
            reason: PmUnresolvedFillReason::MissingLocalMakerOrderProof,
        })
    );
}

#[test]
fn unresolved_fill_conflict_and_capacity_fail_closed_without_eviction() {
    let mut conflict_state = new_state();
    let first = unresolved_fill(
        "unresolved-conflict",
        Some("exact-order"),
        None,
        PmUnresolvedFillReason::MissingDirectOrderRole,
        PmFillSettlementStatus::Matched,
    );
    conflict_state
        .observe_unresolved_fill(envelope(first, 1, 1, None, 1))
        .unwrap();
    let conflict = unresolved_fill(
        "unresolved-conflict",
        Some("exact-order"),
        None,
        PmUnresolvedFillReason::MultipleOrderReferenceKinds,
        PmFillSettlementStatus::Matched,
    );
    assert!(matches!(
        conflict_state.observe_unresolved_fill(envelope(conflict, 1, 2, None, 2)),
        Err(PmPrivateStateError::UnresolvedFill(_))
    ));
    assert_eq!(
        conflict_state.halt(),
        Some(reap_pm_state::PmPrivateHaltReason::ContractViolation)
    );

    let mut state = new_state();
    let mut last = None;
    for index in 0..reap_pm_state::MAX_PM_UNRESOLVED_FILLS {
        let candidate = format!("candidate-{index:04}");
        let observation = unresolved_fill(
            "shared-venue-trade",
            None,
            Some(&candidate),
            PmUnresolvedFillReason::MissingLocalMakerOrderProof,
            PmFillSettlementStatus::Matched,
        );
        state
            .observe_unresolved_fill(envelope(
                observation,
                1,
                u64::try_from(index).unwrap() + 1,
                None,
                u64::try_from(index).unwrap() + 1,
            ))
            .unwrap();
        last = Some(observation);
    }
    let last = last.unwrap();
    assert!(matches!(
        state
            .observe_unresolved_fill(envelope(last, 1, 9_000, None, 9_000))
            .unwrap(),
        PmUnresolvedFillApply::Duplicate(_)
    ));
    assert_eq!(
        state.unresolved_fills().count(),
        reap_pm_state::MAX_PM_UNRESOLVED_FILLS
    );
    let overflow = unresolved_fill(
        "shared-venue-trade",
        None,
        Some("candidate-overflow"),
        PmUnresolvedFillReason::MissingLocalMakerOrderProof,
        PmFillSettlementStatus::Matched,
    );
    assert!(matches!(
        state.observe_unresolved_fill(envelope(overflow, 1, 9_001, None, 9_001)),
        Err(PmPrivateStateError::UnresolvedFill(_))
    ));
    assert_eq!(
        state.unresolved_fills().count(),
        reap_pm_state::MAX_PM_UNRESOLVED_FILLS
    );
    assert_eq!(
        state.halt(),
        Some(reap_pm_state::PmPrivateHaltReason::UnresolvedFillCapacity)
    );
}

#[test]
fn order_capacity_deduplicates_before_saturation_and_never_evicts() {
    let mut state = new_state();
    let reservation =
        PmExactReservation::policy_approved(U256::from_u64(400_000), U256::ZERO).unwrap();
    let mut last = None;
    for index in 0..reap_pm_state::MAX_PM_PRIVATE_ORDERS {
        let venue = format!("capacity-order-{index:04}");
        let event = venue_only_order(&venue);
        state
            .observe_order(
                envelope(
                    event,
                    1,
                    u64::try_from(index).unwrap() + 1,
                    None,
                    u64::try_from(index).unwrap() + 1,
                ),
                PmRemoteOrderKnowledge::Unmanaged(PmReservationKnowledge::Known(reservation)),
            )
            .unwrap();
        last = Some((event, u64::try_from(index).unwrap() + 1));
    }
    let (last, last_occurrence) = last.unwrap();
    assert_eq!(
        state
            .observe_order(
                envelope(last, 1, last_occurrence, None, 9_000),
                PmRemoteOrderKnowledge::Unmanaged(PmReservationKnowledge::Known(reservation)),
            )
            .unwrap(),
        PmOrderApply::Duplicate
    );
    assert_eq!(state.orders().count(), reap_pm_state::MAX_PM_PRIVATE_ORDERS);
    let overflow = venue_only_order("capacity-order-overflow");
    assert!(matches!(
        state.observe_order(
            envelope(overflow, 1, 9_001, None, 9_001),
            PmRemoteOrderKnowledge::Unmanaged(PmReservationKnowledge::Known(reservation)),
        ),
        Err(PmPrivateStateError::Order(_))
    ));
    assert_eq!(state.orders().count(), reap_pm_state::MAX_PM_PRIVATE_ORDERS);
    assert_eq!(
        state.halt(),
        Some(reap_pm_state::PmPrivateHaltReason::OrderCapacity)
    );
}

#[test]
fn fill_capacity_deduplicates_before_saturation_and_never_evicts() {
    let mut state = new_state();
    let mut last = None;
    for index in 0..reap_pm_state::MAX_PM_PRIVATE_FILLS {
        let id = format!("capacity-{index:04}");
        let event = fill(
            "capacity-order",
            &id,
            PmOrderSide::Buy,
            "0.01",
            PmFillFee::Known {
                asset: domain().collateral(),
                delta: PmSignedUnits::ZERO,
            },
        );
        state
            .observe_fill(envelope(
                event,
                1,
                u64::try_from(index).unwrap() + 1,
                None,
                u64::try_from(index).unwrap() + 1,
            ))
            .unwrap();
        last = Some(event);
    }
    let last = last.unwrap();
    assert_eq!(
        state
            .observe_fill(envelope(last, 1, 9_000, None, 9_000))
            .unwrap(),
        PmFillApply::Duplicate
    );
    assert_eq!(state.fills().count(), reap_pm_state::MAX_PM_PRIVATE_FILLS);
    let overflow = fill(
        "capacity-order",
        "capacity-overflow",
        PmOrderSide::Buy,
        "0.01",
        PmFillFee::Known {
            asset: domain().collateral(),
            delta: PmSignedUnits::ZERO,
        },
    );
    assert!(matches!(
        state.observe_fill(envelope(overflow, 1, 9_001, None, 9_001)),
        Err(PmPrivateStateError::Fill(_))
    ));
    assert_eq!(state.fills().count(), reap_pm_state::MAX_PM_PRIVATE_FILLS);
    assert_eq!(
        state.halt(),
        Some(reap_pm_state::PmPrivateHaltReason::FillCapacity)
    );
}

#[test]
fn immediate_owned_fill_changes_exposure_and_ws_rest_duplicates_apply_principal_once() {
    let mut state = new_state();
    make_ready(&mut state);
    let quote = owned_quote_intent(1, 71, PmOrderSide::Buy);
    let client = quote.client_order();
    let venue = venue_order("owned-immediate");
    assert_eq!(
        state.admit_owned_quote(quote).unwrap(),
        PmOwnedQuoteAdmission::Admitted(client)
    );
    state
        .apply_owned_submit_result(client, PmOwnedSubmitResult::Accepted(venue))
        .unwrap();

    let event = fill(
        "owned-immediate",
        "owned-fill-1",
        PmOrderSide::Buy,
        "0.25",
        PmFillFee::Unknown,
    );
    let ticket = state.issue_owned_immediate_ack_ticket().unwrap();
    let ticket_occurrence = ticket.occurrence();
    state
        .observe_owned_immediate_fill(ticket, event, Some(U256::from_u64(250_000)))
        .unwrap();
    assert_eq!(state.provisional_deltas().uncovered_fills(), 1);
    assert_eq!(state.fill_counters().principal_applications(), 1);
    let immediate = state.owned_fills().next().unwrap();
    assert_eq!(immediate.first_occurrence(), ticket_occurrence);
    assert!(immediate.observed_from(PmOwnedObservationSource::ImmediateAcknowledgement));

    assert_eq!(
        state
            .observe_fill(envelope(event, 1, 30, None, 120))
            .unwrap(),
        PmFillApply::Duplicate
    );
    assert_eq!(state.fill_counters().principal_applications(), 1);
    assert!(
        state
            .owned_fills()
            .next()
            .unwrap()
            .observed_from(PmOwnedObservationSource::PrivateWebSocket)
    );

    let prior_watermark = state.fill_watermark().unwrap();
    let cut = boundary(40, 41);
    let account = envelope(
        account_snapshot(
            3,
            cut,
            AccountFacts {
                collateral: Some(9_900_000),
                outcome: Some(5_250_000),
                position: Some((5_250_000, PmPositionAvailability::Tradable)),
                collateral_allowance: Some(10_000_000),
                outcome_approval: Some(true),
                unknown_extra: false,
            },
        ),
        1,
        41,
        Some(3),
        130,
    );
    let fills = envelope(
        fill_query(3, cut, Some(prior_watermark), 2, vec![event]),
        1,
        41,
        Some(3),
        130,
    );
    state.apply_reconciliation(account, fills).unwrap();
    assert_eq!(state.fill_counters().principal_applications(), 1);
    let owned_fill = state.owned_fills().next().unwrap();
    assert!(owned_fill.observed_from(PmOwnedObservationSource::ImmediateAcknowledgement));
    assert!(owned_fill.observed_from(PmOwnedObservationSource::PrivateWebSocket));
    assert!(owned_fill.observed_from(PmOwnedObservationSource::RestReconciliation));
}

#[test]
fn reconciliation_reductions_expose_each_unique_owned_maker_leg_in_input_order() {
    let mut state = new_state();
    make_ready(&mut state);
    let buy = owned_quote_intent(1, 81, PmOrderSide::Buy);
    let sell = owned_quote_intent(2, 82, PmOrderSide::Sell);
    let buy_client = buy.client_order();
    let sell_client = sell.client_order();
    let buy_venue = venue_order("rest-multi-buy");
    let sell_venue = venue_order("rest-multi-sell");
    state.admit_owned_quote(buy).unwrap();
    state
        .apply_owned_submit_result(buy_client, PmOwnedSubmitResult::Accepted(buy_venue))
        .unwrap();
    state.admit_owned_quote(sell).unwrap();
    state
        .apply_owned_submit_result(sell_client, PmOwnedSubmitResult::Accepted(sell_venue))
        .unwrap();

    // One trade can contain more than one maker leg. Fill identity is the
    // venue-order/fill-id pair, so equal trade ids on distinct maker orders
    // remain two exact principal applications.
    let buy_leg = fill(
        "rest-multi-buy",
        "shared-maker-trade",
        PmOrderSide::Buy,
        "0.25",
        PmFillFee::Unknown,
    );
    let sell_leg = fill(
        "rest-multi-sell",
        "shared-maker-trade",
        PmOrderSide::Sell,
        "0.25",
        PmFillFee::Unknown,
    );
    let unowned = fill(
        "rest-multi-external",
        "shared-maker-trade",
        PmOrderSide::Buy,
        "0.25",
        PmFillFee::Unknown,
    );
    let prior_watermark = state.fill_watermark();
    let cut = boundary(40, 41);
    let account = envelope(
        account_snapshot(3, cut, AccountFacts::ready()),
        1,
        41,
        Some(3),
        130,
    );
    let fills = envelope(
        fill_query(3, cut, prior_watermark, 2, vec![buy_leg, sell_leg, unowned]),
        1,
        41,
        Some(3),
        130,
    );
    let mut reductions = PmReconciliationReductions::new();
    state
        .apply_reconciliation_with_reductions(account, fills, &mut reductions)
        .unwrap();

    assert_eq!(reductions.len(), 3);
    assert_eq!(reductions.unique_owned().count(), 2);
    let mut rows = reductions.iter();
    let first = rows.next().unwrap();
    assert_eq!(reductions.get(0), Some(first));
    assert_eq!(reductions.get(3), None);
    let second = rows.next().unwrap();
    let third = rows.next().unwrap();
    assert_eq!(*first.envelope().payload(), buy_leg);
    assert_eq!(*second.envelope().payload(), sell_leg);
    assert_eq!(*third.envelope().payload(), unowned);

    let PmReconciliationFillDisposition::OwnedApplied(first_owned) = first.disposition() else {
        panic!("first maker leg must be a unique owned fill");
    };
    let PmReconciliationFillDisposition::OwnedApplied(second_owned) = second.disposition() else {
        panic!("second maker leg must be a unique owned fill");
    };
    assert_eq!(
        first_owned.source(),
        PmOwnedObservationSource::RestReconciliation
    );
    assert_eq!(
        second_owned.source(),
        PmOwnedObservationSource::RestReconciliation
    );
    assert_eq!(first_owned.observation().key(), buy_leg.fill_key());
    assert_eq!(second_owned.observation().key(), sell_leg.fill_key());
    assert_eq!(
        second_owned.occurrence().reduction_sequence().value(),
        first_owned.occurrence().reduction_sequence().value() + 1
    );
    assert_eq!(
        first_owned.occurrence().private_occurrence(),
        Some(PmPrivateOccurrence::new(
            ConnectionEpoch::new(1),
            IngressSequence::new(41),
        ))
    );
    assert_eq!(
        first_owned.occurrence().snapshot_revision(),
        Some(SnapshotRevision::new(3))
    );
    assert_eq!(
        state
            .fills()
            .find(|fill| fill.key() == buy_leg.fill_key())
            .unwrap()
            .last_occurrence(),
        first_owned.occurrence()
    );
    assert_eq!(
        state
            .fills()
            .find(|fill| fill.key() == sell_leg.fill_key())
            .unwrap()
            .last_occurrence(),
        second_owned.occurrence()
    );
    assert!(matches!(
        third.disposition(),
        PmReconciliationFillDisposition::Unowned(PmOwnedRemoteOrderApply::AmbiguousRemote)
    ));
    assert_eq!(
        state.owned_order(buy_client).unwrap().known_fill_total(),
        U256::from_u64(250_000)
    );
    assert_eq!(
        state.owned_order(sell_client).unwrap().known_fill_total(),
        U256::from_u64(250_000)
    );
}

#[test]
fn reconciliation_reductions_separate_ws_duplicates_from_rows_stale_to_the_cut() {
    let mut state = new_state();
    make_ready(&mut state);
    let quote = owned_quote_intent(1, 83, PmOrderSide::Buy);
    let client = quote.client_order();
    let venue = venue_order("rest-duplicate");
    state.admit_owned_quote(quote).unwrap();
    state
        .apply_owned_submit_result(client, PmOwnedSubmitResult::Accepted(venue))
        .unwrap();
    let event = fill(
        "rest-duplicate",
        "rest-duplicate-fill",
        PmOrderSide::Buy,
        "0.25",
        PmFillFee::Unknown,
    );
    state
        .observe_fill_reduction(envelope(event, 1, 30, None, 120))
        .unwrap();

    let cut = boundary(40, 41);
    let mut reductions = PmReconciliationReductions::new();
    state
        .apply_reconciliation_with_reductions(
            envelope(
                account_snapshot(3, cut, AccountFacts::ready()),
                1,
                41,
                Some(3),
                130,
            ),
            envelope(
                fill_query(3, cut, state.fill_watermark(), 2, vec![event]),
                1,
                41,
                Some(3),
                130,
            ),
            &mut reductions,
        )
        .unwrap();
    let duplicate = reductions.iter().next().unwrap();
    let PmReconciliationFillDisposition::OwnedDuplicate(duplicate_owned) = duplicate.disposition()
    else {
        panic!("REST observation after the WS occurrence must be a duplicate");
    };
    assert!(matches!(
        duplicate_owned.apply(),
        PmOwnedFillApply::Duplicate {
            source_added: true,
            ..
        }
    ));
    assert_eq!(reductions.unique_owned().count(), 0);

    // A later WS occurrence followed by a cut whose request boundary predates
    // that occurrence is an explicitly stale REST row, not another durable
    // application.
    assert_eq!(
        state
            .observe_fill(envelope(event, 1, 50, None, 140))
            .unwrap(),
        PmFillApply::Duplicate
    );
    let stale_cut = boundary(45, 51);
    state
        .apply_reconciliation_with_reductions(
            envelope(
                account_snapshot(4, stale_cut, AccountFacts::ready()),
                1,
                51,
                Some(4),
                150,
            ),
            envelope(
                fill_query(4, stale_cut, state.fill_watermark(), 3, vec![event]),
                1,
                51,
                Some(4),
                150,
            ),
            &mut reductions,
        )
        .unwrap();
    assert!(matches!(
        reductions.iter().next().unwrap().disposition(),
        PmReconciliationFillDisposition::OwnedStale(_)
    ));
    assert_eq!(state.owned_fills().count(), 1);
}

#[test]
fn reconciliation_owned_preflight_failure_commits_neither_cut_nor_sequence_range() {
    let mut seed = new_state();
    make_ready(&mut seed);
    let seed_cut = boundary(40, 41);
    let mut reductions = PmReconciliationReductions::new();
    seed.apply_reconciliation_with_reductions(
        envelope(
            account_snapshot(3, seed_cut, AccountFacts::ready()),
            1,
            41,
            Some(3),
            130,
        ),
        envelope(
            fill_query(
                3,
                seed_cut,
                seed.fill_watermark(),
                2,
                vec![fill(
                    "seed-unowned",
                    "seed-fill",
                    PmOrderSide::Buy,
                    "0.25",
                    PmFillFee::Unknown,
                )],
            ),
            1,
            41,
            Some(3),
            130,
        ),
        &mut reductions,
    )
    .unwrap();
    assert_eq!(reductions.len(), 1);

    let mut state = new_state();
    make_ready(&mut state);
    let quote = owned_quote_intent(1, 84, PmOrderSide::Buy);
    let client = quote.client_order();
    let venue = venue_order("rest-atomic");
    state.admit_owned_quote(quote).unwrap();
    state
        .apply_owned_submit_result(client, PmOwnedSubmitResult::Accepted(venue))
        .unwrap();
    let prior_snapshot = state.account_projection().snapshot();
    let prior_watermark = state.fill_watermark();
    let cut = boundary(40, 41);
    let result = state.apply_reconciliation_with_reductions(
        envelope(
            account_snapshot(
                3,
                cut,
                AccountFacts {
                    collateral: Some(1),
                    ..AccountFacts::ready()
                },
            ),
            1,
            41,
            Some(3),
            130,
        ),
        envelope(
            fill_query(
                3,
                cut,
                prior_watermark,
                2,
                vec![
                    fill(
                        "rest-atomic",
                        "overfill-leg-a",
                        PmOrderSide::Buy,
                        "0.75",
                        PmFillFee::Unknown,
                    ),
                    fill(
                        "rest-atomic",
                        "overfill-leg-b",
                        PmOrderSide::Buy,
                        "0.75",
                        PmFillFee::Unknown,
                    ),
                ],
            ),
            1,
            41,
            Some(3),
            130,
        ),
        &mut reductions,
    );
    assert!(matches!(
        result,
        Err(PmPrivateStateError::OwnedLifecycle(
            PmOwnedOrderLifecycleError::Overfill
        ))
    ));
    assert!(reductions.is_empty());
    assert_eq!(state.account_projection().snapshot(), prior_snapshot);
    assert_eq!(state.fill_watermark(), prior_watermark);
    assert_eq!(state.fills().count(), 0);
    assert_eq!(state.owned_fills().count(), 0);
    assert_eq!(
        state.owned_order(client).unwrap().known_fill_total(),
        U256::ZERO
    );

    let mut control = new_state();
    make_ready(&mut control);
    let control_quote = owned_quote_intent(1, 84, PmOrderSide::Buy);
    let control_client = control_quote.client_order();
    control.admit_owned_quote(control_quote).unwrap();
    control
        .apply_owned_submit_result(
            control_client,
            PmOwnedSubmitResult::Accepted(venue_order("rest-atomic")),
        )
        .unwrap();
    let failed_next = state.issue_owned_immediate_ack_ticket().unwrap();
    let control_next = control.issue_owned_immediate_ack_ticket().unwrap();
    assert_eq!(
        failed_next.occurrence().reduction_sequence(),
        control_next.occurrence().reduction_sequence()
    );
}

#[test]
fn private_reduction_results_retain_exact_inputs_and_owned_transition_facts() {
    let mut state = new_state();
    let quote = owned_quote_intent(1, 76, PmOrderSide::Buy);
    let client = quote.client_order();
    let venue = venue_order("owned-reduction");
    state.admit_owned_quote(quote).unwrap();
    state
        .apply_owned_submit_result(client, PmOwnedSubmitResult::Accepted(venue))
        .unwrap();

    let order_event = order(
        76,
        "owned-reduction",
        PmOrderSide::Buy,
        PmOrderStatus::PartiallyFilled,
        250_000,
    );
    let order_input = envelope(order_event, 1, 30, None, 120);
    let order_reduction = state
        .observe_order_reduction(order_input, PmRemoteOrderKnowledge::Ambiguous)
        .unwrap();
    assert_eq!(order_reduction.envelope(), order_input);
    assert_eq!(
        order_reduction.knowledge(),
        PmRemoteOrderKnowledge::Ambiguous
    );
    assert_eq!(
        order_reduction.owned_remote_apply(),
        PmOwnedRemoteOrderApply::Matched(client)
    );
    let owned_order = order_reduction.owned().unwrap();
    assert_eq!(
        owned_order.source(),
        PmOwnedObservationSource::PrivateWebSocket
    );
    assert_eq!(
        owned_order.occurrence().private_occurrence(),
        Some(PmPrivateOccurrence::new(
            ConnectionEpoch::new(1),
            IngressSequence::new(30),
        ))
    );
    assert!(matches!(
        owned_order.apply(),
        PmOwnedProgressApply::Applied { .. }
    ));
    assert_eq!(owned_order.observation().client_order(), client);
    assert_eq!(owned_order.observation().venue_order(), venue);

    let fill_event = fill(
        "owned-reduction",
        "owned-reduction-fill",
        PmOrderSide::Buy,
        "0.25",
        PmFillFee::Unknown,
    );
    let fill_input = envelope(fill_event, 1, 31, None, 121);
    let fill_reduction = state.observe_fill_reduction(fill_input).unwrap();
    assert_eq!(fill_reduction.envelope(), fill_input);
    assert!(matches!(
        fill_reduction.canonical_apply(),
        PmFillApply::PrincipalApplied { .. }
    ));
    assert_eq!(
        fill_reduction.owned_remote_apply(),
        PmOwnedRemoteOrderApply::Matched(client)
    );
    let owned_fill = fill_reduction.owned().unwrap();
    assert_eq!(
        owned_fill.source(),
        PmOwnedObservationSource::PrivateWebSocket
    );
    assert_eq!(
        owned_fill.occurrence().private_occurrence(),
        Some(PmPrivateOccurrence::new(
            ConnectionEpoch::new(1),
            IngressSequence::new(31),
        ))
    );
    assert!(matches!(
        owned_fill.apply(),
        PmOwnedFillApply::Applied { .. }
    ));
    assert_eq!(owned_fill.observation().key(), fill_event.fill_key());
    assert_eq!(
        owned_fill.observation().quantity(),
        fill_event.execution().quantity()
    );
    assert_eq!(owned_fill.observation().reported_cumulative(), None);
}

#[test]
fn typed_fill_recovery_preserves_original_source_and_exact_principal() {
    let mut state = new_state();
    let quote = owned_quote_intent(1, 77, PmOrderSide::Buy);
    let client = quote.client_order();
    let venue = venue_order("owned-recovery");
    state.admit_owned_quote(quote).unwrap();
    state
        .apply_owned_submit_result(client, PmOwnedSubmitResult::Accepted(venue))
        .unwrap();

    let event = fill(
        "owned-recovery",
        "owned-recovery-fill",
        PmOrderSide::Buy,
        "0.25",
        PmFillFee::Unknown,
    );
    let sequence = PmOwnedReductionSequence::new(10).unwrap();
    let occurrence = PmOwnedObservationOccurrence::new(
        sequence,
        Some(PmPrivateOccurrence::new(
            ConnectionEpoch::new(1),
            IngressSequence::new(30),
        )),
        None,
    )
    .unwrap();
    assert!(matches!(
        state
            .recover_owned_fill(PmOwnedRecoveryFill::new(
                event,
                Some(U256::from_u64(250_000)),
                occurrence,
                PmOwnedObservationSource::PrivateWebSocket,
            ))
            .unwrap(),
        PmOwnedFillApply::Applied {
            client_order,
            cumulative_filled,
            ..
        } if client_order == client && cumulative_filled == U256::from_u64(250_000)
    ));
    assert_eq!(state.fill_counters().principal_applications(), 1);
    assert_eq!(state.provisional_deltas().uncovered_fills(), 1);
    let recovered = state.owned_fills().next().unwrap();
    assert_eq!(recovered.first_occurrence(), occurrence);
    assert!(recovered.observed_from(PmOwnedObservationSource::PrivateWebSocket));
    state
        .finish_owned_recovery(PmOwnedReductionSequence::new(11).unwrap())
        .unwrap();
}

#[test]
fn exact_private_progress_cancel_detail_and_compaction_share_one_owned_aggregate() {
    let mut state = new_state();
    let quote = owned_quote_intent(1, 72, PmOrderSide::Buy);
    let client = quote.client_order();
    let venue = venue_order("owned-progress");
    state.admit_owned_quote(quote).unwrap();
    state
        .apply_owned_submit_result(client, PmOwnedSubmitResult::Accepted(venue))
        .unwrap();

    let partial = order(
        72,
        "owned-progress",
        PmOrderSide::Buy,
        PmOrderStatus::PartiallyFilled,
        250_000,
    );
    state
        .observe_order(
            envelope(partial, 1, 30, None, 120),
            PmRemoteOrderKnowledge::Ambiguous,
        )
        .unwrap();
    assert_eq!(
        state.owned_order(client).unwrap().cumulative_filled(),
        U256::from_u64(250_000)
    );

    state
        .apply_open_orders_snapshot(
            envelope(
                open_orders(1, boundary(35, 36), vec![partial]),
                1,
                36,
                Some(1),
                125,
            ),
            &[PmOpenOrderReservation::new(
                venue,
                PmReservationKnowledge::Known(
                    PmExactReservation::policy_approved(U256::from_u64(400_000), U256::ZERO)
                        .unwrap(),
                ),
            )],
        )
        .unwrap();
    assert_eq!(
        state.owned_order(client).unwrap().cumulative_filled(),
        U256::from_u64(250_000)
    );

    let cancel = match state.request_owned_cancel(client).unwrap() {
        PmOwnedCancelRequestApply::Issued(intent) => intent,
        other => panic!("expected cancel intent, got {other:?}"),
    };
    state
        .apply_owned_cancel_result(cancel, PmOwnedCancelOutcome::Accepted)
        .unwrap();
    let detail = PmExactOrderDetail::new(
        source(),
        scope(),
        snapshot(2),
        boundary(40, 41),
        venue,
        None,
    )
    .unwrap();
    state
        .apply_order_detail(
            envelope(detail, 1, 41, Some(2), 130),
            PmReservationKnowledge::Unknown,
        )
        .unwrap();
    assert!(state.owned_order(client).unwrap().reconciliation_required());
    assert!(state.compact_proven_owned_terminal(client).is_err());

    let remaining = fill(
        "owned-progress",
        "owned-fill-final",
        PmOrderSide::Buy,
        "1",
        PmFillFee::Unknown,
    );
    state
        .observe_fill(envelope(remaining, 1, 42, None, 131))
        .unwrap();
    assert_eq!(
        state.owned_order(client).unwrap().status(),
        Some(PmOrderStatus::Filled)
    );
}

#[test]
fn ambiguous_owned_submit_requires_exactly_one_canonical_refresh() {
    let mut state = new_state();
    let quote = owned_quote_intent(1, 70, PmOrderSide::Buy);
    let client = quote.client_order();
    assert_eq!(
        state.admit_owned_quote(quote).unwrap(),
        PmOwnedQuoteAdmission::Admitted(client)
    );
    let requirements_before = state.refresh_counters().requirements();

    assert_eq!(
        state
            .apply_owned_submit_result(client, PmOwnedSubmitResult::Ambiguous)
            .unwrap(),
        PmOwnedSubmitApply::MarkedAmbiguous
    );
    assert_eq!(
        state
            .pending_refresh_keys()
            .filter(|key| key.reason() == PmRefreshReason::AmbiguousOrder)
            .count(),
        1
    );
    assert_eq!(
        state.refresh_counters().requirements(),
        requirements_before + 1
    );

    assert_eq!(
        state
            .apply_owned_submit_result(client, PmOwnedSubmitResult::Ambiguous)
            .unwrap(),
        PmOwnedSubmitApply::Duplicate
    );
    assert_eq!(
        state
            .pending_refresh_keys()
            .filter(|key| key.reason() == PmRefreshReason::AmbiguousOrder)
            .count(),
        1
    );
    assert_eq!(
        state.refresh_counters().requirements(),
        requirements_before + 1
    );
}

#[test]
fn canonical_private_reserved_capacity_is_nonzero_and_stable_across_mutation() {
    let mut state = new_state();
    let reserved = state.reserved_capacity_bytes();
    assert!(reserved > 0);

    let quote = owned_quote_intent(1, 71, PmOrderSide::Buy);
    let client = quote.client_order();
    assert_eq!(
        state.admit_owned_quote(quote).unwrap(),
        PmOwnedQuoteAdmission::Admitted(client)
    );
    assert_eq!(
        state
            .apply_owned_submit_result(client, PmOwnedSubmitResult::Ambiguous)
            .unwrap(),
        PmOwnedSubmitApply::MarkedAmbiguous
    );
    assert_eq!(state.reserved_capacity_bytes(), reserved);
}

#[test]
fn replacement_preflight_leaves_cancel_mutation_to_the_explicit_owner_path() {
    let mut state = new_state();
    let current = owned_quote_intent(1, 71, PmOrderSide::Buy);
    let client = current.client_order();
    let venue = venue_order("replacement-preflight");
    assert_eq!(
        state.admit_owned_quote(current).unwrap(),
        PmOwnedQuoteAdmission::Admitted(client)
    );
    state
        .apply_owned_submit_result(client, PmOwnedSubmitResult::Accepted(venue))
        .unwrap();

    let replacement = PmOwnedQuoteIntent::new(
        PmOwnedIntentId::new(2).unwrap(),
        PmOwnedQuoteSlotKey::new(scope(), instrument(), PmOrderSide::Buy),
        client_order(72),
        PmPrice::parse_decimal("0.41").unwrap(),
        PmQuantity::parse_decimal("1").unwrap(),
        PmExactReservation::policy_approved(U256::from_u64(410_000), U256::ZERO).unwrap(),
    )
    .unwrap();
    let projected_cancel = match state.preflight_owned_quote(replacement).unwrap() {
        PmOwnedQuoteAdmission::CancelBeforeReplace(cancel) => cancel,
        other => panic!("expected cancel-before-replace preflight, got {other:?}"),
    };
    assert_eq!(projected_cancel.client_order(), client);
    assert_eq!(projected_cancel.venue_order(), venue);
    assert_eq!(
        state.owned_order(client).unwrap().cancel(),
        PmOwnedCancelState::None
    );

    let issued_cancel = match state.request_owned_cancel(client).unwrap() {
        PmOwnedCancelRequestApply::Issued(cancel) => cancel,
        other => panic!("expected explicit cancel mutation, got {other:?}"),
    };
    assert_eq!(issued_cancel, projected_cancel);
    assert_eq!(
        state.owned_order(client).unwrap().cancel(),
        PmOwnedCancelState::Pending
    );
}

#[test]
fn accepted_zero_fill_cancel_can_be_settled_by_exact_detail_and_compacted() {
    let mut state = new_state();
    let quote = owned_quote_intent(1, 73, PmOrderSide::Buy);
    let client = quote.client_order();
    let venue = venue_order("owned-cancelled");
    state.admit_owned_quote(quote).unwrap();
    state
        .apply_owned_submit_result(client, PmOwnedSubmitResult::Accepted(venue))
        .unwrap();
    let cancel = match state.request_owned_cancel(client).unwrap() {
        PmOwnedCancelRequestApply::Issued(intent) => intent,
        other => panic!("expected cancel intent, got {other:?}"),
    };
    state
        .apply_owned_cancel_result(cancel, PmOwnedCancelOutcome::Accepted)
        .unwrap();
    let detail = PmExactOrderDetail::new(
        source(),
        scope(),
        snapshot(1),
        boundary(40, 41),
        venue,
        None,
    )
    .unwrap();
    state
        .apply_order_detail(
            envelope(detail, 1, 41, Some(1), 130),
            PmReservationKnowledge::Unknown,
        )
        .unwrap();
    assert!(!state.owned_order(client).unwrap().reconciliation_required());
    state.compact_proven_owned_terminal(client).unwrap();
    assert_eq!(state.owned_orders().count(), 0);
    assert_eq!(state.orders().count(), 0);
}

#[test]
fn private_aggregate_compaction_reuses_canonical_registration_for_ten_thousand_cycles() {
    let mut state = new_state();
    let price = PmPrice::parse_decimal("0.40").unwrap();
    let quantity = PmQuantity::parse_decimal("1").unwrap();
    let reservation =
        PmExactReservation::policy_approved(U256::from_u64(400_000), U256::ZERO).unwrap();
    for id in 1_u64..=10_000 {
        let mut bytes = [0_u8; 16];
        bytes[8..].copy_from_slice(&id.to_be_bytes());
        let client = PmClientOrderKey::new(account(), PmClientOrderId::from_bytes(bytes).unwrap());
        let quote = PmOwnedQuoteIntent::new(
            PmOwnedIntentId::new(id).unwrap(),
            PmOwnedQuoteSlotKey::new(scope(), instrument(), PmOrderSide::Buy),
            client,
            price,
            quantity,
            reservation,
        )
        .unwrap();
        state.admit_owned_quote(quote).unwrap();
        state
            .apply_owned_submit_result(client, PmOwnedSubmitResult::Rejected)
            .unwrap();
        state.compact_proven_owned_terminal(client).unwrap();
    }
    assert_eq!(state.owned_orders().count(), 0);
    assert_eq!(state.orders().count(), 0);
    assert_eq!(
        state.owned_lifecycle_counters().terminal_compactions(),
        10_000
    );
    assert_eq!(state.order_counters().capacity_failures(), 0);
}

#[test]
fn reconnect_marks_owned_order_unknown_until_exact_new_epoch_progress() {
    let mut state = new_state();
    let quote = owned_quote_intent(1, 74, PmOrderSide::Buy);
    let client = quote.client_order();
    let venue = venue_order("owned-reconnect");
    state.admit_owned_quote(quote).unwrap();
    state
        .apply_owned_submit_result(client, PmOwnedSubmitResult::Accepted(venue))
        .unwrap();
    state
        .observe_reconnect(ConnectionEpoch::new(2), 200)
        .unwrap();
    assert!(state.owned_order(client).unwrap().reconciliation_required());

    let open = order(
        74,
        "owned-reconnect",
        PmOrderSide::Buy,
        PmOrderStatus::Open,
        0,
    );
    assert!(matches!(
        state.observe_order(
            envelope(open, 1, 30, None, 201),
            PmRemoteOrderKnowledge::Ambiguous,
        ),
        Err(PmPrivateStateError::OldConnectionEpoch)
    ));
    assert!(state.owned_order(client).unwrap().reconciliation_required());

    state
        .observe_order(
            envelope(open, 2, 1, None, 202),
            PmRemoteOrderKnowledge::Ambiguous,
        )
        .unwrap();
    assert!(!state.owned_order(client).unwrap().reconciliation_required());
}
