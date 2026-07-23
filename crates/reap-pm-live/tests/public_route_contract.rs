#[allow(dead_code)]
mod support;

use reap_okx_public_source::OkxPublicSessionFault;
use reap_pm_live::{
    OkxPublicCaptureEvent, OkxPublicUnavailable, PmPublicAgedLaneFaultEnactment, PmPublicCapture,
    PmPublicCaptureRunError, PmPublicCaptureTerminalCause, PmPublicLaneService, PmPublicRouteError,
    PmPublicUnavailable, PmServiceTurnError, ServicedLaneItem,
};
use reap_polymarket_adapter::PmPublicSessionFault;

use support::{
    PM_CONNECTION, authoritative, okx_ack, okx_reference, provenance, public_config,
    session_policy, snapshot_one,
};

async fn capture_run(
    path: std::path::PathBuf,
) -> Result<reap_pm_live::PmPublicCaptureRun, PmPublicCaptureRunError> {
    PmPublicCapture::new(public_config())
        .unwrap()
        .start(path, authoritative(), session_policy(), provenance())
        .await
}

#[derive(Default)]
struct PublicRecorder {
    observed: Vec<&'static str>,
    metadata: usize,
    books: usize,
    references: usize,
    pm_unavailable: Vec<PmPublicSessionFault>,
    okx_unavailable: Vec<OkxPublicSessionFault>,
}

impl PmPublicLaneService for PublicRecorder {
    fn on_market(&mut self, _item: ServicedLaneItem<reap_pm_core::PmMarketEvent>) {
        self.observed.push("metadata");
        self.metadata += 1;
    }

    fn on_book(&mut self, _item: ServicedLaneItem<reap_pm_core::PmBookEvent>) {
        self.observed.push("book");
        self.books += 1;
    }

    fn on_reference(&mut self, _item: ServicedLaneItem<reap_pm_core::OkxReferenceEvent>) {
        self.observed.push("reference");
        self.references += 1;
    }

    fn on_pm_public_unavailable(&mut self, item: ServicedLaneItem<PmPublicUnavailable>) {
        self.observed.push("pm-unavailable");
        self.pm_unavailable.push(item.into_value().fault());
    }

    fn on_okx_public_unavailable(&mut self, item: ServicedLaneItem<OkxPublicUnavailable>) {
        self.observed.push("okx-unavailable");
        self.okx_unavailable.push(item.into_value().fault());
    }
}

#[tokio::test]
async fn sole_run_owner_issues_all_five_exact_move_only_public_deliveries() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("public-routes.jsonl");
    let mut run = capture_run(path).await.unwrap();

    run.record_pm_connection_started(60).await.unwrap();
    run.record_okx_connection_started(60).await.unwrap();
    run.record_pm_subscription_sent(100).await.unwrap();
    run.record_okx_subscription_sent(100).await.unwrap();

    run.issue_and_enqueue_pm_metadata(1_700_000_000_000_000_050)
        .unwrap();

    let acknowledgement = run
        .capture_okx_public(1_700_000_000_000_000_105, 105, okx_ack().as_bytes())
        .await
        .unwrap();
    assert!(matches!(
        acknowledgement,
        OkxPublicCaptureEvent::SubscriptionAcknowledged(_)
    ));
    assert!(matches!(
        run.capture_okx_public(1_700_000_000_000_000_106, 106, okx_reference().as_bytes())
            .await
            .unwrap(),
        OkxPublicCaptureEvent::ReferenceEnqueued
    ));
    let mut batch = run
        .capture_pm_public(1_700_000_000_000_000_110, 110, snapshot_one().as_bytes())
        .await
        .unwrap();
    let flow = batch.take_snapshot_flow().expect("snapshot flow");
    let book = batch
        .into_books()
        .into_iter()
        .next()
        .expect("one routed PM snapshot");
    assert_eq!(book.envelope().connection_id().as_str(), PM_CONNECTION);
    assert_eq!(
        book.envelope().ordering().local_ingress_sequence().value(),
        2,
        "metadata and websocket events share one session-owned ingress counter"
    );

    run.commit_then_enqueue_pm_snapshot(book, flow).unwrap();

    let mut recorder = PublicRecorder::default();
    assert_eq!(run.service_lane_turn(115, &mut recorder).unwrap(), 3);
    assert_eq!(
        recorder.observed,
        vec!["metadata", "reference", "book"],
        "the mandatory synchronous consumer receives exact public inputs in service-key order"
    );

    let pm_fault = run
        .record_pm_disconnected(1_700_000_000_000_000_120, 120)
        .await
        .unwrap();
    let okx_fault = run
        .record_okx_disconnected(1_700_000_000_000_000_121, 121)
        .await
        .unwrap();
    assert_eq!(pm_fault, PmPublicSessionFault::Disconnect);
    assert_eq!(okx_fault, OkxPublicSessionFault::Disconnect);
    assert_eq!(run.public_lane_metrics().depth(), 2);

    assert_eq!(run.service_lane_turn(130, &mut recorder).unwrap(), 1);
    assert_eq!(run.service_lane_turn(130, &mut recorder).unwrap(), 1);
    assert_eq!(
        recorder.observed,
        vec![
            "metadata",
            "reference",
            "book",
            "pm-unavailable",
            "okx-unavailable",
        ],
        "all five reached public variants transfer exactly once"
    );
    assert_eq!(recorder.metadata, 1);
    assert_eq!(recorder.books, 1);
    assert_eq!(recorder.references, 1);
    assert_eq!(
        recorder.pm_unavailable,
        vec![PmPublicSessionFault::Disconnect]
    );
    assert_eq!(
        recorder.okx_unavailable,
        vec![OkxPublicSessionFault::Disconnect]
    );

    run.finish().await.unwrap();
}

