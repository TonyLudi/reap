//! Focused mutation-owner lifecycle and durability-boundary tests.

use std::path::{Path, PathBuf};
use std::time::Duration;

use reap_pm_core::{
    ConnectionEpoch, EventClock, EventEnvelope, EventOrdering, EvmAddress, IngressSequence,
    MAX_REQUIRED_SPENDERS, OkxInstrumentId, OkxReferenceHandle, OkxReferenceInstrument,
    PmAccountHandle, PmAccountScope, PmAllowanceEvent, PmAllowanceValue, PmAssetId, PmBalanceEvent,
    PmChainId, PmCompleteAccountSnapshot, PmCompleteFillQuery, PmCompleteOpenOrdersSnapshot,
    PmConditionId, PmConnectionId, PmEnvironmentId, PmErc1155OperatorApproval, PmFillEvent,
    PmFillExecution, PmFillFee, PmFillId, PmFillKey, PmFillQueryCursor, PmFillRole,
    PmFillSettlementStatus, PmFunderId, PmInstrumentHandle, PmMarketHandle, PmMarketId,
    PmMarketLifecycle, PmMarketMetadata, PmOrderIdentity, PmOrderSalt, PmOrderSide, PmOrderStatus,
    PmOutcomeLabel, PmOutcomeMetadata, PmPositionAvailability, PmPositionEvent, PmPrice,
    PmProductSource, PmQuantity, PmReconciliationRequestBoundary, PmReferenceMapping, PmSignerId,
    PmSnapshotEvidence, PmSourceBound, PmSourceHandle, PmSpenderDomain, PmSpenderRequirement,
    PmTick, PmTokenHandle, PmTokenId, PmVenueOrderId, PmVenueOrderKey, SnapshotRevision, U256,
};
use reap_pm_live_contracts::{
    PmAccountConnectivityConfig, PmConnectionRoute, PmConnectivityConfig,
    PmPublicConnectivityConfig,
};
use reap_pm_state::{
    PmCardinalityRiskLimits, PmExactReservation, PmExposureRiskLimits, PmFreshnessRiskLimits,
    PmOpenOrdersApply, PmOrderRiskLimits, PmOwnedCancelState, PmOwnedSubmitState,
    PmReconciliationApply, PmRiskLimits,
};
use reap_pm_strategy::{PmQuotePolicyInput, validate_passive_quote_candidate};
use reap_polymarket_adapter::{
    PmFakeCancelRejectReason, PmFakeCancelScript, PmFakeExecutionError, PmFakeImmediateFill,
    PmFakePlaceRejectReason, PmFakePlaceResult, PmFakePlaceScript, PmFixtureInstrumentScope,
    PmFixtureOwnedExecution, PmPrivateLifecycleObservation,
};
use tempfile::TempDir;

use super::{
    PmCancelMutationAdmission, PmCancelMutationRequest, PmDurableConsequence, PmMutationError,
    PmMutationHalt, PmMutationOwner, PmPersistenceService, PmQuoteMutationAdmission,
    PmQuoteMutationRequest,
};
use crate::coordinator::PmAuthorityRevisions;
use crate::coordinator::effect_queue::{PM_FAKE_EFFECT_CAPACITY, PmFakeEffectQueueError};
use crate::coordinator::effects::PmDurableRecordKind;
use crate::coordinator::persistence::{
    PM_PENDING_PERSISTENCE_CAPACITY, PM_PENDING_PERSISTENCE_MAX_AGE_NS, PmPendingPersistence,
    PmPersistenceError, PmPersistenceIntentIdentity,
};
use crate::fake_effect::PmFakeEffectRole;
use crate::journal::{
    PmJournalCancelReasonV1, PmJournalFillCursorV1, PmJournalFillWatermarkV1,
    PmJournalFingerprintV1, PmJournalRecordV1, PmJournalRecoveredPlaceV1, PmJournalSafetyHaltV1,
    PmJournalSafetyReasonV1, PmJournalScopeV1, PmPendingJournalRecord, recover_pm_mutation_journal,
};
use crate::private_monitor::PmPrivateMonitorRuntime;

const GOAL_F_PUSD: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
const GOAL_F_CTF: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
const GOAL_F_STANDARD_EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";
const INITIAL_CURSOR_BYTE: u8 = 1;
const QUOTE_APPROVED_NS: u64 = 120;
const QUOTE_EXPIRES_NS: u64 = 1_000;
const JOURNAL_HEADER_RECORDS: usize = 1;
const READY_BASELINE_FACT_RECORDS: u64 = 1;
const READY_BASELINE_JOURNAL_RECORDS: usize = 1;

