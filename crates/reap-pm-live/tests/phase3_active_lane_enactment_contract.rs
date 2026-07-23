#[allow(dead_code)]
mod support;

use reap_okx_public_source::OkxPublicSessionFault;
use reap_pm_live::{
    OkxPublicCaptureEvent, PmLaneKind, PmLanePolicy, PmPublicAgedLaneEnactError,
    PmPublicAgedLaneFaultEnactment, PmPublicBookPipelineError, PmPublicBookReadinessReason,
    PmPublicCapture, PmPublicCaptureRun, PmPublicCaptureRunError, PmPublicCaptureTerminalCause,
    PmPublicDataPipelineError, PmPublicLaneEnactError, PmPublicLaneService,
    PmPublicNotificationAdmissionFailure, PmPublicSnapshotCommitError,
};
use reap_pm_state::{PmExternalBookFault, PmPublicReadinessReason};
use reap_polymarket_adapter::PmPublicSessionFault;

use support::{
    authoritative, bbo, okx_ack, okx_reference, provenance, public_config, session_policy,
    snapshot_one,
};

const WALL_BASE: u64 = 1_700_000_000_000_000_000;
const PUBLIC_MAX_AGE_NS: u64 = 500_000_000;
const MAX_PM_EVENTS_PER_FRAME: usize = 64;

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
    PmPublicCapture::new(public_config())
        .unwrap()
        .start(path, authoritative(), session_policy(), provenance())
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

async fn open_pm_snapshot(run: &mut PmPublicCaptureRun) {
    let mut batch = run
        .capture_pm_public(WALL_BASE + 100, 100, snapshot_one().as_bytes())
        .await
        .unwrap();
    let flow = batch.take_snapshot_flow().expect("snapshot flow");
    let delivery = batch
        .into_books()
        .into_iter()
        .next()
        .expect("snapshot delivery");
    run.commit_then_enqueue_pm_snapshot(delivery, flow).unwrap();
    assert_eq!(
        run.service_lane_turn(100, &mut NoopLaneService).unwrap(),
        1,
        "the atomically admitted snapshot is consumed before each focused lane scenario"
    );
}

async fn acknowledge_okx(run: &mut PmPublicCaptureRun) {
    assert!(matches!(
        run.capture_okx_public(WALL_BASE + 90, 90, okx_ack().as_bytes())
            .await
            .unwrap(),
        OkxPublicCaptureEvent::SubscriptionAcknowledged(_)
    ));
}

fn repeated_pm_frame(event: &str, count: usize) -> Vec<u8> {
    assert!((1..=MAX_PM_EVENTS_PER_FRAME).contains(&count));
    format!(
        "[{}]",
        std::iter::repeat_n(event, count)
            .collect::<Vec<_>>()
            .join(",")
    )
    .into_bytes()
}