#[tokio::test]
async fn repeat_metadata_minting_is_rejected_and_terminals_the_artifact() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("public-route-rejections.jsonl");
    let mut run = capture_run(path).await.unwrap();
    run.record_pm_connection_started(60).await.unwrap();
    run.record_pm_subscription_sent(70).await.unwrap();
    run.issue_and_enqueue_pm_metadata(1_700_000_000_000_000_050)
        .unwrap();
    run.service_lane_turn(71, &mut PublicRecorder::default())
        .unwrap();
    assert!(matches!(
        run.issue_and_enqueue_pm_metadata(1_700_000_000_000_000_051),
        Err(reap_pm_live::PmPublicDataPipelineError::Run(
            PmPublicCaptureRunError::Route(PmPublicRouteError::PmMetadataAlreadyIssued)
        ))
    ));

    assert_eq!(run.public_lane_metrics().depth(), 0);
    assert!(run.artifact_terminal());
    assert_eq!(
        run.terminal_cause(),
        Some(PmPublicCaptureTerminalCause::Route)
    );
    assert!(matches!(
        run.finish().await,
        Err(PmPublicCaptureRunError::TerminalFinish { .. })
    ));
}

#[tokio::test]
async fn atomic_data_success_exposes_only_completion_facts_and_queues_both_inputs() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = capture_run(directory.path().join("atomic-data-success.jsonl"))
        .await
        .unwrap();
    run.record_pm_connection_started(60).await.unwrap();
    run.record_okx_connection_started(61).await.unwrap();
    run.record_pm_subscription_sent(100).await.unwrap();
    run.record_okx_subscription_sent(101).await.unwrap();

    let metadata_completion: () = run
        .issue_and_enqueue_pm_metadata(1_700_000_000_000_000_050)
        .unwrap();
    assert_eq!(metadata_completion, ());
    run.capture_okx_public(1_700_000_000_000_000_105, 105, okx_ack().as_bytes())
        .await
        .unwrap();
    let reference_completion = run
        .capture_okx_public(1_700_000_000_000_000_106, 106, okx_reference().as_bytes())
        .await
        .unwrap();
    assert_eq!(
        reference_completion,
        OkxPublicCaptureEvent::ReferenceEnqueued
    );
    assert_eq!(run.public_lane_metrics().depth(), 2);

    let mut recorder = PublicRecorder::default();
    assert_eq!(run.service_lane_turn(107, &mut recorder).unwrap(), 2);
    assert_eq!(recorder.metadata, 1);
    assert_eq!(recorder.references, 1);
    run.finish().await.unwrap();
}

#[tokio::test]
async fn old_pm_unavailable_services_once_before_a_stale_okx_sibling_faults() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = capture_run(directory.path().join("pm-unavailable-non-expiring.jsonl"))
        .await
        .unwrap();
    run.record_pm_connection_started(60).await.unwrap();
    run.record_okx_connection_started(61).await.unwrap();
    run.record_pm_subscription_sent(100).await.unwrap();
    run.record_okx_subscription_sent(101).await.unwrap();
    run.capture_okx_public(1_700_000_000_000_000_105, 105, okx_ack().as_bytes())
        .await
        .unwrap();

    assert_eq!(
        run.record_pm_disconnected(1_700_000_000_000_001_000, 1_000)
            .await
            .unwrap(),
        PmPublicSessionFault::Disconnect
    );
    assert_eq!(
        run.capture_okx_public(1_700_000_000_000_001_001, 1_001, okx_reference().as_bytes(),)
            .await
            .unwrap(),
        OkxPublicCaptureEvent::ReferenceEnqueued
    );

    let observed_now_ns = 1_001 + 500_000_000 + 1;
    let mut recorder = PublicRecorder::default();
    assert_eq!(
        run.service_lane_turn(observed_now_ns, &mut recorder)
            .unwrap(),
        1,
        "an unavailable head is non-expiring and limits the turn to its exact occurrence"
    );
    assert_eq!(
        recorder.pm_unavailable,
        vec![PmPublicSessionFault::Disconnect]
    );
    assert_eq!(run.public_lane_metrics().depth(), 1);

    let failure = run
        .service_lane_turn(observed_now_ns, &mut recorder)
        .unwrap_err();
    assert!(matches!(&failure, PmServiceTurnError::Aged(_)));
    let enacted = run
        .enact_public_lane_aged(
            failure,
            1_700_000_000_000_000_000 + observed_now_ns,
            observed_now_ns,
        )
        .await
        .unwrap();
    let PmPublicAgedLaneFaultEnactment::Okx {
        unavailable_fault,
        purged_queued_deliveries,
        ..
    } = enacted
    else {
        panic!("the stale sibling must retain its exact OKX route");
    };
    assert_eq!(unavailable_fault, OkxPublicSessionFault::Stale);
    assert_eq!(purged_queued_deliveries, 1);
    assert_eq!(
        run.service_lane_turn(observed_now_ns + 1, &mut recorder)
            .unwrap(),
        1
    );
    assert_eq!(recorder.okx_unavailable, vec![OkxPublicSessionFault::Stale]);
    run.finish().await.unwrap();
}