fn fixture() -> PmConnectivityConfig {
    let instrument = PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(0),
        PmTokenHandle::from_ordinal(0),
    );
    let reference = OkxReferenceHandle::from_ordinal(0);
    let eoa = EvmAddress::from_bytes([4; 20]).unwrap();
    let account_scope = PmAccountScope::new(
        PmEnvironmentId::new("mutation-owner-test").unwrap(),
        PmChainId::new(137).unwrap(),
        PmSignerId::new(eoa),
        PmFunderId::new(eoa),
        PmAccountHandle::from_ordinal(4),
    );
    let exchange = EvmAddress::parse(GOAL_F_STANDARD_EXCHANGE).unwrap();
    let token = PmTokenId::new(U256::from_u64(11)).unwrap();
    let collateral_requirement = PmSpenderRequirement::new(
        PmChainId::new(137).unwrap(),
        exchange,
        PmSpenderDomain::Standard,
        PmAssetId::collateral(EvmAddress::parse(GOAL_F_PUSD).unwrap()),
    );
    let outcome_requirement = PmSpenderRequirement::new(
        PmChainId::new(137).unwrap(),
        exchange,
        PmSpenderDomain::Standard,
        PmAssetId::outcome(EvmAddress::parse(GOAL_F_CTF).unwrap(), token),
    );
    let mut references = [None; 16];
    references[0] = Some(reference);
    let mapping = PmReferenceMapping::new(instrument, references, 1).unwrap();
    let mut required_spenders = [None; MAX_REQUIRED_SPENDERS];
    required_spenders[0] = Some(collateral_requirement);
    required_spenders[1] = Some(outcome_requirement);
    let expected_metadata = PmMarketMetadata::new(
        PmConditionId::parse("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .unwrap(),
        PmMarketId::parse("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
            .unwrap(),
        PmOutcomeMetadata::new(token, PmOutcomeLabel::new("Yes").unwrap()),
        PmMarketLifecycle::new(true, false, false, true, true),
        PmTick::parse_decimal("0.01").unwrap(),
        PmQuantity::parse_decimal("1").unwrap(),
        false,
        PmChainId::new(137).unwrap(),
        exchange,
        required_spenders,
        2,
    )
    .unwrap();
    let public = PmPublicConnectivityConfig::new(
        mapping,
        OkxReferenceInstrument::index(OkxInstrumentId::new("BTC-USDT").unwrap()),
        expected_metadata,
        PmConnectionRoute::new(
            PmProductSource::okx_reference(PmSourceHandle::from_ordinal(1), reference),
            PmConnectionId::new("okx-public").unwrap(),
        ),
        PmConnectionRoute::new(
            PmProductSource::polymarket_market(PmSourceHandle::from_ordinal(2), instrument.token()),
            PmConnectionId::new("pm-public").unwrap(),
        ),
    )
    .unwrap();
    let account = PmAccountConnectivityConfig::derive_goal_f(
        &public,
        account_scope,
        PmConnectionRoute::new(
            PmProductSource::polymarket_account(
                PmSourceHandle::from_ordinal(3),
                account_scope.handle(),
            ),
            PmConnectionId::new("pm-account").unwrap(),
        ),
    )
    .unwrap();
    PmConnectivityConfig::new(public, account).unwrap()
}

fn risk_limits() -> PmRiskLimits {
    PmRiskLimits::new(
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
        PmFreshnessRiskLimits::new(
            1_000_000_000,
            1_000_000_000,
            1_000_000_000,
            1_000_000_000,
            1_000_000_000,
            1_000_000_000,
        )
        .unwrap(),
    )
}

fn authority_revisions(value: u64) -> PmAuthorityRevisions {
    PmAuthorityRevisions::new(
        SnapshotRevision::new(value),
        SnapshotRevision::new(value),
        value,
        value,
        value,
    )
    .unwrap()
}

async fn start_owner(config: &PmConnectivityConfig, journal_path: &Path) -> PmMutationOwner {
    let account = config.account();
    let private = PmPrivateMonitorRuntime::new(account, risk_limits()).unwrap();
    let fake = PmFakeEffectRole::new(
        account.account_scope(),
        account.instrument(),
        account.instrument_id(),
    );
    let (owner, recovery) =
        PmMutationOwner::start(config, Box::new(private), fake, journal_path.to_path_buf())
            .await
            .unwrap();
    assert_eq!(recovery.record_count(), 0);
    owner
}

async fn ready_owner() -> (PmConnectivityConfig, TempDir, PathBuf, PmMutationOwner) {
    let config = fixture();
    let directory = tempfile::tempdir().unwrap();
    let journal_path = directory.path().join("pm-mutation.ndjson");
    let mut owner = start_owner(&config, &journal_path).await;
    make_private_ready(&mut owner, &config);
    assert_eq!(owner.counters().fact_records(), READY_BASELINE_FACT_RECORDS);
    drain_persistence(&mut owner, 111).await;
    assert_eq!(
        owner.pop_durable_consequence().unwrap().kind(),
        PmDurableRecordKind::FillWatermarkAdvanced
    );
    assert!(owner.pop_durable_consequence().is_none());
    (config, directory, journal_path, owner)
}

async fn restart_owner(
    config: &PmConnectivityConfig,
    journal_path: &Path,
) -> (PmMutationOwner, crate::journal::PmJournalRecovery) {
    let account = config.account();
    let private = PmPrivateMonitorRuntime::new(account, risk_limits()).unwrap();
    let fake = PmFakeEffectRole::new(
        account.account_scope(),
        account.instrument(),
        account.instrument_id(),
    );
    PmMutationOwner::start(config, Box::new(private), fake, journal_path.to_path_buf())
        .await
        .unwrap()
}

fn make_private_ready(owner: &mut PmMutationOwner, config: &PmConnectivityConfig) {
    let account = config.account();
    let route = account.account_route();
    owner
        .private_mut()
        .prepare_product_private_reconnect(ConnectionEpoch::new(1))
        .unwrap();
    owner
        .private_mut()
        .reduce_serviced_connection_available(
            route.source(),
            route.connection(),
            ConnectionEpoch::new(1),
            1,
        )
        .unwrap();

    let orders_cut = boundary(10, 11);
    let open_orders = PmCompleteOpenOrdersSnapshot::new(
        route.source(),
        account.account_scope(),
        snapshot(1),
        orders_cut,
        Box::new([]),
    )
    .unwrap();
    assert!(matches!(
        owner
            .private_mut()
            .reduce_serviced_open_orders(
                route.source(),
                route.connection(),
                clock(100),
                ordering(1, 11, Some(1)),
                open_orders,
            )
            .unwrap(),
        PmOpenOrdersApply::Applied { .. }
    ));

    let (account_envelope, fill_envelope) =
        reconciliation_pair(config, 2, 20, 21, None, INITIAL_CURSOR_BYTE, 110);
    assert!(matches!(
        owner
            .reduce_serviced_reconciliation(account_envelope, fill_envelope)
            .unwrap(),
        PmReconciliationApply::Applied { .. }
    ));
    owner.update_revisions(authority_revisions(1));
}

fn prime_restarted_private(owner: &mut PmMutationOwner, config: &PmConnectivityConfig) {
    let account = config.account();
    let route = account.account_route();
    owner
        .private_mut()
        .prepare_product_private_reconnect(ConnectionEpoch::new(1))
        .unwrap();
    owner
        .private_mut()
        .reduce_serviced_connection_available(
            route.source(),
            route.connection(),
            ConnectionEpoch::new(1),
            200,
        )
        .unwrap();
    let open_orders = PmCompleteOpenOrdersSnapshot::new(
        route.source(),
        account.account_scope(),
        snapshot(1),
        boundary(210, 211),
        Box::new([]),
    )
    .unwrap();
    assert!(matches!(
        owner
            .private_mut()
            .reduce_serviced_open_orders(
                route.source(),
                route.connection(),
                clock(201),
                ordering(1, 211, Some(1)),
                open_orders,
            )
            .unwrap(),
        PmOpenOrdersApply::Applied { .. }
    ));
}

fn snapshot(revision: u64) -> PmSnapshotEvidence {
    PmSnapshotEvidence::new(SnapshotRevision::new(revision)).unwrap()
}

fn fill_watermark_record(config: &PmConnectivityConfig, cursor_byte: u8) -> PmJournalRecordV1 {
    PmJournalRecordV1::FillWatermarkAdvanced(PmJournalFillWatermarkV1 {
        cursor: PmJournalFillCursorV1 {
            account_scope: config.account().account_scope(),
            opaque: PmJournalFingerprintV1::from_bytes([cursor_byte; 32]),
        },
    })
}

fn boundary(request: u64, completion: u64) -> PmReconciliationRequestBoundary {
    PmReconciliationRequestBoundary::new(
        IngressSequence::new(request),
        IngressSequence::new(completion),
    )
    .unwrap()
}

fn clock(monotonic_ns: u64) -> EventClock {
    EventClock::new(None, 1_000 + monotonic_ns, monotonic_ns, monotonic_ns).unwrap()
}

fn ordering(epoch: u64, ingress: u64, revision: Option<u64>) -> EventOrdering {
    EventOrdering::new(
        ConnectionEpoch::new(epoch),
        revision.map(SnapshotRevision::new),
        None,
        None,
        IngressSequence::new(ingress),
    )
    .unwrap()
}

fn envelope<P: PmSourceBound>(
    config: &PmConnectivityConfig,
    payload: P,
    epoch: u64,
    ingress: u64,
    revision: Option<u64>,
    monotonic_ns: u64,
) -> EventEnvelope<P> {
    let source = config.account().account_route().source();
    EventEnvelope::new(
        source.venue(),
        source,
        config.account().account_route().connection(),
        clock(monotonic_ns),
        ordering(epoch, ingress, revision),
        payload,
    )
    .unwrap()
}

fn reconciliation_pair(
    config: &PmConnectivityConfig,
    revision: u64,
    request: u64,
    completion: u64,
    requested_after: Option<u8>,
    resulting: u8,
    monotonic_ns: u64,
) -> (
    EventEnvelope<PmCompleteAccountSnapshot>,
    EventEnvelope<PmCompleteFillQuery>,
) {
    reconciliation_pair_with_fills(
        config,
        revision,
        request,
        completion,
        requested_after,
        resulting,
        Vec::new(),
        monotonic_ns,
    )
}

#[allow(clippy::too_many_arguments)]
fn reconciliation_pair_with_fills(
    config: &PmConnectivityConfig,
    revision: u64,
    request: u64,
    completion: u64,
    requested_after: Option<u8>,
    resulting: u8,
    fills: Vec<PmFillEvent>,
    monotonic_ns: u64,
) -> (
    EventEnvelope<PmCompleteAccountSnapshot>,
    EventEnvelope<PmCompleteFillQuery>,
) {
    let cut = boundary(request, completion);
    let account = account_snapshot(config, revision, cut);
    let scope = config.account().account_scope();
    let fill_query = PmCompleteFillQuery::new(
        config.account().account_route().source(),
        scope,
        snapshot(revision),
        cut,
        requested_after.map(|byte| PmFillQueryCursor::new(scope, [byte; 32])),
        PmFillQueryCursor::new(scope, [resulting; 32]),
        fills.into_boxed_slice(),
    )
    .unwrap();
    (
        envelope(config, account, 1, completion, Some(revision), monotonic_ns),
        envelope(
            config,
            fill_query,
            1,
            completion,
            Some(revision),
            monotonic_ns,
        ),
    )
}

fn account_snapshot(
    config: &PmConnectivityConfig,
    revision: u64,
    cut: PmReconciliationRequestBoundary,
) -> PmCompleteAccountSnapshot {
    let account = config.account();
    let source = account.account_route().source();
    let scope = account.account_scope();
    let domain = account.trading_domain();
    let evidence = snapshot(revision);
    let balances = vec![
        PmBalanceEvent::new(
            source,
            account.account(),
            domain.collateral(),
            U256::from_u64(10_000_000),
            evidence,
        )
        .unwrap(),
        PmBalanceEvent::new(
            source,
            account.account(),
            domain.outcome(),
            U256::from_u64(5_000_000),
            evidence,
        )
        .unwrap(),
    ];
    let allowances = account
        .required_spenders()
        .iter()
        .copied()
        .map(|spender| {
            let value = if spender.requirement().asset() == domain.collateral() {
                PmAllowanceValue::Erc20(U256::from_u64(10_000_000))
            } else {
                PmAllowanceValue::Erc1155Operator(PmErc1155OperatorApproval::from_bool(true))
            };
            PmAllowanceEvent::new(source, spender, value, evidence).unwrap()
        })
        .collect::<Vec<_>>();
    let positions = vec![
        PmPositionEvent::new(
            source,
            account.account(),
            account.instrument(),
            U256::from_u64(5_000_000),
            PmPositionAvailability::Tradable,
            evidence,
        )
        .unwrap(),
    ];
    PmCompleteAccountSnapshot::new(
        source,
        scope,
        evidence,
        cut,
        vec![domain.collateral(), domain.outcome()].into_boxed_slice(),
        account.required_spenders().to_vec().into_boxed_slice(),
        vec![account.instrument()].into_boxed_slice(),
        balances.into_boxed_slice(),
        allowances.into_boxed_slice(),
        positions.into_boxed_slice(),
    )
    .unwrap()
}

fn venue_order(config: &PmConnectivityConfig, id: &str) -> PmVenueOrderKey {
    PmVenueOrderKey::new(config.account().account(), PmVenueOrderId::new(id).unwrap())
}

fn quote_request(
    config: &PmConnectivityConfig,
    side: PmOrderSide,
    salt: u64,
    expires_at_monotonic_ns: u64,
) -> PmQuoteMutationRequest {
    quote_request_at_probability(config, side, salt, expires_at_monotonic_ns, 0.40)
}

fn quote_request_at_probability(
    config: &PmConnectivityConfig,
    side: PmOrderSide,
    salt: u64,
    expires_at_monotonic_ns: u64,
    fair_probability: f64,
) -> PmQuoteMutationRequest {
    let metadata = config.account().expected_metadata();
    let candidate = validate_passive_quote_candidate(PmQuotePolicyInput::new(
        config.account().instrument(),
        metadata,
        side,
        fair_probability,
        PmQuantity::parse_decimal("1").unwrap(),
        Some(PmPrice::parse_decimal("0.30").unwrap()),
        Some(PmPrice::parse_decimal("0.50").unwrap()),
    ))
    .unwrap();
    let reservation = match side {
        PmOrderSide::Buy => {
            PmExactReservation::policy_approved(candidate.maker_amount(), U256::ZERO).unwrap()
        }
        PmOrderSide::Sell => {
            PmExactReservation::policy_approved(U256::ZERO, U256::from_u64(1_000_000)).unwrap()
        }
    };
    PmQuoteMutationRequest::new(
        candidate,
        reservation,
        PmOrderSalt::from_u64(salt).unwrap(),
        1,
        QUOTE_APPROVED_NS,
        expires_at_monotonic_ns,
        110,
        110,
    )
}

fn cancel_request(client_order: reap_pm_core::PmClientOrderKey) -> PmCancelMutationRequest {
    PmCancelMutationRequest::new(
        client_order,
        PmJournalCancelReasonV1::SafetyHalt,
        PmOrderSalt::from_u64(900).unwrap(),
        2,
        130,
        2_000,
    )
}

fn full_fill(
    config: &PmConnectivityConfig,
    client_order: reap_pm_core::PmClientOrderKey,
    venue_order: PmVenueOrderKey,
    id: &str,
) -> PmFillEvent {
    owned_fill(config, client_order, venue_order, id, "1")
}

fn owned_fill(
    config: &PmConnectivityConfig,
    client_order: reap_pm_core::PmClientOrderKey,
    venue_order: PmVenueOrderKey,
    id: &str,
    quantity: &str,
) -> PmFillEvent {
    PmFillEvent::new(
        config.account().account_route().source(),
        config.account().instrument(),
        PmFillKey::new(venue_order, PmFillId::new(id).unwrap()),
        PmOrderIdentity::new(Some(client_order), Some(venue_order)).unwrap(),
        PmFillExecution::new(
            PmOrderSide::Buy,
            PmFillRole::Maker,
            PmFillSettlementStatus::Matched,
            PmPrice::parse_decimal("0.40").unwrap(),
            PmQuantity::parse_decimal(quantity).unwrap(),
            PmFillFee::Unknown,
        ),
    )
    .unwrap()
}

fn reduce_ws_fill(
    owner: &mut PmMutationOwner,
    config: &PmConnectivityConfig,
    fill: PmFillEvent,
    ingress: u64,
    monotonic_ns: u64,
) {
    let route = config.account().account_route();
    owner
        .reduce_serviced_private_observation(
            route.source(),
            route.connection(),
            clock(monotonic_ns),
            ordering(1, ingress, None),
            PmPrivateLifecycleObservation::Fill(fill),
        )
        .unwrap();
}

fn admit_quote(
    owner: &mut PmMutationOwner,
    config: &PmConnectivityConfig,
    side: PmOrderSide,
    salt: u64,
) -> reap_pm_core::PmClientOrderKey {
    match owner
        .begin_quote(quote_request(config, side, salt, QUOTE_EXPIRES_NS))
        .unwrap()
    {
        PmQuoteMutationAdmission::JournalPending { client_order, .. } => client_order,
        other => panic!("expected pending quote intent, got {other:?}"),
    }
}

async fn wait_for_prepared_quotes(owner: &mut PmMutationOwner, expected: usize, monotonic_ns: u64) {
    let _identities = collect_prepared_quotes(owner, expected, monotonic_ns).await;
}

async fn collect_prepared_quotes(
    owner: &mut PmMutationOwner,
    expected: usize,
    monotonic_ns: u64,
) -> Vec<PmPersistenceIntentIdentity> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut prepared = Vec::with_capacity(expected);
    while prepared.len() < expected {
        match owner.service_persistence(monotonic_ns).unwrap() {
            PmPersistenceService::PreparedQuote { identity } => prepared.push(identity),
            PmPersistenceService::Pending
            | PmPersistenceService::Empty
            | PmPersistenceService::FactAcknowledged { .. } => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "quote intent was not durably acknowledged"
                );
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            other => panic!("unexpected quote persistence outcome: {other:?}"),
        }
    }
    prepared
}