fn snapshot_consistent_bbo() -> String {
    bbo()
        .replace(r#""best_bid":"0.50""#, r#""best_bid":"0.40""#)
        .replace(r#""bid_size":"12.5""#, r#""bid_size":"50""#)
}

async fn fill_with_pm_books(
    run: &mut PmPublicCaptureRun,
    count: usize,
    first_receive_ns: u64,
) -> u64 {
    let mut remaining = count;
    let mut receive_ns = first_receive_ns;
    while remaining != 0 {
        let frame_count = remaining.min(MAX_PM_EVENTS_PER_FRAME);
        let raw = repeated_pm_frame(&snapshot_consistent_bbo(), frame_count);
        let deliveries = run
            .capture_pm_public(WALL_BASE + receive_ns, receive_ns, &raw)
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
    receive_ns
}

async fn capture_okx_reference(
    run: &mut PmPublicCaptureRun,
    receive_ns: u64,
) -> Result<
    OkxPublicCaptureEvent,
    reap_pm_live::PmPublicDataPipelineError<reap_pm_live::OkxPublicReferenceDelivery>,
> {
    run.capture_okx_public(
        WALL_BASE + receive_ns,
        receive_ns,
        okx_reference().as_bytes(),
    )
    .await
}

async fn fill_with_okx_references(
    run: &mut PmPublicCaptureRun,
    count: usize,
    first_receive_ns: u64,
) -> u64 {
    let mut receive_ns = first_receive_ns;
    for _ in 0..count {
        assert!(matches!(
            capture_okx_reference(run, receive_ns).await.unwrap(),
            OkxPublicCaptureEvent::ReferenceEnqueued
        ));
        receive_ns += 1;
    }
    receive_ns
}

fn drain_public_lane(run: &mut PmPublicCaptureRun, monotonic_now_ns: u64) {
    while run.public_lane_metrics().depth() != 0 {
        run.service_lane_turn(monotonic_now_ns, &mut NoopLaneService)
            .unwrap();
    }
}

#[tokio::test]
async fn active_pm_full_maps_to_overflow_and_purges_only_the_pm_route() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("pm-full.jsonl")).await;
    acknowledge_okx(&mut run).await;
    open_pm_snapshot(&mut run).await;

    let capacity = PmLanePolicy::for_lane(PmLaneKind::Public).capacity();
    assert!(matches!(
        capture_okx_reference(&mut run, 900).await.unwrap(),
        OkxPublicCaptureEvent::ReferenceEnqueued
    ));
    let next_receive = fill_with_pm_books(&mut run, capacity - 1, 1_000).await;
    assert_eq!(run.public_lane_metrics().depth(), capacity);

    let rejected = run
        .capture_pm_public(
            WALL_BASE + next_receive,
            next_receive,
            snapshot_consistent_bbo().as_bytes(),
        )
        .await
        .unwrap()
        .into_books()
        .into_iter()
        .next()
        .unwrap();
    let rejected_ordering = rejected.envelope().ordering();
    let failure = run
        .reduce_then_enqueue_pm_book(rejected)
        .unwrap_err()
        .into_lane_failure()
        .expect("the reduced delivery reached the bounded lane");
    assert_eq!(run.public_lane_metrics().rejected_full(), 1);
    assert!(run.has_pending_pm_book_lane_fault());
    assert!(matches!(
        run.capture_pm_public(
            WALL_BASE + next_receive + 1,
            next_receive + 1,
            snapshot_consistent_bbo().as_bytes(),
        )
        .await,
        Err(PmPublicCaptureRunError::PendingPmBookLaneFault)
    ));
    assert!(matches!(
        run.record_pm_disconnected(WALL_BASE + next_receive + 1, next_receive + 1)
            .await,
        Err(PmPublicCaptureRunError::PendingPmBookLaneFault)
    ));

    let enacted = run
        .enact_pm_book_lane_failure(failure, WALL_BASE + next_receive + 1, next_receive + 1)
        .await
        .unwrap();
    assert_eq!(enacted.rejected_ordering(), rejected_ordering);
    assert_eq!(enacted.unavailable_fault(), PmPublicSessionFault::Overflow);
    assert_eq!(
        enacted.reducer_reason(),
        Some(PmPublicReadinessReason::Overflow)
    );
    assert_eq!(enacted.purged_queued_deliveries(), capacity - 1);
    assert_eq!(
        run.public_lane_metrics().depth(),
        2,
        "the sibling OKX route and mandatory PM unavailability both remain admitted"
    );
    assert_eq!(
        run.public_lane_metrics().invalidated_purged(),
        u64::try_from(capacity - 1).unwrap()
    );
    assert_eq!(run.pm_book_counters().external_faults, 1);
    assert_eq!(run.pm_book_counters().overflows, 1);
    assert_eq!(run.pm_book_counters().backlog_aged_faults, 0);
    assert!(!run.artifact_terminal());
    assert!(!run.has_pending_pm_book_lane_fault());

    assert!(matches!(
        run.record_pm_reconnect_scheduled(next_receive + 2).await,
        Err(PmPublicCaptureRunError::PendingPmPublicRouteReconnect {
            epoch: 11,
            pending: 1
        })
    ));
    drain_public_lane(&mut run, next_receive + 2);
    run.record_pm_reconnect_scheduled(next_receive + 2)
        .await
        .unwrap();
    run.record_pm_connection_started(next_receive + 3)
        .await
        .unwrap();
    run.record_pm_subscription_sent(next_receive + 4)
        .await
        .unwrap();
    let mut resync = run
        .capture_pm_public(
            WALL_BASE + next_receive + 5,
            next_receive + 5,
            snapshot_one().as_bytes(),
        )
        .await
        .unwrap();
    let flow = resync.take_snapshot_flow().unwrap();
    let delivery = resync.into_books().into_iter().next().unwrap();
    run.commit_then_enqueue_pm_snapshot(delivery, flow).unwrap();
    assert_eq!(
        run.ready_pm_book_view().unwrap().connection_epoch().value(),
        12
    );

    drain_public_lane(&mut run, next_receive + 6);
    let outcome = run.finish().await.unwrap();
    assert_eq!(outcome.projection().counters().overflows, 1);
    assert_eq!(outcome.projection().counters().backlog_aged_faults, 0);
}

#[tokio::test]
async fn dropped_reduced_full_proof_blocks_capture_and_normal_finish() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("pm-full-dropped.jsonl")).await;
    open_pm_snapshot(&mut run).await;
    let capacity = PmLanePolicy::for_lane(PmLaneKind::Public).capacity();
    let next_receive = fill_with_pm_books(&mut run, capacity, 1_000).await;
    let rejected = run
        .capture_pm_public(
            WALL_BASE + next_receive,
            next_receive,
            snapshot_consistent_bbo().as_bytes(),
        )
        .await
        .unwrap()
        .into_books()
        .into_iter()
        .next()
        .unwrap();
    let before = run.pm_book_counters();
    let failure = run
        .reduce_then_enqueue_pm_book(rejected)
        .unwrap_err()
        .into_lane_failure()
        .unwrap();
    let pending = run.pm_book_counters();
    assert_eq!(
        run.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::PendingLaneFault)
    );
    assert!(run.ready_pm_book_view().is_none());
    assert_eq!(
        run.pm_book_pending_external_fault(),
        Some(PmExternalBookFault::Overflow)
    );
    assert_eq!(
        pending.unavailable_transitions,
        before.unavailable_transitions + 1
    );
    assert_eq!(pending.external_faults, before.external_faults);
    assert_eq!(pending.overflows, before.overflows);
    assert_eq!(pending.invalidations, before.invalidations);
    drop(failure);
    assert!(run.has_pending_pm_book_lane_fault());
    assert_eq!(
        run.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::PendingLaneFault)
    );
    assert_eq!(run.pm_book_counters(), pending);
    assert!(matches!(
        run.finish().await,
        Err(PmPublicCaptureRunError::PendingPmBookLaneFaultFinish { .. })
    ));
}

