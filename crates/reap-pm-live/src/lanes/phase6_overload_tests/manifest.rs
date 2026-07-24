use crate::capture::{
    MAX_PM_PUBLIC_CAPTURE_PENDING_AGE_NS, MAX_PM_PUBLIC_CAPTURE_RAW_FRAMES,
    MAX_PM_PUBLIC_CAPTURE_RAW_PAYLOAD_BYTES, MAX_PM_RAW_PUBLIC_FRAME_BYTES,
};
use crate::{PmLaneKind, PmLanePolicy, SaturationAction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EvidencePath {
    ReachedProduct,
    CratePrivateMechanism,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OverloadCaseId {
    PublicIntegrity,
    PrivateLifecycle,
    Critical,
    PersistenceAcknowledgements,
    CompleteSnapshots,
    ReconciliationRefreshEffects,
    RawCaptureEntries,
    RawCaptureBytes,
    OversizeRawFrame,
    Storage,
    FakeEffect,
    ScheduledActions,
    Telemetry,
}

impl OverloadCaseId {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::PublicIntegrity => "public_integrity",
            Self::PrivateLifecycle => "private_lifecycle",
            Self::Critical => "critical",
            Self::PersistenceAcknowledgements => "persistence_acknowledgements",
            Self::CompleteSnapshots => "complete_snapshots",
            Self::ReconciliationRefreshEffects => "reconciliation_refresh_effects",
            Self::RawCaptureEntries => "raw_capture_entries",
            Self::RawCaptureBytes => "raw_capture_bytes",
            Self::OversizeRawFrame => "oversize_raw_frame",
            Self::Storage => "storage",
            Self::FakeEffect => "fake_effect",
            Self::ScheduledActions => "scheduled_actions",
            Self::Telemetry => "telemetry",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ExpectedOverloadOutcome {
    attempts: usize,
    retained: usize,
    rejected: usize,
    coalesced: usize,
    fail_closed_transitions: usize,
}

impl ExpectedOverloadOutcome {
    const fn bounded(capacity: usize) -> Self {
        Self {
            attempts: capacity + 1,
            retained: capacity,
            rejected: 1,
            coalesced: 0,
            fail_closed_transitions: 1,
        }
    }

    const fn raw_bytes() -> Self {
        Self {
            attempts: 33,
            retained: 32,
            rejected: 1,
            coalesced: 0,
            fail_closed_transitions: 1,
        }
    }

    const fn oversize_raw() -> Self {
        Self {
            attempts: 1,
            retained: 0,
            rejected: 1,
            coalesced: 0,
            fail_closed_transitions: 1,
        }
    }

    const fn telemetry() -> Self {
        Self {
            attempts: 129,
            retained: 128,
            rejected: 0,
            coalesced: 1,
            fail_closed_transitions: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PolicySource {
    Lane(PmLaneKind),
    RawByteSlab,
    RawFrameBound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OverloadCase {
    id: OverloadCaseId,
    path: EvidencePath,
    expected: ExpectedOverloadOutcome,
    source: PolicySource,
}

pub(super) const OVERLOAD_CASES: [OverloadCase; 13] = [
    reached(
        OverloadCaseId::PublicIntegrity,
        ExpectedOverloadOutcome::bounded(8_192),
        PolicySource::Lane(PmLaneKind::Public),
    ),
    reached(
        OverloadCaseId::PrivateLifecycle,
        ExpectedOverloadOutcome::bounded(4_096),
        PolicySource::Lane(PmLaneKind::Private),
    ),
    reached(
        OverloadCaseId::Critical,
        ExpectedOverloadOutcome::bounded(512),
        PolicySource::Lane(PmLaneKind::Critical),
    ),
    reached(
        OverloadCaseId::PersistenceAcknowledgements,
        ExpectedOverloadOutcome::bounded(512),
        PolicySource::Lane(PmLaneKind::Persistence),
    ),
    reached(
        OverloadCaseId::CompleteSnapshots,
        ExpectedOverloadOutcome::bounded(128),
        PolicySource::Lane(PmLaneKind::Reconciliation),
    ),
    mechanism(
        OverloadCaseId::ReconciliationRefreshEffects,
        ExpectedOverloadOutcome::bounded(128),
        PolicySource::Lane(PmLaneKind::ReconciliationRequest),
    ),
    mechanism(
        OverloadCaseId::RawCaptureEntries,
        ExpectedOverloadOutcome::bounded(8_192),
        PolicySource::Lane(PmLaneKind::Capture),
    ),
    reached(
        OverloadCaseId::RawCaptureBytes,
        ExpectedOverloadOutcome::raw_bytes(),
        PolicySource::RawByteSlab,
    ),
    reached(
        OverloadCaseId::OversizeRawFrame,
        ExpectedOverloadOutcome::oversize_raw(),
        PolicySource::RawFrameBound,
    ),
    reached(
        OverloadCaseId::Storage,
        ExpectedOverloadOutcome::bounded(1_024),
        PolicySource::Lane(PmLaneKind::Journal),
    ),
    mechanism(
        OverloadCaseId::FakeEffect,
        ExpectedOverloadOutcome::bounded(256),
        PolicySource::Lane(PmLaneKind::FakeEffect),
    ),
    mechanism(
        OverloadCaseId::ScheduledActions,
        ExpectedOverloadOutcome::bounded(4_096),
        PolicySource::Lane(PmLaneKind::Scheduled),
    ),
    reached(
        OverloadCaseId::Telemetry,
        ExpectedOverloadOutcome::telemetry(),
        PolicySource::Lane(PmLaneKind::Telemetry),
    ),
];

pub(super) const PM_LIVE_DIRECT_MECHANISM_CASES: [OverloadCaseId; 13] = [
    OverloadCaseId::PublicIntegrity,
    OverloadCaseId::PrivateLifecycle,
    OverloadCaseId::Critical,
    OverloadCaseId::PersistenceAcknowledgements,
    OverloadCaseId::CompleteSnapshots,
    OverloadCaseId::ReconciliationRefreshEffects,
    OverloadCaseId::RawCaptureEntries,
    OverloadCaseId::RawCaptureBytes,
    OverloadCaseId::OversizeRawFrame,
    OverloadCaseId::Storage,
    OverloadCaseId::FakeEffect,
    OverloadCaseId::ScheduledActions,
    OverloadCaseId::Telemetry,
];

const fn reached(
    id: OverloadCaseId,
    expected: ExpectedOverloadOutcome,
    source: PolicySource,
) -> OverloadCase {
    OverloadCase {
        id,
        path: EvidencePath::ReachedProduct,
        expected,
        source,
    }
}

const fn mechanism(
    id: OverloadCaseId,
    expected: ExpectedOverloadOutcome,
    source: PolicySource,
) -> OverloadCase {
    OverloadCase {
        id,
        path: EvidencePath::CratePrivateMechanism,
        expected,
        source,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DirectMechanismObservation {
    id: OverloadCaseId,
    attempts: usize,
    retained: usize,
    rejected: usize,
    coalesced: usize,
    fail_closed_transitions: usize,
}

impl DirectMechanismObservation {
    pub(super) const fn new(
        id: OverloadCaseId,
        attempts: usize,
        retained: usize,
        rejected: usize,
        coalesced: usize,
        fail_closed_transitions: usize,
    ) -> Self {
        Self {
            id,
            attempts,
            retained,
            rejected,
            coalesced,
            fail_closed_transitions,
        }
    }
}

pub(super) fn assert_exact_pm_live_direct_mechanism_observations(
    observed: &[DirectMechanismObservation],
) {
    assert_eq!(
        observed.len(),
        PM_LIVE_DIRECT_MECHANISM_CASES.len(),
        "every registered PM-live mechanism must execute once"
    );
    for (observation, expected_id) in observed.iter().zip(PM_LIVE_DIRECT_MECHANISM_CASES) {
        assert_eq!(
            observation.id, expected_id,
            "the executable registry must retain manifest order"
        );
        let case = case(expected_id);
        assert_eq!(
            (
                observation.attempts,
                observation.retained,
                observation.rejected,
                observation.coalesced,
                observation.fail_closed_transitions,
            ),
            (
                case.expected.attempts,
                case.expected.retained,
                case.expected.rejected,
                case.expected.coalesced,
                case.expected.fail_closed_transitions,
            ),
            "{} direct mechanism result differs from its typed contract",
            expected_id.name()
        );
        assert_eq!(
            observation.attempts,
            observation.retained + observation.rejected + observation.coalesced,
            "{} did not account for every executed attempt",
            expected_id.name()
        );
    }
    assert_eq!(
        observed
            .iter()
            .map(|observation| observation.attempts)
            .sum::<usize>(),
        27_309
    );
    assert_eq!(
        observed
            .iter()
            .map(|observation| observation.fail_closed_transitions)
            .sum::<usize>(),
        12
    );
}

fn case(id: OverloadCaseId) -> OverloadCase {
    OVERLOAD_CASES
        .into_iter()
        .find(|case| case.id == id)
        .expect("every executable overload mechanism has one typed manifest row")
}

#[test]
fn typed_overload_manifest_is_exact_and_linked_to_source_policies() {
    assert_eq!(OVERLOAD_CASES.len(), 13);
    assert_eq!(
        OVERLOAD_CASES
            .iter()
            .map(|case| case.expected.attempts)
            .sum::<usize>(),
        27_309
    );
    assert_eq!(
        OVERLOAD_CASES
            .iter()
            .filter(|case| case.path == EvidencePath::ReachedProduct)
            .map(|case| case.expected.attempts)
            .sum::<usize>(),
        14_633
    );
    assert_eq!(
        OVERLOAD_CASES
            .iter()
            .filter(|case| case.path == EvidencePath::CratePrivateMechanism)
            .map(|case| case.expected.attempts)
            .sum::<usize>(),
        12_676
    );
    assert_eq!(
        OVERLOAD_CASES
            .iter()
            .filter(|case| case.path == EvidencePath::ReachedProduct)
            .count(),
        9
    );
    assert_eq!(
        OVERLOAD_CASES
            .iter()
            .filter(|case| case.path == EvidencePath::CratePrivateMechanism)
            .map(|case| case.id)
            .collect::<Vec<_>>(),
        [
            OverloadCaseId::ReconciliationRefreshEffects,
            OverloadCaseId::RawCaptureEntries,
            OverloadCaseId::FakeEffect,
            OverloadCaseId::ScheduledActions,
        ]
    );

    for case in OVERLOAD_CASES {
        assert_source_policy(case);
    }
}

/// Guards registry coverage only. The allocation window executes each exact
/// bounded primitive directly; it does not claim that a ProductRun wrapper is
/// itself inside that window.
#[test]
fn every_reached_case_has_an_executable_direct_allocation_mechanism() {
    let reached = OVERLOAD_CASES
        .iter()
        .filter(|case| case.path == EvidencePath::ReachedProduct)
        .map(|case| case.id)
        .collect::<Vec<_>>();
    assert_eq!(reached.len(), 9);
    for id in reached {
        assert!(
            PM_LIVE_DIRECT_MECHANISM_CASES.contains(&id),
            "{} reached case lost its executable direct allocation mechanism",
            id.name()
        );
    }
}

fn assert_source_policy(case: OverloadCase) {
    match case.source {
        PolicySource::Lane(lane) => {
            let policy = PmLanePolicy::for_lane(lane);
            assert_eq!(
                policy.capacity(),
                case.expected.retained,
                "{} capacity is not sourced from its declared lane policy",
                case.id.name()
            );
            assert_eq!(
                policy.saturation_action(),
                expected_action(case.id),
                "{} action is not sourced from its declared lane policy",
                case.id.name()
            );
            if lane == PmLaneKind::Capture {
                assert_eq!(
                    policy.maximum_age_ns(),
                    Some(MAX_PM_PUBLIC_CAPTURE_PENDING_AGE_NS),
                    "capture pending age must use the canonical capture-lane policy"
                );
            }
        }
        PolicySource::RawByteSlab => {
            assert_eq!(MAX_PM_PUBLIC_CAPTURE_RAW_PAYLOAD_BYTES, 32 * 1024 * 1024);
            assert_eq!(MAX_PM_RAW_PUBLIC_FRAME_BYTES, 1024 * 1024);
            assert_eq!(
                MAX_PM_PUBLIC_CAPTURE_RAW_PAYLOAD_BYTES
                    / u64::try_from(MAX_PM_RAW_PUBLIC_FRAME_BYTES)
                        .expect("fixed raw-frame bound fits u64"),
                u64::try_from(case.expected.retained).expect("fixed retained count fits u64")
            );
            assert_eq!(
                PmLanePolicy::for_lane(PmLaneKind::Capture).saturation_action(),
                expected_action(case.id)
            );
        }
        PolicySource::RawFrameBound => {
            assert_eq!(MAX_PM_RAW_PUBLIC_FRAME_BYTES, 1024 * 1024);
            assert_eq!(case.expected.attempts, 1);
            assert_eq!(case.expected.retained, 0);
            assert_eq!(
                PmLanePolicy::for_lane(PmLaneKind::Capture).saturation_action(),
                expected_action(case.id)
            );
        }
    }
    if case.id == OverloadCaseId::RawCaptureEntries {
        assert_eq!(
            MAX_PM_PUBLIC_CAPTURE_RAW_FRAMES,
            u64::try_from(case.expected.retained).expect("fixed retained count fits u64")
        );
    }
}

const fn expected_action(id: OverloadCaseId) -> SaturationAction {
    match id {
        OverloadCaseId::PublicIntegrity => SaturationAction::InvalidateStreamAndResync,
        OverloadCaseId::PrivateLifecycle => SaturationAction::HaltAccountAndRequireReconciliation,
        OverloadCaseId::Critical | OverloadCaseId::PersistenceAcknowledgements => {
            SaturationAction::GlobalStop
        }
        OverloadCaseId::CompleteSnapshots => SaturationAction::KeepUnreadyAndRetry,
        OverloadCaseId::ReconciliationRefreshEffects => SaturationAction::RetainPendingRefresh,
        OverloadCaseId::RawCaptureEntries
        | OverloadCaseId::RawCaptureBytes
        | OverloadCaseId::OversizeRawFrame => SaturationAction::InvalidateCaptureAndResync,
        OverloadCaseId::Storage => SaturationAction::SuppressDispatchAndHaltQuotes,
        OverloadCaseId::FakeEffect => SaturationAction::RejectEffectAndHaltQuotes,
        OverloadCaseId::ScheduledActions => SaturationAction::SuppressQuoteAndCancelOwned,
        OverloadCaseId::Telemetry => SaturationAction::CoalesceTelemetry,
    }
}
