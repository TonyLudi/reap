mod support;

use reap_okx_public_source::OkxPublicSessionFault;
use reap_pm_live::{
    MAX_PM_RAW_PUBLIC_FRAME_BYTES, OkxPublicCaptureEvent, PmCaptureVerifyError,
    PmCaptureWriteError, PmPublicAgedLaneEnactError, PmPublicBookPipelineError, PmPublicCapture,
    PmPublicCaptureRun, PmPublicCaptureRunError, PmPublicCaptureTerminalCause, PmPublicLaneService,
    PmServiceTurnError,
};
use reap_polymarket_adapter::PmPublicSessionFault;

use support::{
    authoritative, okx_ack, okx_reference, provenance, public_config, session_policy, snapshot_one,
    snapshot_two,
};

const WALL_BASE: u64 = 1_700_000_000_000_000_000;

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

fn assert_terminal_finish(
    error: PmPublicCaptureRunError,
    expected_cause: PmPublicCaptureTerminalCause,
) {
    let PmPublicCaptureRunError::TerminalFinish {
        cause,
        shutdown_error,
    } = error
    else {
        panic!("terminal run must never produce a normal capture outcome: {error:?}");
    };
    assert_eq!(cause, expected_cause);
    assert!(
        shutdown_error.is_none(),
        "a healthy writer still closes cleanly on terminal run shutdown"
    );
}

#[tokio::test]
async fn pm_and_okx_oversize_frames_are_overflow_evidence_and_terminalize_the_shared_artifact() {
    let directory = tempfile::tempdir().unwrap();
    let oversized = vec![0_u8; MAX_PM_RAW_PUBLIC_FRAME_BYTES + 1];

    let mut pm_run = start_live_run(directory.path().join("pm-oversize.jsonl")).await;
    let pm_error = pm_run
        .capture_pm_public(WALL_BASE + 100, 100, &oversized)
        .await
        .unwrap_err();
    assert!(matches!(
        &pm_error,
        PmPublicCaptureRunError::PmCaptureRejected {
            source: PmCaptureWriteError::Contract(PmCaptureVerifyError::RawFrameTooLarge),
            ..
        }
    ));
    let pm_unavailable = pm_error.pm_unavailable().unwrap().envelope();
    assert_eq!(
        pm_unavailable.payload().fault(),
        PmPublicSessionFault::Overflow
    );
    assert_eq!(
        pm_unavailable.received_clock().local_wall_receive_ns(),
        WALL_BASE + 100
    );
    assert_eq!(pm_unavailable.received_clock().monotonic_receive_ns(), 100);
    assert_eq!(
        pm_run
            .terminal_okx_unavailable()
            .unwrap()
            .envelope()
            .payload()
            .fault(),
        OkxPublicSessionFault::Overflow,
        "one writer-capacity failure fail-closes both venue sessions"
    );
    assert!(pm_run.artifact_terminal());
    assert_eq!(
        pm_run.terminal_cause(),
        Some(PmPublicCaptureTerminalCause::CaptureWriter)
    );
    assert!(matches!(
        pm_run
            .capture_okx_public(WALL_BASE + 101, 101, okx_ack().as_bytes())
            .await,
        Err(reap_pm_live::PmPublicDataPipelineError::Run(
            PmPublicCaptureRunError::ArtifactTerminal { .. }
        ))
    ));
    assert!(matches!(
        pm_run.record_pm_reconnect_scheduled(102).await,
        Err(PmPublicCaptureRunError::ArtifactTerminal { .. })
    ));
    assert_terminal_finish(
        pm_run.finish().await.unwrap_err(),
        PmPublicCaptureTerminalCause::CaptureWriter,
    );

    let mut okx_run = start_live_run(directory.path().join("okx-oversize.jsonl")).await;
    let okx_error = okx_run
        .capture_okx_public(WALL_BASE + 200, 200, &oversized)
        .await
        .unwrap_err();
    assert!(matches!(
        okx_error.run_error().unwrap(),
        PmPublicCaptureRunError::OkxCaptureRejected {
            source: PmCaptureWriteError::Contract(PmCaptureVerifyError::RawFrameTooLarge),
            ..
        }
    ));
    let okx_unavailable = okx_error
        .run_error()
        .unwrap()
        .okx_unavailable()
        .unwrap()
        .envelope();
    assert_eq!(
        okx_unavailable.payload().fault(),
        OkxPublicSessionFault::Overflow
    );
    assert_eq!(
        okx_unavailable.received_clock().local_wall_receive_ns(),
        WALL_BASE + 200
    );
    assert_eq!(okx_unavailable.received_clock().monotonic_receive_ns(), 200);
    assert_eq!(
        okx_run
            .terminal_pm_unavailable()
            .unwrap()
            .envelope()
            .payload()
            .fault(),
        PmPublicSessionFault::Overflow,
        "one writer-capacity failure fail-closes both venue sessions"
    );
    assert!(okx_run.artifact_terminal());
    assert_eq!(
        okx_run.terminal_cause(),
        Some(PmPublicCaptureTerminalCause::CaptureWriter)
    );
    assert!(matches!(
        okx_run
            .capture_pm_public(WALL_BASE + 201, 201, snapshot_one().as_bytes())
            .await,
        Err(PmPublicCaptureRunError::ArtifactTerminal { .. })
    ));
    assert!(matches!(
        okx_run.record_okx_reconnect_scheduled(202).await,
        Err(PmPublicCaptureRunError::ArtifactTerminal { .. })
    ));
    assert_terminal_finish(
        okx_run.finish().await.unwrap_err(),
        PmPublicCaptureTerminalCause::CaptureWriter,
    );
}

