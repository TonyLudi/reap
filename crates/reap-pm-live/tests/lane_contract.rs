mod support;

use reap_pm_core::{
    OkxReferenceEvent, PmBookEvent, PmMarketEvent, SnapshotRevision, VenueEventHashAlgorithm,
};
use reap_pm_live::{
    OkxPublicUnavailable, PM_INPUT_SERVICE_PRIORITY, PmLaneKind, PmLanePolicy,
    PmPublicBookReadinessReason, PmPublicCapture, PmPublicCaptureRun, PmPublicCaptureRunError,
    PmPublicLaneService, PmPublicUnavailable, PmServiceTurnError, SaturationAction,
    ServicedLaneItem,
};
use reap_pm_live_contracts::PmCapabilityLane;

use support::{authoritative, provenance, public_config, session_policy, snapshot_one};

async fn live_pm_run(name: &str) -> (tempfile::TempDir, PmPublicCaptureRun) {
    let directory = tempfile::tempdir().unwrap();
    let mut run = PmPublicCapture::new(public_config())
        .unwrap()
        .start(
            directory.path().join(name),
            authoritative(),
            session_policy(),
            provenance(),
        )
        .await
        .unwrap();
    run.record_pm_connection_started(60).await.unwrap();
    run.record_pm_subscription_sent(100).await.unwrap();
    (directory, run)
}

#[derive(Default)]
struct TransferRecorder {
    metadata: usize,
    books: Vec<(u64, u64, Option<u64>, VenueEventHashAlgorithm, usize, u64)>,
}

impl PmPublicLaneService for TransferRecorder {
    fn on_pm_public_unavailable(&mut self, _item: ServicedLaneItem<PmPublicUnavailable>) {}

    fn on_okx_public_unavailable(&mut self, _item: ServicedLaneItem<OkxPublicUnavailable>) {}

    fn on_market(&mut self, _item: ServicedLaneItem<PmMarketEvent>) {
        self.metadata += 1;
    }

    fn on_book(&mut self, item: ServicedLaneItem<PmBookEvent>) {
        let ordering = item.ordering();
        let venue_hash = ordering
            .venue_hash()
            .expect("the admitted snapshot retains its verified venue hash");
        self.books.push((
            ordering.connection_epoch().value(),
            ordering.local_ingress_sequence().value(),
            ordering.snapshot_revision().map(SnapshotRevision::value),
            venue_hash.algorithm(),
            venue_hash.len(),
            item.clock().queue_age_ns(),
        ));
    }

    fn on_reference(&mut self, _item: ServicedLaneItem<OkxReferenceEvent>) {}
}

struct PanicOnMarket;

impl PmPublicLaneService for PanicOnMarket {
    fn on_pm_public_unavailable(&mut self, _item: ServicedLaneItem<PmPublicUnavailable>) {}

    fn on_okx_public_unavailable(&mut self, _item: ServicedLaneItem<OkxPublicUnavailable>) {}

    fn on_market(&mut self, _item: ServicedLaneItem<PmMarketEvent>) {
        panic!("consumer failed during exact occurrence transfer");
    }

    fn on_book(&mut self, _item: ServicedLaneItem<PmBookEvent>) {}

    fn on_reference(&mut self, _item: ServicedLaneItem<OkxReferenceEvent>) {}
}

#[tokio::test]
async fn run_owned_snapshot_service_preserves_route_ordering_and_service_clock() {
    let (_directory, mut run) = live_pm_run("lane-snapshot.jsonl").await;
    let mut batch = run
        .capture_pm_public(1_700_000_000_123_456_789, 110, snapshot_one().as_bytes())
        .await
        .unwrap();
    let flow = batch.take_snapshot_flow().expect("snapshot flow");
    let delivery = batch
        .into_books()
        .into_iter()
        .next()
        .expect("snapshot delivery");
    run.commit_then_enqueue_pm_snapshot(delivery, flow).unwrap();

    let mut recorder = TransferRecorder::default();
    assert_eq!(run.service_lane_turn(115, &mut recorder).unwrap(), 1);
    assert_eq!(
        recorder.books,
        vec![(11, 1, Some(1), VenueEventHashAlgorithm::Sha1, 20, 5)]
    );
    run.finish().await.unwrap();
}

