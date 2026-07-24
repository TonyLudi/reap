use std::process::Command;

use super::manifest::{
    DirectMechanismObservation, OverloadCaseId, PM_LIVE_DIRECT_MECHANISM_CASES,
    assert_exact_pm_live_direct_mechanism_observations,
};
use super::support::{account_ingress, account_scope, instrument, internal_ingress};
use super::*;
use crate::capture::{MAX_PM_RAW_PUBLIC_FRAME_BYTES, Phase6RawCapacityProbe, PmCaptureVerifyError};
use crate::coordinator::{Phase6FakeEffectAllocationProbe, Phase6StorageAllocationProbe};
use crate::public_routes::PmPublicRouteAuthorityId;
use crate::schedule::{PmQuoteScheduleRole, PmScheduledActionKey, PmScheduledActionKind};
use reap_benchmark_allocator::{AllocationSnapshot, MeasurementWindow};
use reap_pm_core::{
    ConnectionEpoch, EventOrdering, IngressSequence, OkxReferenceEvent, OkxReferenceHandle,
    OkxReferencePrice, PmConnectionId, PmOrderSide, PmProductSource, PmSourceHandle,
    ReceivedEventClock,
};

mod refresh;
mod state_dense;

const CHILD_ENV: &str = "REAP_PHASE6_OVERLOAD_ALLOCATION_CHILD";
const EXACT_TEST: &str = "lanes::phase6_overload_tests::allocation::thirteen_pm_live_overload_mechanisms_are_allocation_free";

type DirectMechanismRunner = fn(&mut MeasurementWindow) -> DirectMechanismObservation;

#[derive(Clone, Copy)]
struct RegisteredDirectMechanism {
    id: OverloadCaseId,
    run: DirectMechanismRunner,
}

const DIRECT_MECHANISMS: [RegisteredDirectMechanism; 13] = [
    registered(OverloadCaseId::PublicIntegrity, measure_public_integrity),
    registered(OverloadCaseId::PrivateLifecycle, measure_private_lifecycle),
    registered(OverloadCaseId::Critical, measure_critical),
    registered(
        OverloadCaseId::PersistenceAcknowledgements,
        measure_persistence_acknowledgements,
    ),
    registered(
        OverloadCaseId::CompleteSnapshots,
        measure_complete_snapshots,
    ),
    registered(
        OverloadCaseId::ReconciliationRefreshEffects,
        refresh::measure_reconciliation_refresh,
    ),
    registered(OverloadCaseId::RawCaptureEntries, measure_raw_entries),
    registered(OverloadCaseId::RawCaptureBytes, measure_raw_bytes),
    registered(OverloadCaseId::OversizeRawFrame, measure_oversize_raw),
    registered(OverloadCaseId::Storage, measure_storage),
    registered(OverloadCaseId::FakeEffect, measure_fake_effect),
    registered(OverloadCaseId::ScheduledActions, measure_schedule),
    registered(OverloadCaseId::Telemetry, measure_telemetry),
];

const fn registered(id: OverloadCaseId, run: DirectMechanismRunner) -> RegisteredDirectMechanism {
    RegisteredDirectMechanism { id, run }
}

#[test]
fn thirteen_pm_live_overload_mechanisms_are_allocation_free() {
    if std::env::var_os(CHILD_ENV).is_none() {
        let status = Command::new(std::env::current_exe().expect("current lib-test executable"))
            .arg("--exact")
            .arg(EXACT_TEST)
            .arg("--test-threads=1")
            .env(CHILD_ENV, "1")
            .status()
            .expect("isolated Phase-6 allocation child starts");
        assert!(status.success(), "isolated allocation evidence failed");
        return;
    }

    run_isolated_allocation_evidence();
}