#[tokio::test]
async fn empty_and_nonempty_malformed_frames_have_typed_terminal_classification() {
    let directory = tempfile::tempdir().unwrap();

    let mut pm_empty = start_live_run(directory.path().join("pm-empty.jsonl")).await;
    let error = pm_empty
        .capture_pm_public(WALL_BASE + 100, 100, &[])
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        PmPublicCaptureRunError::PmCaptureRejected {
            source: PmCaptureWriteError::Contract(PmCaptureVerifyError::InvalidRawFrame(_)),
            ..
        }
    ));
    assert_eq!(
        error.pm_unavailable().unwrap().envelope().payload().fault(),
        PmPublicSessionFault::InvalidTransition
    );
    assert!(pm_empty.artifact_terminal());
    assert_terminal_finish(
        pm_empty.finish().await.unwrap_err(),
        PmPublicCaptureTerminalCause::CaptureWriter,
    );

    let mut pm_malformed = start_live_run(directory.path().join("pm-malformed.jsonl")).await;
    let error = pm_malformed
        .capture_pm_public(WALL_BASE + 200, 200, br#"{"#)
        .await
        .unwrap_err();
    let PmPublicCaptureRunError::PmClassify {
        unavailable: Some(unavailable),
        ..
    } = &error
    else {
        panic!("nonempty PM wire failure must retain typed classify evidence: {error:?}");
    };
    assert_eq!(
        unavailable.envelope().payload().fault(),
        PmPublicSessionFault::InvalidTransition
    );
    assert!(pm_malformed.artifact_terminal());
    assert_terminal_finish(
        pm_malformed.finish().await.unwrap_err(),
        PmPublicCaptureTerminalCause::IngressSessionClassification,
    );

    let mut okx_empty = start_live_run(directory.path().join("okx-empty.jsonl")).await;
    let error = okx_empty
        .capture_okx_public(WALL_BASE + 300, 300, &[])
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        reap_pm_live::PmPublicDataPipelineError::Run(PmPublicCaptureRunError::OkxCaptureRejected {
            source: PmCaptureWriteError::Contract(PmCaptureVerifyError::InvalidRawFrame(_)),
            ..
        })
    ));
    assert_eq!(
        error
            .run_error()
            .unwrap()
            .okx_unavailable()
            .unwrap()
            .envelope()
            .payload()
            .fault(),
        OkxPublicSessionFault::InvalidTransition
    );
    assert!(okx_empty.artifact_terminal());
    assert_terminal_finish(
        okx_empty.finish().await.unwrap_err(),
        PmPublicCaptureTerminalCause::CaptureWriter,
    );

    let mut okx_malformed = start_live_run(directory.path().join("okx-malformed.jsonl")).await;
    let error = okx_malformed
        .capture_okx_public(WALL_BASE + 400, 400, br#"{"#)
        .await
        .unwrap_err();
    let PmPublicCaptureRunError::OkxClassify {
        unavailable: Some(unavailable),
        ..
    } = error.run_error().unwrap()
    else {
        panic!("nonempty OKX wire failure must retain typed classify evidence: {error:?}");
    };
    assert_eq!(
        unavailable.envelope().payload().fault(),
        OkxPublicSessionFault::InvalidTransition
    );
    assert!(okx_malformed.artifact_terminal());
    assert_terminal_finish(
        okx_malformed.finish().await.unwrap_err(),
        PmPublicCaptureTerminalCause::IngressSessionClassification,
    );
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one end-to-end terminal contract enumerates every mutation gate against one owner"
)]
async fn terminal_run_returns_move_only_inputs_and_blocks_every_mutation_without_side_effects() {
    let directory = tempfile::tempdir().unwrap();
    let target_path = directory.path().join("terminal-target.jsonl");
    let mut target = start_live_run(target_path.clone()).await;
    let terminal = target
        .capture_pm_public(WALL_BASE + 100, 100, &[])
        .await
        .unwrap_err();
    assert!(matches!(
        terminal,
        PmPublicCaptureRunError::PmCaptureRejected { .. }
    ));
    let retained_peer = target
        .terminal_okx_unavailable()
        .unwrap()
        .envelope()
        .clone();

    let mut producer = start_live_run(directory.path().join("producer.jsonl")).await;
    let mut aged_producer = start_live_run(directory.path().join("aged-producer.jsonl")).await;
    aged_producer
        .capture_okx_public(WALL_BASE + 90, 90, okx_ack().as_bytes())
        .await
        .unwrap();
    assert!(matches!(
        aged_producer
            .capture_okx_public(WALL_BASE + 92, 92, okx_reference().as_bytes())
            .await
            .unwrap(),
        OkxPublicCaptureEvent::ReferenceEnqueued
    ));
    let aged_depth = aged_producer.public_lane_metrics().depth();
    let aged_failure = aged_producer
        .service_lane_turn(500_000_093, &mut NoopLaneService)
        .unwrap_err();
    let expected_aged_evidence = match &aged_failure {
        PmServiceTurnError::Aged(aged) => {
            let evidence = aged.evidence();
            (
                aged.lane(),
                aged.action(),
                evidence.key(),
                evidence.connection(),
                evidence.ordering(),
                evidence.received_clock(),
                evidence.observed_now_ns(),
            )
        }
        other => panic!("producer route must yield an aged failure: {other:?}"),
    };

    let book = producer
        .capture_pm_public(WALL_BASE + 110, 110, snapshot_one().as_bytes())
        .await
        .unwrap()
        .into_books()
        .into_iter()
        .next()
        .unwrap();
    let mut snapshot_producer =
        start_live_run(directory.path().join("snapshot-producer.jsonl")).await;
    let mut snapshot_batch = snapshot_producer
        .capture_pm_public(WALL_BASE + 210, 210, snapshot_two().as_bytes())
        .await
        .unwrap();
    let snapshot_flow = snapshot_batch.take_snapshot_flow().unwrap();
    let snapshot_delivery = snapshot_batch.into_books().into_iter().next().unwrap();
    let expected_snapshot_envelope = snapshot_delivery.envelope().clone();
    let expected_snapshot_flow = (
        snapshot_flow.connection_epoch(),
        snapshot_flow.snapshot_revision(),
        snapshot_flow.local_ingress_sequence(),
    );

    assert!(matches!(
        target
            .capture_pm_public(WALL_BASE + 300, 300, snapshot_one().as_bytes())
            .await,
        Err(PmPublicCaptureRunError::ArtifactTerminal { .. })
    ));
    assert!(matches!(
        target
            .capture_okx_public(WALL_BASE + 301, 301, okx_ack().as_bytes())
            .await,
        Err(reap_pm_live::PmPublicDataPipelineError::Run(
            PmPublicCaptureRunError::ArtifactTerminal { .. }
        ))
    ));
    assert!(matches!(
        target.record_pm_connection_started(302).await,
        Err(PmPublicCaptureRunError::ArtifactTerminal { .. })
    ));
    assert!(matches!(
        target.record_okx_connection_started(303).await,
        Err(PmPublicCaptureRunError::ArtifactTerminal { .. })
    ));
    assert!(matches!(
        target.record_pm_subscription_sent(304).await,
        Err(PmPublicCaptureRunError::ArtifactTerminal { .. })
    ));
    assert!(matches!(
        target.record_okx_subscription_sent(305).await,
        Err(PmPublicCaptureRunError::ArtifactTerminal { .. })
    ));
    assert!(matches!(
        target.record_pm_disconnected(WALL_BASE + 306, 306).await,
        Err(PmPublicCaptureRunError::ArtifactTerminal { .. })
    ));
    assert!(matches!(
        target.record_okx_disconnected(WALL_BASE + 307, 307).await,
        Err(PmPublicCaptureRunError::ArtifactTerminal { .. })
    ));
    assert!(matches!(
        target.record_pm_reconnect_scheduled(308).await,
        Err(PmPublicCaptureRunError::ArtifactTerminal { .. })
    ));
    assert!(matches!(
        target.record_okx_reconnect_scheduled(309).await,
        Err(PmPublicCaptureRunError::ArtifactTerminal { .. })
    ));
    assert!(matches!(
        target
            .record_pm_heartbeat_ping_sent(WALL_BASE + 310, 310)
            .await,
        Err(PmPublicCaptureRunError::ArtifactTerminal { .. })
    ));
    assert!(matches!(
        target.record_freshness_timer(311).await,
        Err(PmPublicCaptureRunError::ArtifactTerminal { .. })
    ));
    assert!(matches!(
        target.issue_and_enqueue_pm_metadata(WALL_BASE + 312),
        Err(reap_pm_live::PmPublicDataPipelineError::Run(
            PmPublicCaptureRunError::ArtifactTerminal { .. }
        ))
    ));

    let initial_depth = target.public_lane_metrics().depth();

    let expected_book = book.envelope().clone();
    let reducer_counters = target.pm_book_counters();
    let reducer_readiness = target.pm_book_readiness();
    let reducer_ingress = target.pm_book_last_ingress_sequence();
    let reducer_snapshot_hash = target.pm_book_last_verified_snapshot_hash();
    let PmPublicBookPipelineError::Reduce(PmPublicCaptureRunError::PmBookReduceRunTerminal {
        delivery,
    }) = target.reduce_then_enqueue_pm_book(book).unwrap_err()
    else {
        panic!("terminal reducer-first admission must retain its exact delivery");
    };
    assert_eq!(delivery.envelope(), &expected_book);

    assert_eq!(
        target.public_lane_metrics().depth(),
        initial_depth,
        "terminal admission and enactment cannot touch the lane"
    );

    let PmPublicAgedLaneEnactError::RunTerminal { failure } = target
        .enact_public_lane_aged(aged_failure, WALL_BASE + 500_000_093, 500_000_093)
        .await
        .unwrap_err()
    else {
        panic!("terminal aged enactment must return its evidence");
    };
    let actual_aged_evidence = match &failure {
        PmServiceTurnError::Aged(aged) => {
            let evidence = aged.evidence();
            (
                aged.lane(),
                aged.action(),
                evidence.key(),
                evidence.connection(),
                evidence.ordering(),
                evidence.received_clock(),
                evidence.observed_now_ns(),
            )
        }
        other => panic!("terminal enactment returned the wrong failure: {other:?}"),
    };
    assert_eq!(actual_aged_evidence, expected_aged_evidence);
    assert_eq!(
        aged_producer.public_lane_metrics().depth(),
        aged_depth,
        "terminal aged enactment is non-consuming"
    );
    aged_producer
        .enact_public_lane_aged(failure, WALL_BASE + 500_000_093, 500_000_093)
        .await
        .unwrap();
    assert_eq!(
        aged_producer
            .service_lane_turn(500_000_094, &mut NoopLaneService)
            .unwrap(),
        1
    );

    let snapshot_error = target.commit_then_enqueue_pm_snapshot(snapshot_delivery, snapshot_flow);
    let PmPublicBookPipelineError::Reduce(error) = snapshot_error.unwrap_err() else {
        panic!("terminal snapshot fails before lane admission");
    };
    let (returned_delivery, returned_flow) = error.snapshot_terminal_inputs().unwrap();
    assert_eq!(returned_delivery.envelope(), &expected_snapshot_envelope);
    assert_eq!(
        (
            returned_flow.connection_epoch(),
            returned_flow.snapshot_revision(),
            returned_flow.local_ingress_sequence(),
        ),
        expected_snapshot_flow
    );

    assert_eq!(target.pm_book_counters(), reducer_counters);
    assert_eq!(target.pm_book_readiness(), reducer_readiness);
    assert_eq!(target.pm_book_last_ingress_sequence(), reducer_ingress);
    assert_eq!(
        target.pm_book_last_verified_snapshot_hash(),
        reducer_snapshot_hash
    );
    assert!(target.ready_pm_book_view().is_none());
    assert_eq!(
        target.terminal_okx_unavailable().unwrap().envelope(),
        &retained_peer,
        "post-terminal calls cannot advance or replace session evidence"
    );

    assert_terminal_finish(
        target.finish().await.unwrap_err(),
        PmPublicCaptureTerminalCause::CaptureWriter,
    );
    assert_eq!(
        std::fs::read_to_string(&target_path)
            .unwrap()
            .lines()
            .count(),
        5,
        "only the header and four pre-terminal lifecycle records were written"
    );
    drop(producer);
    aged_producer.finish().await.unwrap();
    assert!(matches!(
        snapshot_producer.finish().await,
        Err(PmPublicCaptureRunError::PendingPmBookReductionFinish { pending: 1, .. })
    ));
}