#[tokio::test]
async fn fresh_sibling_cannot_service_the_owners_public_item() {
    let (_owner_directory, mut owner) = live_pm_run("owner.jsonl").await;
    let (_sibling_directory, mut sibling) = live_pm_run("sibling.jsonl").await;
    owner
        .issue_and_enqueue_pm_metadata(1_700_000_000_000_000_050)
        .unwrap();

    let mut sibling_recorder = TransferRecorder::default();
    assert_eq!(
        sibling
            .service_lane_turn(101, &mut sibling_recorder)
            .unwrap(),
        0
    );
    assert_eq!(sibling_recorder.metadata, 0);
    assert_eq!(owner.public_lane_metrics().depth(), 1);

    let mut owner_recorder = TransferRecorder::default();
    assert_eq!(
        owner.service_lane_turn(101, &mut owner_recorder).unwrap(),
        1
    );
    assert_eq!(owner_recorder.metadata, 1);
    assert_eq!(owner.public_lane_metrics().depth(), 0);
    owner.finish().await.unwrap();
    sibling.finish().await.unwrap();
}

#[tokio::test]
async fn dropping_the_owner_destroys_its_queued_public_state() {
    let (owner_directory, mut owner) = live_pm_run("drop-owner.jsonl").await;
    owner
        .issue_and_enqueue_pm_metadata(1_700_000_000_000_000_050)
        .unwrap();
    assert_eq!(owner.public_lane_metrics().depth(), 1);
    drop(owner);
    drop(owner_directory);

    let (_sibling_directory, mut sibling) = live_pm_run("after-drop.jsonl").await;
    assert_eq!(
        sibling
            .service_lane_turn(101, &mut TransferRecorder::default())
            .unwrap(),
        0
    );
    sibling.finish().await.unwrap();
}

#[tokio::test]
async fn finish_rejects_an_unserviced_public_obligation() {
    let (_directory, mut run) = live_pm_run("queued-finish.jsonl").await;
    run.issue_and_enqueue_pm_metadata(1_700_000_000_000_000_050)
        .unwrap();

    assert!(matches!(
        run.finish().await,
        Err(PmPublicCaptureRunError::QueuedPublicLaneFinish {
            pending: 1,
            shutdown_error: None,
        })
    ));
}

#[tokio::test]
async fn callback_unwind_poison_blocks_service_readiness_mutation_and_finish() {
    let (_directory, mut run) = live_pm_run("callback-poison.jsonl").await;
    run.issue_and_enqueue_pm_metadata(1_700_000_000_000_000_050)
        .unwrap();

    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = run.service_lane_turn(101, &mut PanicOnMarket);
    }));
    assert!(unwind.is_err());
    assert_eq!(run.public_lane_metrics().depth(), 0);
    assert!(matches!(
        run.service_lane_turn(102, &mut TransferRecorder::default()),
        Err(PmServiceTurnError::ConsumerTransferPoisoned)
    ));
    assert_eq!(
        run.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::ConsumerTransferPoisoned)
    );
    assert!(run.ready_pm_book_view().is_none());
    assert!(matches!(
        run.record_okx_connection_started(103).await,
        Err(PmPublicCaptureRunError::PublicConsumerTransferPoisoned)
    ));
    assert!(matches!(
        run.finish().await,
        Err(
            PmPublicCaptureRunError::PublicConsumerTransferPoisonedFinish {
                shutdown_error: None,
            }
        )
    ));
}

#[tokio::test]
async fn later_clock_failure_reports_prior_progress_and_leaves_the_head_for_retry() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = PmPublicCapture::new(public_config())
        .unwrap()
        .start(
            directory.path().join("partial-clock-progress.jsonl"),
            authoritative(),
            session_policy(),
            provenance(),
        )
        .await
        .unwrap();
    run.record_pm_connection_started(60).await.unwrap();
    run.record_okx_connection_started(61).await.unwrap();
    run.record_pm_subscription_sent(99).await.unwrap();
    run.record_okx_subscription_sent(100).await.unwrap();
    run.issue_and_enqueue_pm_metadata(1_700_000_000_000_000_050)
        .unwrap();
    run.capture_okx_public(
        1_700_000_000_000_000_105,
        105,
        support::okx_ack().as_bytes(),
    )
    .await
    .unwrap();
    assert_eq!(
        run.capture_okx_public(
            1_700_000_000_000_000_106,
            106,
            support::okx_reference().as_bytes(),
        )
        .await
        .unwrap(),
        reap_pm_live::OkxPublicCaptureEvent::ReferenceEnqueued
    );

    let mut recorder = TransferRecorder::default();
    assert_eq!(run.service_lane_turn(100, &mut recorder).unwrap(), 1);
    assert_eq!(recorder.metadata, 1);
    assert_eq!(run.public_lane_metrics().depth(), 1);
    assert!(matches!(
        run.service_lane_turn(100, &mut recorder),
        Err(PmServiceTurnError::DeliveryClock(_) | PmServiceTurnError::EventClock(_))
    ));
    assert_eq!(
        run.public_lane_metrics().depth(),
        1,
        "a failed service-clock preflight must not pop the retained head"
    );
    assert_eq!(run.service_lane_turn(107, &mut recorder).unwrap(), 1);
    assert_eq!(run.public_lane_metrics().depth(), 0);
    run.finish().await.unwrap();
}

