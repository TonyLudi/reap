use reap_pm_core::{
    ConnectionEpoch, EvmAddress, IngressSequence, MAX_PM_BOOK_LEVELS, PmAssetId, PmBookDeltaBatch,
    PmBookLevel, PmBookQuantity, PmBookSide, PmBookSnapshot, PmBookUpdate, PmChainId,
    PmConditionId, PmEventError, PmInstrumentHandle, PmMarketHandle, PmMarketId, PmMarketLifecycle,
    PmMarketMetadata, PmOutcomeLabel, PmOutcomeMetadata, PmPrice, PmQuantity, PmSpenderDomain,
    PmSpenderRequirement, PmTick, PmTokenHandle, PmTokenId, SnapshotRevision, U256, VenueEventHash,
};
use reap_pm_state::{
    PmBookBatchEvidence, PmBookCounters, PmBookFreshness, PmBookReducer, PmBookTopCheck,
    PmBookTransition, PmDomainFingerprint, PmExternalBookFault, PmMetadataContract,
    PmMetadataContractError, PmMetadataDrift, PmMetadataFingerprint, PmMetadataObservation,
    PmProtocolProfile, PmPublicReadinessReason, PmUnitContract,
};

const INSTRUMENT: PmInstrumentHandle = PmInstrumentHandle::new(
    PmMarketHandle::from_ordinal(7),
    PmTokenHandle::from_ordinal(11),
);
const OTHER_INSTRUMENT: PmInstrumentHandle = PmInstrumentHandle::new(
    PmMarketHandle::from_ordinal(8),
    PmTokenHandle::from_ordinal(12),
);

#[derive(Clone, Copy)]
struct MarketSpec {
    condition_byte: u8,
    market_byte: u8,
    token: u64,
    label: &'static str,
    lifecycle: PmMarketLifecycle,
    tick_units: u32,
    minimum: &'static str,
    negative_risk: bool,
    chain: u64,
    exchange_byte: u8,
    collateral_byte: u8,
}

impl Default for MarketSpec {
    fn default() -> Self {
        Self {
            condition_byte: 0x11,
            market_byte: 0x22,
            token: 37,
            label: "YES",
            lifecycle: PmMarketLifecycle::new(true, false, false, true, true),
            tick_units: 100,
            minimum: "0.01",
            negative_risk: false,
            chain: 137,
            exchange_byte: 0x33,
            collateral_byte: 0x44,
        }
    }
}

fn market(spec: MarketSpec) -> PmMarketMetadata {
    let chain = PmChainId::new(spec.chain).unwrap();
    let exchange = EvmAddress::from_bytes([spec.exchange_byte; 20]).unwrap();
    let domain = if spec.negative_risk {
        PmSpenderDomain::NegativeRisk
    } else {
        PmSpenderDomain::Standard
    };
    let mut spenders = [None; 8];
    spenders[0] = Some(PmSpenderRequirement::new(
        chain,
        exchange,
        domain,
        PmAssetId::collateral(EvmAddress::from_bytes([spec.collateral_byte; 20]).unwrap()),
    ));
    PmMarketMetadata::new(
        PmConditionId::from_bytes([spec.condition_byte; 32]).unwrap(),
        PmMarketId::from_bytes([spec.market_byte; 32]).unwrap(),
        PmOutcomeMetadata::new(
            PmTokenId::new(U256::from_u64(spec.token)).unwrap(),
            PmOutcomeLabel::new(spec.label).unwrap(),
        ),
        spec.lifecycle,
        PmTick::from_units(spec.tick_units).unwrap(),
        PmQuantity::parse_decimal(spec.minimum).unwrap(),
        spec.negative_risk,
        chain,
        exchange,
        spenders,
        1,
    )
    .unwrap()
}

fn fingerprint(byte: u8) -> PmMetadataFingerprint {
    PmMetadataFingerprint::new([byte; 32]).unwrap()
}

fn domain(byte: u8) -> PmDomainFingerprint {
    PmDomainFingerprint::new([byte; 32]).unwrap()
}

fn base_contract() -> PmMetadataContract {
    PmMetadataContract::goal_f_clob_v2(market(MarketSpec::default()), domain(0x55))
}

fn contract_with_market(market: PmMarketMetadata) -> PmMetadataContract {
    PmMetadataContract::goal_f_clob_v2(market, domain(0x55))
}

fn reducer_with_freshness(metadata_age: u64, book_age: u64) -> PmBookReducer {
    PmBookReducer::new(
        INSTRUMENT,
        fingerprint(0x66),
        base_contract(),
        PmBookFreshness::new(metadata_age, book_age).unwrap(),
    )
    .unwrap()
}

fn observation(
    instrument: PmInstrumentHandle,
    revision: u64,
    observed_fingerprint: PmMetadataFingerprint,
    contract: PmMetadataContract,
    receive_ns: u64,
) -> PmMetadataObservation {
    PmMetadataObservation::new(
        instrument,
        SnapshotRevision::new(revision),
        observed_fingerprint,
        contract,
        receive_ns,
    )
    .unwrap()
}

fn base_observation(revision: u64, receive_ns: u64) -> PmMetadataObservation {
    observation(
        INSTRUMENT,
        revision,
        fingerprint(0x66),
        base_contract(),
        receive_ns,
    )
}

fn evidence(
    instrument: PmInstrumentHandle,
    epoch: u64,
    metadata_revision: u64,
    snapshot_revision: u64,
    ingress: u64,
    receive_ns: u64,
    venue_hash: Option<VenueEventHash>,
) -> PmBookBatchEvidence {
    PmBookBatchEvidence::new(
        instrument,
        ConnectionEpoch::new(epoch),
        SnapshotRevision::new(metadata_revision),
        SnapshotRevision::new(snapshot_revision),
        IngressSequence::new(ingress),
        receive_ns,
        venue_hash,
    )
    .unwrap()
}

fn ev(
    epoch: u64,
    metadata_revision: u64,
    snapshot_revision: u64,
    ingress: u64,
    receive_ns: u64,
) -> PmBookBatchEvidence {
    evidence(
        INSTRUMENT,
        epoch,
        metadata_revision,
        snapshot_revision,
        ingress,
        receive_ns,
        None,
    )
}

fn snapshot_hash() -> VenueEventHash {
    VenueEventHash::sha1([0xa5; 20]).unwrap()
}

fn snapshot_ev(
    epoch: u64,
    metadata_revision: u64,
    snapshot_revision: u64,
    ingress: u64,
    receive_ns: u64,
) -> PmBookBatchEvidence {
    evidence(
        INSTRUMENT,
        epoch,
        metadata_revision,
        snapshot_revision,
        ingress,
        receive_ns,
        Some(snapshot_hash()),
    )
}