#[tokio::test]
async fn explicit_disconnect_reconnect_accepts_fresh_pm_snapshot_and_okx_ack_in_one_artifact() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("recoverable-reconnect.jsonl")).await;

    assert!(matches!(
        run.capture_okx_public(WALL_BASE + 90, 90, okx_ack().as_bytes())
            .await
            .unwrap(),
        OkxPublicCaptureEvent::SubscriptionAcknowledged(_)
    ));
    let mut first = run
        .capture_pm_public(WALL_BASE + 100, 100, snapshot_one().as_bytes())
        .await
        .unwrap();
    let first_flow = first.take_snapshot_flow().unwrap();
    let first_delivery = first.into_books().into_iter().next().unwrap();
    assert_eq!(
        first_delivery
            .envelope()
            .ordering()
            .local_ingress_sequence()
            .value(),
        1
    );
    run.commit_then_enqueue_pm_snapshot(first_delivery, first_flow)
        .unwrap();
    assert!(run.pm_book_readiness().is_ready());

    let pm_fault = run
        .record_pm_disconnected(WALL_BASE + 200, 200)
        .await
        .unwrap();
    let okx_fault = run
        .record_okx_disconnected(WALL_BASE + 201, 201)
        .await
        .unwrap();
    assert_eq!(pm_fault, PmPublicSessionFault::Disconnect);
    assert_eq!(okx_fault, OkxPublicSessionFault::Disconnect);
    assert_eq!(run.service_lane_turn(202, &mut NoopLaneService).unwrap(), 1);
    assert_eq!(run.service_lane_turn(202, &mut NoopLaneService).unwrap(), 1);
    assert_eq!(
        run.record_pm_reconnect_scheduled(210)
            .await
            .unwrap()
            .as_nanos(),
        10
    );
    assert_eq!(
        run.record_okx_reconnect_scheduled(211)
            .await
            .unwrap()
            .as_nanos(),
        10
    );
    run.record_pm_connection_started(220).await.unwrap();
    run.record_okx_connection_started(221).await.unwrap();
    run.record_pm_subscription_sent(230).await.unwrap();
    run.record_okx_subscription_sent(231).await.unwrap();

    assert!(matches!(
        run.capture_okx_public(WALL_BASE + 240, 240, okx_ack().as_bytes())
            .await
            .unwrap(),
        OkxPublicCaptureEvent::SubscriptionAcknowledged(_)
    ));
    let ignored = br#"[{"event_type":"last_trade_price"}]"#;
    assert_eq!(
        run.capture_pm_public(WALL_BASE + 241, 241, ignored)
            .await
            .unwrap()
            .ignored_public_trades(),
        1
    );
    assert_eq!(
        run.capture_pm_public(WALL_BASE + 242, 242, ignored)
            .await
            .unwrap()
            .ignored_public_trades(),
        1
    );
    let mut fresh = run
        .capture_pm_public(WALL_BASE + 10_000, 10_000, snapshot_two().as_bytes())
        .await
        .unwrap();
    let fresh_flow = fresh.take_snapshot_flow().unwrap();
    let fresh_delivery = fresh.into_books().into_iter().next().unwrap();
    assert_eq!(
        fresh_delivery
            .envelope()
            .ordering()
            .local_ingress_sequence()
            .value(),
        1,
        "ignored raw ingress is not misrepresented as normalized venue ingress"
    );
    run.commit_then_enqueue_pm_snapshot(fresh_delivery, fresh_flow)
        .unwrap();
    assert!(run.pm_book_readiness().is_ready());
    assert_eq!(
        run.ready_pm_book_view().unwrap().connection_epoch().value(),
        12
    );
    assert_eq!(run.pm_book_counters().snapshots_committed, 2);
    assert_eq!(run.pm_book_counters().gaps, 0);
    assert!(!run.artifact_terminal());

    assert_eq!(
        run.service_lane_turn(10_001, &mut NoopLaneService).unwrap(),
        1
    );
    let outcome = run.finish().await.unwrap();
    assert_eq!(outcome.projection().counters().disconnects, 1);
    assert_eq!(outcome.projection().counters().okx_disconnects, 1);
    assert_eq!(outcome.projection().counters().reconnects, 1);
    assert_eq!(outcome.projection().counters().okx_reconnects, 1);
    assert_eq!(outcome.projection().counters().snapshots_committed, 2);
}