async fn wait_for_quote_invalidation(
    owner: &mut PmMutationOwner,
    monotonic_ns: u64,
) -> PmPersistenceIntentIdentity {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        match owner.service_persistence(monotonic_ns).unwrap() {
            PmPersistenceService::QuoteInvalidated { identity } => return identity,
            PmPersistenceService::Pending
            | PmPersistenceService::Empty
            | PmPersistenceService::FactAcknowledged { .. } => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "quote invalidation did not observe durable intent acknowledgement"
                );
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            other => panic!("unexpected quote invalidation persistence outcome: {other:?}"),
        }
    }
}

async fn wait_for_prepared_cancel(owner: &mut PmMutationOwner, monotonic_ns: u64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        match owner.service_persistence(monotonic_ns).unwrap() {
            PmPersistenceService::PreparedCancel { .. } => return,
            PmPersistenceService::Pending
            | PmPersistenceService::Empty
            | PmPersistenceService::FactAcknowledged { .. } => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "cancel intent was not durably acknowledged"
                );
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            other => panic!("unexpected cancel persistence outcome: {other:?}"),
        }
    }
}

async fn drain_persistence(owner: &mut PmMutationOwner, monotonic_ns: u64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while owner.pending_persistence() != 0 {
        match owner.service_persistence(monotonic_ns).unwrap() {
            PmPersistenceService::Pending | PmPersistenceService::Empty => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "journal facts were not durably acknowledged"
                );
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            PmPersistenceService::FactAcknowledged { .. } => {}
            other => panic!("unexpected persistence outcome while draining facts: {other:?}"),
        }
    }
}