#[allow(clippy::too_many_arguments)]
fn assert_snapshot_commit(
    transition: PmBookTransition,
    epoch: u64,
    metadata_revision: u64,
    snapshot_revision: u64,
    ingress: u64,
    expected_levels: u16,
    venue_hash: VenueEventHash,
) {
    let PmBookTransition::SnapshotCommitted {
        revision,
        levels,
        proof,
    } = transition
    else {
        panic!("expected snapshot commit transition");
    };
    assert_eq!(revision, SnapshotRevision::new(snapshot_revision));
    assert_eq!(levels, expected_levels);
    assert_eq!(proof.instrument(), INSTRUMENT);
    assert_eq!(proof.metadata_fingerprint(), fingerprint(0x66));
    assert_eq!(proof.connection_epoch(), ConnectionEpoch::new(epoch));
    assert_eq!(
        proof.metadata_revision(),
        SnapshotRevision::new(metadata_revision)
    );
    assert_eq!(
        proof.snapshot_revision(),
        SnapshotRevision::new(snapshot_revision)
    );
    assert_eq!(
        proof.local_ingress_sequence(),
        IngressSequence::new(ingress)
    );
    assert_eq!(proof.venue_hash(), venue_hash);
}

fn level(side: PmBookSide, price_units: u32, quantity: &str) -> PmBookLevel {
    PmBookLevel::new(
        side,
        PmPrice::from_units(price_units).unwrap(),
        PmBookQuantity::parse_decimal(quantity).unwrap(),
    )
}

fn delete(side: PmBookSide, price_units: u32) -> PmBookLevel {
    PmBookLevel::new(
        side,
        PmPrice::from_units(price_units).unwrap(),
        PmBookQuantity::Delete,
    )
}

fn top(bid_units: u32, ask_units: u32) -> PmBookTopCheck {
    PmBookTopCheck::new(
        Some(PmPrice::from_units(bid_units).unwrap()),
        Some(PmPrice::from_units(ask_units).unwrap()),
    )
}

fn snapshot_from(levels: Vec<PmBookLevel>) -> Result<PmBookSnapshot, PmEventError> {
    PmBookSnapshot::new(levels.into_boxed_slice())
}

fn delta_from(changes: Vec<PmBookLevel>) -> Result<PmBookDeltaBatch, PmEventError> {
    delta_with_top(changes, 500_000, 600_000)
}

fn delta_with_top(
    changes: Vec<PmBookLevel>,
    bid_units: u32,
    ask_units: u32,
) -> Result<PmBookDeltaBatch, PmEventError> {
    PmBookDeltaBatch::new(changes.into_boxed_slice(), top(bid_units, ask_units))
}

fn base_snapshot() -> PmBookSnapshot {
    snapshot_from(vec![
        level(PmBookSide::Ask, 700_000, "7"),
        level(PmBookSide::Bid, 400_000, "4"),
        level(PmBookSide::Ask, 600_000, "6"),
        level(PmBookSide::Bid, 500_000, "5"),
    ])
    .unwrap()
}

fn canonical_base_levels() -> Vec<PmBookLevel> {
    vec![
        level(PmBookSide::Bid, 500_000, "5"),
        level(PmBookSide::Bid, 400_000, "4"),
        level(PmBookSide::Ask, 600_000, "6"),
        level(PmBookSide::Ask, 700_000, "7"),
    ]
}

fn bootstrap(freshness: PmBookFreshness) -> PmBookReducer {
    let mut reducer =
        PmBookReducer::new(INSTRUMENT, fingerprint(0x66), base_contract(), freshness).unwrap();
    assert_eq!(
        reducer.apply_metadata(base_observation(1, 100)),
        Ok(PmBookTransition::MetadataAccepted {
            revision: SnapshotRevision::new(1)
        })
    );
    assert_eq!(
        reducer.begin_epoch(ConnectionEpoch::new(1)),
        Ok(PmBookTransition::EpochStarted {
            epoch: ConnectionEpoch::new(1)
        })
    );
    let transition = reducer
        .apply_snapshot(snapshot_ev(1, 1, 10, 1, 110), &base_snapshot())
        .unwrap();
    assert_snapshot_commit(transition, 1, 1, 10, 1, 4, snapshot_hash());
    assert!(reducer.readiness().is_ready());
    reducer
}

#[test]
fn constructor_values_reject_zeroes_and_expected_contract_mismatch() {
    assert_eq!(
        PmMetadataFingerprint::new([0; 32]),
        Err(PmMetadataContractError::ZeroMetadataFingerprint)
    );
    assert_eq!(
        PmDomainFingerprint::new([0; 32]),
        Err(PmMetadataContractError::ZeroDomainFingerprint)
    );
    assert_eq!(
        PmBookFreshness::new(0, 1),
        Err(PmMetadataContractError::ZeroFreshnessLimit)
    );
    assert_eq!(
        PmUnitContract::new(1, 0, 1, 1),
        Err(PmMetadataContractError::ZeroUnit)
    );

    let unsupported = PmMetadataContract::new(
        market(MarketSpec::default()),
        PmProtocolProfile::Unsupported(3),
        PmUnitContract::goal_f_clob_v2(),
        domain(0x55),
    );
    assert_eq!(
        PmBookReducer::new(
            INSTRUMENT,
            fingerprint(0x66),
            unsupported,
            PmBookFreshness::new(1, 1).unwrap()
        )
        .unwrap_err(),
        PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::Protocol)
    );

    let wrong_lot = PmMetadataContract::new(
        market(MarketSpec::default()),
        PmProtocolProfile::ClobV2,
        PmUnitContract::new(1_000_000, 1_000_000, 1_000_000, 1).unwrap(),
        domain(0x55),
    );
    assert_eq!(
        PmBookReducer::new(
            INSTRUMENT,
            fingerprint(0x66),
            wrong_lot,
            PmBookFreshness::new(1, 1).unwrap()
        )
        .unwrap_err(),
        PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::Lot)
    );
}