#[tokio::test]
async fn old_okx_unavailable_services_once_before_a_stale_pm_sibling_faults() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = capture_run(directory.path().join("okx-unavailable-non-expiring.jsonl"))
        .await
        .unwrap();
    run.record_pm_connection_started(60).await.unwrap();
    run.record_okx_connection_started(61).await.unwrap();
    run.record_pm_subscription_sent(100).await.unwrap();
    run.record_okx_subscription_sent(101).await.unwrap();

    assert_eq!(
        run.record_okx_disconnected(1_700_000_000_000_001_000, 1_000)
            .await
            .unwrap(),
        OkxPublicSessionFault::Disconnect
    );
    let mut batch = run
        .capture_pm_public(1_700_000_000_000_001_001, 1_001, snapshot_one().as_bytes())
        .await
        .unwrap();
    let flow = batch.take_snapshot_flow().unwrap();
    let snapshot = batch.into_books().into_iter().next().unwrap();
    run.commit_then_enqueue_pm_snapshot(snapshot, flow).unwrap();

    let observed_now_ns = 1_001 + 500_000_000 + 1;
    let mut recorder = PublicRecorder::default();
    assert_eq!(
        run.service_lane_turn(observed_now_ns, &mut recorder)
            .unwrap(),
        1
    );
    assert_eq!(
        recorder.okx_unavailable,
        vec![OkxPublicSessionFault::Disconnect]
    );
    let failure = run
        .service_lane_turn(observed_now_ns, &mut recorder)
        .unwrap_err();
    assert!(matches!(&failure, PmServiceTurnError::Aged(_)));
    let enacted = run
        .enact_public_lane_aged(
            failure,
            1_700_000_000_000_000_000 + observed_now_ns,
            observed_now_ns,
        )
        .await
        .unwrap();
    let PmPublicAgedLaneFaultEnactment::Polymarket {
        unavailable_fault,
        purged_queued_deliveries,
        ..
    } = enacted
    else {
        panic!("the stale sibling must retain its exact PM route");
    };
    assert_eq!(unavailable_fault, PmPublicSessionFault::Stale);
    assert_eq!(purged_queued_deliveries, 1);
    assert_eq!(
        run.service_lane_turn(observed_now_ns + 1, &mut recorder)
            .unwrap(),
        1
    );
    assert_eq!(recorder.pm_unavailable, vec![PmPublicSessionFault::Stale]);
    run.finish().await.unwrap();
}

#[tokio::test]
async fn disconnected_phase_rejects_protocol_input_and_terminals_the_artifact() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reconnect-gate.jsonl");
    let mut run = capture_run(path).await.unwrap();
    run.record_pm_connection_started(60).await.unwrap();
    run.record_okx_connection_started(60).await.unwrap();
    run.record_pm_subscription_sent(100).await.unwrap();
    run.record_okx_subscription_sent(100).await.unwrap();
    run.capture_okx_public(1_700_000_000_000_000_105, 105, okx_ack().as_bytes())
        .await
        .unwrap();

    let _pm_fault = run
        .record_pm_disconnected(1_700_000_000_000_000_110, 110)
        .await
        .unwrap();
    let _okx_fault = run
        .record_okx_disconnected(1_700_000_000_000_000_111, 111)
        .await
        .unwrap();
    assert!(matches!(
        run.capture_pm_public(1_700_000_000_000_000_112, 112, snapshot_one().as_bytes(),)
            .await,
        Err(PmPublicCaptureRunError::InvalidLifecyclePhase)
    ));
    assert!(run.artifact_terminal());
    assert_eq!(
        run.terminal_cause(),
        Some(PmPublicCaptureTerminalCause::Lifecycle)
    );
    assert!(matches!(
        run.capture_okx_public(1_700_000_000_000_000_113, 113, okx_reference().as_bytes(),)
            .await,
        Err(reap_pm_live::PmPublicDataPipelineError::Run(
            PmPublicCaptureRunError::ArtifactTerminal { .. }
        ))
    ));
}
