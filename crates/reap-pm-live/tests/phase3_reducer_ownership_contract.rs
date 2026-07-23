#[allow(dead_code)]
mod support;

use reap_pm_live::{
    OkxPublicCaptureEvent, PmLaneKind, PmLanePolicy, PmPublicAgedLaneFaultEnactment,
    PmPublicBookReadinessReason, PmPublicCapture, PmPublicCaptureRun, PmPublicLaneAdmissionError,
    PmPublicLaneService, PmServiceTurnError,
};
use reap_pm_state::{PmBookCounters, PmExternalBookFault, PmPublicReadinessReason};
use reap_polymarket_adapter::PmPublicSessionFault;

const WALL: u64 = 1_700_000_000_000_000_000;
const PUBLIC_MAX_AGE_NS: u64 = 500_000_000;

#[derive(Default)]
struct NoopLaneService;

impl PmPublicLaneService for NoopLaneService {
    fn on_pm_public_unavailable(
        &mut self,
        _item: reap_pm_live::ServicedLaneItem<reap_pm_live::PmPublicUnavailable>,
    ) {
    }

    fn on_okx_public_unavailable(
        &mut self,
        _item: reap_pm_live::ServicedLaneItem<reap_pm_live::OkxPublicUnavailable>,
    ) {
    }

    fn on_market(&mut self, _item: reap_pm_live::ServicedLaneItem<reap_pm_core::PmMarketEvent>) {}

    fn on_book(&mut self, _item: reap_pm_live::ServicedLaneItem<reap_pm_core::PmBookEvent>) {}

    fn on_reference(
        &mut self,
        _item: reap_pm_live::ServicedLaneItem<reap_pm_core::OkxReferenceEvent>,
    ) {
    }
}

async fn start_run(path: std::path::PathBuf) -> PmPublicCaptureRun {
    PmPublicCapture::new(support::public_config())
        .unwrap()
        .start(
            path,
            support::authoritative(),
            support::session_policy(),
            support::provenance(),
        )
        .await
        .unwrap()
}

async fn start_live_run(path: std::path::PathBuf) -> PmPublicCaptureRun {
    let mut run = start_run(path).await;
    run.record_pm_connection_started(60).await.unwrap();
    run.record_okx_connection_started(61).await.unwrap();
    run.record_pm_subscription_sent(80).await.unwrap();
    run.record_okx_subscription_sent(81).await.unwrap();
    run
}

async fn acknowledge_okx(run: &mut PmPublicCaptureRun) {
    assert!(matches!(
        run.capture_okx_public(WALL + 90, 90, support::okx_ack().as_bytes())
            .await
            .unwrap(),
        OkxPublicCaptureEvent::SubscriptionAcknowledged(_)
    ));
}

async fn capture_okx_reference(run: &mut PmPublicCaptureRun, receive_ns: u64) {
    assert_eq!(
        run.capture_okx_public(
            WALL + receive_ns,
            receive_ns,
            support::okx_reference().as_bytes(),
        )
        .await
        .unwrap(),
        OkxPublicCaptureEvent::ReferenceEnqueued
    );
}

async fn open_snapshot(run: &mut PmPublicCaptureRun, receive_ns: u64) {
    let mut batch = run
        .capture_pm_public(
            WALL + receive_ns,
            receive_ns,
            support::snapshot_one().as_bytes(),
        )
        .await
        .unwrap();
    let flow = batch.take_snapshot_flow().unwrap();
    let delivery = batch.into_books().into_iter().next().unwrap();
    run.commit_then_enqueue_pm_snapshot(delivery, flow).unwrap();
    assert_eq!(
        run.service_lane_turn(receive_ns, &mut NoopLaneService)
            .unwrap(),
        1
    );
}

async fn reconnect_and_open_first_snapshot(run: &mut PmPublicCaptureRun, receive_ns: u64) {
    assert_eq!(
        run.service_lane_turn(receive_ns, &mut NoopLaneService)
            .unwrap(),
        1,
        "the internally admitted unavailable occurrence must be consumed before reconnect"
    );
    run.record_pm_reconnect_scheduled(receive_ns).await.unwrap();
    run.record_pm_connection_started(receive_ns + 1)
        .await
        .unwrap();
    run.record_pm_subscription_sent(receive_ns + 2)
        .await
        .unwrap();
    open_snapshot(run, receive_ns + 3).await;
    assert_eq!(
        run.ready_pm_book_view().unwrap().connection_epoch().value(),
        12
    );
}