#[tokio::test]
async fn sibling_full_proof_cannot_mutate_or_service_the_owner_lane() {
    let directory = tempfile::tempdir().unwrap();
    let mut owner = start_live_run(directory.path().join("pm-full-owner.jsonl")).await;
    let mut sibling = start_live_run(directory.path().join("pm-full-sibling.jsonl")).await;
    open_pm_snapshot(&mut owner).await;
    open_pm_snapshot(&mut sibling).await;
    let capacity = PmLanePolicy::for_lane(PmLaneKind::Public).capacity();
    let next_receive = fill_with_pm_books(&mut owner, capacity, 1_000).await;
    let rejected = owner
        .capture_pm_public(
            WALL_BASE + next_receive,
            next_receive,
            snapshot_consistent_bbo().as_bytes(),
        )
        .await
        .unwrap()
        .into_books()
        .into_iter()
        .next()
        .unwrap();
    let failure = owner
        .reduce_then_enqueue_pm_book(rejected)
        .unwrap_err()
        .into_lane_failure()
        .unwrap();
    let owner_before = owner.pm_book_counters();
    let sibling_before = sibling.pm_book_counters();
    let failure = match sibling
        .enact_pm_book_lane_failure(failure, WALL_BASE + next_receive + 1, next_receive + 1)
        .await
        .unwrap_err()
    {
        PmPublicLaneEnactError::PendingBookFaultMismatch { failure } => failure,
        other => panic!("sibling must reject the owner's Full proof: {other:?}"),
    };
    assert_eq!(sibling.pm_book_counters(), sibling_before);
    assert_eq!(owner.pm_book_counters(), owner_before);
    assert!(owner.has_pending_pm_book_lane_fault());
    assert_eq!(owner.public_lane_metrics().depth(), capacity);
    assert_eq!(sibling.public_lane_metrics().depth(), 0);

    owner
        .enact_pm_book_lane_failure(failure, WALL_BASE + next_receive + 2, next_receive + 2)
        .await
        .unwrap();
    assert_eq!(owner.public_lane_metrics().depth(), 1);
    drain_public_lane(&mut owner, next_receive + 3);
    owner.finish().await.unwrap();
    sibling.finish().await.unwrap();
}

#[tokio::test]
async fn authentic_full_with_regressed_clock_still_invalidates_bound_reducer() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("pm-full-regressed-clock.jsonl")).await;
    open_pm_snapshot(&mut run).await;
    let capacity = PmLanePolicy::for_lane(PmLaneKind::Public).capacity();
    let next_receive = fill_with_pm_books(&mut run, capacity, 1_000).await;
    let rejected = run
        .capture_pm_public(
            WALL_BASE + next_receive,
            next_receive,
            snapshot_consistent_bbo().as_bytes(),
        )
        .await
        .unwrap()
        .into_books()
        .into_iter()
        .next()
        .unwrap();
    let failure = run
        .reduce_then_enqueue_pm_book(rejected)
        .unwrap_err()
        .into_lane_failure()
        .unwrap();
    let before = run.pm_book_counters();

    assert!(matches!(
        run.enact_pm_book_lane_failure(failure, WALL_BASE + next_receive + 1, next_receive - 1,)
            .await,
        Err(PmPublicLaneEnactError::Fault { .. })
    ));
    assert_eq!(
        run.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::ArtifactTerminal)
    );
    assert_eq!(
        run.pm_book_counters().external_faults,
        before.external_faults + 1
    );
    assert_eq!(
        run.pm_book_counters().invalid_transitions,
        before.invalid_transitions + 1
    );
    assert!(!run.has_pending_pm_book_lane_fault());
    assert_eq!(
        run.terminal_cause(),
        Some(PmPublicCaptureTerminalCause::Lane)
    );
    assert!(matches!(
        run.finish().await,
        Err(PmPublicCaptureRunError::TerminalFinish {
            cause: PmPublicCaptureTerminalCause::Lane,
            ..
        })
    ));
}