#[test]
fn metadata_epoch_snapshot_lifecycle_and_exact_baseline_counters() {
    let mut reducer = reducer_with_freshness(1_000, 1_000);
    assert_eq!(
        reducer.readiness().reason(),
        Some(PmPublicReadinessReason::MetadataMissing)
    );

    reducer.apply_metadata(base_observation(1, 100)).unwrap();
    assert_eq!(
        reducer.readiness().reason(),
        Some(PmPublicReadinessReason::ConnectionUnavailable)
    );
    reducer.begin_epoch(ConnectionEpoch::new(1)).unwrap();
    assert_eq!(
        reducer.readiness().reason(),
        Some(PmPublicReadinessReason::SnapshotMissing)
    );

    let hash = VenueEventHash::sha1([0x77; 20]).unwrap();
    let transition = reducer
        .apply_snapshot(
            evidence(INSTRUMENT, 1, 1, 10, 1, 110, Some(hash)),
            &base_snapshot(),
        )
        .unwrap();
    assert_snapshot_commit(transition, 1, 1, 10, 1, 4, hash);
    assert_eq!(reducer.levels(), canonical_base_levels());
    assert_eq!(reducer.last_verified_snapshot_hash(), Some(hash));
    assert!(reducer.readiness().is_ready());
    assert_eq!(
        reducer.readiness().metadata_revision(),
        Some(SnapshotRevision::new(1))
    );
    assert_eq!(
        reducer.readiness().snapshot_revision(),
        Some(SnapshotRevision::new(10))
    );
    assert_eq!(
        reducer.counters(),
        PmBookCounters {
            metadata_inputs: 1,
            metadata_accepted: 1,
            metadata_rejected: 0,
            epoch_attempts: 1,
            epochs_started: 1,
            reconnects: 0,
            snapshot_attempts: 1,
            snapshots_committed: 1,
            snapshot_levels_committed: 4,
            resync_snapshots: 0,
            delta_batch_attempts: 0,
            delta_batches_committed: 0,
            delta_changes_committed: 0,
            delta_top_checks: 0,
            delta_top_checks_confirmed: 0,
            top_checks: 0,
            top_checks_confirmed: 0,
            freshness_checks: 0,
            freshness_confirmed: 0,
            tick_size_changes: 0,
            external_faults: 0,
            duplicate_ingress: 0,
            reordered_ingress: 0,
            disconnects: 0,
            heartbeat_timeouts: 0,
            backlog_aged_faults: 0,
            gaps: 0,
            overflows: 0,
            hash_mismatches: 0,
            bbo_mismatches: 0,
            invalid_transitions: 0,
            clock_regressions: 0,
            invalidations: 0,
            unavailable_transitions: 0,
            stale_invalidations: 0,
        }
    );
}

#[test]
fn metadata_lifecycle_and_all_authoritative_drift_dimensions_are_typed() {
    let lifecycle_cases = [
        (
            PmMarketLifecycle::new(false, false, false, true, true),
            PmPublicReadinessReason::MarketInactive,
        ),
        (
            PmMarketLifecycle::new(true, true, false, true, true),
            PmPublicReadinessReason::MarketClosed,
        ),
        (
            PmMarketLifecycle::new(true, false, true, true, true),
            PmPublicReadinessReason::MarketArchived,
        ),
        (
            PmMarketLifecycle::new(true, false, false, false, true),
            PmPublicReadinessReason::OrdersNotAccepted,
        ),
        (
            PmMarketLifecycle::new(true, false, false, true, false),
            PmPublicReadinessReason::OrderBookDisabled,
        ),
    ];
    for (lifecycle, expected) in lifecycle_cases {
        let spec = MarketSpec {
            lifecycle,
            ..MarketSpec::default()
        };
        assert_metadata_rejection(contract_with_market(market(spec)), expected);
    }

    let identity = MarketSpec {
        condition_byte: 0x12,
        ..MarketSpec::default()
    };
    assert_metadata_rejection(
        contract_with_market(market(identity)),
        PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::Identity),
    );

    let label = MarketSpec {
        label: "NO",
        ..MarketSpec::default()
    };
    assert_metadata_rejection(
        contract_with_market(market(label)),
        PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::OutcomeLabel),
    );

    let protocol = PmMetadataContract::new(
        market(MarketSpec::default()),
        PmProtocolProfile::Unsupported(9),
        PmUnitContract::goal_f_clob_v2(),
        domain(0x55),
    );
    assert_metadata_rejection(
        protocol,
        PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::Protocol),
    );

    let units = PmMetadataContract::new(
        market(MarketSpec::default()),
        PmProtocolProfile::ClobV2,
        PmUnitContract::new(1_000_000, 2_000_000, 1_000_000, 10_000).unwrap(),
        domain(0x55),
    );
    assert_metadata_rejection(
        units,
        PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::Units),
    );

    let lot = PmMetadataContract::new(
        market(MarketSpec::default()),
        PmProtocolProfile::ClobV2,
        PmUnitContract::new(1_000_000, 1_000_000, 1_000_000, 20_000).unwrap(),
        domain(0x55),
    );
    assert_metadata_rejection(
        lot,
        PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::Lot),
    );

    let grid = MarketSpec {
        tick_units: 1_000,
        ..MarketSpec::default()
    };
    assert_metadata_rejection(
        contract_with_market(market(grid)),
        PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::Grid),
    );

    let minimum = MarketSpec {
        minimum: "0.02",
        ..MarketSpec::default()
    };
    assert_metadata_rejection(
        contract_with_market(market(minimum)),
        PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::Minimum),
    );

    let negative_risk = MarketSpec {
        negative_risk: true,
        ..MarketSpec::default()
    };
    assert_metadata_rejection(
        contract_with_market(market(negative_risk)),
        PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::NegativeRisk),
    );

    let domain_drift =
        PmMetadataContract::goal_f_clob_v2(market(MarketSpec::default()), domain(0x56));
    assert_metadata_rejection(
        domain_drift,
        PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::Domain),
    );

    let spender = MarketSpec {
        collateral_byte: 0x45,
        ..MarketSpec::default()
    };
    assert_metadata_rejection(
        contract_with_market(market(spender)),
        PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::RequiredSpenders),
    );
}

fn assert_metadata_rejection(
    observed_contract: PmMetadataContract,
    expected: PmPublicReadinessReason,
) {
    let mut reducer = reducer_with_freshness(1_000, 1_000);
    assert_eq!(
        reducer
            .apply_metadata(observation(
                INSTRUMENT,
                1,
                fingerprint(0x66),
                observed_contract,
                100
            ))
            .unwrap_err(),
        expected
    );
    assert_eq!(reducer.readiness().reason(), Some(expected));
    assert_eq!(reducer.counters().metadata_inputs, 1);
    assert_eq!(reducer.counters().metadata_accepted, 0);
    assert_eq!(reducer.counters().metadata_rejected, 1);
    assert_eq!(reducer.counters().invalidations, 1);
}

#[test]
fn metadata_identity_fingerprint_revision_and_clock_fail_closed() {
    let mut wrong_instrument = reducer_with_freshness(1_000, 1_000);
    assert_eq!(
        wrong_instrument
            .apply_metadata(observation(
                OTHER_INSTRUMENT,
                1,
                fingerprint(0x66),
                base_contract(),
                100
            ))
            .unwrap_err(),
        PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::Instrument)
    );

    let mut wrong_fingerprint = reducer_with_freshness(1_000, 1_000);
    assert_eq!(
        wrong_fingerprint
            .apply_metadata(observation(
                INSTRUMENT,
                1,
                fingerprint(0x67),
                base_contract(),
                100
            ))
            .unwrap_err(),
        PmPublicReadinessReason::MetadataFingerprintMismatch
    );

    let mut reducer = reducer_with_freshness(1_000, 1_000);
    reducer.apply_metadata(base_observation(2, 100)).unwrap();
    assert_eq!(
        reducer
            .apply_metadata(base_observation(2, 101))
            .unwrap_err(),
        PmPublicReadinessReason::MetadataRevisionNotIncreasing
    );

    let mut reducer = reducer_with_freshness(1_000, 1_000);
    reducer.apply_metadata(base_observation(1, 100)).unwrap();
    assert_eq!(
        reducer.apply_metadata(base_observation(2, 99)).unwrap_err(),
        PmPublicReadinessReason::ClockRegression
    );
    assert_eq!(reducer.counters().clock_regressions, 1);
}

