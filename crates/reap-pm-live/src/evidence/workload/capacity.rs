use reap_pm_state::MAX_PM_REFRESH_OBLIGATIONS;

use super::{EvidenceRun, invariant};
use crate::PM_INPUT_SERVICE_PRIORITY;
use crate::evidence::PmEvidenceError;
use crate::evidence::contract::{MAX_REPLAY_WORKING_BYTES, MAX_RESERVED_CAPACITY_BYTES};
use crate::evidence::report::{CapacityReport, LanePressureReport, RefreshPressureReport};
use crate::lanes::{PmLaneKind, PmLanePolicy, SaturationAction};

impl EvidenceRun {
    pub(super) fn capacity_report(&mut self) -> Result<CapacityReport, PmEvidenceError> {
        let scheduler = self
            .owner
            .scheduler_metrics(self.cursor.monotonic_ns)
            .map_err(invariant)?;
        let schedule = self
            .owner
            .evidence_schedule_metrics(self.cursor.monotonic_ns)
            .map_err(invariant)?;
        let lanes = PM_INPUT_SERVICE_PRIORITY
            .into_iter()
            .map(|lane| {
                let metrics = scheduler
                    .lane(lane)
                    .expect("every input service lane has fixed metrics");
                LanePressureReport {
                    lane: lane_label(lane),
                    depth: metrics.queue().depth(),
                    high_water: metrics.queue().high_water(),
                    capacity: metrics.policy().capacity(),
                    nominal_high_water: metrics.policy().nominal_high_water(),
                    maximum_age_limit_ns: metrics.policy().maximum_age_ns(),
                    maximum_observed_age_ns: metrics.maximum_observed_age_ns(),
                    saturation_action: saturation_action_label(
                        metrics.policy().saturation_action(),
                    ),
                    serviced: metrics.serviced(),
                    age_faults: metrics.age_faults(),
                    rejected_full: metrics.queue().rejected_full(),
                    coalesced: metrics.queue().coalesced(),
                    invalidated_purged: metrics.queue().invalidated_purged(),
                }
            })
            .collect();
        let persistence = self.owner.persistence_metrics();
        let fake = self.owner.fake_effect_metrics();
        let output = self.owner.product_effect_metrics();
        let refresh = self.owner.refresh_obligation_metrics();
        let coordinator = self.owner.counters();
        let journal_policy = PmLanePolicy::for_lane(PmLaneKind::Journal);
        let fake_policy = PmLanePolicy::for_lane(PmLaneKind::FakeEffect);
        let schedule_policy = PmLanePolicy::for_lane(PmLaneKind::Scheduled);
        let refresh_policy = PmLanePolicy::for_lane(PmLaneKind::ReconciliationRequest);
        let capture_policy = PmLanePolicy::for_lane(PmLaneKind::Capture);
        let report = CapacityReport {
            reserved_capacity_bytes: self.owner.reserved_capacity_bytes(),
            reserved_capacity_limit_bytes: MAX_RESERVED_CAPACITY_BYTES,
            replay_working_limit_bytes: MAX_REPLAY_WORKING_BYTES,
            persistence_capacity: persistence.capacity(),
            persistence_depth: persistence.depth(),
            persistence_nominal_high_water: journal_policy.nominal_high_water(),
            fake_effect_capacity: fake.capacity(),
            schedule_capacity: schedule.capacity(),
            schedule_nominal_high_water: schedule.nominal_high_water(),
            schedule_depth: schedule.depth(),
            schedule_high_water: schedule.high_water(),
            schedule_admitted: schedule.admitted(),
            schedule_duplicate_suppressed: schedule.duplicate_suppressed(),
            schedule_rescheduled: schedule.rescheduled(),
            schedule_removed: schedule.removed(),
            schedule_serviced: schedule.serviced(),
            schedule_rejected_full: schedule.rejected_full(),
            schedule_clock_regressions: schedule.clock_regressions(),
            schedule_current_due_age_ns: schedule.current_due_age_ns(),
            schedule_maximum_due_age_ns: schedule.maximum_due_age_ns(),
            schedule_maximum_permitted_due_age_ns: schedule.maximum_permitted_due_age_ns(),
            schedule_fail_closed: schedule.fail_closed(),
            copied_correlation_capacity: crate::coordinator::MAX_COPIED_EFFECT_CORRELATIONS,
            copied_correlation_high_water: usize::from(coordinator.correlation_high_water()),
            copied_output_capacity: output.capacity(),
            copied_output_depth: output.depth(),
            copied_output_high_water: output.high_water(),
            copied_output_rejected_full: output.rejected_full(),
            copied_output_age_faults: 0,
            copied_output_saturation_action: "reject_copied_output_and_halt_quotes",
            persistence_high_water: usize::from(persistence.high_water()),
            persistence_maximum_age_limit_ns: journal_policy.maximum_age_ns(),
            persistence_maximum_age_ns: persistence.maximum_observed_age_ns(),
            persistence_admitted: persistence.admitted(),
            persistence_acknowledged: persistence.acknowledged(),
            persistence_saturations: persistence.saturations(),
            persistence_age_faults: persistence.age_faults(),
            persistence_globally_stopped: persistence.globally_stopped(),
            persistence_saturation_action: saturation_action_label(
                journal_policy.saturation_action(),
            ),
            fake_effect_depth: fake.depth(),
            fake_effect_nominal_high_water: fake_policy.nominal_high_water(),
            fake_effect_high_water: usize::from(fake.high_water()),
            fake_effect_maximum_age_limit_ns: fake_policy.maximum_age_ns(),
            fake_effect_maximum_age_ns: fake.maximum_observed_age_ns(),
            fake_effect_queued: fake.queued(),
            fake_effect_blocked: fake.blocked(),
            fake_effect_retained: fake.retained(),
            fake_effect_serviced: fake.serviced(),
            fake_effect_saturations: fake.saturations(),
            fake_effect_age_faults: fake.age_faults(),
            fake_effect_clock_regressions: fake.clock_regressions(),
            fake_effect_saturation_action: saturation_action_label(fake_policy.saturation_action()),
            refresh: RefreshPressureReport {
                capacity: MAX_PM_REFRESH_OBLIGATIONS,
                total_pending: refresh.total_pending(),
                total_in_flight: refresh.total_in_flight(),
                ambiguous_order_pending: refresh.ambiguous_order_pending(),
                ambiguous_order_in_flight: refresh.ambiguous_order_in_flight(),
                fill_observed_pending: refresh.fill_observed_pending(),
                fill_observed_in_flight: refresh.fill_observed_in_flight(),
                fill_observed_high_water: refresh.fill_observed_high_water(),
                external_ingress_pending: refresh.external_ingress_pending(),
                external_ingress_in_flight: refresh.external_ingress_in_flight(),
                external_ingress_high_water: refresh.external_ingress_high_water(),
                oldest_in_flight_age_ns: refresh.oldest_in_flight_age_ns(),
                maximum_observed_age_ns: refresh.maximum_observed_age_ns(),
                maximum_age_limit_ns: refresh_policy.maximum_age_ns(),
                retry_effects: refresh.retry_effects(),
                duplicate_or_superseded_admissions: refresh.duplicate_or_superseded_admissions(),
                saturation_action: saturation_action_label(refresh_policy.saturation_action()),
            },
            raw_entry_capacity: usize::try_from(crate::MAX_PM_PUBLIC_CAPTURE_RAW_FRAMES)
                .map_err(invariant)?,
            raw_entry_high_water: 0,
            raw_entry_rejections: 0,
            raw_payload_byte_capacity: usize::try_from(
                crate::MAX_PM_PUBLIC_CAPTURE_RAW_PAYLOAD_BYTES,
            )
            .map_err(invariant)?,
            raw_payload_byte_high_water: 0,
            raw_payload_rejections: 0,
            raw_oversize_rejections: 0,
            raw_maximum_age_limit_ns: capture_policy.maximum_age_ns(),
            raw_age_faults: 0,
            raw_saturation_action: saturation_action_label(capture_policy.saturation_action()),
            lanes,
        };
        debug_assert_eq!(
            report.schedule_nominal_high_water,
            schedule_policy.nominal_high_water()
        );
        validate_capacity_report(&report)?;
        Ok(report)
    }
}