#[tokio::test]
async fn pm_aged_preflight_failure_invalidates_after_canonical_run_service() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("pm-aged-pending-regression.jsonl")).await;
    open_pm_snapshot(&mut run).await;
    let deliveries = run
        .capture_pm_public(
            WALL_BASE + 1_000,
            1_000,
            &repeated_pm_frame(&snapshot_consistent_bbo(), 2),
        )
        .await
        .unwrap()
        .into_books()
        .into_iter();
    for delivery in deliveries {
        run.reduce_then_enqueue_pm_book(delivery).unwrap();
    }
    assert_eq!(run.pending_pm_book_reduction_count(), 0);
    let before = run.pm_book_counters();
    let observed_now_ns = 1_000 + PUBLIC_MAX_AGE_NS + 1;
    let failure = run
        .service_lane_turn(observed_now_ns, &mut NoopLaneService)
        .unwrap_err();

    assert!(matches!(
        run.enact_public_lane_aged(failure, 0, observed_now_ns)
            .await,
        Err(PmPublicAgedLaneEnactError::Fault { .. })
    ));
    assert_eq!(
        run.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::ArtifactTerminal)
    );
    assert_eq!(
        run.pm_book_counters().external_faults,
        before.external_faults + 1
    );
    assert_eq!(run.pending_pm_book_reduction_count(), 0);
    assert_eq!(
        run.terminal_cause(),
        Some(PmPublicCaptureTerminalCause::Lane)
    );
    assert!(matches!(
        run.finish().await,
        Err(PmPublicCaptureRunError::TerminalFinish {
            cause: PmPublicCaptureTerminalCause::Lane,
            ..
        })
    ));
}

#[tokio::test]
async fn first_full_proof_cannot_be_overwritten_by_a_second_frame_delivery() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("pm-full-multi-event.jsonl")).await;
    open_pm_snapshot(&mut run).await;
    let capacity = PmLanePolicy::for_lane(PmLaneKind::Public).capacity();
    let next_receive = fill_with_pm_books(&mut run, capacity, 1_000).await;
    let mut deliveries = run
        .capture_pm_public(
            WALL_BASE + next_receive,
            next_receive,
            &repeated_pm_frame(&snapshot_consistent_bbo(), 2),
        )
        .await
        .unwrap()
        .into_books()
        .into_iter();
    let first = deliveries.next().unwrap();
    let second = deliveries.next().unwrap();
    let first_ordering = first.envelope().ordering();
    let failure = run
        .reduce_then_enqueue_pm_book(first)
        .unwrap_err()
        .into_lane_failure()
        .unwrap();
    assert_eq!(run.pending_pm_book_reduction_count(), 1);
    let PmPublicBookPipelineError::Reduce(PmPublicCaptureRunError::PmBookReducePendingLaneFault {
        delivery: second,
    }) = run.reduce_then_enqueue_pm_book(second).unwrap_err()
    else {
        panic!("the second exact delivery must be returned before reduction");
    };
    drop(second);
    assert_eq!(run.pending_pm_book_reduction_count(), 1);

    let enacted = run
        .enact_pm_book_lane_failure(failure, WALL_BASE + next_receive + 1, next_receive + 1)
        .await
        .unwrap();
    assert_eq!(enacted.rejected_ordering(), first_ordering);
    assert_eq!(run.pending_pm_book_reduction_count(), 0);
    assert!(!run.has_pending_pm_book_lane_fault());
    drain_public_lane(&mut run, next_receive + 2);
    run.finish().await.unwrap();
}