fn repeated_snapshot_consistent_top(count: usize) -> Vec<u8> {
    let top = support::bbo()
        .replace(r#""best_bid":"0.50""#, r#""best_bid":"0.40""#)
        .replace(r#""bid_size":"12.5""#, r#""bid_size":"50""#);
    format!(
        "[{}]",
        std::iter::repeat_n(top, count)
            .collect::<Vec<_>>()
            .join(",")
    )
    .into_bytes()
}

async fn fill_public_lane_with_pm_books(run: &mut PmPublicCaptureRun, count: usize) {
    let mut remaining = count;
    let mut receive_ns = 120;
    while remaining != 0 {
        let frame_count = remaining.min(64);
        let deliveries = run
            .capture_pm_public(
                WALL + receive_ns,
                receive_ns,
                &repeated_snapshot_consistent_top(frame_count),
            )
            .await
            .unwrap()
            .into_books();
        assert_eq!(deliveries.len(), frame_count);
        for delivery in deliveries {
            run.reduce_then_enqueue_pm_book(delivery).unwrap();
        }
        remaining -= frame_count;
        receive_ns += 1;
    }
}

#[tokio::test]
async fn internally_prepared_reducer_has_exact_pristine_start_history() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_run(directory.path().join("pristine-start.jsonl")).await;
    assert_eq!(
        run.pm_book_counters(),
        PmBookCounters {
            metadata_inputs: 1,
            metadata_accepted: 1,
            epoch_attempts: 1,
            epochs_started: 1,
            ..PmBookCounters::default()
        }
    );
    let readiness = run.pm_book_readiness();
    assert_eq!(
        readiness.reason(),
        Some(PmPublicBookReadinessReason::LifecycleUnavailable)
    );
    assert_eq!(readiness.metadata_revision().unwrap().value(), 7);
    assert_eq!(readiness.snapshot_revision(), None);
    assert_eq!(run.pm_book_last_ingress_sequence(), None);
    assert_eq!(run.pm_book_last_verified_snapshot_hash(), None);
    assert_eq!(run.pm_book_pending_external_fault(), None);
    assert!(run.ready_pm_book_view().is_none());

    run.record_pm_connection_started(60).await.unwrap();
    run.record_pm_subscription_sent(80).await.unwrap();
    assert_eq!(
        run.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::Reducer(
            PmPublicReadinessReason::SnapshotMissing
        ))
    );
    assert!(run.ready_pm_book_view().is_none());
    run.finish().await.unwrap();
}

#[tokio::test]
async fn pre_snapshot_metadata_full_latches_without_false_transition_and_recovers() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("metadata-full.jsonl")).await;
    open_snapshot(&mut run, 110).await;
    let capacity = PmLanePolicy::for_lane(PmLaneKind::Public).capacity();
    fill_public_lane_with_pm_books(&mut run, capacity).await;
    let before = run.pm_book_counters();
    let failure = run
        .issue_and_enqueue_pm_metadata(WALL + 199)
        .unwrap_err()
        .into_lane_failure()
        .unwrap();
    assert!(matches!(&failure, PmPublicLaneAdmissionError::Lane(_)));
    assert_eq!(
        run.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::PendingLaneFault)
    );
    assert_eq!(
        run.pm_book_pending_external_fault(),
        Some(PmExternalBookFault::Overflow)
    );
    let pending = run.pm_book_counters();
    assert_eq!(
        pending.unavailable_transitions,
        before.unavailable_transitions + 1
    );
    assert_eq!(pending.external_faults, before.external_faults);
    assert_eq!(pending.overflows, before.overflows);
    assert_eq!(pending.invalidations, before.invalidations);

    let enacted = run
        .enact_pm_metadata_lane_failure(failure, WALL + 300, 300)
        .await
        .unwrap();
    assert_eq!(enacted.unavailable_fault(), PmPublicSessionFault::Overflow);
    assert_eq!(
        enacted.reducer_reason(),
        Some(PmPublicReadinessReason::Overflow)
    );
    let after = run.pm_book_counters();
    assert_eq!(after.external_faults, before.external_faults + 1);
    assert_eq!(after.overflows, before.overflows + 1);
    assert_eq!(after.invalidations, before.invalidations + 1);
    assert_eq!(
        after.unavailable_transitions,
        before.unavailable_transitions + 1
    );
    assert_eq!(run.pm_book_pending_external_fault(), None);
    assert!(!run.artifact_terminal());

    reconnect_and_open_first_snapshot(&mut run, 303).await;
    assert!(run.pm_book_readiness().is_ready());
    run.finish().await.unwrap();
}

