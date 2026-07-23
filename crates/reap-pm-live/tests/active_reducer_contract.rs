#[allow(dead_code)]
mod support;

use reap_pm_live::{
    PmPublicBookPipelineError, PmPublicBookReadinessReason, PmPublicBookReduceError,
    PmPublicCapture, PmPublicCaptureRun, PmPublicCaptureRunError, PmPublicDataPipelineError,
};
use reap_pm_state::PmPublicReadinessReason;
use reap_polymarket_adapter::PmPublicSessionError;

const WALL: u64 = 1_700_000_000_000_000_000;

struct Consume;

impl reap_pm_live::PmPublicLaneService for Consume {
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

fn service_all_public(run: &mut PmPublicCaptureRun, now_ns: u64) {
    let mut consumer = Consume;
    while run.public_lane_metrics().depth() != 0 {
        assert!(
            run.service_lane_turn(now_ns, &mut consumer).unwrap() > 0,
            "a nonempty public lane must make service progress"
        );
    }
}

async fn start_live_run(path: std::path::PathBuf) -> PmPublicCaptureRun {
    let mut run = PmPublicCapture::new(support::public_config())
        .unwrap()
        .start(
            path,
            support::authoritative(),
            support::session_policy(),
            support::provenance(),
        )
        .await
        .unwrap();
    run.record_pm_connection_started(60).await.unwrap();
    run.record_okx_connection_started(61).await.unwrap();
    run.record_pm_subscription_sent(100).await.unwrap();
    run.record_okx_subscription_sent(101).await.unwrap();
    run
}

async fn open_snapshot(run: &mut PmPublicCaptureRun) {
    let mut batch = run
        .capture_pm_public(WALL + 110, 110, support::snapshot_one().as_bytes())
        .await
        .unwrap();
    let flow = batch.take_snapshot_flow().unwrap();
    let delivery = batch.into_books().into_iter().next().unwrap();
    run.commit_then_enqueue_pm_snapshot(delivery, flow).unwrap();
}

fn two_top_checks() -> Vec<u8> {
    let top = support::bbo()
        .replace(r#""best_bid":"0.50""#, r#""best_bid":"0.40""#)
        .replace(r#""bid_size":"12.5""#, r#""bid_size":"50""#);
    format!("[{top},{top}]").into_bytes()
}

#[tokio::test]
async fn sibling_run_delivery_cannot_cross_active_run_authority() {
    let directory = tempfile::tempdir().unwrap();
    let mut source = start_live_run(directory.path().join("source.jsonl")).await;
    let mut target = start_live_run(directory.path().join("target.jsonl")).await;
    open_snapshot(&mut source).await;
    open_snapshot(&mut target).await;

    let delivery = source
        .capture_pm_public(WALL + 120, 120, support::delta().as_bytes())
        .await
        .unwrap()
        .into_books()
        .into_iter()
        .next()
        .unwrap();
    let error = target.reduce_then_enqueue_pm_book(delivery).unwrap_err();
    assert!(matches!(
        error,
        PmPublicBookPipelineError::Reduce(
            PmPublicCaptureRunError::PmBookReductionOrderMismatch { .. }
        )
    ));
    assert_eq!(target.terminal_cause(), None);
    assert!(target.terminal_okx_unavailable().is_none());
    assert!(!source.artifact_terminal());
    service_all_public(&mut target, 120);
    target.finish().await.unwrap();
    assert!(matches!(
        source.finish().await,
        Err(PmPublicCaptureRunError::PendingPmBookReductionFinish { pending: 1, .. })
    ));
}

#[tokio::test]
async fn pending_obligation_suppresses_ready_view_until_exact_commit() {
    let directory = tempfile::tempdir().unwrap();
    let mut target = start_live_run(directory.path().join("target-authority.jsonl")).await;
    open_snapshot(&mut target).await;
    assert!(target.ready_pm_book_view().is_some());

    let delivery = target
        .capture_pm_public(WALL + 120, 120, support::delta().as_bytes())
        .await
        .unwrap()
        .into_books()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(target.pending_pm_book_reduction_count(), 1);
    assert_eq!(
        target.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::PendingReduction)
    );
    assert!(
        target.ready_pm_book_view().is_none(),
        "a captured-but-unreduced delivery suppresses quote authority"
    );
    assert!(!target.artifact_terminal());

    target.reduce_then_enqueue_pm_book(delivery).unwrap();
    assert_eq!(target.pending_pm_book_reduction_count(), 0);
    assert!(target.pm_book_readiness().is_ready());
    assert!(target.ready_pm_book_view().is_some());
    service_all_public(&mut target, 120);
    target.finish().await.unwrap();
}

#[tokio::test]
async fn multi_event_reducer_obligations_are_exact_ordered_and_finish_enforced() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("ordered.jsonl")).await;
    open_snapshot(&mut run).await;
    let mut deliveries = run
        .capture_pm_public(WALL + 120, 120, &two_top_checks())
        .await
        .unwrap()
        .into_books()
        .into_iter();
    let first = deliveries.next().unwrap();
    let second = deliveries.next().unwrap();
    assert_eq!(run.pending_pm_book_reduction_count(), 2);
    assert_eq!(
        run.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::PendingReduction)
    );
    assert!(run.ready_pm_book_view().is_none());
    assert!(matches!(
        run.reduce_then_enqueue_pm_book(first).unwrap(),
        reap_pm_state::PmBookTransition::TopConfirmed
    ));
    assert_eq!(run.pending_pm_book_reduction_count(), 1);
    assert_eq!(
        run.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::PendingReduction)
    );
    assert!(run.ready_pm_book_view().is_none());
    assert!(matches!(
        run.capture_pm_public(WALL + 121, 121, support::bbo().as_bytes())
            .await,
        Err(PmPublicCaptureRunError::PendingPmBookReductions { pending: 1 })
    ));
    assert!(matches!(
        run.issue_and_enqueue_pm_metadata(WALL + 121),
        Err(PmPublicDataPipelineError::Run(
            PmPublicCaptureRunError::PendingPmBookReductions { pending: 1 }
        ))
    ));
    assert!(matches!(
        run.record_freshness_timer(121).await,
        Err(PmPublicCaptureRunError::PendingPmBookReductions { pending: 1 })
    ));
    assert!(matches!(
        run.reduce_then_enqueue_pm_book(second).unwrap(),
        reap_pm_state::PmBookTransition::TopConfirmed
    ));
    assert_eq!(run.pending_pm_book_reduction_count(), 0);
    assert!(run.pm_book_readiness().is_ready());
    assert!(run.ready_pm_book_view().is_some());
    service_all_public(&mut run, 120);
    run.finish().await.unwrap();

    let mut dropped_all = start_live_run(directory.path().join("dropped-all.jsonl")).await;
    open_snapshot(&mut dropped_all).await;
    let deliveries = dropped_all
        .capture_pm_public(WALL + 120, 120, &two_top_checks())
        .await
        .unwrap()
        .into_books();
    drop(deliveries);
    assert_eq!(dropped_all.pending_pm_book_reduction_count(), 2);
    assert_eq!(
        dropped_all.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::PendingReduction)
    );
    assert!(dropped_all.ready_pm_book_view().is_none());
    assert!(matches!(
        dropped_all.finish().await,
        Err(PmPublicCaptureRunError::PendingPmBookReductionFinish { pending: 2, .. })
    ));

    let mut dropped = start_live_run(directory.path().join("dropped.jsonl")).await;
    open_snapshot(&mut dropped).await;
    let mut deliveries = dropped
        .capture_pm_public(WALL + 120, 120, &two_top_checks())
        .await
        .unwrap()
        .into_books()
        .into_iter();
    dropped
        .reduce_then_enqueue_pm_book(deliveries.next().unwrap())
        .unwrap();
    drop(deliveries);
    assert_eq!(
        dropped.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::PendingReduction)
    );
    assert!(dropped.ready_pm_book_view().is_none());
    assert!(matches!(
        dropped.finish().await,
        Err(PmPublicCaptureRunError::PendingPmBookReductionFinish { pending: 1, .. })
    ));

    let mut reordered = start_live_run(directory.path().join("reordered.jsonl")).await;
    open_snapshot(&mut reordered).await;
    let mut deliveries = reordered
        .capture_pm_public(WALL + 120, 120, &two_top_checks())
        .await
        .unwrap()
        .into_books()
        .into_iter();
    let _first = deliveries.next().unwrap();
    let second = deliveries.next().unwrap();
    assert!(matches!(
        reordered.reduce_then_enqueue_pm_book(second),
        Err(PmPublicBookPipelineError::Reduce(
            PmPublicCaptureRunError::PmBookReductionOrderMismatch { .. }
        ))
    ));
    assert!(matches!(
        reordered.finish().await,
        Err(PmPublicCaptureRunError::PendingPmBookReductionFinish { pending: 2, .. })
    ));
}

