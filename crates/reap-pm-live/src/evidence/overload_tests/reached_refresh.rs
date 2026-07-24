use reap_polymarket_adapter::PmFixtureFeeEvidence;

use crate::PmProductEffect;

use super::reached_mutation_support::{
    completion, drain_outputs, establish_live_quote, fill_batch, start,
};

#[test]
fn one_product_refresh_ticket_is_retained_deduplicated_and_exactly_completed() {
    super::run_product_test(|| async {
        let (_directory, mut run) =
            start("refresh", super::super::ReachedOverloadProfile::Standard).await;
        let venue = establish_live_quote(&mut run, "phase6-refresh-owned-order").await;
        assert_eq!(run.tracked_quote_slots_for_test(), 1);
        let frame = fill_batch(venue, 1).replace(r#""size":"0.01""#, r#""size":"5""#);

        run.ingest_private_fixture(
            completion(20, 2_099),
            frame.as_bytes(),
            PmFixtureFeeEvidence::Unknown,
        )
        .expect("unique fill reaches private lane");
        run.service_turn(2_100).expect("unique fill services");
        assert_eq!(
            run.tracked_quote_slots_for_test(),
            0,
            "a private unique terminal fill clears its tracked quote immediately"
        );
        let mut refresh_effects = 0;
        while let Some(effect) = run.pop_effect() {
            refresh_effects +=
                usize::from(matches!(effect, PmProductEffect::ReconciliationRefresh(_)));
        }
        assert_eq!(refresh_effects, 1);
        let admitted = run.refresh_obligation_metrics();
        assert_eq!(admitted.fill_observed_pending(), 1);
        assert_eq!(admitted.fill_observed_in_flight(), 1);
        assert_eq!(admitted.fill_observed_admissions(), 1);
        assert_eq!(admitted.fill_observed_effects(), 1);
        assert_eq!(admitted.fill_observed_completions(), 0);

        run.ingest_private_fixture(
            completion(21, 2_101),
            frame.as_bytes(),
            PmFixtureFeeEvidence::Unknown,
        )
        .expect("duplicate fill reaches private lane");
        run.service_turn(2_102).expect("duplicate fill services");
        let mut duplicate_refresh_effects = 0;
        while let Some(effect) = run.pop_effect() {
            duplicate_refresh_effects +=
                usize::from(matches!(effect, PmProductEffect::ReconciliationRefresh(_)));
        }
        assert_eq!(duplicate_refresh_effects, 0);
        let duplicate = run.refresh_obligation_metrics();
        assert_eq!(duplicate.fill_observed_admissions(), 1);
        assert_eq!(duplicate.fill_observed_effects(), 1);

        let raw_fills = [frame.as_bytes()];
        super::super::complete_reached_overload_reconciliation(&mut run, 1, &raw_fills)
            .await
            .expect("paired Applied reconciliation completes the exact ticket");
        drain_outputs(&mut run);
        let completed = run.refresh_obligation_metrics();
        assert_eq!(completed.fill_observed_pending(), 0);
        assert_eq!(completed.fill_observed_in_flight(), 0);
        assert_eq!(completed.fill_observed_completions(), 1);
        let _ = run.shutdown().await;
    });
}