async fn place_resting_quote(
    owner: &mut PmMutationOwner,
    config: &PmConnectivityConfig,
    side: PmOrderSide,
    salt: u64,
    venue: PmVenueOrderKey,
) -> reap_pm_core::PmClientOrderKey {
    let client_order = admit_quote(owner, config, side, salt);
    wait_for_prepared_quotes(owner, 1, 121).await;
    owner
        .execute_next_quote(
            PmFakePlaceScript::acknowledged(venue, Box::new([])).unwrap(),
            122,
        )
        .unwrap();
    client_order
}

fn late_ack_result(
    config: &PmConnectivityConfig,
    client_order: reap_pm_core::PmClientOrderKey,
    venue_order: PmVenueOrderKey,
) -> Result<PmFakePlaceResult, PmFakeExecutionError> {
    let account = config.account();
    let execution = PmFixtureOwnedExecution::new(account.account_scope(), account.instrument());
    let scope =
        PmFixtureInstrumentScope::from_metadata(account.instrument(), account.expected_metadata())
            .expect("fixture metadata matches its configured instrument");
    let command = execution.place_command(
        scope,
        client_order,
        PmOrderSalt::from_u64(777).unwrap(),
        PmOrderSide::Buy,
        PmPrice::parse_decimal("0.40").unwrap(),
        PmQuantity::parse_decimal("1").unwrap(),
        3,
    )?;
    execution.execute_place(
        command,
        PmFakePlaceScript::acknowledged(venue_order, Box::new([]))?,
    )
}