fn validate_capacity_report(report: &CapacityReport) -> Result<(), PmEvidenceError> {
    let lane_failure = report.lanes.iter().find(|lane| {
        lane.depth != 0
            || lane.high_water > lane.nominal_high_water
            || lane.high_water > lane.capacity
            || lane
                .maximum_age_limit_ns
                .is_some_and(|limit| lane.maximum_observed_age_ns > limit)
            || lane.age_faults != 0
            || lane.rejected_full != 0
            || lane.coalesced != 0
            || lane.invalidated_purged != 0
    });
    let refresh = report.refresh;
    if report.reserved_capacity_bytes > report.reserved_capacity_limit_bytes
        || report.replay_working_limit_bytes != MAX_REPLAY_WORKING_BYTES
        || report.lanes.len() != PM_INPUT_SERVICE_PRIORITY.len()
        || lane_failure.is_some()
        || report.persistence_depth != 0
        || report.persistence_high_water != 1
        || report.persistence_high_water > report.persistence_nominal_high_water
        || report.persistence_admitted != report.persistence_acknowledged
        || report.persistence_saturations != 0
        || report.persistence_age_faults != 0
        || report.persistence_globally_stopped
        || report
            .persistence_maximum_age_limit_ns
            .is_some_and(|limit| report.persistence_maximum_age_ns > limit)
        || report.schedule_capacity != crate::schedule::MAX_PM_SCHEDULED_ACTIONS
        || report.schedule_nominal_high_water != 64
        || report.schedule_depth != 0
        || report.schedule_high_water > report.schedule_nominal_high_water
        || report.schedule_high_water > report.schedule_capacity
        || report.schedule_admitted
            != report
                .schedule_serviced
                .saturating_add(report.schedule_removed)
        || report.schedule_rejected_full != 0
        || report.schedule_clock_regressions != 0
        || report.schedule_current_due_age_ns != 0
        || report.schedule_maximum_due_age_ns > report.schedule_maximum_permitted_due_age_ns
        || report.schedule_maximum_permitted_due_age_ns != 100_000_000
        || report.schedule_fail_closed
        || report.copied_correlation_high_water != 1
        || report.copied_correlation_high_water > report.copied_correlation_capacity
        || report.fake_effect_depth != 0
        || report.fake_effect_queued != 0
        || report.fake_effect_blocked != 0
        || report.fake_effect_retained != 0
        || report.fake_effect_high_water > report.fake_effect_nominal_high_water
        || report.fake_effect_saturations != 0
        || report.fake_effect_age_faults != 0
        || report.fake_effect_clock_regressions != 0
        || report
            .fake_effect_maximum_age_limit_ns
            .is_some_and(|limit| report.fake_effect_maximum_age_ns > limit)
        || report.copied_output_depth != 0
        || report.copied_output_high_water > report.copied_output_capacity
        || report.copied_output_rejected_full != 0
        || report.copied_output_age_faults != 0
        || refresh.total_pending != 0
        || refresh.total_in_flight != 0
        || (refresh.total_in_flight == 0 && refresh.oldest_in_flight_age_ns != 0)
        || refresh.ambiguous_order_pending != 0
        || refresh.ambiguous_order_in_flight != 0
        || refresh.fill_observed_pending != 0
        || refresh.fill_observed_in_flight != 0
        || refresh.fill_observed_high_water != 1
        || refresh.external_ingress_pending != 0
        || refresh.external_ingress_in_flight != 0
        || refresh.retry_effects != 0
        || refresh.duplicate_or_superseded_admissions != 0
        || refresh
            .maximum_age_limit_ns
            .is_some_and(|limit| refresh.maximum_observed_age_ns > limit)
        || report.raw_entry_high_water != 0
        || report.raw_entry_rejections != 0
        || report.raw_payload_byte_high_water != 0
        || report.raw_payload_rejections != 0
        || report.raw_oversize_rejections != 0
        || report.raw_age_faults != 0
    {
        return Err(PmEvidenceError::invariant(format!(
            "nominal capacity acceptance failed: report={report:?}, first_lane_failure={lane_failure:?}"
        )));
    }
    Ok(())
}

