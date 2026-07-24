use reap_pm_core::{ConnectionEpoch, EventOrdering, IngressSequence, ReceivedEventClock};
use reap_pm_state::{
    PmPrivateExternalIngressFailure, PmPrivateExternalIngressFault, PmPrivateExternalIngressLane,
};
use reap_polymarket_adapter::PmFixtureCompletionOccurrence;

use crate::{
    PmCompleteFailClosedMetrics, PmControlReason, PmLaneKind, PmProductEffect, PmRefreshEffectKind,
    PmTelemetryKind, SaturationAction,
};

const OWNER_MEMORY_BOUND_BYTES: usize = 64 * 1024 * 1024;

fn occurrence(sequence: u64) -> PmFixtureCompletionOccurrence {
    PmFixtureCompletionOccurrence::new(
        ReceivedEventClock::new(None, 1_700_000_000_000_000_000 + sequence, 100 + sequence)
            .expect("fixed receive clock"),
        EventOrdering::new(
            ConnectionEpoch::new(1),
            None,
            None,
            None,
            IngressSequence::new(sequence),
        )
        .expect("fixed ordering"),
    )
}

async fn start(
    case: &str,
) -> (
    tempfile::TempDir,
    crate::PmProductRun<super::super::fixture::Phase6Model>,
) {
    let directory = tempfile::tempdir().expect("temporary evidence directory");
    let run = super::super::start_reached_overload_product(
        directory.path().join(format!("{case}-capture.jsonl")),
        directory.path().join(format!("{case}-journal.jsonl")),
    )
    .await
    .expect("fixed reached product starts");
    (directory, run)
}

#[test]
fn critical_row_reaches_product_513_times_and_latches_one_global_stop() {
    super::run_product_test(|| async {
        let (_directory, mut run) = start("critical").await;
        let reserved = run.reserved_capacity_bytes();
        for attempt in 1..=512_u64 {
            run.request_shutdown(occurrence(attempt))
                .expect("first 512 reached critical inputs");
        }
        assert!(run.request_shutdown(occurrence(513)).is_err());

        let metrics = run.scheduler_metrics(10_000).expect("scheduler metrics");
        let lane = metrics.lane(PmLaneKind::Critical).expect("critical lane");
        assert_eq!(lane.queue().depth(), 512);
        assert_eq!(lane.queue().high_water(), 512);
        assert_eq!(lane.queue().rejected_full(), 1);
        assert_eq!(
            metrics
                .fail_closed()
                .transitions(SaturationAction::GlobalStop),
            1
        );
        assert!(metrics.fail_closed().global_stopped());
        assert!(metrics.fail_closed().fake_dispatch_suppressed());
        assert_eq!(run.halt(), Some(PmControlReason::SchedulerOverload));
        assert_eq!(run.fake_effect_metrics().serviced(), 0);
        assert_eq!(run.pending_effect_outputs(), 0);
        assert_eq!(run.reserved_capacity_bytes(), reserved);
        assert!(reserved <= OWNER_MEMORY_BOUND_BYTES);
        let _ = run.shutdown().await;
    });
}

#[test]
fn private_row_reaches_product_4097_times_and_requires_complete_reconciliation() {
    super::run_product_test(|| async {
        let (_directory, mut run) = start("private").await;
        let reserved = run.reserved_capacity_bytes();
        run.connect_private_fixture(occurrence(1))
            .expect("configured fixture connection");
        let fault = PmPrivateExternalIngressFault::new(
            PmPrivateExternalIngressLane::Reconnect,
            PmPrivateExternalIngressFailure::Service,
        );
        for attempt in 2..=4_096_u64 {
            run.mark_private_fixture_unavailable(occurrence(attempt), fault)
                .expect("first 4096 reached private inputs");
        }
        assert!(
            run.mark_private_fixture_unavailable(occurrence(4_097), fault)
                .is_err()
        );

        let metrics = run.scheduler_metrics(10_000).expect("scheduler metrics");
        let lane = metrics.lane(PmLaneKind::Private).expect("private lane");
        assert_eq!(lane.queue().depth(), 4_096);
        assert_eq!(lane.queue().high_water(), 4_096);
        assert_eq!(lane.queue().rejected_full(), 1);
        assert_eq!(
            metrics
                .fail_closed()
                .transitions(SaturationAction::HaltAccountAndRequireReconciliation),
            1
        );
        assert!(metrics.fail_closed().account_halted());
        assert!(metrics.fail_closed().account_unready());
        assert!(metrics.fail_closed().complete_reconciliation_required());
        assert_eq!(run.halt(), None);
        assert_eq!(run.mutation_halt(), None);
        assert_eq!(run.fake_effect_metrics().serviced(), 0);
        let Some(PmProductEffect::FailClosedHaltOrCancel(halt)) = run.pop_effect() else {
            panic!("private saturation emits its scoped halt first");
        };
        assert_eq!(halt.reason(), PmControlReason::SchedulerOverload);
        let Some(PmProductEffect::ReconciliationRefresh(refresh)) = run.pop_effect() else {
            panic!("private saturation emits complete reconciliation second");
        };
        assert_eq!(refresh.kind(), PmRefreshEffectKind::CompleteReconciliation);
        assert!(run.pop_effect().is_none());
        assert_eq!(run.reserved_capacity_bytes(), reserved);
        assert!(reserved <= OWNER_MEMORY_BOUND_BYTES);
        let _ = run.shutdown().await;
    });
}

