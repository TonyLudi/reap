//! Contract tests for the canonical-private-to-durable-journal bridge.

use std::path::{Path, PathBuf};
use std::time::Duration;

use reap_pm_core::{
    ConnectionEpoch, EventClock, EventEnvelope, EventOrdering, EvmAddress, IngressSequence,
    MAX_REQUIRED_SPENDERS, OkxInstrumentId, OkxReferenceHandle, OkxReferenceInstrument,
    PmAccountHandle, PmAccountScope, PmAllowanceEvent, PmAllowanceValue, PmAssetId, PmBalanceEvent,
    PmChainId, PmClientOrderKey, PmCompleteAccountSnapshot, PmCompleteFillQuery,
    PmCompleteOpenOrdersSnapshot, PmConditionId, PmConnectionId, PmEnvironmentId,
    PmErc1155OperatorApproval, PmFillEvent, PmFillExecution, PmFillFee, PmFillId, PmFillKey,
    PmFillQueryCursor, PmFillRole, PmFillSettlementStatus, PmFunderId, PmInstrumentHandle,
    PmMarketHandle, PmMarketId, PmMarketLifecycle, PmMarketMetadata, PmOrderEvent, PmOrderIdentity,
    PmOrderProgress, PmOrderSalt, PmOrderSide, PmOrderStatus, PmOutcomeLabel, PmOutcomeMetadata,
    PmPositionAvailability, PmPositionEvent, PmPrice, PmProductSource, PmQuantity,
    PmReconciliationRequestBoundary, PmReferenceMapping, PmSignerId, PmSnapshotEvidence,
    PmSourceBound, PmSourceHandle, PmSpenderDomain, PmSpenderRequirement, PmTick, PmTokenHandle,
    PmTokenId, PmVenueOrderId, PmVenueOrderKey, SnapshotRevision, U256,
};
use reap_pm_live_contracts::{
    PmAccountConnectivityConfig, PmConnectionRoute, PmConnectivityConfig,
    PmPublicConnectivityConfig,
};
use reap_pm_state::{
    PmAccountSnapshotApply, PmCardinalityRiskLimits, PmExactReservation, PmExposureRiskLimits,
    PmFreshnessRiskLimits, PmOpenOrdersApply, PmOrderRiskLimits, PmOwnedSubmitState,
    PmReconciliationApply, PmReconciliationFillDisposition, PmReconciliationReductions,
    PmRiskLimits,
};
use reap_pm_strategy::{PmQuotePolicyInput, validate_passive_quote_candidate};
use reap_polymarket_adapter::{
    PmFakeImmediateFill, PmFakePlaceScript, PmPrivateLifecycleObservation,
};
use tempfile::TempDir;

use super::super::mutation::{
    PmMutationError, PmMutationHalt, PmMutationOwner, PmPersistenceService,
    PmQuoteMutationAdmission, PmQuoteMutationRequest,
};
use crate::coordinator::PmAuthorityRevisions;
use crate::fake_effect::PmFakeEffectRole;
use crate::journal::{
    PmJournalFillOccurrenceV1, PmJournalFillSourceV1, PmJournalOrderProgressSourceV1,
    PmJournalRecoveredObservationV1, PmJournalScopeV1, PmJournalTerminalStatusV1,
};
use crate::private_monitor::PmPrivateMonitorRuntime;

const GOAL_F_PUSD: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
const GOAL_F_CTF: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
const GOAL_F_STANDARD_EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";
const INITIAL_CURSOR_BYTE: u8 = 1;

