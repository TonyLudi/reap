use reap_pm_core::PmOrderSide;

use crate::{PmMutationHalt, PmScheduledActionKind};

use super::reached_mutation_support::{
    OWNER_MEMORY_BOUND_BYTES, drain_outputs, establish_live_quote, fill_batches, ingest_fill_batch,
    start,
};

#[test]
fn storage_row_reaches_product_1025_times_and_releases_rejected_quote_capacity() {
    super::run_product_test(|| async {
        let (_directory, mut run) =
            start("storage", super::super::ReachedOverloadProfile::Storage1024).await;
        let reserved = run.reserved_capacity_bytes();
        assert!(reserved <= OWNER_MEMORY_BOUND_BYTES);
        let venue = establish_live_quote(&mut run, "phase6-storage-owned-order").await;

        ingest_fill_batch(&mut run, venue, 1_024, 20, 2_100);
        let filled = run.mutation_counters();
        assert_eq!(filled.unique_fills(), 1_024);
        assert_eq!(run.persistence_metrics().depth(), 1_024);
        assert_eq!(run.persistence_metrics().high_water(), 1_024);
        let reconciliation_frames = fill_batches(venue, 1_024);
        let reconciliation_frame_refs = reconciliation_frames
            .iter()
            .map(|frame| frame.as_bytes())
            .collect::<Vec<_>>();
        super::super::reconcile_reached_overload_fills_without_watermark_advance(
            &mut run,
            &reconciliation_frame_refs,
        )
        .expect("exact duplicate fill cut restores private convergence");
        assert_eq!(run.persistence_metrics().depth(), 1_024);
        let before_effect = run.fake_effect_metrics();
        let before_counters = run.mutation_counters();

        run.schedule(
            PmOrderSide::Buy,
            PmScheduledActionKind::QuoteEvaluation,
            2_200,
            2_199,
            1_700_000_000_001,
        )
        .expect("final quote attempt reaches the due schedule");
        assert!(
            run.service_turn(2_200).is_err(),
            "1025th persistence-storage quote admission must reject"
        );

        let after = run.mutation_counters();
        assert_eq!(after.quote_attempts(), before_counters.quote_attempts() + 1);
        assert_eq!(after.quote_intents(), before_counters.quote_intents());
        assert_eq!(after.fact_records(), before_counters.fact_records());
        assert_eq!(
            run.mutation_halt(),
            Some(PmMutationHalt::PersistenceSaturated)
        );
        let persistence = run.persistence_metrics();
        assert_eq!(persistence.capacity(), 1_024);
        assert_eq!(persistence.depth(), 1_024);
        assert_eq!(persistence.saturations(), 1);
        assert!(persistence.globally_stopped());

        let after_effect = run.fake_effect_metrics();
        assert_eq!(after_effect.depth(), 0, "rejected permit is not retained");
        assert_eq!(
            after_effect.reservations(),
            before_effect.reservations() + 1
        );
        assert_eq!(
            after_effect.released_before_journal(),
            before_effect.released_before_journal() + 1
        );
        assert_eq!(after_effect.serviced(), before_effect.serviced());
        assert_eq!(
            drain_outputs(&mut run),
            0,
            "rejected quote never dispatches"
        );
        assert_eq!(run.reserved_capacity_bytes(), reserved);
        let _ = run.shutdown().await;
    });
}