#[test]
fn snapshot_validation_is_atomic_and_requires_two_nonempty_uncrossed_sides() {
    let invalid_snapshots = [
        (
            snapshot_from(vec![]).unwrap(),
            PmPublicReadinessReason::EmptyBook,
        ),
        (
            snapshot_from(vec![
                level(PmBookSide::Bid, 500_000, "1"),
                level(PmBookSide::Bid, 500_000, "2"),
                level(PmBookSide::Ask, 600_000, "1"),
            ])
            .unwrap(),
            PmPublicReadinessReason::DuplicateLevel,
        ),
        (
            snapshot_from(vec![
                level(PmBookSide::Bid, 500_050, "1"),
                level(PmBookSide::Ask, 600_000, "1"),
            ])
            .unwrap(),
            PmPublicReadinessReason::PriceOffTick,
        ),
        (
            snapshot_from(vec![
                level(PmBookSide::Bid, 600_000, "1"),
                level(PmBookSide::Ask, 500_000, "1"),
            ])
            .unwrap(),
            PmPublicReadinessReason::CrossedBook,
        ),
        (
            snapshot_from(vec![level(PmBookSide::Bid, 500_000, "1")]).unwrap(),
            PmPublicReadinessReason::EmptyBook,
        ),
    ];

    for (index, (snapshot, expected)) in invalid_snapshots.into_iter().enumerate() {
        let mut reducer = reducer_with_freshness(1_000, 1_000);
        reducer.apply_metadata(base_observation(1, 100)).unwrap();
        reducer.begin_epoch(ConnectionEpoch::new(1)).unwrap();
        assert_eq!(
            reducer
                .apply_snapshot(snapshot_ev(1, 1, 10, 1, 110 + index as u64), &snapshot,)
                .unwrap_err(),
            expected
        );
        assert!(reducer.levels().is_empty());
        assert!(!reducer.readiness().is_ready());
        assert_eq!(reducer.counters().snapshot_attempts, 1);
        assert_eq!(reducer.counters().snapshots_committed, 0);
    }
}

#[test]
fn delta_batch_commits_canonically_and_failed_batch_is_fully_atomic_until_resync() {
    let mut reducer = bootstrap(PmBookFreshness::new(1_000, 1_000).unwrap());
    let delta = delta_with_top(
        vec![
            level(PmBookSide::Ask, 650_000, "6.5"),
            delete(PmBookSide::Bid, 400_000),
            level(PmBookSide::Bid, 500_000, "50"),
            delete(PmBookSide::Ask, 600_000),
            level(PmBookSide::Bid, 450_000, "4.5"),
        ],
        500_000,
        650_000,
    )
    .unwrap();
    assert_eq!(
        reducer.apply_delta_batch(ev(1, 1, 10, 5, 120), &delta),
        Ok(PmBookTransition::DeltaBatchCommitted {
            revision: SnapshotRevision::new(10),
            changes: 5
        })
    );
    let committed = vec![
        level(PmBookSide::Bid, 500_000, "50"),
        level(PmBookSide::Bid, 450_000, "4.5"),
        level(PmBookSide::Ask, 650_000, "6.5"),
        level(PmBookSide::Ask, 700_000, "7"),
    ];
    assert_eq!(reducer.levels(), committed);
    assert_eq!(reducer.last_verified_snapshot_hash(), Some(snapshot_hash()));

    let duplicate = delta_from(vec![
        level(PmBookSide::Bid, 450_000, "40"),
        delete(PmBookSide::Bid, 450_000),
    ])
    .unwrap();
    assert_eq!(
        reducer
            .apply_delta_batch(ev(1, 1, 10, 6, 130), &duplicate)
            .unwrap_err(),
        PmPublicReadinessReason::DuplicateLevel
    );
    assert_eq!(reducer.levels(), committed);
    assert_eq!(
        reducer.readiness().reason(),
        Some(PmPublicReadinessReason::DuplicateLevel)
    );

    let transition = reducer
        .apply_snapshot(snapshot_ev(1, 1, 11, 7, 140), &base_snapshot())
        .unwrap();
    assert_snapshot_commit(transition, 1, 1, 11, 7, 4, snapshot_hash());
    assert!(reducer.readiness().is_ready());
    assert_eq!(
        reducer.counters(),
        PmBookCounters {
            metadata_inputs: 1,
            metadata_accepted: 1,
            metadata_rejected: 0,
            epoch_attempts: 1,
            epochs_started: 1,
            reconnects: 0,
            snapshot_attempts: 2,
            snapshots_committed: 2,
            snapshot_levels_committed: 8,
            resync_snapshots: 1,
            delta_batch_attempts: 2,
            delta_batches_committed: 1,
            delta_changes_committed: 5,
            delta_top_checks: 1,
            delta_top_checks_confirmed: 1,
            top_checks: 0,
            top_checks_confirmed: 0,
            freshness_checks: 0,
            freshness_confirmed: 0,
            tick_size_changes: 0,
            external_faults: 0,
            duplicate_ingress: 0,
            reordered_ingress: 0,
            disconnects: 0,
            heartbeat_timeouts: 0,
            backlog_aged_faults: 0,
            gaps: 0,
            overflows: 0,
            hash_mismatches: 0,
            bbo_mismatches: 0,
            invalid_transitions: 0,
            clock_regressions: 0,
            invalidations: 1,
            unavailable_transitions: 1,
            stale_invalidations: 0,
        }
    );
}

#[test]
fn delta_supplied_top_is_checked_before_the_atomic_candidate_commits() {
    let mut reducer = bootstrap(PmBookFreshness::new(1_000, 1_000).unwrap());
    let before = reducer.levels().to_vec();
    let mismatch = delta_with_top(
        vec![level(PmBookSide::Bid, 500_000, "50")],
        400_000,
        600_000,
    )
    .unwrap();

    assert_eq!(
        reducer
            .apply_delta_batch(ev(1, 1, 10, 2, 120), &mismatch)
            .unwrap_err(),
        PmPublicReadinessReason::BboMismatch
    );
    assert_eq!(reducer.levels(), before);
    assert_eq!(reducer.counters().delta_top_checks, 1);
    assert_eq!(reducer.counters().delta_top_checks_confirmed, 0);
    assert_eq!(reducer.counters().bbo_mismatches, 1);
    assert!(!reducer.readiness().is_ready());
}