#[tokio::test]
async fn pre_snapshot_metadata_aged_latches_without_false_transition_and_recovers() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("metadata-aged.jsonl")).await;
    run.issue_and_enqueue_pm_metadata(WALL + 50).unwrap();
    let before = run.pm_book_counters();
    let observed_now_ns = 50 + PUBLIC_MAX_AGE_NS + 1;
    let failure = run
        .service_lane_turn(observed_now_ns, &mut NoopLaneService)
        .unwrap_err();
    assert!(matches!(&failure, PmServiceTurnError::Aged(_)));
    assert_eq!(
        run.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::PendingLaneFault)
    );
    assert_eq!(
        run.pm_book_pending_external_fault(),
        Some(PmExternalBookFault::BacklogAged)
    );
    assert_eq!(run.pm_book_counters(), before);

    let enacted = run
        .enact_public_lane_aged(failure, WALL + observed_now_ns, observed_now_ns)
        .await
        .unwrap();
    let PmPublicAgedLaneFaultEnactment::Polymarket {
        unavailable_fault,
        reducer_reason,
        purged_queued_deliveries,
        ..
    } = enacted
    else {
        panic!("aged PM metadata must retain its PM route");
    };
    assert_eq!(unavailable_fault, PmPublicSessionFault::Stale);
    assert_eq!(reducer_reason, PmPublicReadinessReason::BookStale);
    assert_eq!(purged_queued_deliveries, 1);
    let after = run.pm_book_counters();
    assert_eq!(after.external_faults, before.external_faults + 1);
    assert_eq!(after.backlog_aged_faults, before.backlog_aged_faults + 1);
    assert_eq!(after.stale_invalidations, before.stale_invalidations + 1);
    assert_eq!(after.invalidations, before.invalidations + 1);
    assert_eq!(
        after.unavailable_transitions,
        before.unavailable_transitions
    );
    assert_eq!(run.pm_book_pending_external_fault(), None);
    assert!(!run.artifact_terminal());

    reconnect_and_open_first_snapshot(&mut run, observed_now_ns + 1).await;
    assert!(run.pm_book_readiness().is_ready());
    run.finish().await.unwrap();
}

#[tokio::test]
async fn sibling_run_cannot_latch_or_terminalize_another_runs_aged_head() {
    let directory = tempfile::tempdir().unwrap();
    let mut owner = start_live_run(directory.path().join("aged-owner.jsonl")).await;
    let mut sibling = start_live_run(directory.path().join("aged-sibling.jsonl")).await;
    open_snapshot(&mut owner, 110).await;
    open_snapshot(&mut sibling, 110).await;

    let delivery = owner
        .capture_pm_public(WALL + 120, 120, support::delta().as_bytes())
        .await
        .unwrap()
        .into_books()
        .into_iter()
        .next()
        .unwrap();
    owner.reduce_then_enqueue_pm_book(delivery).unwrap();
    let observed_now_ns = 120 + PUBLIC_MAX_AGE_NS + 1;
    let sibling_counters = sibling.pm_book_counters();
    let sibling_readiness = sibling.pm_book_readiness();
    assert_eq!(
        sibling
            .service_lane_turn(observed_now_ns, &mut NoopLaneService)
            .unwrap(),
        0,
        "a fresh sibling Run cannot observe or service the owner's aged head"
    );
    assert_eq!(sibling.pm_book_counters(), sibling_counters);
    assert_eq!(sibling.pm_book_readiness(), sibling_readiness);
    assert_eq!(sibling.pm_book_pending_external_fault(), None);
    assert!(!sibling.has_pending_pm_book_lane_fault());
    assert!(!sibling.artifact_terminal());
    assert!(sibling.ready_pm_book_view().is_some());
    let owner_failure = owner
        .service_lane_turn(observed_now_ns, &mut NoopLaneService)
        .unwrap_err();
    assert_eq!(
        owner.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::PendingLaneFault)
    );
    assert_eq!(
        owner.pm_book_pending_external_fault(),
        Some(PmExternalBookFault::BacklogAged)
    );
    owner
        .enact_public_lane_aged(owner_failure, WALL + observed_now_ns, observed_now_ns)
        .await
        .unwrap();
    assert_eq!(owner.pm_book_counters().backlog_aged_faults, 1);
    assert!(!owner.artifact_terminal());
    assert_eq!(
        owner
            .service_lane_turn(observed_now_ns + 1, &mut NoopLaneService)
            .unwrap(),
        1
    );
    owner.finish().await.unwrap();
    sibling.finish().await.unwrap();
}