#[tokio::test(flavor = "current_thread")]
async fn quote_intent_is_durable_before_first_fake_dispatch() {
    let (config, _directory, journal_path, mut owner) = ready_owner().await;
    let client = admit_quote(&mut owner, &config, PmOrderSide::Buy, 1);

    assert_eq!(owner.pending_persistence(), 1);
    assert_eq!(owner.pending_effects(), 0);
    assert_eq!(
        owner.private_mut().owned_order(client).unwrap().submit(),
        PmOwnedSubmitState::Pending
    );
    assert!(matches!(
        owner.execute_next_quote(
            PmFakePlaceScript::rejected(PmFakePlaceRejectReason::FixtureRejected),
            121,
        ),
        Err(PmMutationError::EffectKindMismatch)
    ));
    assert_eq!(owner.counters().place_results(), 0);

    wait_for_prepared_quotes(&mut owner, 1, 121).await;
    assert_eq!(owner.pending_persistence(), 0);
    assert_eq!(owner.pending_effects(), 1);
    let venue = venue_order(&config, "storage-before-dispatch");
    owner
        .execute_next_quote(
            PmFakePlaceScript::acknowledged(venue, Box::new([])).unwrap(),
            122,
        )
        .unwrap();
    let order = owner.private_mut().owned_order(client).unwrap();
    assert_eq!(order.submit(), PmOwnedSubmitState::Accepted);
    assert_eq!(order.venue_order(), Some(venue));
    assert_eq!(order.status(), Some(PmOrderStatus::Open));

    drain_persistence(&mut owner, 123).await;
    owner.shutdown().await.unwrap();
    let scope = PmJournalScopeV1::from_config(&config).unwrap();
    let recovery = recover_pm_mutation_journal(journal_path, &scope).unwrap();
    assert_eq!(
        recovery.record_count(),
        JOURNAL_HEADER_RECORDS + READY_BASELINE_JOURNAL_RECORDS + 2
    );
    let recovered = recovery.recovered_orders().next().unwrap();
    assert_eq!(recovered.place(), PmJournalRecoveredPlaceV1::Bound);
    assert_eq!(recovered.venue_order(), Some(venue));
}

#[tokio::test(flavor = "current_thread")]
async fn immediate_full_fill_reduces_exact_principal_and_two_durable_facts() {
    let (config, _directory, journal_path, mut owner) = ready_owner().await;
    let client = admit_quote(&mut owner, &config, PmOrderSide::Buy, 1);
    wait_for_prepared_quotes(&mut owner, 1, 121).await;
    let venue = venue_order(&config, "immediate-full");
    let immediate = PmFakeImmediateFill::new(
        PmFillId::new("immediate-full-fill").unwrap(),
        PmPrice::parse_decimal("0.40").unwrap(),
        PmQuantity::parse_decimal("1").unwrap(),
        PmFillFee::Unknown,
    );
    owner
        .execute_next_quote(
            PmFakePlaceScript::acknowledged(venue, vec![immediate].into_boxed_slice()).unwrap(),
            122,
        )
        .unwrap();

    let order = owner.private_mut().owned_order(client).unwrap();
    let full = PmQuantity::parse_decimal("1").unwrap().protocol_units();
    assert_eq!(order.submit(), PmOwnedSubmitState::Accepted);
    assert_eq!(order.status(), Some(PmOrderStatus::Filled));
    assert_eq!(order.cumulative_filled(), full);
    assert_eq!(order.known_fill_total(), full);
    assert_eq!(order.remaining(), U256::ZERO);
    assert_eq!(owner.counters().place_results(), 1);
    assert_eq!(owner.counters().unique_fills(), 1);
    assert_eq!(
        owner.counters().fact_records(),
        READY_BASELINE_FACT_RECORDS + 2
    );
    assert_eq!(owner.pending_persistence(), 2);
    assert_eq!(
        owner.pop_durable_consequence().unwrap().kind(),
        PmDurableRecordKind::PlaceResult
    );
    assert_eq!(
        owner.pop_durable_consequence().unwrap().kind(),
        PmDurableRecordKind::FillApplied
    );

    drain_persistence(&mut owner, 123).await;
    owner.shutdown().await.unwrap();
    let scope = PmJournalScopeV1::from_config(&config).unwrap();
    let recovery = recover_pm_mutation_journal(journal_path, &scope).unwrap();
    assert_eq!(
        recovery.record_count(),
        JOURNAL_HEADER_RECORDS + READY_BASELINE_JOURNAL_RECORDS + 3
    );
    assert_eq!(recovery.fill_key_count(), 1);
    let recovered = recovery.recovered_orders().next().unwrap();
    assert_eq!(recovered.place(), PmJournalRecoveredPlaceV1::Bound);
    assert_eq!(recovered.known_fill_total(), full);
    assert_eq!(recovered.effective_cumulative(), full);
}

#[tokio::test(flavor = "current_thread")]
async fn fake_place_rejection_is_terminal_and_durable_without_venue_binding() {
    let (config, _directory, journal_path, mut owner) = ready_owner().await;
    let client = admit_quote(&mut owner, &config, PmOrderSide::Buy, 1);
    wait_for_prepared_quotes(&mut owner, 1, 121).await;
    owner
        .execute_next_quote(
            PmFakePlaceScript::rejected(PmFakePlaceRejectReason::PostOnlyWouldTake),
            122,
        )
        .unwrap();

    let order = owner.private_mut().owned_order(client).unwrap();
    assert_eq!(order.submit(), PmOwnedSubmitState::Rejected);
    assert_eq!(order.status(), Some(PmOrderStatus::Rejected));
    assert_eq!(order.venue_order(), None);
    assert_eq!(owner.counters().place_results(), 1);
    assert_eq!(
        owner.pop_durable_consequence().unwrap().kind(),
        PmDurableRecordKind::PlaceResult
    );

    drain_persistence(&mut owner, 123).await;
    owner.shutdown().await.unwrap();
    let scope = PmJournalScopeV1::from_config(&config).unwrap();
    let recovery = recover_pm_mutation_journal(journal_path, &scope).unwrap();
    assert_eq!(
        recovery.record_count(),
        JOURNAL_HEADER_RECORDS + READY_BASELINE_JOURNAL_RECORDS + 2
    );
    assert_eq!(recovery.owned_order_count(), 1);
    assert_eq!(recovery.compacted_intent_id(), 0);
    assert_eq!(
        recovery.recovered_orders().next().unwrap().place(),
        PmJournalRecoveredPlaceV1::Rejected
    );
}