fn run_isolated_allocation_evidence() {
    let mut window =
        reap_benchmark_allocator::start_measurement().expect("exclusive allocation window");
    window.pause().expect("pause fixture construction");
    let mut observed = Vec::with_capacity(DIRECT_MECHANISMS.len());
    for (registered, expected_id) in DIRECT_MECHANISMS
        .into_iter()
        .zip(PM_LIVE_DIRECT_MECHANISM_CASES)
    {
        assert_eq!(
            registered.id, expected_id,
            "the executable direct-mechanism registry differs from the typed manifest"
        );
        observed.push((registered.run)(&mut window));
    }

    state_dense::measure_dense_state_indexes(&mut window);
    assert_exact_pm_live_direct_mechanism_observations(&observed);
    let terminal = window.stop().expect("stop paused allocation window");
    assert_eq!(terminal, AllocationSnapshot::default());
}

fn measure_case(window: &mut MeasurementWindow, case: OverloadCaseId, operation: impl FnOnce()) {
    let before = window.checkpoint().expect("paused pre-case checkpoint");
    window.resume().expect("resume owner-loop measurement");
    operation();
    let after = window.checkpoint().expect("active post-case checkpoint");
    window.pause().expect("pause after owner-loop measurement");

    assert_eq!(
        after.allocation_calls,
        before.allocation_calls,
        "{} allocated",
        case.name()
    );
    assert_eq!(
        after.allocated_bytes,
        before.allocated_bytes,
        "{} allocated bytes",
        case.name()
    );
    assert_eq!(
        after.deallocation_calls,
        before.deallocation_calls,
        "{} deallocated",
        case.name()
    );
    assert_eq!(
        after.deallocated_bytes,
        before.deallocated_bytes,
        "{} deallocated bytes",
        case.name()
    );
    assert_eq!(
        after.live_bytes_delta,
        before.live_bytes_delta,
        "{} changed live bytes",
        case.name()
    );
    assert_eq!(
        after.peak_live_bytes_delta,
        before.peak_live_bytes_delta,
        "{} changed peak live bytes",
        case.name()
    );
}

fn measure_public_integrity(window: &mut MeasurementWindow) -> DirectMechanismObservation {
    let mut lane = PmPublicLaneState::new();
    let reference = OkxReferenceEvent::new(
        PmProductSource::okx_reference(
            PmSourceHandle::from_ordinal(1),
            OkxReferenceHandle::from_ordinal(1),
        ),
        OkxReferenceHandle::from_ordinal(1),
        OkxReferencePrice::parse_decimal("50000.125").expect("fixed reference"),
    )
    .expect("fixed reference event");
    let connection = PmConnectionId::new("phase6-allocation").expect("fixed connection");
    let fixtures = (1..=8_193_u64)
        .map(|sequence| {
            let clock = ReceivedEventClock::new(None, 10_000 + sequence, sequence)
                .expect("fixed receive clock");
            let ordering = EventOrdering::new(
                ConnectionEpoch::new(1),
                None,
                None,
                None,
                IngressSequence::new(sequence),
            )
            .expect("fixed ordering");
            let ingress = PmIngressOrder::from_ordering(connection, ordering);
            let key = PmServiceKey::derived(clock, reference.source(), ingress, 5);
            (key, ingress, clock)
        })
        .collect::<Vec<_>>();
    let route = PmPublicRouteLaneEvidence {
        authority_id: PmPublicRouteAuthorityId::for_test(1),
        source: reference.source(),
        head: PmPublicAgedHead::OkxReference,
    };

    let mut attempts = 0;
    let mut retained = 0;
    let mut rejected = 0;
    let mut fail_closed_transitions = 0;
    measure_case(window, OverloadCaseId::PublicIntegrity, || {
        for (key, ingress, clock) in fixtures.iter().take(8_192).copied() {
            attempts += 1;
            assert_eq!(lane.queue.prepare(key), Admission::Insert);
            lane.queue.insert(
                key,
                LaneItem::new(
                    key,
                    ingress,
                    clock,
                    route,
                    PmPublicInput::Reference(reference),
                ),
            );
            retained += 1;
        }
        let (rejected_key, _, _) = fixtures[8_192];
        attempts += 1;
        assert_eq!(
            lane.queue.prepare(rejected_key),
            Admission::Full(SaturationAction::InvalidateStreamAndResync)
        );
        rejected += 1;
        fail_closed_transitions += 1;
    });
    assert_eq!(lane.metrics().depth(), 8_192);
    assert_eq!(lane.metrics().high_water(), 8_192);
    assert_eq!(lane.metrics().rejected_full(), 1);
    DirectMechanismObservation::new(
        OverloadCaseId::PublicIntegrity,
        attempts,
        retained,
        rejected,
        0,
        fail_closed_transitions,
    )
}