#[tokio::test]
async fn timer_stale_book_can_still_latch_exact_aged_delivery_and_resync() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("timer-then-aged.jsonl")).await;
    open_snapshot(&mut run, 110).await;
    let delivery = run
        .capture_pm_public(WALL + 120, 120, support::delta().as_bytes())
        .await
        .unwrap()
        .into_books()
        .into_iter()
        .next()
        .unwrap();
    run.reduce_then_enqueue_pm_book(delivery).unwrap();

    let timer = run.record_freshness_timer(1_121).await.unwrap();
    assert_eq!(
        timer.unavailable_reason(),
        Some(PmPublicReadinessReason::BookStale)
    );
    let before = run.pm_book_counters();
    assert_eq!(
        run.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::Reducer(
            PmPublicReadinessReason::BookStale
        ))
    );

    let observed_now_ns = 120 + PUBLIC_MAX_AGE_NS + 1;
    let failure = run
        .service_lane_turn(observed_now_ns, &mut NoopLaneService)
        .unwrap_err();
    assert_eq!(
        run.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::PendingLaneFault)
    );
    assert_eq!(
        run.pm_book_pending_external_fault(),
        Some(PmExternalBookFault::BacklogAged)
    );
    assert_eq!(
        run.pm_book_counters(),
        before,
        "an already-stale reducer must not invent a second unavailable transition while the exact aged proof is pending"
    );

    run.enact_public_lane_aged(failure, WALL + observed_now_ns, observed_now_ns)
        .await
        .unwrap();
    let after = run.pm_book_counters();
    assert_eq!(after.external_faults, before.external_faults + 1);
    assert_eq!(after.backlog_aged_faults, before.backlog_aged_faults + 1);
    assert_eq!(after.stale_invalidations, before.stale_invalidations + 1);
    assert_eq!(after.invalidations, before.invalidations + 1);
    assert_eq!(
        after.unavailable_transitions,
        before.unavailable_transitions
    );

    assert_eq!(
        run.service_lane_turn(observed_now_ns + 1, &mut NoopLaneService)
            .unwrap(),
        1
    );
    run.record_pm_reconnect_scheduled(observed_now_ns + 1)
        .await
        .unwrap();
    run.record_pm_connection_started(observed_now_ns + 2)
        .await
        .unwrap();
    run.record_pm_subscription_sent(observed_now_ns + 3)
        .await
        .unwrap();
    let mut snapshot = run
        .capture_pm_public(
            WALL + observed_now_ns + 4,
            observed_now_ns + 4,
            support::snapshot_two().as_bytes(),
        )
        .await
        .unwrap();
    let flow = snapshot.take_snapshot_flow().unwrap();
    let delivery = snapshot.into_books().into_iter().next().unwrap();
    run.commit_then_enqueue_pm_snapshot(delivery, flow).unwrap();
    assert!(run.pm_book_readiness().is_ready());
    assert_eq!(
        run.service_lane_turn(observed_now_ns + 4, &mut NoopLaneService)
            .unwrap(),
        1
    );
    run.finish().await.unwrap();
}

