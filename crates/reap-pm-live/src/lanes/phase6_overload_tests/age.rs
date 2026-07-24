use reap_pm_core::PmOrderSide;

use super::support::{account_ingress, account_scope, instrument, internal_ingress};
use super::*;
use crate::schedule::{
    PmQuoteScheduleRole, PmScheduleError, PmScheduledActionKey, PmScheduledActionKind,
};

#[test]
fn complete_state_lane_boundaries_match_their_saturation_outcomes() {
    for (lane_kind, product_source) in [
        (PmLaneKind::Critical, false),
        (PmLaneKind::Persistence, false),
        (PmLaneKind::Private, true),
        (PmLaneKind::Reconciliation, true),
    ] {
        let maximum = PmLanePolicy::for_lane(lane_kind)
            .maximum_age_ns()
            .expect("state-bearing lane has an age bound");
        let source = if product_source {
            PmCompleteSourceKind::PolymarketAccount
        } else {
            PmCompleteSourceKind::InternalSignal
        };

        let mut inclusive = PmCompleteLane::<u64>::new(lane_kind);
        let ingress = if product_source {
            account_ingress(1, 1)
        } else {
            internal_ingress(1, 1)
        };
        inclusive.enqueue(ingress, 1, 0, source).expect("admitted");
        inclusive
            .check_age(1 + maximum)
            .expect("the exact age boundary is inclusive");

        let mut exceeded = PmCompleteLane::<u64>::new(lane_kind);
        let ingress = if product_source {
            account_ingress(1, 1)
        } else {
            internal_ingress(1, 1)
        };
        exceeded.enqueue(ingress, 1, 0, source).expect("admitted");
        let fault = match exceeded.check_age(1 + maximum + 1) {
            Err(PmCompleteLaneCheckError::Aged(fault)) => fault,
            _ => panic!("{lane_kind:?} must fail one nanosecond past its age bound"),
        };
        assert_eq!(fault.lane(), lane_kind);
        assert_eq!(fault.observed_age_ns(), maximum + 1);
        assert_eq!(fault.maximum_age_ns(), maximum);
        assert_eq!(
            fault.action(),
            PmLanePolicy::for_lane(lane_kind).saturation_action()
        );
        assert_eq!(exceeded.metrics().queue().depth(), 1);
        assert_eq!(exceeded.metrics().age_faults(), 1);
    }
}

#[test]
fn schedule_boundary_has_the_same_fail_closed_action_as_saturation() {
    let configured = instrument();
    let key = PmScheduledActionKey::new(
        account_scope(1),
        configured,
        PmOrderSide::Buy,
        PmScheduledActionKind::Freshness,
    );
    let maximum = PmLanePolicy::for_lane(PmLaneKind::Scheduled)
        .maximum_age_ns()
        .expect("scheduled age");

    let mut inclusive = PmQuoteScheduleRole::new(configured);
    inclusive.schedule(key, 10, 1, 1_000).expect("schedule");
    assert!(
        inclusive
            .pop_due(10 + maximum)
            .expect("inclusive boundary")
            .is_some()
    );

    let mut exceeded = PmQuoteScheduleRole::new(configured);
    exceeded.schedule(key, 10, 1, 1_000).expect("schedule");
    let error = exceeded
        .pop_due(10 + maximum + 1)
        .expect_err("one nanosecond past the boundary fails closed");
    assert!(matches!(
        &error,
        PmScheduleError::Aged {
            action: SaturationAction::SuppressQuoteAndCancelOwned,
            due_age_ns,
            maximum_due_age_ns,
            ..
        } if *due_age_ns == maximum + 1 && *maximum_due_age_ns == maximum
    ));
    assert_eq!(
        PmCompleteServiceError::Schedule(error).action(),
        Some(SaturationAction::SuppressQuoteAndCancelOwned)
    );
    assert!(
        exceeded
            .projection(10 + maximum + 1)
            .expect("projection")
            .metrics()
            .fail_closed()
    );
}