#[test]
fn invalid_delta_classes_preserve_the_committed_book_and_invalidate_readiness() {
    let cases = [
        (
            delta_from(vec![]).unwrap(),
            PmPublicReadinessReason::InvalidTransition,
        ),
        (
            delta_from(vec![delete(PmBookSide::Bid, 300_000)]).unwrap(),
            PmPublicReadinessReason::MissingDeleteLevel,
        ),
        (
            delta_from(vec![level(PmBookSide::Bid, 650_000, "1")]).unwrap(),
            PmPublicReadinessReason::CrossedBook,
        ),
        (
            delta_from(vec![
                delete(PmBookSide::Ask, 600_000),
                delete(PmBookSide::Ask, 700_000),
            ])
            .unwrap(),
            PmPublicReadinessReason::EmptyBook,
        ),
        (
            delta_from(vec![level(PmBookSide::Bid, 450_050, "1")]).unwrap(),
            PmPublicReadinessReason::PriceOffTick,
        ),
    ];
    for (batch, expected) in cases {
        let mut reducer = bootstrap(PmBookFreshness::new(1_000, 1_000).unwrap());
        let before = reducer.levels().to_vec();
        assert_eq!(
            reducer
                .apply_delta_batch(ev(1, 1, 10, 2, 120), &batch)
                .unwrap_err(),
            expected
        );
        assert_eq!(reducer.levels(), before);
        assert_eq!(reducer.readiness().reason(), Some(expected));
        assert_eq!(reducer.counters().delta_batch_attempts, 1);
        assert_eq!(reducer.counters().delta_batches_committed, 0);
        assert_eq!(reducer.counters().unavailable_transitions, 1);
    }
}

#[test]
fn delta_before_snapshot_and_evidence_mismatches_fail_closed() {
    let mut before_snapshot = reducer_with_freshness(1_000, 1_000);
    before_snapshot
        .apply_metadata(base_observation(1, 100))
        .unwrap();
    before_snapshot
        .begin_epoch(ConnectionEpoch::new(1))
        .unwrap();
    let delta = delta_from(vec![level(PmBookSide::Bid, 500_000, "1")]).unwrap();
    assert_eq!(
        before_snapshot
            .apply_delta_batch(ev(1, 1, 10, 1, 110), &delta)
            .unwrap_err(),
        PmPublicReadinessReason::SnapshotMissing
    );

    let mut wrong_epoch = bootstrap(PmBookFreshness::new(1_000, 1_000).unwrap());
    assert_eq!(
        wrong_epoch
            .apply_delta_batch(ev(2, 1, 10, 2, 120), &delta)
            .unwrap_err(),
        PmPublicReadinessReason::ConnectionEpochMismatch
    );

    let mut wrong_metadata = bootstrap(PmBookFreshness::new(1_000, 1_000).unwrap());
    assert_eq!(
        wrong_metadata
            .apply_delta_batch(ev(1, 2, 10, 2, 120), &delta)
            .unwrap_err(),
        PmPublicReadinessReason::MetadataRevisionMismatch
    );

    let mut wrong_snapshot = bootstrap(PmBookFreshness::new(1_000, 1_000).unwrap());
    assert_eq!(
        wrong_snapshot
            .apply_delta_batch(ev(1, 1, 11, 2, 120), &delta)
            .unwrap_err(),
        PmPublicReadinessReason::SnapshotRevisionMismatch
    );

    let mut wrong_instrument = bootstrap(PmBookFreshness::new(1_000, 1_000).unwrap());
    assert_eq!(
        wrong_instrument
            .apply_delta_batch(evidence(OTHER_INSTRUMENT, 1, 1, 10, 2, 120, None), &delta)
            .unwrap_err(),
        PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::Instrument)
    );
}

#[test]
fn snapshot_checksum_is_mandatory_sha1_and_singular_updates_cannot_invent_one() {
    let mut missing = reducer_with_freshness(1_000, 1_000);
    missing.apply_metadata(base_observation(1, 100)).unwrap();
    missing.begin_epoch(ConnectionEpoch::new(1)).unwrap();
    assert_eq!(
        missing
            .apply_snapshot(ev(1, 1, 10, 1, 110), &base_snapshot())
            .unwrap_err(),
        PmPublicReadinessReason::HashMismatch
    );
    assert_eq!(missing.counters().hash_mismatches, 1);
    assert_eq!(missing.last_verified_snapshot_hash(), None);

    let mut wrong_algorithm = reducer_with_freshness(1_000, 1_000);
    wrong_algorithm
        .apply_metadata(base_observation(1, 100))
        .unwrap();
    wrong_algorithm
        .begin_epoch(ConnectionEpoch::new(1))
        .unwrap();
    assert_eq!(
        wrong_algorithm
            .apply_snapshot(
                evidence(
                    INSTRUMENT,
                    1,
                    1,
                    10,
                    1,
                    110,
                    Some(VenueEventHash::sha256([0xa5; 32]).unwrap())
                ),
                &base_snapshot()
            )
            .unwrap_err(),
        PmPublicReadinessReason::HashMismatch
    );
    assert_eq!(wrong_algorithm.counters().hash_mismatches, 1);
    assert_eq!(wrong_algorithm.last_verified_snapshot_hash(), None);

    let mut synthetic_delta_hash = bootstrap(PmBookFreshness::new(1_000, 1_000).unwrap());
    let retained = synthetic_delta_hash.last_verified_snapshot_hash();
    let update = delta_from(vec![level(PmBookSide::Bid, 500_000, "50")]).unwrap();
    assert_eq!(
        synthetic_delta_hash
            .apply_delta_batch(
                evidence(
                    INSTRUMENT,
                    1,
                    1,
                    10,
                    2,
                    120,
                    Some(VenueEventHash::sha1([0x99; 20]).unwrap())
                ),
                &update
            )
            .unwrap_err(),
        PmPublicReadinessReason::HashMismatch
    );
    assert_eq!(synthetic_delta_hash.last_verified_snapshot_hash(), retained);
    assert_eq!(synthetic_delta_hash.counters().hash_mismatches, 1);
}