const fn lane_label(lane: PmLaneKind) -> &'static str {
    match lane {
        PmLaneKind::Critical => "critical",
        PmLaneKind::Persistence => "persistence",
        PmLaneKind::Private => "private",
        PmLaneKind::Scheduled => "scheduled",
        PmLaneKind::Public => "public",
        PmLaneKind::Reconciliation => "reconciliation",
        PmLaneKind::Telemetry => "telemetry",
        PmLaneKind::ReconciliationRequest => "reconciliation_request",
        PmLaneKind::Capture => "capture",
        PmLaneKind::Journal => "journal",
        PmLaneKind::FakeEffect => "fake_effect",
    }
}

const fn saturation_action_label(action: SaturationAction) -> &'static str {
    match action {
        SaturationAction::GlobalStop => "global_stop",
        SaturationAction::HaltAccountAndRequireReconciliation => {
            "halt_account_and_require_reconciliation"
        }
        SaturationAction::InvalidateStreamAndResync => "invalidate_stream_and_resync",
        SaturationAction::KeepUnreadyAndRetry => "keep_unready_and_retry",
        SaturationAction::RetainPendingRefresh => "retain_pending_refresh",
        SaturationAction::InvalidateCaptureAndResync => "invalidate_capture_and_resync",
        SaturationAction::SuppressDispatchAndHaltQuotes => "suppress_dispatch_and_halt_quotes",
        SaturationAction::RejectEffectAndHaltQuotes => "reject_effect_and_halt_quotes",
        SaturationAction::SuppressQuoteAndCancelOwned => "suppress_quote_and_cancel_owned",
        SaturationAction::CoalesceTelemetry => "coalesce_telemetry",
    }
}