#[tokio::test]
async fn dropped_pm_and_okx_aged_proofs_keep_quote_authority_closed() {
    let directory = tempfile::tempdir().unwrap();
    let mut pm = start_live_run(directory.path().join("dropped-pm-aged.jsonl")).await;
    open_snapshot(&mut pm, 110).await;
    let delivery = pm
        .capture_pm_public(WALL + 120, 120, support::delta().as_bytes())
        .await
        .unwrap()
        .into_books()
        .into_iter()
        .next()
        .unwrap();
    pm.reduce_then_enqueue_pm_book(delivery).unwrap();
    let pm_before = pm.pm_book_counters();
    let observed_now_ns = 120 + PUBLIC_MAX_AGE_NS + 1;
    let pm_failure = pm
        .service_lane_turn(observed_now_ns, &mut NoopLaneService)
        .unwrap_err();
    let pm_pending = pm.pm_book_counters();
    assert_eq!(
        pm.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::PendingLaneFault)
    );
    assert!(pm.ready_pm_book_view().is_none());
    assert_eq!(
        pm.pm_book_pending_external_fault(),
        Some(PmExternalBookFault::BacklogAged)
    );
    assert_eq!(
        pm_pending.unavailable_transitions,
        pm_before.unavailable_transitions + 1
    );
    assert_eq!(pm_pending.external_faults, pm_before.external_faults);
    assert_eq!(
        pm_pending.backlog_aged_faults,
        pm_before.backlog_aged_faults
    );
    assert_eq!(pm_pending.invalidations, pm_before.invalidations);
    {
        let _intentionally_abandoned = pm_failure;
    }
    assert_eq!(pm.pm_book_counters(), pm_pending);
    assert_eq!(
        pm.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::PendingLaneFault)
    );
    assert!(matches!(
        pm.finish().await,
        Err(reap_pm_live::PmPublicCaptureRunError::PendingPmBookLaneFaultFinish { .. })
    ));

    let mut okx = start_live_run(directory.path().join("dropped-okx-aged.jsonl")).await;
    acknowledge_okx(&mut okx).await;
    open_snapshot(&mut okx, 110).await;
    capture_okx_reference(&mut okx, 120).await;
    let okx_before = okx.pm_book_counters();
    let okx_failure = okx
        .service_lane_turn(observed_now_ns, &mut NoopLaneService)
        .unwrap_err();
    assert_eq!(okx.pm_book_counters(), okx_before);
    assert_eq!(
        okx.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::PendingLaneFault)
    );
    assert!(okx.ready_pm_book_view().is_none());
    assert_eq!(okx.pm_book_pending_external_fault(), None);
    {
        let _intentionally_abandoned = okx_failure;
    }
    assert_eq!(okx.pm_book_counters(), okx_before);
    assert!(matches!(
        okx.finish().await,
        Err(reap_pm_live::PmPublicCaptureRunError::PendingPmBookLaneFaultFinish { .. })
    ));
}

#[tokio::test]
async fn dropped_okx_full_proof_blocks_quotes_without_mutating_pm_reducer() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("dropped-okx-full.jsonl")).await;
    acknowledge_okx(&mut run).await;
    open_snapshot(&mut run, 110).await;
    let capacity = PmLanePolicy::for_lane(PmLaneKind::Public).capacity();
    fill_public_lane_with_pm_books(&mut run, capacity).await;
    let before = run.pm_book_counters();
    let failure = run
        .capture_okx_public(WALL + 300, 300, support::okx_reference().as_bytes())
        .await
        .unwrap_err()
        .into_lane_failure()
        .unwrap();
    assert_eq!(run.pm_book_counters(), before);
    assert_eq!(
        run.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::PendingLaneFault)
    );
    assert!(run.ready_pm_book_view().is_none());
    assert_eq!(run.pm_book_pending_external_fault(), None);
    {
        let _intentionally_abandoned = failure;
    }
    assert_eq!(run.pm_book_counters(), before);
    assert!(matches!(
        run.finish().await,
        Err(reap_pm_live::PmPublicCaptureRunError::PendingPmBookLaneFaultFinish { .. })
    ));
}