#[test]
fn local_ingress_jumps_are_allowed_but_duplicate_and_reordered_values_invalidate() {
    let no_op_update = delta_from(vec![level(PmBookSide::Bid, 500_000, "55")]).unwrap();
    let mut reducer = bootstrap(PmBookFreshness::new(1_000, 1_000).unwrap());

    // Local ingress is only a reducer-order check. A jump is not evidence of a venue gap.
    reducer
        .apply_delta_batch(ev(1, 1, 10, 50, 120), &no_op_update)
        .unwrap();
    assert_eq!(reducer.counters().gaps, 0);

    assert_eq!(
        reducer
            .apply_delta_batch(ev(1, 1, 10, 50, 121), &no_op_update)
            .unwrap_err(),
        PmPublicReadinessReason::DuplicateIngress
    );
    assert_eq!(reducer.counters().duplicate_ingress, 1);

    reducer
        .apply_snapshot(snapshot_ev(1, 1, 11, 51, 130), &base_snapshot())
        .unwrap();
    assert_eq!(
        reducer
            .apply_delta_batch(ev(1, 1, 11, 49, 131), &no_op_update)
            .unwrap_err(),
        PmPublicReadinessReason::ReorderedIngress
    );
    assert_eq!(reducer.counters().reordered_ingress, 1);
}

#[test]
fn pure_reducer_faults_require_newer_snapshot_and_reconnect_faults_require_next_epoch() {
    let fault_cases = [
        (PmExternalBookFault::Gap, PmPublicReadinessReason::Gap),
        (
            PmExternalBookFault::Overflow,
            PmPublicReadinessReason::Overflow,
        ),
        (
            PmExternalBookFault::BacklogAged,
            PmPublicReadinessReason::BookStale,
        ),
        (
            PmExternalBookFault::InvalidTransition,
            PmPublicReadinessReason::InvalidTransition,
        ),
        (
            PmExternalBookFault::HashMismatch,
            PmPublicReadinessReason::HashMismatch,
        ),
    ];
    for (fault, expected) in fault_cases {
        let mut reducer = bootstrap(PmBookFreshness::new(1_000, 1_000).unwrap());
        let before = reducer.levels().to_vec();
        assert_eq!(
            reducer
                .apply_external_fault(ConnectionEpoch::new(1), fault)
                .unwrap_err(),
            expected
        );
        assert_eq!(reducer.levels(), before);
        assert!(!reducer.readiness().is_ready());
        // The pure reducer permits an owning composition to resynchronize.
        // A capture owner can still classify faults as artifact-terminal when
        // continuing would make verified replay diverge from the live path.
        reducer
            .apply_snapshot(snapshot_ev(1, 1, 11, 2, 120), &base_snapshot())
            .unwrap();
        assert!(reducer.readiness().is_ready());
        assert_eq!(reducer.counters().external_faults, 1);
        assert_eq!(reducer.counters().resync_snapshots, 1);
        assert_eq!(
            reducer.counters().stale_invalidations,
            u64::from(fault == PmExternalBookFault::BacklogAged)
        );
        assert_eq!(
            reducer.counters().backlog_aged_faults,
            u64::from(fault == PmExternalBookFault::BacklogAged)
        );
        assert_eq!(reducer.readiness().reason(), None);
    }

    let mut disconnected = bootstrap(PmBookFreshness::new(1_000, 1_000).unwrap());
    assert_eq!(
        disconnected
            .apply_external_fault(ConnectionEpoch::new(1), PmExternalBookFault::Disconnect)
            .unwrap_err(),
        PmPublicReadinessReason::Disconnected
    );
    assert_eq!(
        disconnected
            .apply_snapshot(snapshot_ev(1, 1, 11, 2, 120), &base_snapshot())
            .unwrap_err(),
        PmPublicReadinessReason::ConnectionEpochMismatch
    );
    assert_eq!(
        disconnected
            .begin_epoch(ConnectionEpoch::new(3))
            .unwrap_err(),
        PmPublicReadinessReason::ConnectionEpochInvalid
    );

    let mut heartbeat = bootstrap(PmBookFreshness::new(1_000, 1_000).unwrap());
    assert_eq!(
        heartbeat
            .apply_external_fault(
                ConnectionEpoch::new(1),
                PmExternalBookFault::HeartbeatTimeout,
            )
            .unwrap_err(),
        PmPublicReadinessReason::HeartbeatTimeout
    );
    assert_eq!(
        heartbeat.readiness().reason(),
        Some(PmPublicReadinessReason::HeartbeatTimeout)
    );
    assert_eq!(heartbeat.counters().external_faults, 1);
    assert_eq!(heartbeat.counters().heartbeat_timeouts, 1);
    assert_eq!(heartbeat.counters().backlog_aged_faults, 0);
    assert_eq!(heartbeat.counters().disconnects, 0);
    assert_eq!(
        heartbeat
            .apply_snapshot(snapshot_ev(1, 1, 11, 2, 120), &base_snapshot())
            .unwrap_err(),
        PmPublicReadinessReason::ConnectionEpochMismatch
    );
    heartbeat.begin_epoch(ConnectionEpoch::new(2)).unwrap();
    assert_eq!(
        heartbeat.readiness().reason(),
        Some(PmPublicReadinessReason::SnapshotMissing)
    );
    assert_eq!(heartbeat.counters().reconnects, 1);
    heartbeat
        .apply_snapshot(snapshot_ev(2, 1, 11, 1, 130), &base_snapshot())
        .unwrap();
    assert!(heartbeat.readiness().is_ready());
    assert_eq!(heartbeat.counters().external_faults, 1);
    assert_eq!(heartbeat.counters().heartbeat_timeouts, 1);
    assert_eq!(heartbeat.counters().reconnects, 1);
    disconnected.begin_epoch(ConnectionEpoch::new(2)).unwrap();
    disconnected
        .apply_snapshot(snapshot_ev(2, 1, 11, 1, 130), &base_snapshot())
        .unwrap();
    assert!(disconnected.readiness().is_ready());
    assert_eq!(disconnected.counters().disconnects, 1);
    assert_eq!(disconnected.counters().reconnects, 1);
}