#[tokio::test(flavor = "current_thread")]
async fn ambiguous_timeout_then_late_fake_ack_converges_and_replays_bound() {
    let (config, _directory, journal_path, mut owner) = ready_owner().await;
    let client = admit_quote(&mut owner, &config, PmOrderSide::Buy, 1);
    wait_for_prepared_quotes(&mut owner, 1, 121).await;
    owner
        .execute_next_quote(PmFakePlaceScript::acknowledgement_unknown(), 122)
        .unwrap();
    let ambiguous = owner.private_mut().owned_order(client).unwrap();
    assert_eq!(ambiguous.submit(), PmOwnedSubmitState::Ambiguous);
    assert!(ambiguous.reconciliation_required());

    let venue = venue_order(&config, "late-ack");
    let late = late_ack_result(&config, client, venue).unwrap();
    owner.reduce_serviced_fake_place(late, 123).unwrap();
    let converged = owner.private_mut().owned_order(client).unwrap();
    assert_eq!(converged.submit(), PmOwnedSubmitState::Accepted);
    assert_eq!(converged.venue_order(), Some(venue));
    assert_eq!(converged.status(), Some(PmOrderStatus::Open));
    assert!(!converged.reconciliation_required());
    assert_eq!(owner.counters().place_results(), 2);
    assert_eq!(
        owner.counters().fact_records(),
        READY_BASELINE_FACT_RECORDS + 2
    );

    drain_persistence(&mut owner, 124).await;
    owner.shutdown().await.unwrap();
    let scope = PmJournalScopeV1::from_config(&config).unwrap();
    let recovery = recover_pm_mutation_journal(journal_path, &scope).unwrap();
    assert_eq!(
        recovery.record_count(),
        JOURNAL_HEADER_RECORDS + READY_BASELINE_JOURNAL_RECORDS + 3
    );
    assert_eq!(recovery.unresolved_order_count(), 0);
    let recovered = recovery.recovered_orders().next().unwrap();
    assert_eq!(recovered.place(), PmJournalRecoveredPlaceV1::Bound);
    assert_eq!(recovered.venue_order(), Some(venue));
}

#[tokio::test(flavor = "current_thread")]
async fn both_sides_admitted_under_one_revision_remain_dispatchable_in_owner_order() {
    let (config, _directory, _journal_path, mut owner) = ready_owner().await;
    let buy = admit_quote(&mut owner, &config, PmOrderSide::Buy, 1);
    let sell = admit_quote(&mut owner, &config, PmOrderSide::Sell, 2);
    assert_eq!(owner.pending_persistence(), 2);
    assert_eq!(owner.pending_effects(), 0);

    let prepared = collect_prepared_quotes(&mut owner, 2, 121).await;
    assert_eq!(owner.pending_effects(), 2);
    let first_client = match prepared[0] {
        PmPersistenceIntentIdentity::Quote { client_order, .. } => client_order,
        PmPersistenceIntentIdentity::Cancel { .. } => {
            panic!("prepared quote carried cancel identity")
        }
    };
    let second_client = match prepared[1] {
        PmPersistenceIntentIdentity::Quote { client_order, .. } => client_order,
        PmPersistenceIntentIdentity::Cancel { .. } => {
            panic!("prepared quote carried cancel identity")
        }
    };
    assert!(
        (first_client == buy && second_client == sell)
            || (first_client == sell && second_client == buy)
    );
    let first_venue = venue_order(&config, "paired-buy");
    let first_fill = PmFakeImmediateFill::new(
        PmFillId::new("paired-buy-fill").unwrap(),
        PmPrice::parse_decimal("0.40").unwrap(),
        PmQuantity::parse_decimal("0.25").unwrap(),
        PmFillFee::Unknown,
    );
    owner
        .execute_next_quote(
            PmFakePlaceScript::acknowledged(first_venue, vec![first_fill].into_boxed_slice())
                .unwrap(),
            122,
        )
        .unwrap();
    let first = owner.private_mut().owned_order(first_client).unwrap();
    assert_eq!(first.venue_order(), Some(first_venue));
    assert_eq!(first.status(), Some(PmOrderStatus::PartiallyFilled));
    assert_eq!(
        owner
            .private_mut()
            .owned_order(second_client)
            .unwrap()
            .submit(),
        PmOwnedSubmitState::Pending
    );

    let second_venue = venue_order(&config, "paired-sell");
    owner
        .execute_next_quote(
            PmFakePlaceScript::acknowledged(second_venue, Box::new([])).unwrap(),
            123,
        )
        .unwrap();
    let second = owner.private_mut().owned_order(second_client).unwrap();
    assert_eq!(second.venue_order(), Some(second_venue));
    assert_eq!(second.status(), Some(PmOrderStatus::Open));
    assert_eq!(owner.pending_effects(), 0);
    assert_eq!(owner.counters().prepared_quotes(), 2);
    assert_eq!(owner.counters().place_results(), 2);
    assert_eq!(owner.counters().unique_fills(), 1);

    drain_persistence(&mut owner, 124).await;
    owner.shutdown().await.unwrap();
}

#[derive(Clone, Copy)]
enum InvalidationCase {
    Missing,
    Changed,
    Expired,
}

async fn assert_quote_invalidated_after_durable_intent(case: InvalidationCase) {
    let (config, _directory, journal_path, mut owner) = ready_owner().await;
    let client = admit_quote(&mut owner, &config, PmOrderSide::Buy, 1);
    assert_eq!(owner.retained_effect_permits(), 1);
    assert_eq!(owner.pending_effects(), 0);
    let service_ns = match case {
        InvalidationCase::Missing => {
            owner.invalidate_revisions();
            121
        }
        InvalidationCase::Changed => {
            owner.update_revisions(authority_revisions(2));
            121
        }
        InvalidationCase::Expired => QUOTE_EXPIRES_NS,
    };
    let identity = wait_for_quote_invalidation(&mut owner, service_ns).await;
    assert!(matches!(
        identity,
        PmPersistenceIntentIdentity::Quote {
            client_order,
            ..
        } if client_order == client
    ));

    assert_eq!(owner.retained_effect_permits(), 0);
    assert_eq!(owner.pending_effects(), 0);
    assert_eq!(owner.effects.metrics().invalidated_after_durability(), 1);
    assert_eq!(owner.counters().prepared_quotes(), 0);
    assert_eq!(owner.counters().place_results(), 0);
    assert_eq!(
        owner.counters().fact_records(),
        READY_BASELINE_FACT_RECORDS + 1
    );
    let retained = owner.private_mut().owned_order(client).unwrap();
    assert_eq!(retained.submit(), PmOwnedSubmitState::Rejected);
    assert_eq!(retained.status(), Some(PmOrderStatus::Rejected));
    assert_eq!(retained.venue_order(), None);
    let consequence = owner.pop_durable_consequence().unwrap();
    assert_eq!(consequence.kind(), PmDurableRecordKind::PlaceResult);
    assert_eq!(consequence.client_order(), Some(client));
    assert!(owner.pop_durable_consequence().is_none());
    assert!(matches!(
        owner.execute_next_quote(
            PmFakePlaceScript::rejected(PmFakePlaceRejectReason::FixtureRejected),
            service_ns.saturating_add(1),
        ),
        Err(PmMutationError::EffectKindMismatch)
    ));

    drain_persistence(&mut owner, service_ns.saturating_add(2)).await;
    owner.shutdown().await.unwrap();
    let scope = PmJournalScopeV1::from_config(&config).unwrap();
    let recovery = recover_pm_mutation_journal(journal_path, &scope).unwrap();
    assert_eq!(
        recovery.record_count(),
        JOURNAL_HEADER_RECORDS + READY_BASELINE_JOURNAL_RECORDS + 2
    );
    assert_eq!(recovery.owned_order_count(), 1);
    assert_eq!(recovery.compacted_intent_id(), 0);
    assert_eq!(
        recovery.recovered_orders().next().unwrap().place(),
        PmJournalRecoveredPlaceV1::Rejected
    );
}