#[test]
fn all_eleven_lane_policies_match_the_frozen_oracle() {
    let expected = [
        (
            PmLaneKind::Critical,
            512,
            32,
            Some(250_000_000),
            SaturationAction::GlobalStop,
            Some(512),
        ),
        (
            PmLaneKind::Persistence,
            512,
            32,
            Some(250_000_000),
            SaturationAction::GlobalStop,
            Some(512),
        ),
        (
            PmLaneKind::Private,
            4_096,
            64,
            Some(250_000_000),
            SaturationAction::HaltAccountAndRequireReconciliation,
            Some(64),
        ),
        (
            PmLaneKind::Scheduled,
            4_096,
            64,
            Some(100_000_000),
            SaturationAction::SuppressQuoteAndCancelOwned,
            Some(16),
        ),
        (
            PmLaneKind::Public,
            8_192,
            256,
            Some(500_000_000),
            SaturationAction::InvalidateStreamAndResync,
            Some(256),
        ),
        (
            PmLaneKind::Reconciliation,
            128,
            16,
            Some(5_000_000_000),
            SaturationAction::KeepUnreadyAndRetry,
            Some(8),
        ),
        (
            PmLaneKind::Telemetry,
            128,
            32,
            None,
            SaturationAction::CoalesceTelemetry,
            Some(1),
        ),
        (
            PmLaneKind::ReconciliationRequest,
            128,
            16,
            Some(1_000_000_000),
            SaturationAction::RetainPendingRefresh,
            None,
        ),
        (
            PmLaneKind::Capture,
            8_192,
            256,
            Some(500_000_000),
            SaturationAction::InvalidateCaptureAndResync,
            None,
        ),
        (
            PmLaneKind::Journal,
            1_024,
            128,
            Some(1_000_000_000),
            SaturationAction::SuppressDispatchAndHaltQuotes,
            None,
        ),
        (
            PmLaneKind::FakeEffect,
            256,
            32,
            Some(250_000_000),
            SaturationAction::RejectEffectAndHaltQuotes,
            None,
        ),
    ];

    assert_eq!(expected.len(), PmCapabilityLane::ALL.len());
    for (plan_lane, expected_policy) in PmCapabilityLane::ALL.into_iter().zip(expected) {
        let (lane, capacity, high_water, age, action, burst) = expected_policy;
        assert_eq!(PmLaneKind::from(plan_lane), lane);
        let actual = PmLanePolicy::for_lane(lane);
        assert_eq!(actual.capacity(), capacity);
        assert_eq!(actual.nominal_high_water(), high_water);
        assert_eq!(actual.maximum_age_ns(), age);
        assert_eq!(actual.saturation_action(), action);
        assert_eq!(actual.service_burst(), burst);
    }
}

#[test]
fn future_input_service_priority_is_an_explicit_policy_oracle_only() {
    assert_eq!(
        PM_INPUT_SERVICE_PRIORITY,
        [
            PmLaneKind::Critical,
            PmLaneKind::Persistence,
            PmLaneKind::Private,
            PmLaneKind::Scheduled,
            PmLaneKind::Public,
            PmLaneKind::Reconciliation,
            PmLaneKind::Telemetry,
        ]
    );
    for (rank, lane) in PM_INPUT_SERVICE_PRIORITY.into_iter().enumerate() {
        assert_eq!(lane.service_priority_rank(), Some(rank as u8));
    }
    for lane in [
        PmLaneKind::ReconciliationRequest,
        PmLaneKind::Capture,
        PmLaneKind::Journal,
        PmLaneKind::FakeEffect,
    ] {
        assert_eq!(lane.service_priority_rank(), None);
    }
}