#[test]
fn pending_lane_faults_cover_initial_and_reconnect_pre_snapshot_state_without_false_availability() {
    for (fault, pending_reason, final_reason) in [
        (
            PmExternalBookFault::Overflow,
            PmPublicReadinessReason::PendingOverflow,
            PmPublicReadinessReason::Overflow,
        ),
        (
            PmExternalBookFault::BacklogAged,
            PmPublicReadinessReason::PendingBookStale,
            PmPublicReadinessReason::BookStale,
        ),
    ] {
        let mut reducer = reducer_with_freshness(1_000, 1_000);
        reducer.apply_metadata(base_observation(1, 100)).unwrap();
        reducer.begin_epoch(ConnectionEpoch::new(1)).unwrap();
        assert!(reducer.is_pristine_pre_snapshot());
        let before = reducer.counters();

        let authority = reducer
            .begin_pending_external_fault(ConnectionEpoch::new(1), fault)
            .unwrap();
        assert_eq!(reducer.readiness().reason(), Some(pending_reason));
        assert_eq!(reducer.pending_external_fault(), Some(fault));
        assert_eq!(
            reducer.counters(),
            before,
            "an already-unavailable pre-snapshot reducer must not invent another unavailable transition"
        );

        assert_eq!(
            reducer
                .finalize_pending_external_fault(&authority, fault)
                .unwrap_err(),
            final_reason
        );
        let after = reducer.counters();
        assert_eq!(after.external_faults, before.external_faults + 1);
        assert_eq!(after.invalidations, before.invalidations + 1);
        assert_eq!(
            after.unavailable_transitions,
            before.unavailable_transitions
        );
        assert_eq!(
            after.overflows,
            before.overflows + u64::from(fault == PmExternalBookFault::Overflow)
        );
        assert_eq!(
            after.backlog_aged_faults,
            before.backlog_aged_faults + u64::from(fault == PmExternalBookFault::BacklogAged)
        );
        assert_eq!(
            after.stale_invalidations,
            before.stale_invalidations + u64::from(fault == PmExternalBookFault::BacklogAged)
        );
        assert_eq!(reducer.pending_external_fault(), None);

        reducer.begin_epoch(ConnectionEpoch::new(2)).unwrap();
        reducer
            .apply_snapshot(snapshot_ev(2, 1, 10, 1, 110), &base_snapshot())
            .unwrap();
        assert!(reducer.readiness().is_ready());
    }

    let mut reconnected = bootstrap(PmBookFreshness::new(1_000, 1_000).unwrap());
    reconnected
        .apply_external_fault(ConnectionEpoch::new(1), PmExternalBookFault::Disconnect)
        .unwrap_err();
    reconnected.begin_epoch(ConnectionEpoch::new(2)).unwrap();
    assert_eq!(
        reconnected.readiness().reason(),
        Some(PmPublicReadinessReason::SnapshotMissing)
    );
    let before = reconnected.counters();
    let authority = reconnected
        .begin_pending_external_fault(ConnectionEpoch::new(2), PmExternalBookFault::Overflow)
        .unwrap();
    assert_eq!(reconnected.counters(), before);
    assert_eq!(
        reconnected
            .finalize_pending_external_fault(&authority, PmExternalBookFault::Overflow)
            .unwrap_err(),
        PmPublicReadinessReason::Overflow
    );
    assert_eq!(
        reconnected.counters().unavailable_transitions,
        before.unavailable_transitions
    );
    reconnected.begin_epoch(ConnectionEpoch::new(3)).unwrap();
    reconnected
        .apply_snapshot(snapshot_ev(3, 1, 11, 1, 130), &base_snapshot())
        .unwrap();
    assert!(reconnected.readiness().is_ready());

    let mut metadata_stale = bootstrap(PmBookFreshness::new(20, 1_000).unwrap());
    assert_eq!(
        metadata_stale.check_freshness(121).unwrap_err(),
        PmPublicReadinessReason::MetadataStale
    );
    let before = metadata_stale.counters();
    let authority = metadata_stale
        .begin_pending_external_fault(ConnectionEpoch::new(1), PmExternalBookFault::BacklogAged)
        .unwrap();
    assert_eq!(metadata_stale.counters(), before);
    assert_eq!(
        metadata_stale
            .finalize_pending_external_fault(&authority, PmExternalBookFault::BacklogAged)
            .unwrap_err(),
        PmPublicReadinessReason::BookStale
    );
    assert_eq!(
        metadata_stale.counters().unavailable_transitions,
        before.unavailable_transitions
    );
    metadata_stale.begin_epoch(ConnectionEpoch::new(2)).unwrap();
    assert_eq!(
        metadata_stale
            .apply_snapshot(snapshot_ev(2, 1, 11, 1, 130), &base_snapshot())
            .unwrap_err(),
        PmPublicReadinessReason::BookStale,
        "an aged-lane reason cannot restore metadata validity"
    );
    metadata_stale
        .apply_metadata(base_observation(2, 131))
        .unwrap();
    metadata_stale
        .apply_snapshot(snapshot_ev(2, 2, 11, 1, 132), &base_snapshot())
        .unwrap();
    assert!(metadata_stale.readiness().is_ready());
}

#[test]
fn snapshot_revision_must_increase_across_resync_metadata_refresh_and_reconnect() {
    let mut reducer = bootstrap(PmBookFreshness::new(1_000, 1_000).unwrap());
    reducer
        .apply_external_fault(ConnectionEpoch::new(1), PmExternalBookFault::Gap)
        .unwrap_err();
    assert_eq!(
        reducer
            .apply_snapshot(snapshot_ev(1, 1, 10, 2, 120), &base_snapshot())
            .unwrap_err(),
        PmPublicReadinessReason::SnapshotRevisionMismatch
    );
    reducer
        .apply_snapshot(snapshot_ev(1, 1, 11, 3, 121), &base_snapshot())
        .unwrap();

    reducer.apply_metadata(base_observation(2, 130)).unwrap();
    assert_eq!(
        reducer
            .apply_snapshot(snapshot_ev(1, 2, 11, 4, 131), &base_snapshot())
            .unwrap_err(),
        PmPublicReadinessReason::SnapshotRevisionMismatch
    );
    reducer
        .apply_snapshot(snapshot_ev(1, 2, 12, 5, 132), &base_snapshot())
        .unwrap();

    reducer
        .apply_external_fault(ConnectionEpoch::new(1), PmExternalBookFault::Disconnect)
        .unwrap_err();
    reducer.begin_epoch(ConnectionEpoch::new(2)).unwrap();
    assert_eq!(
        reducer
            .apply_snapshot(snapshot_ev(2, 2, 12, 1, 140), &base_snapshot())
            .unwrap_err(),
        PmPublicReadinessReason::SnapshotRevisionMismatch
    );
    reducer
        .apply_snapshot(snapshot_ev(2, 2, 13, 2, 141), &base_snapshot())
        .unwrap();
    assert!(reducer.readiness().is_ready());
}