#[tokio::test]
async fn pending_full_blocks_unrelated_writer_progress_until_exact_enactment() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("pm-full-writer-gate.jsonl")).await;
    acknowledge_okx(&mut run).await;
    open_pm_snapshot(&mut run).await;
    let capacity = PmLanePolicy::for_lane(PmLaneKind::Public).capacity();
    let next_receive = fill_with_pm_books(&mut run, capacity, 1_000).await;
    let rejected = run
        .capture_pm_public(
            WALL_BASE + next_receive,
            next_receive,
            snapshot_consistent_bbo().as_bytes(),
        )
        .await
        .unwrap()
        .into_books()
        .into_iter()
        .next()
        .unwrap();
    let failure = run
        .reduce_then_enqueue_pm_book(rejected)
        .unwrap_err()
        .into_lane_failure()
        .unwrap();

    assert!(matches!(
        run.capture_okx_public(
            WALL_BASE + 1_000_000_000,
            1_000_000_000,
            okx_reference().as_bytes(),
        )
        .await,
        Err(PmPublicDataPipelineError::Run(
            PmPublicCaptureRunError::PendingPmBookLaneFault
        ))
    ));
    let before = run.pm_book_counters();
    let enacted = run
        .enact_pm_book_lane_failure(failure, WALL_BASE + next_receive + 1, next_receive + 1)
        .await
        .unwrap();
    assert_eq!(enacted.unavailable_fault(), PmPublicSessionFault::Overflow);
    let after = run.pm_book_counters();
    assert_eq!(after.external_faults, before.external_faults + 1);
    assert_eq!(after.invalid_transitions, before.invalid_transitions);
    assert_eq!(after.invalidations, before.invalidations + 1);
    assert_eq!(after.overflows, before.overflows + 1);
    assert_eq!(after.backlog_aged_faults, before.backlog_aged_faults);
    assert!(!run.has_pending_pm_book_lane_fault());
    assert!(!run.artifact_terminal());
    drain_public_lane(&mut run, next_receive + 2);
    run.finish().await.unwrap();
}

#[tokio::test]
async fn active_okx_full_maps_to_overflow_and_purges_only_the_okx_route() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("okx-full.jsonl")).await;
    acknowledge_okx(&mut run).await;
    open_pm_snapshot(&mut run).await;

    let capacity = PmLanePolicy::for_lane(PmLaneKind::Public).capacity();
    let next_receive = fill_with_pm_books(&mut run, capacity - 1, 1_000).await;
    assert!(matches!(
        capture_okx_reference(&mut run, next_receive + 1)
            .await
            .unwrap(),
        OkxPublicCaptureEvent::ReferenceEnqueued
    ));
    assert_eq!(run.public_lane_metrics().depth(), capacity);

    let pipeline = capture_okx_reference(&mut run, next_receive + 2)
        .await
        .unwrap_err();
    let rejected_ordering = pipeline
        .lane_failure()
        .unwrap()
        .delivery()
        .envelope()
        .ordering();
    let failure = pipeline.into_lane_failure().unwrap();
    let reducer_before = run.pm_book_counters();
    let enacted = run
        .enact_okx_reference_lane_failure(failure, WALL_BASE + next_receive + 3, next_receive + 3)
        .await
        .unwrap();

    assert_eq!(enacted.rejected_ordering(), rejected_ordering);
    assert_eq!(enacted.unavailable_fault(), OkxPublicSessionFault::Overflow);
    assert_eq!(enacted.reducer_reason(), None);
    assert_eq!(enacted.purged_queued_deliveries(), 1);
    assert_eq!(
        run.public_lane_metrics().depth(),
        capacity,
        "the retained PM route plus mandatory OKX unavailability refill the lane"
    );
    assert_eq!(run.public_lane_metrics().invalidated_purged(), 1);
    assert_eq!(run.pm_book_counters(), reducer_before);
    assert!(!run.artifact_terminal());

    drain_public_lane(&mut run, next_receive + 4);
    let outcome = run.finish().await.unwrap();
    assert_eq!(outcome.projection().counters().okx_disconnects, 1);
    assert_eq!(outcome.projection().counters().overflows, 0);
}