fn measure_private_lifecycle(window: &mut MeasurementWindow) -> DirectMechanismObservation {
    measure_complete_lane(
        window,
        OverloadCaseId::PrivateLifecycle,
        PmLaneKind::Private,
        4_096,
        true,
        SaturationAction::HaltAccountAndRequireReconciliation,
    )
}

fn measure_critical(window: &mut MeasurementWindow) -> DirectMechanismObservation {
    measure_complete_lane(
        window,
        OverloadCaseId::Critical,
        PmLaneKind::Critical,
        512,
        false,
        SaturationAction::GlobalStop,
    )
}

fn measure_persistence_acknowledgements(
    window: &mut MeasurementWindow,
) -> DirectMechanismObservation {
    measure_complete_lane(
        window,
        OverloadCaseId::PersistenceAcknowledgements,
        PmLaneKind::Persistence,
        512,
        false,
        SaturationAction::GlobalStop,
    )
}

fn measure_complete_snapshots(window: &mut MeasurementWindow) -> DirectMechanismObservation {
    measure_complete_lane(
        window,
        OverloadCaseId::CompleteSnapshots,
        PmLaneKind::Reconciliation,
        128,
        true,
        SaturationAction::KeepUnreadyAndRetry,
    )
}

fn measure_complete_lane(
    window: &mut MeasurementWindow,
    case: OverloadCaseId,
    lane_kind: PmLaneKind,
    capacity: usize,
    product_source: bool,
    expected_action: SaturationAction,
) -> DirectMechanismObservation {
    let mut lane = PmCompleteLane::<u64>::new(lane_kind);
    let fixtures = (1..=capacity + 1)
        .map(|attempt| {
            let attempt = u64::try_from(attempt).expect("bounded attempt");
            if product_source {
                account_ingress(attempt, attempt)
            } else {
                internal_ingress(attempt, attempt)
            }
        })
        .collect::<Vec<_>>();
    let source = if product_source {
        PmCompleteSourceKind::PolymarketAccount
    } else {
        PmCompleteSourceKind::InternalSignal
    };

    let mut attempts = 0;
    let mut retained = 0;
    let mut rejected = 0;
    let mut fail_closed_transitions = 0;
    measure_case(window, case, || {
        for (index, ingress) in fixtures.iter().take(capacity).copied().enumerate() {
            attempts += 1;
            let attempt = u64::try_from(index + 1).expect("bounded attempt");
            lane.enqueue(ingress, attempt, 0, source)
                .expect("within fixed capacity");
            retained += 1;
        }
        attempts += 1;
        let rejected_value = u64::try_from(capacity + 1).expect("bounded rejection");
        assert!(matches!(
            lane.enqueue(fixtures[capacity], rejected_value, 0, source),
            Err(PmCompleteLaneEnqueueError::Full { action, .. })
                if action == expected_action
        ));
        rejected += 1;
        fail_closed_transitions += 1;
    });
    assert_eq!(lane.metrics().queue().depth(), capacity);
    assert_eq!(lane.metrics().queue().high_water(), capacity);
    assert_eq!(lane.metrics().queue().rejected_full(), 1);
    DirectMechanismObservation::new(
        case,
        attempts,
        retained,
        rejected,
        0,
        fail_closed_transitions,
    )
}

