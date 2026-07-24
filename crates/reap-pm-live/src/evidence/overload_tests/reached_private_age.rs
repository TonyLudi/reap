use reap_polymarket_adapter::PmFixtureFeeEvidence;

use crate::{
    PmControlReason, PmLaneKind, PmLanePolicy, PmProductEffect, PmRefreshEffectKind,
    SaturationAction,
};

use super::reached_mutation_support::{completion, drain_outputs, start};

fn unowned_order_frame(id: &str) -> String {
    format!(
        r#"{{"event_type":"order","id":"{id}","market":"{}","asset_id":"{}","side":"BUY","original_size":"5","size_matched":"0","price":"0.40","status":"LIVE","maker_address":"{}","type":"PLACEMENT"}}"#,
        super::super::fixture::MARKET,
        super::super::fixture::TOKEN,
        super::super::fixture::PM_FUNDER,
    )
}

#[test]
fn separate_private_age_episodes_retain_and_complete_the_newer_refresh_requirement() {
    super::run_product_test(|| async {
        let (_directory, mut run) = start(
            "private-age",
            super::super::ReachedOverloadProfile::Standard,
        )
        .await;
        let maximum_age_ns = PmLanePolicy::for_lane(PmLaneKind::Private)
            .maximum_age_ns()
            .expect("private lane has a fixed age");
        let first_receive_ns = 3_000;
        let first_fault_ns = first_receive_ns + maximum_age_ns + 1;
        let first_frame = unowned_order_frame("phase6-private-age-first");

        run.ingest_private_fixture(
            completion(30, first_receive_ns),
            first_frame.as_bytes(),
            PmFixtureFeeEvidence::Unknown,
        )
        .expect("first valid private observation reaches the product");
        let first = run
            .service_turn(first_fault_ns)
            .expect_err("one nanosecond beyond the private age must fault");
        assert_eq!(
            first.saturation_action(),
            Some(SaturationAction::HaltAccountAndRequireReconciliation)
        );
        let Some(PmProductEffect::FailClosedHaltOrCancel(halt)) = run.pop_effect() else {
            panic!("the first private age episode emits its scoped halt first");
        };
        assert_eq!(halt.reason(), PmControlReason::SchedulerOverload);
        let Some(PmProductEffect::ReconciliationRefresh(refresh)) = run.pop_effect() else {
            panic!("the first private age episode emits complete reconciliation second");
        };
        assert_eq!(refresh.kind(), PmRefreshEffectKind::CompleteReconciliation);
        assert!(run.pop_effect().is_none());
        let first_ticket = run.refresh_obligation_metrics();
        assert_eq!(first_ticket.external_ingress_pending(), 1);
        assert_eq!(first_ticket.external_ingress_in_flight(), 1);
        assert_eq!(first_ticket.external_ingress_high_water(), 1);
        assert_eq!(first_ticket.external_ingress_admissions(), 1);
        assert_eq!(first_ticket.external_ingress_effects(), 1);
        assert_eq!(first_ticket.external_ingress_completions(), 0);
        assert_eq!(run.halt(), None);
        assert_eq!(run.mutation_halt(), None);

        let first_drain = run
            .service_turn(first_fault_ns)
            .expect("the exact first aged backlog drains on its recovery turn");
        assert_eq!(first_drain.for_lane(PmLaneKind::Private), Some(1));
        drain_outputs(&mut run);

        let second_receive_ns = first_fault_ns + 1;
        let second_fault_ns = second_receive_ns + maximum_age_ns + 1;
        let second_frame = unowned_order_frame("phase6-private-age-second");
        run.ingest_private_fixture(
            completion(31, second_receive_ns),
            second_frame.as_bytes(),
            PmFixtureFeeEvidence::Unknown,
        )
        .expect("later valid private observation reaches the product");
        let second = run
            .service_turn(second_fault_ns)
            .expect_err("the later private episode receives its own age fault");
        assert_eq!(
            second.saturation_action(),
            Some(SaturationAction::HaltAccountAndRequireReconciliation)
        );
        let Some(PmProductEffect::FailClosedHaltOrCancel(halt)) = run.pop_effect() else {
            panic!("the second private age episode emits its scoped halt");
        };
        assert_eq!(halt.reason(), PmControlReason::SchedulerOverload);
        assert!(
            run.pop_effect().is_none(),
            "the newer requirement stays pending behind the exact in-flight ticket"
        );
        let superseded = run.refresh_obligation_metrics();
        assert_eq!(superseded.external_ingress_pending(), 1);
        assert_eq!(superseded.external_ingress_in_flight(), 1);
        assert_eq!(superseded.external_ingress_admissions(), 1);
        assert_eq!(superseded.external_ingress_effects(), 1);
        assert_eq!(superseded.external_ingress_completions(), 0);
        assert_eq!(superseded.duplicate_or_superseded_admissions(), 1);
        assert_eq!(run.halt(), None);
        assert_eq!(run.mutation_halt(), None);

        let second_drain = run
            .service_turn(second_fault_ns)
            .expect("the exact second aged backlog drains on its recovery turn");
        assert_eq!(second_drain.for_lane(PmLaneKind::Private), Some(1));
        drain_outputs(&mut run);

        super::super::complete_reached_overload_reconciliation(&mut run, 1, &[])
            .await
            .expect("first paired Applied completion advances to the newer requirement");
        let advanced = run.refresh_obligation_metrics();
        assert_eq!(advanced.external_ingress_pending(), 1);
        assert_eq!(advanced.external_ingress_in_flight(), 1);
        assert_eq!(advanced.external_ingress_admissions(), 2);
        assert_eq!(advanced.external_ingress_effects(), 2);
        assert_eq!(advanced.external_ingress_completions(), 1);

        super::super::complete_reached_overload_reconciliation(&mut run, 2, &[])
            .await
            .expect("second paired Applied completion clears the newer requirement");
        let completed = run.refresh_obligation_metrics();
        assert_eq!(completed.external_ingress_pending(), 0);
        assert_eq!(completed.external_ingress_in_flight(), 0);
        assert_eq!(completed.external_ingress_admissions(), 2);
        assert_eq!(completed.external_ingress_effects(), 2);
        assert_eq!(completed.external_ingress_completions(), 2);
        assert_eq!(run.halt(), None);
        assert_eq!(run.mutation_halt(), None);
        let _ = run.shutdown().await;
    });
}