#[tokio::test]
async fn active_pm_aged_maps_to_backlog_aged_and_preserves_the_okx_route() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("pm-aged.jsonl")).await;
    acknowledge_okx(&mut run).await;
    open_pm_snapshot(&mut run).await;

    let pm = run
        .capture_pm_public(
            WALL_BASE + 1_000,
            1_000,
            &repeated_pm_frame(&snapshot_consistent_bbo(), 2),
        )
        .await
        .unwrap()
        .into_books();
    assert_eq!(pm.len(), 2);
    for delivery in pm {
        run.reduce_then_enqueue_pm_book(delivery).unwrap();
    }
    assert!(matches!(
        capture_okx_reference(&mut run, 1_001).await.unwrap(),
        OkxPublicCaptureEvent::ReferenceEnqueued
    ));

    let observed_now_ns = 1_000 + PUBLIC_MAX_AGE_NS + 1;
    let failure = run
        .service_lane_turn(observed_now_ns, &mut NoopLaneService)
        .unwrap_err();
    let enacted = run
        .enact_public_lane_aged(failure, WALL_BASE + observed_now_ns, observed_now_ns)
        .await
        .unwrap();
    let PmPublicAgedLaneFaultEnactment::Polymarket {
        unavailable_fault,
        reducer_reason,
        purged_queued_deliveries,
    } = enacted
    else {
        panic!("fresh aged PM head must retain one copied unavailable fault");
    };
    assert_eq!(unavailable_fault, PmPublicSessionFault::Stale);
    assert_eq!(reducer_reason, PmPublicReadinessReason::BookStale);
    assert_eq!(purged_queued_deliveries, 2);
    assert_eq!(
        run.public_lane_metrics().depth(),
        2,
        "the preserved OKX route and mandatory PM unavailability remain admitted"
    );
    assert_eq!(run.public_lane_metrics().invalidated_purged(), 2);
    assert_eq!(run.pm_book_counters().external_faults, 1);
    assert_eq!(run.pm_book_counters().backlog_aged_faults, 1);
    assert_eq!(run.pm_book_counters().stale_invalidations, 1);
    assert_eq!(run.pm_book_counters().overflows, 0);
    assert!(!run.artifact_terminal());

    assert_eq!(
        run.service_lane_turn(1_001, &mut NoopLaneService).unwrap(),
        1,
        "the preserved OKX route remains independently serviceable"
    );
    assert!(matches!(
        run.record_pm_reconnect_scheduled(observed_now_ns + 1).await,
        Err(PmPublicCaptureRunError::PendingPmPublicRouteReconnect {
            epoch: 11,
            pending: 1
        })
    ));
    assert_eq!(
        run.service_lane_turn(observed_now_ns, &mut NoopLaneService)
            .unwrap(),
        1,
        "the mandatory PM unavailable occurrence is serviced before reconnect"
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
    let mut resync = run
        .capture_pm_public(
            WALL_BASE + observed_now_ns + 4,
            observed_now_ns + 4,
            snapshot_one().as_bytes(),
        )
        .await
        .unwrap();
    let flow = resync.take_snapshot_flow().unwrap();
    let delivery = resync.into_books().into_iter().next().unwrap();
    run.commit_then_enqueue_pm_snapshot(delivery, flow).unwrap();
    assert_eq!(
        run.ready_pm_book_view().unwrap().connection_epoch().value(),
        12
    );

    drain_public_lane(&mut run, observed_now_ns + 5);
    let outcome = run.finish().await.unwrap();
    assert_eq!(outcome.projection().counters().backlog_aged_faults, 1);
    assert_eq!(outcome.projection().counters().overflows, 0);
}

#[tokio::test]
async fn active_okx_aged_maps_to_stale_and_preserves_the_pm_route() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("okx-aged.jsonl")).await;
    acknowledge_okx(&mut run).await;
    open_pm_snapshot(&mut run).await;

    assert!(matches!(
        capture_okx_reference(&mut run, 1_000).await.unwrap(),
        OkxPublicCaptureEvent::ReferenceEnqueued
    ));
    let retained_pm = run
        .capture_pm_public(
            WALL_BASE + 1_001,
            1_001,
            snapshot_consistent_bbo().as_bytes(),
        )
        .await
        .unwrap()
        .into_books()
        .into_iter()
        .next()
        .unwrap();
    run.reduce_then_enqueue_pm_book(retained_pm).unwrap();
    assert!(matches!(
        capture_okx_reference(&mut run, 1_002).await.unwrap(),
        OkxPublicCaptureEvent::ReferenceEnqueued
    ));

    let reducer_before = run.pm_book_counters();
    let observed_now_ns = 1_000 + PUBLIC_MAX_AGE_NS + 1;
    let failure = run
        .service_lane_turn(observed_now_ns, &mut NoopLaneService)
        .unwrap_err();
    let enacted = run
        .enact_public_lane_aged(failure, WALL_BASE + observed_now_ns, observed_now_ns)
        .await
        .unwrap();
    let PmPublicAgedLaneFaultEnactment::Okx {
        unavailable_fault,
        purged_queued_deliveries,
    } = enacted
    else {
        panic!("fresh aged OKX head must retain one copied unavailable fault");
    };
    assert_eq!(unavailable_fault, OkxPublicSessionFault::Stale);
    assert_eq!(purged_queued_deliveries, 2);
    assert_eq!(
        run.public_lane_metrics().depth(),
        2,
        "the preserved PM route and mandatory OKX unavailability remain admitted"
    );
    assert_eq!(run.public_lane_metrics().invalidated_purged(), 2);
    assert_eq!(run.pm_book_counters(), reducer_before);
    assert!(!run.artifact_terminal());

    assert_eq!(
        run.service_lane_turn(2_000, &mut NoopLaneService).unwrap(),
        1,
        "the preserved PM route remains independently serviceable"
    );
    assert_eq!(
        run.service_lane_turn(observed_now_ns, &mut NoopLaneService)
            .unwrap(),
        1,
        "the mandatory OKX unavailable occurrence transfers exactly once"
    );
    let outcome = run.finish().await.unwrap();
    assert_eq!(outcome.projection().counters().okx_disconnects, 1);
}

