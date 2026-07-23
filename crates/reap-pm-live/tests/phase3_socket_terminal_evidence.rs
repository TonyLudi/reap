mod support;

use futures_util::{SinkExt, StreamExt};
use reap_okx_public_source::OkxPublicSessionFault;
use reap_pm_live::{
    MAX_PM_RAW_PUBLIC_FRAME_BYTES, PmCaptureVerifyError, PmCaptureWriteError,
    PmPublicAgedLaneEnactError, PmPublicBookReadinessReason, PmPublicCapture, PmPublicCaptureRun,
    PmPublicCaptureRunError, PmPublicCaptureTerminalCause, PmPublicLaneService,
};
use reap_pm_state::PmExternalBookFault;
use reap_polymarket_adapter::PmPublicSessionFault;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_tungstenite::{WebSocketStream, accept_async, connect_async, tungstenite::Message};

use support::{
    authoritative, bbo, okx_ack, provenance, public_config, session_policy, snapshot_one,
};

const WALL_BASE: u64 = 1_700_000_000_000_000_000;
const PUBLIC_MAX_AGE_NS: u64 = 500_000_000;

fn snapshot_consistent_bbo() -> String {
    bbo()
        .replace(r#""best_bid":"0.50""#, r#""best_bid":"0.40""#)
        .replace(r#""bid_size":"12.5""#, r#""bid_size":"50""#)
}

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
        panic!("terminal run must not yield a normal capture outcome: {error:?}");
    };
    assert_eq!(cause, expected_cause);
    assert!(
        shutdown_error.is_none(),
        "the healthy writer must still close cleanly"
    );
}

async fn spawn_single_frame_server(
    expected_subscription: Vec<u8>,
    frame: Vec<u8>,
) -> (std::net::SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        assert_eq!(
            receive_application_bytes(&mut socket).await,
            expected_subscription
        );
        socket.send(Message::Binary(frame.into())).await.unwrap();
        socket.close(None).await.unwrap();
    });
    (address, task)
}

async fn receive_application_bytes<S>(socket: &mut WebSocketStream<S>) -> Vec<u8>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        let message = socket.next().await.unwrap().unwrap();
        match message {
            Message::Text(_) | Message::Binary(_) => return message.into_data().to_vec(),
            Message::Ping(payload) => socket.send(Message::Pong(payload)).await.unwrap(),
            Message::Pong(_) => {}
            Message::Close(_) => panic!("socket closed before the application frame arrived"),
            Message::Frame(_) => unreachable!("tungstenite does not expose raw frames"),
        }
    }
}

async fn receive_pm_socket_frame(
    run: &PmPublicCaptureRun,
    frame: Vec<u8>,
) -> (Vec<u8>, JoinHandle<()>) {
    let subscription = run.pm_subscription_bytes().to_vec();
    let (address, server) = spawn_single_frame_server(subscription.clone(), frame).await;
    let (mut socket, _) = connect_async(format!("ws://{address}")).await.unwrap();
    socket
        .send(Message::Binary(subscription.into()))
        .await
        .unwrap();
    let raw = receive_application_bytes(&mut socket).await;
    (raw, server)
}

