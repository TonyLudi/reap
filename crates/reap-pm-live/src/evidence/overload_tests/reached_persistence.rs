use crate::{PmControlReason, PmLaneKind, SaturationAction};

use super::reached_mutation_support::{
    OWNER_MEMORY_BOUND_BYTES, completion, drain_outputs, establish_live_quote, ingest_fill_batch,
    start, wait_for_ack_admission,
};

#[test]
fn persistence_ack_row_reaches_product_513_times_and_globally_stops() {
    super::run_product_test(|| async {
        let (_directory, mut run) = start(
            "persistence",
            super::super::ReachedOverloadProfile::Persistence513,
        )
        .await;
        let reserved = run.reserved_capacity_bytes();
        assert!(reserved <= OWNER_MEMORY_BOUND_BYTES);
        let venue = establish_live_quote(&mut run, "phase6-persistence-owned-order").await;
        ingest_fill_batch(&mut run, venue, 513, 20, 2_100);
        assert_eq!(run.mutation_counters().unique_fills(), 513);
        assert_eq!(run.persistence_metrics().depth(), 513);
        let before_effect = run.fake_effect_metrics();

        for attempt in 1..=512_u64 {
            wait_for_ack_admission(&mut run, 100 + attempt, 3_000 + attempt)
                .await
                .expect("first 512 durable acknowledgements enter the persistence lane");
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            match run.poll_persistence_fixture(completion(613, 3_513), 3_513) {
                Ok(false) if tokio::time::Instant::now() < deadline => {
                    tokio::task::yield_now().await
                }
                Ok(false) => panic!("timed out waiting for the 513th durable acknowledgement"),
                Ok(true) => panic!("513th persistence acknowledgement must be rejected"),
                Err(_) => break,
            }
        }

        let metrics = run.scheduler_metrics(4_000).expect("scheduler metrics");
        let lane = metrics
            .lane(PmLaneKind::Persistence)
            .expect("persistence lane");
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
        assert_eq!(
            run.fake_effect_metrics().serviced(),
            before_effect.serviced()
        );
        assert_eq!(drain_outputs(&mut run), 0);
        assert_eq!(run.reserved_capacity_bytes(), reserved);
        let _ = run.shutdown().await;
    });
}