fn measure_raw_entries(window: &mut MeasurementWindow) -> DirectMechanismObservation {
    let mut probe = Phase6RawCapacityProbe::new();
    let mut attempts = 0;
    let mut retained = 0;
    let mut rejected = 0;
    let mut fail_closed_transitions = 0;
    measure_case(window, OverloadCaseId::RawCaptureEntries, || {
        for _ in 0..8_192 {
            attempts += 1;
            probe.attempt(1).expect("first 8192 raw entries");
            retained += 1;
        }
        attempts += 1;
        assert!(matches!(
            probe.attempt(1),
            Err(PmCaptureVerifyError::TooManyRawFrames)
        ));
        rejected += 1;
        fail_closed_transitions += 1;
    });
    assert_eq!(probe.frames(), 8_192);
    assert_eq!(probe.payload_bytes(), 8_192);
    DirectMechanismObservation::new(
        OverloadCaseId::RawCaptureEntries,
        attempts,
        retained,
        rejected,
        0,
        fail_closed_transitions,
    )
}

fn measure_raw_bytes(window: &mut MeasurementWindow) -> DirectMechanismObservation {
    let mut probe = Phase6RawCapacityProbe::new();
    let mut attempts = 0;
    let mut retained = 0;
    let mut rejected = 0;
    let mut fail_closed_transitions = 0;
    measure_case(window, OverloadCaseId::RawCaptureBytes, || {
        for _ in 0..32 {
            attempts += 1;
            probe
                .attempt(MAX_PM_RAW_PUBLIC_FRAME_BYTES)
                .expect("first 32 one-MiB frames");
            retained += 1;
        }
        attempts += 1;
        assert!(matches!(
            probe.attempt(MAX_PM_RAW_PUBLIC_FRAME_BYTES),
            Err(PmCaptureVerifyError::RawPayloadTooLarge)
        ));
        rejected += 1;
        fail_closed_transitions += 1;
    });
    assert_eq!(probe.frames(), 32);
    DirectMechanismObservation::new(
        OverloadCaseId::RawCaptureBytes,
        attempts,
        retained,
        rejected,
        0,
        fail_closed_transitions,
    )
}

fn measure_oversize_raw(window: &mut MeasurementWindow) -> DirectMechanismObservation {
    let mut probe = Phase6RawCapacityProbe::new();
    let mut attempts = 0;
    let mut rejected = 0;
    let mut fail_closed_transitions = 0;
    measure_case(window, OverloadCaseId::OversizeRawFrame, || {
        attempts += 1;
        assert!(matches!(
            probe.attempt(MAX_PM_RAW_PUBLIC_FRAME_BYTES + 1),
            Err(PmCaptureVerifyError::RawFrameTooLarge)
        ));
        rejected += 1;
        fail_closed_transitions += 1;
    });
    assert_eq!(probe.frames(), 0);
    DirectMechanismObservation::new(
        OverloadCaseId::OversizeRawFrame,
        attempts,
        0,
        rejected,
        0,
        fail_closed_transitions,
    )
}

fn measure_storage(window: &mut MeasurementWindow) -> DirectMechanismObservation {
    let mut probe = Phase6StorageAllocationProbe::new();
    let mut attempts = 0;
    let mut retained = 0;
    let mut rejected = 0;
    let mut fail_closed_transitions = 0;
    measure_case(window, OverloadCaseId::Storage, || {
        for attempt in 1..=1_024_u64 {
            attempts += 1;
            probe.push_fact(attempt).expect("first 1024 storage pushes");
            retained += 1;
        }
        attempts += 1;
        assert!(probe.push_fact(1_025).is_err());
        rejected += 1;
        fail_closed_transitions += 1;
    });
    assert_eq!(probe.metrics().depth(), 1_024);
    assert_eq!(probe.metrics().high_water(), 1_024);
    assert_eq!(probe.metrics().saturations(), 1);
    assert!(probe.metrics().globally_stopped());
    DirectMechanismObservation::new(
        OverloadCaseId::Storage,
        attempts,
        retained,
        rejected,
        0,
        fail_closed_transitions,
    )
}