#[test]
fn bbo_check_is_exact_and_does_not_refresh_book_freshness() {
    let mut reducer = bootstrap(PmBookFreshness::new(10_000, 20).unwrap());
    assert_eq!(
        reducer.check_top(
            ev(1, 1, 10, 2, 125),
            PmBookTopCheck::new(
                Some(PmPrice::from_units(500_000).unwrap()),
                Some(PmPrice::from_units(600_000).unwrap())
            )
        ),
        Ok(PmBookTransition::TopConfirmed)
    );
    assert_eq!(
        reducer.check_freshness(130),
        Ok(PmBookTransition::FreshnessConfirmed)
    );
    assert_eq!(
        reducer.check_freshness(131).unwrap_err(),
        PmPublicReadinessReason::BookStale
    );
    assert_eq!(reducer.counters().top_checks, 1);
    assert_eq!(reducer.counters().top_checks_confirmed, 1);
    assert_eq!(reducer.counters().freshness_checks, 2);
    assert_eq!(reducer.counters().freshness_confirmed, 1);
    assert_eq!(reducer.counters().stale_invalidations, 1);
    assert_eq!(reducer.counters().backlog_aged_faults, 0);

    let mut mismatch = bootstrap(PmBookFreshness::new(1_000, 1_000).unwrap());
    let before = mismatch.levels().to_vec();
    assert_eq!(
        mismatch
            .check_top(
                ev(1, 1, 10, 2, 120),
                PmBookTopCheck::new(
                    Some(PmPrice::from_units(400_000).unwrap()),
                    Some(PmPrice::from_units(600_000).unwrap())
                )
            )
            .unwrap_err(),
        PmPublicReadinessReason::BboMismatch
    );
    assert_eq!(mismatch.levels(), before);
    assert_eq!(mismatch.counters().bbo_mismatches, 1);
    assert_eq!(mismatch.counters().unavailable_transitions, 1);
}

#[test]
fn stale_metadata_requires_fresh_authority_and_snapshot_while_clock_regression_is_typed() {
    let mut reducer = bootstrap(PmBookFreshness::new(20, 1_000).unwrap());
    assert_eq!(
        reducer.check_freshness(121).unwrap_err(),
        PmPublicReadinessReason::MetadataStale
    );
    assert_eq!(
        reducer
            .apply_snapshot(snapshot_ev(1, 1, 11, 2, 122), &base_snapshot())
            .unwrap_err(),
        PmPublicReadinessReason::MetadataStale
    );
    reducer.apply_metadata(base_observation(2, 130)).unwrap();
    assert_eq!(
        reducer.readiness().reason(),
        Some(PmPublicReadinessReason::SnapshotMissing)
    );
    reducer
        .apply_snapshot(snapshot_ev(1, 2, 11, 2, 131), &base_snapshot())
        .unwrap();
    assert!(reducer.readiness().is_ready());

    let mut regression = bootstrap(PmBookFreshness::new(1_000, 1_000).unwrap());
    assert_eq!(
        regression.check_freshness(109).unwrap_err(),
        PmPublicReadinessReason::ClockRegression
    );
    assert_eq!(regression.counters().clock_regressions, 1);

    let mut ingress_regression = bootstrap(PmBookFreshness::new(1_000, 1_000).unwrap());
    let update = delta_from(vec![level(PmBookSide::Bid, 500_000, "50")]).unwrap();
    assert_eq!(
        ingress_regression
            .apply_delta_batch(ev(1, 1, 10, 2, 109), &update)
            .unwrap_err(),
        PmPublicReadinessReason::ClockRegression
    );
    assert_eq!(ingress_regression.counters().clock_regressions, 1);
}

#[test]
fn tick_change_is_ordered_evidence_and_grid_drift_needs_new_metadata_and_snapshot() {
    let mut reducer = bootstrap(PmBookFreshness::new(1_000, 1_000).unwrap());
    let old = PmTick::from_units(100).unwrap();
    let new = PmTick::from_units(1_000).unwrap();
    assert_eq!(
        reducer
            .tick_size_changed(ev(1, 1, 10, 2, 120), old, new)
            .unwrap_err(),
        PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::Grid)
    );
    assert_eq!(reducer.counters().tick_size_changes, 1);
    assert_eq!(reducer.counters().metadata_rejected, 1);
    assert_eq!(
        reducer.readiness().reason(),
        Some(PmPublicReadinessReason::MetadataDrift(
            PmMetadataDrift::Grid
        ))
    );

    // The configured contract is intentionally immutable. A new observation that
    // still matches it is required before a strictly newer resync snapshot.
    reducer.apply_metadata(base_observation(2, 130)).unwrap();
    reducer
        .apply_snapshot(snapshot_ev(1, 2, 11, 3, 131), &base_snapshot())
        .unwrap();
    assert!(reducer.readiness().is_ready());

    let mut wrong_epoch = bootstrap(PmBookFreshness::new(1_000, 1_000).unwrap());
    assert_eq!(
        wrong_epoch
            .tick_size_changed(ev(2, 1, 10, 2, 120), old, new)
            .unwrap_err(),
        PmPublicReadinessReason::ConnectionEpochMismatch
    );
    assert_eq!(wrong_epoch.counters().tick_size_changes, 0);
}

#[test]
fn exact_level_bound_and_full_capacity_atomic_replacement_are_supported() {
    let mut levels = Vec::with_capacity(usize::from(MAX_PM_BOOK_LEVELS));
    for index in 0..1_024_u32 {
        levels.push(level(PmBookSide::Ask, 800_000 + index * 100, "1"));
        levels.push(level(PmBookSide::Bid, 100 + index * 100, "1"));
    }
    let snapshot = snapshot_from(levels).unwrap();

    let mut reducer = reducer_with_freshness(1_000, 1_000);
    reducer.apply_metadata(base_observation(1, 100)).unwrap();
    reducer.begin_epoch(ConnectionEpoch::new(1)).unwrap();
    reducer
        .apply_snapshot(snapshot_ev(1, 1, 10, 1, 110), &snapshot)
        .unwrap();
    assert_eq!(reducer.levels().len(), usize::from(MAX_PM_BOOK_LEVELS));

    // Venue serialization order is not mutation order: the reducer applies the
    // delete first internally, so replacing one level at capacity remains valid.
    let replacement = delta_with_top(
        vec![
            level(PmBookSide::Bid, 200_000, "2"),
            delete(PmBookSide::Bid, 100),
        ],
        200_000,
        800_000,
    )
    .unwrap();
    assert_eq!(
        reducer.apply_update(ev(1, 1, 10, 2, 120), &PmBookUpdate::DeltaBatch(replacement)),
        Ok(PmBookTransition::DeltaBatchCommitted {
            revision: SnapshotRevision::new(10),
            changes: 2
        })
    );
    assert_eq!(reducer.levels().len(), usize::from(MAX_PM_BOOK_LEVELS));
    assert!(
        reducer
            .levels()
            .iter()
            .any(|candidate| *candidate == level(PmBookSide::Bid, 200_000, "2"))
    );
    assert!(
        reducer
            .levels()
            .iter()
            .all(|candidate| *candidate != level(PmBookSide::Bid, 100, "1"))
    );
    assert_eq!(reducer.counters().snapshot_levels_committed, 2_048);
    assert_eq!(reducer.counters().delta_changes_committed, 2);

    let too_many = vec![level(PmBookSide::Bid, 100, "1"); usize::from(MAX_PM_BOOK_LEVELS) + 1];
    assert_eq!(
        snapshot_from(too_many.clone()),
        Err(PmEventError::TooManyBookLevels)
    );
    assert_eq!(delta_from(too_many), Err(PmEventError::TooManyBookLevels));
}