#[tokio::test(flavor = "current_thread")]
async fn missing_revision_after_intent_durability_journals_local_rejection_without_dispatch() {
    assert_quote_invalidated_after_durable_intent(InvalidationCase::Missing).await;
}

#[tokio::test(flavor = "current_thread")]
async fn changed_revision_after_intent_durability_journals_local_rejection_without_dispatch() {
    assert_quote_invalidated_after_durable_intent(InvalidationCase::Changed).await;
}

#[tokio::test(flavor = "current_thread")]
async fn expired_approval_after_intent_durability_journals_local_rejection_without_dispatch() {
    assert_quote_invalidated_after_durable_intent(InvalidationCase::Expired).await;
}

#[tokio::test(flavor = "current_thread")]
async fn cancel_outcomes_map_to_exact_owned_states() {
    let cases = [
        (
            PmFakeCancelScript::accepted(),
            Some(PmOrderStatus::Cancelled),
            PmOwnedCancelState::Accepted,
            true,
        ),
        (
            PmFakeCancelScript::rejected(PmFakeCancelRejectReason::FixtureRejected),
            Some(PmOrderStatus::Open),
            PmOwnedCancelState::Rejected,
            false,
        ),
        (
            PmFakeCancelScript::already_filled(),
            Some(PmOrderStatus::Filled),
            PmOwnedCancelState::FilledRace,
            true,
        ),
        (
            PmFakeCancelScript::acknowledgement_unknown(),
            Some(PmOrderStatus::Open),
            PmOwnedCancelState::Ambiguous,
            true,
        ),
    ];

    for (index, (script, status, cancel, reconciliation_required)) in cases.into_iter().enumerate()
    {
        let (config, _directory, _journal_path, mut owner) = ready_owner().await;
        let venue = venue_order(&config, &format!("cancel-outcome-{index}"));
        let client = place_resting_quote(&mut owner, &config, PmOrderSide::Buy, 1, venue).await;
        assert!(matches!(
            owner.begin_cancel(cancel_request(client)).unwrap(),
            PmCancelMutationAdmission::JournalPending {
                client_order,
                venue_order
            } if client_order == client && venue_order == venue
        ));
        wait_for_prepared_cancel(&mut owner, 131).await;
        owner.execute_next_cancel(script, 132).unwrap();

        let order = owner.private_mut().owned_order(client).unwrap();
        assert_eq!(order.status(), status);
        assert_eq!(order.cancel(), cancel);
        assert_eq!(order.reconciliation_required(), reconciliation_required);
        assert_eq!(owner.counters().cancel_results(), 1);
        assert_eq!(
            owner.pop_durable_consequence().unwrap().kind(),
            PmDurableRecordKind::PlaceResult
        );
        assert_eq!(
            owner.pop_durable_consequence().unwrap().kind(),
            PmDurableRecordKind::CancelResult
        );

        drain_persistence(&mut owner, 133).await;
        owner.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn replacement_preflight_leaves_cancel_state_for_the_durable_cancel_owner() {
    let (config, _directory, _journal_path, mut owner) = ready_owner().await;
    let venue = venue_order(&config, "replacement-preflight");
    let client = place_resting_quote(&mut owner, &config, PmOrderSide::Buy, 1, venue).await;
    let persistence_before = owner.pending_persistence();
    let counters_before = owner.counters();
    assert_eq!(
        owner.private_mut().owned_order(client).unwrap().cancel(),
        PmOwnedCancelState::None
    );

    assert!(matches!(
        owner
            .begin_quote(quote_request_at_probability(
                &config,
                PmOrderSide::Buy,
                2,
                QUOTE_EXPIRES_NS,
                0.41,
            ))
            .unwrap(),
        PmQuoteMutationAdmission::CancelBeforeReplace {
            current,
            venue_order,
        } if current == client && venue_order == venue
    ));
    assert_eq!(
        owner.private_mut().owned_order(client).unwrap().cancel(),
        PmOwnedCancelState::None
    );
    assert_eq!(owner.pending_persistence(), persistence_before);
    assert_eq!(
        owner.counters().cancel_before_replace(),
        counters_before.cancel_before_replace() + 1
    );
    assert_eq!(
        owner.counters().cancel_intents(),
        counters_before.cancel_intents()
    );

    assert!(matches!(
        owner.begin_cancel(cancel_request(client)).unwrap(),
        PmCancelMutationAdmission::JournalPending {
            client_order,
            venue_order,
        } if client_order == client && venue_order == venue
    ));
    assert_eq!(
        owner.private_mut().owned_order(client).unwrap().cancel(),
        PmOwnedCancelState::Pending
    );
    assert_eq!(owner.pending_persistence(), persistence_before + 1);
    assert_eq!(
        owner.counters().cancel_intents(),
        counters_before.cancel_intents() + 1
    );

    wait_for_prepared_cancel(&mut owner, 131).await;
    owner
        .execute_next_cancel(PmFakeCancelScript::accepted(), 132)
        .unwrap();
    drain_persistence(&mut owner, 133).await;
    owner.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn fill_winning_cancel_race_converges_for_late_cancel_result() {
    let (config, _directory, _journal_path, mut owner) = ready_owner().await;
    let venue = venue_order(&config, "cancel-fill-race");
    let client = place_resting_quote(&mut owner, &config, PmOrderSide::Buy, 1, venue).await;
    assert!(matches!(
        owner.begin_cancel(cancel_request(client)).unwrap(),
        PmCancelMutationAdmission::JournalPending { .. }
    ));
    wait_for_prepared_cancel(&mut owner, 131).await;

    let fill = full_fill(&config, client, venue, "fill-wins");
    reduce_ws_fill(&mut owner, &config, fill, 30, 132);
    let filled = owner.private_mut().owned_order(client).unwrap();
    assert_eq!(filled.status(), Some(PmOrderStatus::Filled));
    assert_eq!(filled.cancel(), PmOwnedCancelState::FilledRace);

    owner
        .execute_next_cancel(PmFakeCancelScript::accepted(), 133)
        .unwrap();
    let converged = owner.private_mut().owned_order(client).unwrap();
    assert_eq!(converged.status(), Some(PmOrderStatus::Filled));
    assert_eq!(converged.cancel(), PmOwnedCancelState::FilledRace);
    assert_eq!(
        converged.cumulative_filled(),
        PmQuantity::parse_decimal("1").unwrap().protocol_units()
    );
    assert_eq!(owner.counters().unique_fills(), 1);
    assert_eq!(owner.counters().cancel_results(), 1);

    drain_persistence(&mut owner, 134).await;
    owner.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn safety_cancel_remains_serviceable_after_quote_revision_invalidation() {
    let (config, _directory, _journal_path, mut owner) = ready_owner().await;
    let resting_venue = venue_order(&config, "revision-independent-cancel");
    let resting =
        place_resting_quote(&mut owner, &config, PmOrderSide::Buy, 1, resting_venue).await;

    let invalidated = admit_quote(&mut owner, &config, PmOrderSide::Sell, 2);
    owner.invalidate_revisions();
    let identity = wait_for_quote_invalidation(&mut owner, 124).await;
    assert!(matches!(
        identity,
        PmPersistenceIntentIdentity::Quote {
            client_order,
            ..
        } if client_order == invalidated
    ));
    assert_eq!(owner.halt(), None);

    assert!(matches!(
        owner.begin_cancel(cancel_request(resting)).unwrap(),
        PmCancelMutationAdmission::JournalPending {
            client_order,
            venue_order
        } if client_order == resting && venue_order == resting_venue
    ));
    wait_for_prepared_cancel(&mut owner, 131).await;
    owner
        .execute_next_cancel(PmFakeCancelScript::accepted(), 132)
        .unwrap();
    let cancelled = owner.private_mut().owned_order(resting).unwrap();
    assert_eq!(cancelled.status(), Some(PmOrderStatus::Cancelled));
    assert_eq!(cancelled.cancel(), PmOwnedCancelState::Accepted);
    assert_eq!(owner.counters().prepared_cancels(), 1);
    assert_eq!(owner.counters().cancel_results(), 1);

    drain_persistence(&mut owner, 133).await;
    owner.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn persistence_age_failure_retains_permit_and_never_dispatches() {
    let (config, _directory, journal_path, mut owner) = ready_owner().await;
    let client = admit_quote(&mut owner, &config, PmOrderSide::Buy, 1);
    let failed_at = QUOTE_APPROVED_NS + PM_PENDING_PERSISTENCE_MAX_AGE_NS + 1;
    let outcome = owner.service_persistence(failed_at).unwrap();
    assert!(matches!(
        outcome,
        PmPersistenceService::IntentFailed {
            identity: PmPersistenceIntentIdentity::Quote {
                client_order,
                ..
            }
        } if client_order == client
    ));
    assert_eq!(owner.halt(), Some(PmMutationHalt::PersistenceAgeExceeded));
    assert_eq!(owner.pending_effects(), 0);
    assert_eq!(owner.retained_effect_permits(), 1);
    assert_eq!(owner.counters().prepared_quotes(), 0);
    assert_eq!(owner.counters().place_results(), 0);
    assert_eq!(owner.counters().fact_records(), READY_BASELINE_FACT_RECORDS);
    assert!(owner.pop_durable_consequence().is_none());
    assert!(matches!(
        owner.execute_next_quote(
            PmFakePlaceScript::rejected(PmFakePlaceRejectReason::FixtureRejected),
            failed_at + 1,
        ),
        Err(PmMutationError::EffectKindMismatch)
    ));

    owner.shutdown().await.unwrap();
    let scope = PmJournalScopeV1::from_config(&config).unwrap();
    let recovery = recover_pm_mutation_journal(journal_path, &scope).unwrap();
    assert_eq!(
        recovery.record_count(),
        JOURNAL_HEADER_RECORDS + READY_BASELINE_JOURNAL_RECORDS + 1
    );
    assert_eq!(recovery.unresolved_order_count(), 1);
    assert_eq!(
        recovery.recovered_orders().next().unwrap().place(),
        PmJournalRecoveredPlaceV1::Unknown
    );
}

#[tokio::test(flavor = "current_thread")]
async fn invalid_fact_clock_aborts_move_only_fill_compaction_ticket() {
    let (config, _directory, _journal_path, mut owner) = ready_owner().await;
    let compaction = owner.prepare_fill_watermark_compaction().unwrap();
    assert!(owner.private_mut().fill_watermark_compaction_pending());

    assert!(matches!(
        owner.record_fill_watermark_fact(fill_watermark_record(&config, 2), compaction, 0),
        Err(PmMutationError::Persistence(
            PmPersistenceError::InvalidMonotonicTime
        ))
    ));
    assert!(!owner.private_mut().fill_watermark_compaction_pending());
    assert!(owner.ensure_fill_watermark_compaction_available().is_ok());
    assert_eq!(owner.pending_persistence(), 0);
    assert_eq!(owner.counters().fact_records(), READY_BASELINE_FACT_RECORDS);
    owner.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn fact_capacity_failure_aborts_move_only_fill_compaction_ticket() {
    let (config, _directory, _journal_path, mut owner) = ready_owner().await;
    for correlation in 1..=PM_PENDING_PERSISTENCE_CAPACITY {
        owner.durable_consequences.push_back(PmDurableConsequence {
            kind: PmDurableRecordKind::SafetyHalt,
            client_order: None,
            correlation: u64::try_from(correlation).unwrap(),
        });
    }
    let compaction = owner.prepare_fill_watermark_compaction().unwrap();
    assert!(owner.private_mut().fill_watermark_compaction_pending());

    assert!(matches!(
        owner.record_fill_watermark_fact(fill_watermark_record(&config, 2), compaction, 200),
        Err(PmMutationError::DurableConsequenceSaturated)
    ));
    assert_eq!(
        owner.halt(),
        Some(PmMutationHalt::DurableConsequenceSaturated)
    );
    assert!(!owner.private_mut().fill_watermark_compaction_pending());
    assert!(owner.ensure_fill_watermark_compaction_available().is_ok());
    assert_eq!(owner.pending_persistence(), 0);
    assert_eq!(owner.counters().fact_records(), READY_BASELINE_FACT_RECORDS);
    owner.shutdown().await.unwrap();
}

mod admission_boundaries;
mod recovery;
mod risk_cancel;
mod safety;