#[tokio::test]
async fn atomic_pm_disconnect_admission_is_never_aged_and_transfers_once() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("pm-unavailable-aged.jsonl")).await;
    let fault = run
        .record_pm_disconnected(WALL_BASE + 1_000, 1_000)
        .await
        .unwrap();
    assert_eq!(fault, PmPublicSessionFault::Disconnect);
    assert_eq!(run.public_lane_metrics().depth(), 1);
    let observed_now_ns = 1_000 + PUBLIC_MAX_AGE_NS + 1;
    assert_eq!(
        run.service_lane_turn(observed_now_ns, &mut NoopLaneService)
            .unwrap(),
        1,
        "must-deliver PM unavailability is serviceable regardless of queue age"
    );
    assert_eq!(run.public_lane_metrics().depth(), 0);
    assert_eq!(run.public_lane_metrics().invalidated_purged(), 0);
    assert_eq!(run.pm_book_counters().external_faults, 1);
    assert_eq!(run.pm_book_counters().disconnects, 1);
    assert_eq!(run.pm_book_counters().backlog_aged_faults, 0);

    let outcome = run.finish().await.unwrap();
    assert_eq!(outcome.projection().counters().disconnects, 1);
    assert_eq!(outcome.projection().counters().external_faults, 1);
}

#[tokio::test]
async fn atomic_okx_disconnect_admission_is_never_aged_and_transfers_once() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("okx-unavailable-aged.jsonl")).await;
    let reducer_before = run.pm_book_counters();
    let fault = run
        .record_okx_disconnected(WALL_BASE + 1_000, 1_000)
        .await
        .unwrap();
    assert_eq!(fault, OkxPublicSessionFault::Disconnect);
    assert_eq!(run.public_lane_metrics().depth(), 1);
    let observed_now_ns = 1_000 + PUBLIC_MAX_AGE_NS + 1;
    assert_eq!(
        run.service_lane_turn(observed_now_ns, &mut NoopLaneService)
            .unwrap(),
        1,
        "must-deliver OKX unavailability is serviceable regardless of queue age"
    );
    assert_eq!(run.public_lane_metrics().depth(), 0);
    assert_eq!(run.public_lane_metrics().invalidated_purged(), 0);
    assert_eq!(run.pm_book_counters(), reducer_before);

    let outcome = run.finish().await.unwrap();
    assert_eq!(outcome.projection().counters().okx_disconnects, 1);
}

#[tokio::test]
async fn atomic_pm_disconnect_purges_own_route_before_admitting_copied_fault() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("pm-unavailable-full.jsonl")).await;
    acknowledge_okx(&mut run).await;

    let capacity = PmLanePolicy::for_lane(PmLaneKind::Public).capacity();
    let next_receive = fill_with_okx_references(&mut run, capacity - 1, 1_000).await;
    run.issue_and_enqueue_pm_metadata(WALL_BASE + next_receive)
        .unwrap();
    assert_eq!(run.public_lane_metrics().depth(), capacity);

    let fault = run
        .record_pm_disconnected(WALL_BASE + next_receive + 1, next_receive + 1)
        .await
        .unwrap();
    assert_eq!(fault, PmPublicSessionFault::Disconnect);
    assert_eq!(run.public_lane_metrics().rejected_full(), 0);
    assert_eq!(run.public_lane_metrics().depth(), capacity);
    assert_eq!(
        run.public_lane_metrics().invalidated_purged(),
        1,
        "the PM metadata route is purged before its unavailable occurrence is admitted"
    );
    assert_eq!(run.pm_book_counters().disconnects, 1);
    assert_eq!(run.pm_book_counters().external_faults, 1);
    assert!(!run.artifact_terminal());
    drain_public_lane(&mut run, next_receive + 2);
    let outcome = run.finish().await.unwrap();
    assert_eq!(outcome.projection().counters().disconnects, 1);
    assert_eq!(outcome.projection().counters().external_faults, 1);
}

#[tokio::test]
async fn atomic_okx_disconnect_admission_full_terminalizes_with_copied_fault() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("okx-unavailable-full.jsonl")).await;
    acknowledge_okx(&mut run).await;
    open_pm_snapshot(&mut run).await;

    let capacity = PmLanePolicy::for_lane(PmLaneKind::Public).capacity();
    let next_receive = fill_with_pm_books(&mut run, capacity, 1_000).await;
    assert_eq!(run.public_lane_metrics().depth(), capacity);

    let error = run
        .record_okx_disconnected(WALL_BASE + next_receive + 2, next_receive + 2)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PmPublicCaptureRunError::NotificationAdmission(PmPublicNotificationAdmissionFailure::Okx {
            fault: OkxPublicSessionFault::Disconnect
        })
    ));
    assert_eq!(run.public_lane_metrics().rejected_full(), 1);
    assert_eq!(
        run.public_lane_metrics().depth(),
        0,
        "terminal notification admission purges every Run-owned public obligation"
    );
    assert_eq!(
        run.public_lane_metrics().invalidated_purged(),
        u64::try_from(capacity).unwrap()
    );
    assert!(run.artifact_terminal());
    assert!(matches!(
        run.finish().await,
        Err(
            PmPublicCaptureRunError::NotificationAdmissionTerminalFinish {
                failure: PmPublicNotificationAdmissionFailure::Okx {
                    fault: OkxPublicSessionFault::Disconnect
                },
                ..
            }
        )
    ));
}

