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
    PmFillApply, PmFillFeeState, PmFreshnessRiskLimits, PmOpenOrdersApply, PmOrderApply,
    PmOrderOwnership, PmOrderRiskLimits, PmOwnedOrderRegistration, PmPositionKnowledge,
    PmPrivateConvergence, PmPrivateDependency, PmPrivateExternalIngressFailure,
    PmPrivateExternalIngressFault, PmPrivateExternalIngressLane, PmPrivateHaltReason,
    PmPrivateQuoteRequest, PmPrivateReadiness, PmPrivateReadinessReason, PmPrivateState,
    PmPrivateStateConfig, PmPrivateStateError, PmRefreshAdmission, PmRefreshReason,
    PmRefreshRequired, PmRemoteOrderKnowledge, PmReservationKnowledge, PmRiskDecision,
    PmRiskDependency, PmRiskLimits, PmRiskReason, PmUnresolvedFillApply,
    PmUnresolvedFillObservation, PmUnresolvedFillReason,
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
        projection.last_occurrence().ingress(),
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