fn measure_fake_effect(window: &mut MeasurementWindow) -> DirectMechanismObservation {
    let mut probe =
        Phase6FakeEffectAllocationProbe::new().expect("fixed fake-effect allocation probe");
    let mut attempts = 0;
    let mut retained = 0;
    let mut rejected = 0;
    let mut fail_closed_transitions = 0;
    measure_case(window, OverloadCaseId::FakeEffect, || {
        for attempt in 0..256_u64 {
            attempts += 1;
            probe
                .attempt(100 + attempt)
                .expect("first 256 fake effects");
            retained += 1;
        }
        attempts += 1;
        assert!(probe.attempt(356).is_err());
        rejected += 1;
        fail_closed_transitions += 1;
    });
    assert_eq!(probe.metrics().depth(), 256);
    assert_eq!(probe.metrics().saturations(), 1);
    DirectMechanismObservation::new(
        OverloadCaseId::FakeEffect,
        attempts,
        retained,
        rejected,
        0,
        fail_closed_transitions,
    )
}

fn measure_schedule(window: &mut MeasurementWindow) -> DirectMechanismObservation {
    let instrument = instrument();
    let scopes = (0..2_048_u16).map(account_scope).collect::<Vec<_>>();
    let mut keys = Vec::with_capacity(4_097);
    for scope in scopes {
        for side in [PmOrderSide::Buy, PmOrderSide::Sell] {
            keys.push(PmScheduledActionKey::new(
                scope,
                instrument,
                side,
                PmScheduledActionKind::QuoteEvaluation,
            ));
        }
    }
    keys.push(PmScheduledActionKey::new(
        account_scope(4_000),
        instrument,
        PmOrderSide::Buy,
        PmScheduledActionKind::Freshness,
    ));
    let mut owner = PmQuoteScheduleRole::new(instrument);

    let mut attempts = 0;
    let mut retained = 0;
    let mut rejected = 0;
    let mut fail_closed_transitions = 0;
    measure_case(window, OverloadCaseId::ScheduledActions, || {
        for (attempt, key) in keys.iter().take(4_096).copied().enumerate() {
            attempts += 1;
            owner
                .schedule(
                    key,
                    10_000 + u64::try_from(attempt).expect("bounded"),
                    100,
                    1_000,
                )
                .expect("first 4096 scheduled actions");
            retained += 1;
        }
        attempts += 1;
        assert!(owner.schedule(keys[4_096], 20_000, 100, 1_000).is_err());
        rejected += 1;
        fail_closed_transitions += 1;
    });
    let metrics = owner.projection(100).expect("schedule metrics").metrics();
    assert_eq!(metrics.depth(), 4_096);
    assert_eq!(metrics.rejected_full(), 1);
    DirectMechanismObservation::new(
        OverloadCaseId::ScheduledActions,
        attempts,
        retained,
        rejected,
        0,
        fail_closed_transitions,
    )
}

fn measure_telemetry(window: &mut MeasurementWindow) -> DirectMechanismObservation {
    let mut lane = PmCompleteLane::<u64>::new(PmLaneKind::Telemetry);
    let fixtures = (1..=129_u64)
        .map(|attempt| internal_ingress(attempt, attempt))
        .collect::<Vec<_>>();
    let mut attempts = 0;
    measure_case(window, OverloadCaseId::Telemetry, || {
        for (index, ingress) in fixtures.iter().copied().enumerate() {
            attempts += 1;
            lane.enqueue(
                ingress,
                u64::try_from(index + 1).expect("bounded"),
                0,
                PmCompleteSourceKind::InternalSignal,
            )
            .expect("telemetry inserts or coalesces");
        }
    });
    assert_eq!(lane.metrics().queue().depth(), 128);
    assert_eq!(lane.metrics().queue().coalesced(), 1);
    assert_eq!(lane.metrics().queue().rejected_full(), 0);
    DirectMechanismObservation::new(
        OverloadCaseId::Telemetry,
        attempts,
        lane.metrics().queue().depth(),
        0,
        usize::try_from(lane.metrics().queue().coalesced())
            .expect("fixed coalesced count fits usize"),
        0,
    )
}