#[tokio::test]
async fn same_config_sibling_snapshot_delivery_and_flow_are_not_substitutable() {
    let directory = tempfile::tempdir().unwrap();

    let mut flow_target = start_live_run(directory.path().join("flow-target.jsonl")).await;
    let mut flow_sibling = start_live_run(directory.path().join("flow-sibling.jsonl")).await;
    let mut target_batch = flow_target
        .capture_pm_public(WALL_BASE + 100, 100, snapshot_one().as_bytes())
        .await
        .unwrap();
    let _target_flow = target_batch.take_snapshot_flow().unwrap();
    let target_delivery = target_batch.into_books().into_iter().next().unwrap();
    let mut sibling_batch = flow_sibling
        .capture_pm_public(WALL_BASE + 100, 100, snapshot_one().as_bytes())
        .await
        .unwrap();
    let sibling_flow = sibling_batch.take_snapshot_flow().unwrap();
    let _sibling_delivery = sibling_batch.into_books().into_iter().next().unwrap();
    assert!(matches!(
        flow_target.commit_then_enqueue_pm_snapshot(target_delivery, sibling_flow),
        Err(PmPublicBookPipelineError::Reduce(
            PmPublicCaptureRunError::PmSnapshotCommit {
                source: PmPublicSnapshotCommitError::RouteAuthorityMismatch,
                ..
            }
        ))
    ));
    assert!(flow_target.artifact_terminal());
    assert!(matches!(
        flow_target.finish().await,
        Err(PmPublicCaptureRunError::TerminalFinish { .. })
    ));
    assert!(matches!(
        flow_sibling.finish().await,
        Err(PmPublicCaptureRunError::PendingPmBookReductionFinish { pending: 1, .. })
    ));

    let mut delivery_target = start_live_run(directory.path().join("delivery-target.jsonl")).await;
    let mut delivery_sibling =
        start_live_run(directory.path().join("delivery-sibling.jsonl")).await;
    let mut target_batch = delivery_target
        .capture_pm_public(WALL_BASE + 100, 100, snapshot_one().as_bytes())
        .await
        .unwrap();
    let target_flow = target_batch.take_snapshot_flow().unwrap();
    let _target_delivery = target_batch.into_books().into_iter().next().unwrap();
    let mut sibling_batch = delivery_sibling
        .capture_pm_public(WALL_BASE + 100, 100, snapshot_one().as_bytes())
        .await
        .unwrap();
    let _sibling_flow = sibling_batch.take_snapshot_flow().unwrap();
    let sibling_delivery = sibling_batch.into_books().into_iter().next().unwrap();
    assert!(matches!(
        delivery_target.commit_then_enqueue_pm_snapshot(sibling_delivery, target_flow),
        Err(PmPublicBookPipelineError::Reduce(
            PmPublicCaptureRunError::PmSnapshotReductionOrderMismatch { .. }
        ))
    ));
    assert!(!delivery_target.artifact_terminal());
    assert!(matches!(
        delivery_target.finish().await,
        Err(PmPublicCaptureRunError::PendingPmBookReductionFinish { pending: 1, .. })
    ));
    assert!(matches!(
        delivery_sibling.finish().await,
        Err(PmPublicCaptureRunError::PendingPmBookReductionFinish { pending: 1, .. })
    ));
}

#[tokio::test]
async fn run_owned_reducer_commits_snapshot_and_opens_protocol_flow() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("run-owned-reducer.jsonl")).await;
    let mut batch = run
        .capture_pm_public(WALL_BASE + 100, 100, snapshot_one().as_bytes())
        .await
        .unwrap();
    let flow = batch.take_snapshot_flow().unwrap();
    let delivery = batch.into_books().into_iter().next().unwrap();
    assert_eq!(run.pending_pm_book_reduction_count(), 1);
    assert_eq!(
        run.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::PendingReduction)
    );
    run.commit_then_enqueue_pm_snapshot(delivery, flow).unwrap();
    assert_eq!(run.pending_pm_book_reduction_count(), 0);
    assert!(run.pm_book_readiness().is_ready());
    assert!(run.ready_pm_book_view().is_some());
    assert_eq!(run.service_lane_turn(100, &mut NoopLaneService).unwrap(), 1);
    run.finish().await.unwrap();
}