fn fixture() -> PmConnectivityConfig {
    let instrument = PmInstrumentHandle::new(
        PmMarketHandle::from_ordinal(0),
        PmTokenHandle::from_ordinal(0),
    );
    let reference = OkxReferenceHandle::from_ordinal(0);
    let eoa = EvmAddress::from_bytes([4; 20]).unwrap();
    let account_scope = PmAccountScope::new(
        PmEnvironmentId::new("fixture").unwrap(),
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

async fn start_owner(config: &PmConnectivityConfig, journal_path: &Path) -> PmMutationOwner {
    let account = config.account();
    let private = PmPrivateMonitorRuntime::new(account, risk_limits()).unwrap();
    let fake = PmFakeEffectRole::new(
        account.account_scope(),
        account.instrument(),
        account.instrument_id(),
    );
    let (owner, recovery) =
        PmMutationOwner::start(config, private, fake, journal_path.to_path_buf())
            .await
            .unwrap();
    assert_eq!(recovery.record_count(), 0);
    owner
}

#[test]
fn mutation_owner_stack_footprint_is_explicit() {
    let config = fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("future-size.ndjson");
    let restart_history = write_restart_history(&config, &path);
    let client = PmJournalScopeV1::from_config(&config)
        .unwrap()
        .client_order_for_intent(1)
        .unwrap();
    let restarted_history = assert_restarted_history(&config, &path, client);
    assert!(std::mem::size_of::<PmMutationOwner>() <= 32 * 1_024);
    assert!(std::mem::size_of::<PmPrivateMonitorRuntime>() <= 32 * 1_024);
    assert!(std::mem::size_of::<crate::journal::PmJournalRecordV1>() <= 16 * 1_024);
    assert!(std::mem::size_of_val(&restart_history) <= 128 * 1_024);
    assert!(std::mem::size_of_val(&restarted_history) <= 128 * 1_024);
}

async fn ready_owner() -> (PmConnectivityConfig, TempDir, PathBuf, PmMutationOwner) {
    let config = fixture();
    let directory = tempfile::tempdir().unwrap();
    let journal_path = directory.path().join("pm-mutation.ndjson");
    let mut owner = start_owner(&config, &journal_path).await;
    make_private_ready(&mut owner, &config);
    settle_readiness_watermark(&mut owner).await;
    (config, directory, journal_path, owner)
}

async fn settle_readiness_watermark(owner: &mut PmMutationOwner) {
    drain_persistence(owner, 111).await;
    assert_eq!(
        owner.pop_durable_consequence().unwrap().kind(),
        crate::coordinator::effects::PmDurableRecordKind::FillWatermarkAdvanced
    );
    assert!(owner.pop_durable_consequence().is_none());
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

    let (account_envelope, fill_envelope) = reconciliation_pair(
        config,
        2,
        20,
        21,
        None,
        INITIAL_CURSOR_BYTE,
        Vec::new(),
        110,
    );
    assert!(matches!(
        owner
            .reduce_serviced_reconciliation(account_envelope, fill_envelope)
            .unwrap(),
        PmReconciliationApply::Applied { .. }
    ));
    owner.update_revisions(
        PmAuthorityRevisions::new(SnapshotRevision::new(1), SnapshotRevision::new(1), 1, 1, 1)
            .unwrap(),
    );
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

#[allow(clippy::too_many_arguments)]
fn reconciliation_pair(
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

fn fill_event(
    config: &PmConnectivityConfig,
    client_order: Option<PmClientOrderKey>,
    venue_order: PmVenueOrderKey,
    fill_id: &str,
    quantity: &str,
) -> PmFillEvent {
    let source = config.account().account_route().source();
    PmFillEvent::new(
        source,
        config.account().instrument(),
        PmFillKey::new(venue_order, PmFillId::new(fill_id).unwrap()),
        PmOrderIdentity::new(client_order, Some(venue_order)).unwrap(),
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

fn order_event(
    config: &PmConnectivityConfig,
    client_order: Option<PmClientOrderKey>,
    venue_order: PmVenueOrderKey,
    status: PmOrderStatus,
    cumulative_filled: U256,
) -> PmOrderEvent {
    PmOrderEvent::new(
        config.account().account_route().source(),
        config.account().instrument(),
        PmOrderIdentity::new(client_order, Some(venue_order)).unwrap(),
        PmOrderSide::Buy,
        PmPrice::parse_decimal("0.40").unwrap(),
        PmOrderProgress::new(
            PmQuantity::parse_decimal("1").unwrap(),
            cumulative_filled,
            status,
        )
        .unwrap(),
    )
    .unwrap()
}

fn quote_request(config: &PmConnectivityConfig, salt: u64) -> PmQuoteMutationRequest {
    let metadata = config.account().expected_metadata();
    let candidate = validate_passive_quote_candidate(PmQuotePolicyInput::new(
        config.account().instrument(),
        metadata,
        PmOrderSide::Buy,
        0.40,
        PmQuantity::parse_decimal("1").unwrap(),
        Some(PmPrice::parse_decimal("0.30").unwrap()),
        Some(PmPrice::parse_decimal("0.50").unwrap()),
    ))
    .unwrap();
    PmQuoteMutationRequest::new(
        candidate,
        PmExactReservation::policy_approved(U256::from_u64(400_000), U256::ZERO).unwrap(),
        PmOrderSalt::from_u64(salt).unwrap(),
        1,
        120,
        1_000,
        110,
        110,
    )
}

async fn place_quote(
    owner: &mut PmMutationOwner,
    config: &PmConnectivityConfig,
    venue: PmVenueOrderKey,
    immediate_fills: Box<[PmFakeImmediateFill]>,
) -> PmClientOrderKey {
    let client_order = match owner.begin_quote(quote_request(config, 1)).unwrap() {
        PmQuoteMutationAdmission::JournalPending { client_order, .. } => client_order,
        other => panic!("expected pending quote intent, got {other:?}"),
    };
    wait_for_prepared_quote(owner, 121).await;
    owner
        .execute_next_quote(
            PmFakePlaceScript::acknowledged(venue, immediate_fills).unwrap(),
            122,
        )
        .unwrap();
    client_order
}

async fn wait_for_prepared_quote(owner: &mut PmMutationOwner, monotonic_ns: u64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        match owner.service_persistence(monotonic_ns).unwrap() {
            PmPersistenceService::PreparedQuote { .. } => return,
            PmPersistenceService::Pending | PmPersistenceService::Empty => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "quote intent was not durably acknowledged"
                );
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            other => panic!("unexpected quote persistence outcome: {other:?}"),
        }
    }
}

async fn drain_persistence(owner: &mut PmMutationOwner, monotonic_ns: u64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if owner.pending_persistence() == 0 {
            return;
        }
        match owner.service_persistence(monotonic_ns).unwrap() {
            PmPersistenceService::Pending => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "journal facts were not durably acknowledged"
                );
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            PmPersistenceService::FactAcknowledged { .. } | PmPersistenceService::Empty => {}
            PmPersistenceService::IntentFailed { .. }
            | PmPersistenceService::FactFailed
            | PmPersistenceService::PreparedQuote { .. }
            | PmPersistenceService::PreparedCancel { .. }
            | PmPersistenceService::QuoteInvalidated { .. } => {
                panic!("unexpected persistence outcome while draining facts")
            }
        }
    }
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

fn reduce_ws_order(
    owner: &mut PmMutationOwner,
    config: &PmConnectivityConfig,
    order: PmOrderEvent,
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
            PmPrivateLifecycleObservation::Order(order),
        )
        .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn unique_ws_owned_fill_journals_once() {
    let (config, _directory, _path, mut owner) = ready_owner().await;
    let venue = venue_order(&config, "ws-once-order");
    let client = place_quote(&mut owner, &config, venue, Box::new([])).await;
    drain_persistence(&mut owner, 123).await;
    let fill = fill_event(&config, Some(client), venue, "ws-once-fill", "0.25");

    reduce_ws_fill(&mut owner, &config, fill, 30, 130);
    assert_eq!(owner.counters().unique_fills(), 1);
    assert_eq!(owner.counters().duplicate_fills(), 0);
    assert_eq!(owner.pending_persistence(), 1);

    reduce_ws_fill(&mut owner, &config, fill, 31, 131);
    assert_eq!(owner.counters().unique_fills(), 1);
    assert_eq!(owner.counters().duplicate_fills(), 1);
    assert_eq!(owner.pending_persistence(), 1);

    drain_persistence(&mut owner, 132).await;
    owner.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn immediate_ack_then_ws_duplicate_never_appends_a_second_fill_fact() {
    let (config, _directory, _path, mut owner) = ready_owner().await;
    let venue = venue_order(&config, "immediate-order");
    let immediate = PmFakeImmediateFill::new(
        PmFillId::new("immediate-fill").unwrap(),
        PmPrice::parse_decimal("0.40").unwrap(),
        PmQuantity::parse_decimal("0.25").unwrap(),
        PmFillFee::Unknown,
    );
    let client = place_quote(
        &mut owner,
        &config,
        venue,
        vec![immediate].into_boxed_slice(),
    )
    .await;
    assert_eq!(owner.counters().unique_fills(), 1);
    assert_eq!(owner.pending_persistence(), 2);

    let fill = fill_event(&config, Some(client), venue, "immediate-fill", "0.25");
    reduce_ws_fill(&mut owner, &config, fill, 30, 130);
    assert_eq!(owner.counters().unique_fills(), 1);
    assert_eq!(owner.counters().duplicate_fills(), 1);
    assert_eq!(owner.pending_persistence(), 2);

    drain_persistence(&mut owner, 131).await;
    owner.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn rest_unique_and_duplicate_dispositions_have_exact_fill_and_watermark_consequences() {
    let (config, _directory, _path, mut owner) = ready_owner().await;
    let venue = venue_order(&config, "rest-order");
    let client = place_quote(&mut owner, &config, venue, Box::new([])).await;
    drain_persistence(&mut owner, 123).await;
    let fill = fill_event(&config, None, venue, "rest-fill", "0.25");

    let (account, fills) = reconciliation_pair(
        &config,
        3,
        40,
        41,
        Some(INITIAL_CURSOR_BYTE),
        2,
        vec![fill],
        140,
    );
    owner
        .reduce_serviced_reconciliation(account, fills)
        .unwrap();
    assert_eq!(owner.counters().unique_fills(), 1);
    assert_eq!(owner.counters().duplicate_fills(), 0);
    assert_eq!(owner.pending_persistence(), 2);
    assert!(matches!(
        owner.reconciliation_reduction(0).unwrap().disposition(),
        PmReconciliationFillDisposition::OwnedApplied(_)
    ));
    drain_persistence(&mut owner, 141).await;

    let (account, fills) = reconciliation_pair(&config, 4, 50, 51, Some(2), 3, vec![fill], 150);
    owner
        .reduce_serviced_reconciliation(account, fills)
        .unwrap();
    assert_eq!(owner.counters().unique_fills(), 1);
    assert_eq!(owner.counters().duplicate_fills(), 1);
    // The duplicate fill creates no second FillApplied record. The changed
    // complete cursor creates exactly one watermark record.
    assert_eq!(owner.pending_persistence(), 1);
    assert!(matches!(
        owner.reconciliation_reduction(0).unwrap().disposition(),
        PmReconciliationFillDisposition::OwnedDuplicate(_)
    ));
    assert_eq!(
        owner
            .private_mut()
            .owned_order(client)
            .unwrap()
            .known_fill_total(),
        U256::from_u64(250_000)
    );

    drain_persistence(&mut owner, 151).await;
    owner.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn oversized_reconciliation_is_rejected_before_canonical_state_changes() {
    let (config, _directory, _path, mut owner) = ready_owner().await;
    let mut rows = Vec::with_capacity(owner.persistence_capacity() + 1);
    for index in 0..=owner.persistence_capacity() {
        let venue = venue_order(&config, &format!("unowned-order-{index:04}"));
        rows.push(fill_event(
            &config,
            None,
            venue,
            &format!("unowned-fill-{index:04}"),
            "1",
        ));
    }
    let (account, fills) = reconciliation_pair(
        &config,
        3,
        40,
        41,
        Some(INITIAL_CURSOR_BYTE),
        2,
        rows.clone(),
        140,
    );
    assert!(matches!(
        owner.reduce_serviced_reconciliation(account, fills),
        Err(PmMutationError::DurableConsequenceSaturated)
    ));
    assert_eq!(
        owner.halt(),
        Some(PmMutationHalt::DurableConsequenceSaturated)
    );
    assert_eq!(owner.counters().unique_fills(), 0);
    assert_eq!(owner.counters().fact_records(), 1);
    assert_eq!(owner.pending_persistence(), 0);
    assert_eq!(owner.pending_durable_consequences(), 0);

    // Reusing the exact pair directly through the canonical owner must still
    // apply. If the coordinator had partially committed before discovering
    // persistence saturation, this would be a duplicate or cursor failure.
    let (account, fills) =
        reconciliation_pair(&config, 3, 40, 41, Some(INITIAL_CURSOR_BYTE), 2, rows, 140);
    let mut scratch = PmReconciliationReductions::new();
    assert!(matches!(
        owner
            .private_mut()
            .reduce_serviced_reconciliation(account, fills, &mut scratch)
            .unwrap(),
        PmReconciliationApply::Applied {
            account: PmAccountSnapshotApply::Applied { .. },
            ..
        }
    ));
    assert_eq!(scratch.len(), owner.persistence_capacity() + 1);
    assert!(scratch.iter().all(|row| matches!(
        row.disposition(),
        PmReconciliationFillDisposition::Unowned(_)
    )));
    owner.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn restart_preserves_ws_and_rest_source_occurrence_and_cumulative_facts() {
    let config = fixture();
    let _directory = tempfile::tempdir().unwrap();
    let path = _directory.path().join("pm-mutation.ndjson");
    let client = Box::pin(write_restart_history(&config, &path)).await;
    Box::pin(assert_restarted_history(&config, &path, client)).await;

    let scope = PmJournalScopeV1::from_config(&config).unwrap();
    let recovered_again = crate::journal::recover_pm_mutation_journal(path, &scope).unwrap();
    assert_eq!(recovered_again.fill_key_count(), 2);
}

async fn write_restart_history(config: &PmConnectivityConfig, path: &Path) -> PmClientOrderKey {
    let mut owner = start_owner(config, path).await;
    make_private_ready(&mut owner, config);
    settle_readiness_watermark(&mut owner).await;
    let venue = venue_order(config, "restart-order");
    let client = place_quote(&mut owner, config, venue, Box::new([])).await;
    drain_persistence(&mut owner, 123).await;

    let ws_fill = fill_event(config, Some(client), venue, "restart-ws", "0.25");
    reduce_ws_fill(&mut owner, config, ws_fill, 30, 130);
    let rest_fill = fill_event(config, None, venue, "restart-rest", "0.25");
    let (account, fills) = reconciliation_pair(
        config,
        3,
        40,
        41,
        Some(INITIAL_CURSOR_BYTE),
        2,
        vec![ws_fill, rest_fill],
        140,
    );
    owner
        .reduce_serviced_reconciliation(account, fills)
        .unwrap();
    assert_eq!(owner.counters().unique_fills(), 2);
    drain_persistence(&mut owner, 141).await;
    owner.shutdown().await.unwrap();
    client
}

async fn assert_restarted_history(
    config: &PmConnectivityConfig,
    path: &Path,
    client: PmClientOrderKey,
) {
    let account = config.account();
    let private = PmPrivateMonitorRuntime::new(account, risk_limits()).unwrap();
    let fake = PmFakeEffectRole::new(
        account.account_scope(),
        account.instrument(),
        account.instrument_id(),
    );
    let (mut restarted, recovery) =
        PmMutationOwner::start(config, private, fake, path.to_path_buf())
            .await
            .unwrap();
    let mut recovered = recovery.recovered_fills();
    let ws = recovered.next().unwrap();
    let rest = recovered.next().unwrap();
    assert!(recovered.next().is_none());

    assert_eq!(ws.source, PmJournalFillSourceV1::PrivateWebsocket);
    assert_eq!(
        ws.occurrence.connection,
        Some(account.account_route().connection())
    );
    assert_eq!(
        ws.occurrence.connection_epoch,
        Some(ConnectionEpoch::new(1))
    );
    assert_eq!(
        ws.occurrence.ingress_sequence,
        Some(IngressSequence::new(30))
    );
    assert_eq!(ws.occurrence.snapshot_revision, None);
    assert_eq!(ws.occurrence.monotonic_service_ns, 130);
    assert_eq!(ws.fill.delta, PmQuantity::parse_decimal("0.25").unwrap());
    assert_eq!(ws.fill.cumulative, U256::from_u64(250_000));
    assert_eq!(ws.fill.remaining, U256::from_u64(750_000));

    assert_eq!(rest.source, PmJournalFillSourceV1::RestReconciliation);
    assert_eq!(
        rest.occurrence.connection,
        Some(account.account_route().connection())
    );
    assert_eq!(
        rest.occurrence.connection_epoch,
        Some(ConnectionEpoch::new(1))
    );
    assert_eq!(
        rest.occurrence.ingress_sequence,
        Some(IngressSequence::new(41))
    );
    assert_eq!(
        rest.occurrence.snapshot_revision,
        Some(SnapshotRevision::new(3))
    );
    assert_eq!(rest.occurrence.monotonic_service_ns, 140);
    assert_eq!(rest.fill.cumulative, U256::from_u64(500_000));
    assert_eq!(rest.fill.remaining, U256::from_u64(500_000));
    assert!(rest.occurrence.owner_sequence > ws.occurrence.owner_sequence);

    let order = restarted.private_mut().owned_order(client).unwrap();
    assert_eq!(order.submit(), PmOwnedSubmitState::Accepted);
    assert_eq!(order.known_fill_total(), U256::from_u64(500_000));
    assert_eq!(order.cumulative_filled(), U256::from_u64(500_000));
    assert_eq!(
        restarted.halt(),
        Some(PmMutationHalt::RecoveryReconciliationRequired)
    );
    restarted.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn partial_fill_then_expired_terminal_survives_exact_restart() {
    let config = fixture();
    let _directory = tempfile::tempdir().unwrap();
    let path = _directory.path().join("pm-expired-restart.ndjson");
    let (client, venue) = Box::pin(write_partial_expired_history(&config, &path)).await;
    Box::pin(assert_partial_expired_restart(
        &config, &path, client, venue,
    ))
    .await;
}

async fn write_partial_expired_history(
    config: &PmConnectivityConfig,
    path: &Path,
) -> (PmClientOrderKey, PmVenueOrderKey) {
    let mut owner = start_owner(config, path).await;
    make_private_ready(&mut owner, config);
    settle_readiness_watermark(&mut owner).await;
    let venue = venue_order(config, "expired-order");
    let client = place_quote(&mut owner, config, venue, Box::new([])).await;
    drain_persistence(&mut owner, 123).await;

    reduce_ws_fill(
        &mut owner,
        config,
        fill_event(config, Some(client), venue, "expired-fill", "0.25"),
        30,
        130,
    );
    reduce_ws_order(
        &mut owner,
        config,
        order_event(
            config,
            Some(client),
            venue,
            PmOrderStatus::Expired,
            U256::from_u64(250_000),
        ),
        31,
        131,
    );

    {
        let projection = owner.private_mut().owned_order(client).unwrap();
        assert_eq!(projection.status(), Some(PmOrderStatus::Expired));
        assert_eq!(projection.cumulative_filled(), U256::from_u64(250_000));
        assert_eq!(projection.known_fill_total(), U256::from_u64(250_000));
    }
    assert_eq!(owner.pending_persistence(), 2);
    drain_persistence(&mut owner, 132).await;
    Box::pin(owner.shutdown()).await.unwrap();
    (client, venue)
}

async fn assert_partial_expired_restart(
    config: &PmConnectivityConfig,
    path: &Path,
    client: PmClientOrderKey,
    venue: PmVenueOrderKey,
) {
    let account = config.account();
    let private = PmPrivateMonitorRuntime::new(account, risk_limits()).unwrap();
    let fake = PmFakeEffectRole::new(
        account.account_scope(),
        account.instrument(),
        account.instrument_id(),
    );
    let (mut restarted, recovery) =
        PmMutationOwner::start(config, private, fake, path.to_path_buf())
            .await
            .unwrap();
    {
        let mut observations = recovery.recovered_observations();
        let fill = match observations.next().unwrap() {
            PmJournalRecoveredObservationV1::FillApplied(fill) => fill,
            other => panic!("expected fill before terminal, got {other:?}"),
        };
        let terminal = match observations.next().unwrap() {
            PmJournalRecoveredObservationV1::OrderTerminal(terminal) => terminal,
            other => panic!("expected terminal after fill, got {other:?}"),
        };
        assert!(observations.next().is_none());

        assert_eq!(fill.fill.client_order, client);
        assert_eq!(fill.source, PmJournalFillSourceV1::PrivateWebsocket);
        assert_ws_occurrence(config, fill.occurrence, 30, 130);
        assert_eq!(terminal.client_order, client);
        assert_eq!(terminal.venue_order, venue);
        assert_eq!(terminal.status, PmJournalTerminalStatusV1::Expired);
        assert_eq!(
            terminal.source,
            PmJournalOrderProgressSourceV1::PrivateWebsocket
        );
        assert_eq!(terminal.cumulative, U256::from_u64(250_000));
        assert_eq!(terminal.remaining, U256::from_u64(750_000));
        assert_ws_occurrence(config, terminal.occurrence, 31, 131);
        assert!(terminal.occurrence.owner_sequence > fill.occurrence.owner_sequence);
        assert_eq!(
            recovery.last_owned_observation_sequence(),
            terminal.occurrence.owner_sequence.value()
        );

        let projection = restarted.private_mut().owned_order(client).unwrap();
        assert_eq!(projection.submit(), PmOwnedSubmitState::Accepted);
        assert_eq!(projection.venue_order(), Some(venue));
        assert_eq!(projection.status(), Some(PmOrderStatus::Expired));
        assert_eq!(projection.cumulative_filled(), U256::from_u64(250_000));
        assert_eq!(projection.known_fill_total(), U256::from_u64(250_000));
        assert_eq!(projection.remaining(), U256::from_u64(750_000));
    }
    drop(recovery);
    Box::pin(restarted.shutdown()).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn interleaved_fill_terminal_fill_replays_in_owner_order_and_converges() {
    let config = fixture();
    let _directory = tempfile::tempdir().unwrap();
    let path = _directory.path().join("pm-cancelled-fill-race.ndjson");
    let (client, venue) = Box::pin(write_cancelled_fill_race(&config, &path)).await;
    Box::pin(assert_cancelled_fill_race_restart(
        &config, &path, client, venue,
    ))
    .await;
}

async fn write_cancelled_fill_race(
    config: &PmConnectivityConfig,
    path: &Path,
) -> (PmClientOrderKey, PmVenueOrderKey) {
    let mut owner = start_owner(config, path).await;
    make_private_ready(&mut owner, config);
    settle_readiness_watermark(&mut owner).await;
    let venue = venue_order(config, "cancelled-fill-race-order");
    let client = place_quote(&mut owner, config, venue, Box::new([])).await;
    drain_persistence(&mut owner, 123).await;

    reduce_ws_fill(
        &mut owner,
        config,
        fill_event(config, Some(client), venue, "race-fill-1", "0.25"),
        30,
        130,
    );
    reduce_ws_order(
        &mut owner,
        config,
        order_event(
            config,
            None,
            venue,
            PmOrderStatus::Cancelled,
            U256::from_u64(250_000),
        ),
        31,
        131,
    );
    reduce_ws_fill(
        &mut owner,
        config,
        fill_event(config, None, venue, "race-fill-2", "0.75"),
        32,
        132,
    );

    {
        let projection = owner.private_mut().owned_order(client).unwrap();
        assert_eq!(projection.status(), Some(PmOrderStatus::Filled));
        assert_eq!(projection.cumulative_filled(), U256::from_u64(1_000_000));
        assert_eq!(projection.known_fill_total(), U256::from_u64(1_000_000));
        assert!(!projection.reconciliation_required());
    }
    assert_eq!(owner.pending_persistence(), 3);
    drain_persistence(&mut owner, 133).await;
    Box::pin(owner.shutdown()).await.unwrap();
    (client, venue)
}

async fn assert_cancelled_fill_race_restart(
    config: &PmConnectivityConfig,
    path: &Path,
    client: PmClientOrderKey,
    venue: PmVenueOrderKey,
) {
    let account = config.account();
    let private = PmPrivateMonitorRuntime::new(account, risk_limits()).unwrap();
    let fake = PmFakeEffectRole::new(
        account.account_scope(),
        account.instrument(),
        account.instrument_id(),
    );
    let (mut restarted, recovery) =
        PmMutationOwner::start(config, private, fake, path.to_path_buf())
            .await
            .unwrap();
    {
        let mut observations = recovery.recovered_observations();
        let first = match observations.next().unwrap() {
            PmJournalRecoveredObservationV1::FillApplied(fill) => fill,
            other => panic!("expected first fill, got {other:?}"),
        };
        let terminal = match observations.next().unwrap() {
            PmJournalRecoveredObservationV1::OrderTerminal(terminal) => terminal,
            other => panic!("expected interleaved terminal, got {other:?}"),
        };
        let second = match observations.next().unwrap() {
            PmJournalRecoveredObservationV1::FillApplied(fill) => fill,
            other => panic!("expected later fill, got {other:?}"),
        };
        assert!(observations.next().is_none());

        assert_eq!(first.fill.client_order, client);
        assert_eq!(terminal.client_order, client);
        assert_eq!(terminal.venue_order, venue);
        assert_eq!(terminal.status, PmJournalTerminalStatusV1::Cancelled);
        assert_eq!(second.fill.client_order, client);
        assert_eq!(
            terminal.occurrence.owner_sequence.value(),
            first.occurrence.owner_sequence.value() + 1
        );
        assert_eq!(
            second.occurrence.owner_sequence.value(),
            terminal.occurrence.owner_sequence.value() + 1
        );
        assert_ws_occurrence(config, first.occurrence, 30, 130);
        assert_ws_occurrence(config, terminal.occurrence, 31, 131);
        assert_ws_occurrence(config, second.occurrence, 32, 132);
        assert_eq!(second.fill.cumulative, U256::from_u64(1_000_000));
        assert_eq!(second.fill.remaining, U256::ZERO);

        let projection = restarted.private_mut().owned_order(client).unwrap();
        assert_eq!(projection.submit(), PmOwnedSubmitState::Accepted);
        assert_eq!(projection.status(), Some(PmOrderStatus::Filled));
        assert_eq!(projection.cumulative_filled(), U256::from_u64(1_000_000));
        assert_eq!(projection.known_fill_total(), U256::from_u64(1_000_000));
        assert_eq!(projection.remaining(), U256::ZERO);
        assert!(!projection.reconciliation_required());
    }
    drop(recovery);
    Box::pin(restarted.shutdown()).await.unwrap();
}

fn assert_ws_occurrence(
    config: &PmConnectivityConfig,
    occurrence: PmJournalFillOccurrenceV1,
    ingress: u64,
    monotonic_ns: u64,
) {
    assert_eq!(
        occurrence.connection,
        Some(config.account().account_route().connection())
    );
    assert_eq!(occurrence.connection_epoch, Some(ConnectionEpoch::new(1)));
    assert_eq!(
        occurrence.ingress_sequence,
        Some(IngressSequence::new(ingress))
    );
    assert_eq!(occurrence.snapshot_revision, None);
    assert_eq!(occurrence.monotonic_service_ns, monotonic_ns);
}