#[test]
fn scheduled_row_reaches_4097_attempts_and_enacts_safety_once() {
    super::run_product_test(|| async {
        let (_directory, mut run) = start("scheduled").await;
        let reserved = run.reserved_capacity_bytes();
        assert_eq!(
            run.phase6_reach_schedule_full(100, 1_700_000_000_000),
            Some((4_097, SaturationAction::SuppressQuoteAndCancelOwned))
        );

        let metrics = run.scheduler_metrics(100).expect("scheduler metrics");
        let lane = metrics.lane(PmLaneKind::Scheduled).expect("scheduled lane");
        assert_eq!(lane.queue().depth(), 4_096);
        assert_eq!(lane.queue().high_water(), 4_096);
        assert_eq!(lane.queue().rejected_full(), 1);
        assert_eq!(
            metrics
                .fail_closed()
                .transitions(SaturationAction::SuppressQuoteAndCancelOwned),
            1
        );
        assert!(metrics.fail_closed().global_stopped());
        assert!(metrics.fail_closed().quote_suppressed());
        assert!(metrics.fail_closed().cancel_owned_required());
        assert_eq!(run.halt(), Some(PmControlReason::SchedulerOverload));
        assert_eq!(run.mutation_halt(), None);

        let Some(PmProductEffect::FailClosedHaltOrCancel(halt)) = run.pop_effect() else {
            panic!("fresh fixture has no cancel candidate and emits one final halt");
        };
        assert_eq!(halt.reason(), PmControlReason::SchedulerOverload);
        assert_eq!(halt.cancel_intent(), None);
        assert!(run.pop_effect().is_none());
        let mutation_before = run.mutation_counters();
        let queue_before = lane.queue();

        run.service_turn(101)
            .expect("the rejected scheduling obligation is not retried");
        assert!(run.pop_effect().is_none());
        let after = run
            .scheduler_metrics(101)
            .expect("stable scheduler metrics");
        assert_eq!(
            after
                .lane(PmLaneKind::Scheduled)
                .expect("scheduled lane")
                .queue(),
            queue_before
        );
        assert_eq!(
            after
                .fail_closed()
                .transitions(SaturationAction::SuppressQuoteAndCancelOwned),
            1
        );
        assert_eq!(run.mutation_counters(), mutation_before);
        assert_eq!(run.reserved_capacity_bytes(), reserved);
        assert_eq!(run.halt(), Some(PmControlReason::SchedulerOverload));
        assert!(reserved <= OWNER_MEMORY_BOUND_BYTES);
        let _ = run.shutdown().await;
    });
}

#[test]
fn telemetry_row_reaches_product_129_times_and_only_coalesces_once() {
    super::run_product_test(|| async {
        let (_directory, mut run) = super::reached_mutation_support::start(
            "telemetry",
            super::super::ReachedOverloadProfile::Standard,
        )
        .await;
        let _venue = super::reached_mutation_support::establish_live_quote(
            &mut run,
            "phase6-telemetry-order",
        )
        .await;
        let reserved = run.reserved_capacity_bytes();
        for attempt in 1..=128_u64 {
            run.emit_telemetry(
                occurrence(10_000 + attempt),
                PmTelemetryKind::Metric,
                attempt,
            )
            .expect("telemetry is the only coalescing input");
        }

        let before_metrics = run.scheduler_metrics(20_000).expect("scheduler metrics");
        let before_lane = before_metrics
            .lane(PmLaneKind::Telemetry)
            .expect("telemetry lane");
        assert_eq!(before_lane.queue().depth(), 128);
        assert_eq!(before_lane.queue().high_water(), 128);
        assert_eq!(before_lane.queue().coalesced(), 0);
        let before_readiness = run.public_capture().pm_book_readiness();
        assert!(before_readiness.is_ready());
        let before_state = run.telemetry_overload_state();
        assert!(before_state.mutation_revision_authority_present());
        assert!(!before_state.reconciliation_gate());
        assert!(!before_state.reconciliation_recovered());
        let before_refresh = run.refresh_obligation_metrics();
        let before_mutation = run.mutation_counters();
        let before_halt = run.halt();
        let before_mutation_halt = run.mutation_halt();
        let before_effects = run.product_effect_metrics();
        let before_pending_outputs = run.pending_effect_outputs();

        run.emit_telemetry(occurrence(10_129), PmTelemetryKind::Metric, 129)
            .expect("129th telemetry input coalesces observationally");

        let metrics = run.scheduler_metrics(20_000).expect("scheduler metrics");
        let lane = metrics.lane(PmLaneKind::Telemetry).expect("telemetry lane");
        assert_eq!(lane.queue().depth(), 128);
        assert_eq!(lane.queue().high_water(), 128);
        assert_eq!(lane.queue().coalesced(), 1);
        assert_eq!(lane.queue().rejected_full(), 0);
        assert_eq!(
            metrics.fail_closed(),
            PmCompleteFailClosedMetrics::default()
        );
        assert_eq!(run.public_capture().pm_book_readiness(), before_readiness);
        assert_eq!(run.telemetry_overload_state(), before_state);
        assert_eq!(run.refresh_obligation_metrics(), before_refresh);
        assert_eq!(run.mutation_counters(), before_mutation);
        assert_eq!(run.halt(), before_halt);
        assert_eq!(run.mutation_halt(), before_mutation_halt);
        assert_eq!(run.product_effect_metrics(), before_effects);
        assert_eq!(run.pending_effect_outputs(), before_pending_outputs);
        assert_eq!(run.reserved_capacity_bytes(), reserved);
        assert!(reserved <= OWNER_MEMORY_BOUND_BYTES);
        let _ = run.shutdown().await;
    });
}