#[tokio::test]
async fn loopback_oversize_pm_frame_terminalizes_both_sessions_and_a_fresh_run_recovers() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_live_run(directory.path().join("socket-overflow.jsonl")).await;
    let oversized = vec![0_u8; MAX_PM_RAW_PUBLIC_FRAME_BYTES + 1];
    assert!(oversized.len() > 1024 * 1024);

    let (raw, server) = receive_pm_socket_frame(&run, oversized).await;
    assert_eq!(raw.len(), MAX_PM_RAW_PUBLIC_FRAME_BYTES + 1);
    let error = run
        .capture_pm_public(WALL_BASE + 100, 100, &raw)
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        PmPublicCaptureRunError::PmCaptureRejected {
            source: PmCaptureWriteError::Contract(PmCaptureVerifyError::RawFrameTooLarge),
            ..
        }
    ));
    let pm_unavailable = error.pm_unavailable().unwrap().envelope();
    assert_eq!(
        pm_unavailable.payload().fault(),
        PmPublicSessionFault::Overflow
    );
    assert_eq!(
        pm_unavailable.received_clock().local_wall_receive_ns(),
        WALL_BASE + 100
    );
    assert_eq!(pm_unavailable.received_clock().monotonic_receive_ns(), 100);
    let okx_unavailable = run.terminal_okx_unavailable().unwrap().envelope();
    assert_eq!(
        okx_unavailable.payload().fault(),
        OkxPublicSessionFault::Overflow
    );
    assert_eq!(
        okx_unavailable.received_clock().local_wall_receive_ns(),
        WALL_BASE + 100
    );
    assert_eq!(okx_unavailable.received_clock().monotonic_receive_ns(), 100);
    assert_eq!(
        run.terminal_cause(),
        Some(PmPublicCaptureTerminalCause::CaptureWriter)
    );

    let gated_counters = run.pm_book_counters();
    let gated_readiness = run.pm_book_readiness();
    assert!(matches!(
        run.record_pm_reconnect_scheduled(101).await,
        Err(PmPublicCaptureRunError::ArtifactTerminal {
            cause: PmPublicCaptureTerminalCause::CaptureWriter
        })
    ));
    assert_eq!(run.pm_book_counters(), gated_counters);
    assert_eq!(run.pm_book_readiness(), gated_readiness);
    assert!(matches!(
        run.record_freshness_timer(102).await,
        Err(PmPublicCaptureRunError::ArtifactTerminal {
            cause: PmPublicCaptureTerminalCause::CaptureWriter
        })
    ));
    assert!(matches!(
        run.issue_and_enqueue_pm_metadata(WALL_BASE + 103),
        Err(reap_pm_live::PmPublicDataPipelineError::Run(
            PmPublicCaptureRunError::ArtifactTerminal {
                cause: PmPublicCaptureTerminalCause::CaptureWriter
            }
        ))
    ));
    assert_eq!(
        run.terminal_cause(),
        Some(PmPublicCaptureTerminalCause::CaptureWriter),
        "later rejected mutations cannot replace the originating cause"
    );
    assert_terminal_finish(
        run.finish().await.unwrap_err(),
        PmPublicCaptureTerminalCause::CaptureWriter,
    );
    server.await.unwrap();

    let mut fresh = start_live_run(directory.path().join("socket-fresh.jsonl")).await;
    let ignored = br#"[{"event_type":"last_trade_price"}]"#.to_vec();
    let (raw, server) = receive_pm_socket_frame(&fresh, ignored).await;
    let batch = fresh
        .capture_pm_public(WALL_BASE + 200, 200, &raw)
        .await
        .unwrap();
    assert_eq!(batch.ignored_public_trades(), 1);
    assert!(!fresh.artifact_terminal());
    fresh.finish().await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn classification_snapshot_and_lifecycle_paths_retain_their_originating_terminal_cause() {
    let directory = tempfile::tempdir().unwrap();

    let mut classification =
        start_live_run(directory.path().join("classification-origin.jsonl")).await;
    assert!(matches!(
        classification
            .capture_pm_public(WALL_BASE + 100, 100, br#"{"#)
            .await,
        Err(PmPublicCaptureRunError::PmClassify { .. })
    ));
    assert_eq!(
        classification.terminal_cause(),
        Some(PmPublicCaptureTerminalCause::IngressSessionClassification)
    );
    assert!(matches!(
        classification.record_pm_connection_started(101).await,
        Err(PmPublicCaptureRunError::ArtifactTerminal {
            cause: PmPublicCaptureTerminalCause::IngressSessionClassification
        })
    ));
    assert_terminal_finish(
        classification.finish().await.unwrap_err(),
        PmPublicCaptureTerminalCause::IngressSessionClassification,
    );

    let mut snapshot = start_live_run(directory.path().join("snapshot-origin.jsonl")).await;
    let mut target_batch = snapshot
        .capture_pm_public(WALL_BASE + 200, 200, snapshot_one().as_bytes())
        .await
        .unwrap();
    let _target_flow = target_batch.take_snapshot_flow().unwrap();
    let delivery = target_batch.into_books().into_iter().next().unwrap();

    let mut sibling = start_live_run(directory.path().join("snapshot-sibling-origin.jsonl")).await;
    let mut sibling_batch = sibling
        .capture_pm_public(WALL_BASE + 201, 201, snapshot_one().as_bytes())
        .await
        .unwrap();
    let sibling_flow = sibling_batch.take_snapshot_flow().unwrap();
    let _sibling_delivery = sibling_batch.into_books().into_iter().next().unwrap();

    assert!(matches!(
        snapshot.commit_then_enqueue_pm_snapshot(delivery, sibling_flow),
        Err(reap_pm_live::PmPublicBookPipelineError::Reduce(
            PmPublicCaptureRunError::PmSnapshotCommit { .. }
        ))
    ));
    assert_eq!(
        snapshot.terminal_cause(),
        Some(PmPublicCaptureTerminalCause::SnapshotReducer)
    );
    assert!(matches!(
        snapshot.record_freshness_timer(202).await,
        Err(PmPublicCaptureRunError::ArtifactTerminal {
            cause: PmPublicCaptureTerminalCause::SnapshotReducer
        })
    ));
    assert_terminal_finish(
        snapshot.finish().await.unwrap_err(),
        PmPublicCaptureTerminalCause::SnapshotReducer,
    );
    assert!(matches!(
        sibling.finish().await,
        Err(PmPublicCaptureRunError::PendingPmBookReductionFinish { pending: 1, .. })
    ));

    let mut lifecycle = start_run(directory.path().join("lifecycle-origin.jsonl")).await;
    assert!(matches!(
        lifecycle.record_pm_subscription_sent(60).await,
        Err(PmPublicCaptureRunError::InvalidLifecyclePhase)
    ));
    assert_eq!(
        lifecycle.terminal_cause(),
        Some(PmPublicCaptureTerminalCause::Lifecycle)
    );
    assert!(matches!(
        lifecycle.capture_pm_public(WALL_BASE + 300, 300, &[]).await,
        Err(PmPublicCaptureRunError::ArtifactTerminal {
            cause: PmPublicCaptureTerminalCause::Lifecycle
        })
    ));
    assert_terminal_finish(
        lifecycle.finish().await.unwrap_err(),
        PmPublicCaptureTerminalCause::Lifecycle,
    );
}

#[tokio::test]
async fn aged_lane_writer_preflight_failure_applies_only_terminal_invalid_transition() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("lane-writer-preflight.jsonl");
    let mut run = start_live_run(path.clone()).await;

    let mut snapshot = run
        .capture_pm_public(WALL_BASE + 100, 100, snapshot_one().as_bytes())
        .await
        .unwrap();
    let flow = snapshot.take_snapshot_flow().unwrap();
    let delivery = snapshot.into_books().into_iter().next().unwrap();
    run.commit_then_enqueue_pm_snapshot(delivery, flow).unwrap();

    let delivery = run
        .capture_pm_public(WALL_BASE + 200, 200, snapshot_consistent_bbo().as_bytes())
        .await
        .unwrap()
        .into_books()
        .into_iter()
        .next()
        .unwrap();
    run.reduce_then_enqueue_pm_book(delivery).unwrap();

    // Advance only the writer clock beyond the otherwise valid aged-lane
    // observation. This makes the disconnect lifecycle record fail in its
    // pure preflight, after route/session/reducer evidence has authenticated.
    run.capture_okx_public(
        WALL_BASE + 1_000_000_000,
        1_000_000_000,
        okx_ack().as_bytes(),
    )
    .await
    .unwrap();
    let observed_now_ns = 200 + PUBLIC_MAX_AGE_NS + 1;
    let baseline_counters = run.pm_book_counters();
    let baseline_ingress = run.pm_book_last_ingress_sequence();
    let baseline_snapshot_hash = run.pm_book_last_verified_snapshot_hash();
    let failure = run
        .service_lane_turn(observed_now_ns, &mut NoopLaneService)
        .unwrap_err();
    let pending_counters = run.pm_book_counters();
    assert_eq!(
        run.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::PendingLaneFault)
    );
    assert_eq!(
        run.pm_book_pending_external_fault(),
        Some(PmExternalBookFault::BacklogAged)
    );
    assert_eq!(
        pending_counters.unavailable_transitions,
        baseline_counters.unavailable_transitions + 1
    );
    assert_eq!(
        pending_counters.external_faults,
        baseline_counters.external_faults
    );
    assert_eq!(
        pending_counters.backlog_aged_faults,
        baseline_counters.backlog_aged_faults
    );
    assert_eq!(
        pending_counters.invalidations,
        baseline_counters.invalidations
    );

    let error = run
        .enact_public_lane_aged(failure, WALL_BASE + observed_now_ns, observed_now_ns)
        .await
        .unwrap_err();
    let PmPublicAgedLaneEnactError::LifecycleWrite {
        source: PmCaptureWriteError::Contract(PmCaptureVerifyError::InvalidLifecycle),
        purged_queued_deliveries,
        terminal_pm_unavailable: Some(pm_unavailable),
        terminal_okx_unavailable,
        ..
    } = error
    else {
        panic!("expected typed lifecycle-writer preflight failure: {error:?}");
    };
    assert_eq!(
        purged_queued_deliveries, 2,
        "the canonical snapshot and the later book are both Run-owned public obligations"
    );
    assert_eq!(
        pm_unavailable.envelope().payload().fault(),
        PmPublicSessionFault::InvalidTransition,
        "terminal close must not masquerade as the unapplied stale fault"
    );
    if let Some(okx_unavailable) = terminal_okx_unavailable {
        assert_eq!(
            okx_unavailable.envelope().payload().fault(),
            OkxPublicSessionFault::InvalidTransition
        );
    }
    let counters = run.pm_book_counters();
    let mut expected_counters = baseline_counters;
    expected_counters.external_faults += 1;
    expected_counters.invalid_transitions += 1;
    expected_counters.invalidations += 1;
    expected_counters.unavailable_transitions += 1;
    assert_eq!(
        counters, expected_counters,
        "only the terminal InvalidTransition fallback may mutate the reducer; stale/backlog-aged remains unapplied"
    );
    assert_eq!(
        run.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::ArtifactTerminal)
    );
    assert_eq!(run.pm_book_pending_external_fault(), None);
    assert_eq!(run.pm_book_last_ingress_sequence(), baseline_ingress);
    assert_eq!(
        run.pm_book_last_verified_snapshot_hash(),
        baseline_snapshot_hash
    );
    assert_eq!(run.public_lane_metrics().depth(), 0);
    assert_eq!(
        run.terminal_cause(),
        Some(PmPublicCaptureTerminalCause::CaptureWriter)
    );
    assert!(matches!(
        run.record_pm_reconnect_scheduled(observed_now_ns + 1).await,
        Err(PmPublicCaptureRunError::ArtifactTerminal {
            cause: PmPublicCaptureTerminalCause::CaptureWriter
        })
    ));
    assert_terminal_finish(
        run.finish().await.unwrap_err(),
        PmPublicCaptureTerminalCause::CaptureWriter,
    );

    assert_eq!(
        std::fs::read_to_string(path).unwrap().lines().count(),
        8,
        "failed writer preflight must not append a disconnect lifecycle record"
    );
}