#[tokio::test]
async fn reducer_rejection_retains_exact_reason_and_terminals_both_venues() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("bbo-rejection.jsonl")).await;
    open_snapshot(&mut run).await;
    let delivery = run
        .capture_pm_public(WALL + 120, 120, support::bbo().as_bytes())
        .await
        .unwrap()
        .into_books()
        .into_iter()
        .next()
        .unwrap();
    let error = run.reduce_then_enqueue_pm_book(delivery).unwrap_err();
    assert!(matches!(
        error,
        PmPublicBookPipelineError::Reduce(PmPublicCaptureRunError::PmBookReduce {
            source: PmPublicBookReduceError::Reducer(PmPublicReadinessReason::BboMismatch),
            ..
        })
    ));
    assert!(run.pm_book_readiness().reason().is_some());
    assert!(run.ready_pm_book_view().is_none());
    assert_eq!(run.pm_book_counters().bbo_mismatches, 1);
    assert!(run.terminal_okx_unavailable().is_some());
    assert!(matches!(
        run.finish().await,
        Err(PmPublicCaptureRunError::TerminalFinish { .. })
    ));
}

#[tokio::test]
async fn disconnect_and_reconnect_advance_session_and_reducer_together() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("reconnect.jsonl")).await;
    let fault = run.record_pm_disconnected(WALL + 110, 110).await.unwrap();
    assert_eq!(
        fault,
        reap_polymarket_adapter::PmPublicSessionFault::Disconnect
    );
    assert!(run.pm_book_readiness().reason().is_some());
    assert!(run.ready_pm_book_view().is_none());
    assert_eq!(run.pm_book_counters().external_faults, 1);
    assert_eq!(run.pm_book_counters().disconnects, 1);

    assert_eq!(run.service_lane_turn(111, &mut Consume).unwrap(), 1);
    run.record_pm_reconnect_scheduled(112).await.unwrap();
    assert!(run.pm_book_readiness().reason().is_some());
    assert_eq!(run.pm_book_counters().reconnects, 1);
    assert!(run.ready_pm_book_view().is_none());
    run.finish().await.unwrap();
}

#[tokio::test]
async fn heartbeat_preview_does_not_mutate_before_due_or_before_timeout() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("heartbeat.jsonl")).await;
    assert!(matches!(
        run.record_pm_heartbeat_ping_sent(WALL + 109, 109).await,
        Err(PmPublicCaptureRunError::HeartbeatPingNotDue)
    ));
    run.record_pm_heartbeat_ping_sent(WALL + 110, 110)
        .await
        .unwrap();
    assert!(matches!(
        run.record_pm_heartbeat_ping_sent(WALL + 114, 114).await,
        Err(PmPublicCaptureRunError::HeartbeatPingNotDue)
    ));
    let error = run
        .record_pm_heartbeat_ping_sent(WALL + 115, 115)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PmPublicCaptureRunError::PmHeartbeat {
            source: PmPublicSessionError::HeartbeatTimeout { deadline_ns: 115 },
            ..
        }
    ));
    assert!(run.pm_book_readiness().reason().is_some());
    assert!(run.ready_pm_book_view().is_none());
    assert_eq!(run.pm_book_counters().external_faults, 1);
    assert_eq!(run.pm_book_counters().heartbeat_timeouts, 1);
    service_all_public(&mut run, 115);
    run.finish().await.unwrap();
}
