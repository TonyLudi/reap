mod support;

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use futures_util::{SinkExt, StreamExt};
use reap_benchmark_allocator::TrackingAllocator;
use reap_capture_framing::sha256_hex;
use reap_okx_public_source::OkxPublicSessionFault;
use reap_pm_core::ConnectionEpoch;
use reap_pm_live::{
    MAX_PM_PUBLIC_CAPTURE_ENCODED_BYTES, MAX_PM_PUBLIC_CAPTURE_RAW_FRAMES,
    MAX_PM_PUBLIC_CAPTURE_RAW_PAYLOAD_BYTES, MAX_PM_PUBLIC_CAPTURE_RECORDS,
    MAX_PM_RAW_PUBLIC_FRAME_BYTES, PM_PUBLIC_CAPTURE_SCHEMA_VERSION, PmCaptureScope,
    PmCaptureSessionPolicy, PmCaptureVerifyError, PmCaptureWriteError, PmPublicBookReadinessReason,
    PmPublicCapture, PmPublicCaptureRunError, PmPublicCaptureTerminalCause, PmReplayError,
    PmReplayLogicalEvent, replay_pm_public_capture, verify_pm_public_capture,
};
use reap_pm_state::{PmBookTransition, PmMetadataDrift, PmPublicReadinessReason};
use reap_polymarket_adapter::{
    PM_PUBLIC_PING_BYTES, PM_PUBLIC_PONG_BYTES, PmPublicSessionError, PmPublicSessionFault,
};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{WebSocketStream, accept_async, connect_async, tungstenite::Message};

use support::{
    authoritative, bbo, capture_header, delta, delta_two, max_ignored_trade_frame, okx_ack,
    okx_reference, provenance, public_config, session_policy, snapshot_one, snapshot_two,
    tick_change,
};

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

const PHASE6_RECOVERY_CHILD: &str = "REAP_PHASE6_RECOVERY_EVIDENCE_CHILD";
const PHASE6_RECOVERY_TEST: &str =
    "phase6_real_mutation_artifacts_recover_to_the_same_bounded_projection";

#[test]
fn phase6_real_mutation_artifacts_recover_to_the_same_bounded_projection() {
    if std::env::var_os(PHASE6_RECOVERY_CHILD).is_some() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread recovery evidence runtime builds")
            .block_on(Box::pin(run_phase6_recovery_evidence()));
        return;
    }

    let status = std::process::Command::new(
        std::env::current_exe().expect("combined-replay test executable is available"),
    )
    .env(PHASE6_RECOVERY_CHILD, "1")
    .args(["--exact", PHASE6_RECOVERY_TEST, "--test-threads=1"])
    .status()
    .expect("isolated recovery evidence subprocess starts");
    assert!(
        status.success(),
        "isolated recovery evidence subprocess failed with {status}"
    );
}

async fn run_phase6_recovery_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let first_path = directory.path().join("phase6-first.jsonl");
    let second_path = directory.path().join("phase6-second.jsonl");
    let first_report: serde_json::Value = serde_json::from_str(
        &reap_pm_live::run_pm_combined_replay_evidence(first_path.clone())
            .await
            .unwrap(),
    )
    .unwrap();
    let second_report: serde_json::Value = serde_json::from_str(
        &reap_pm_live::run_pm_combined_replay_evidence(second_path.clone())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(first_path).unwrap(),
        std::fs::read(second_path).unwrap()
    );
    for field in [
        "schema_version",
        "target",
        "fixture_revision",
        "build_revision",
        "rustc",
        "host",
        "replay_working_limit_bytes",
        "artifact_bytes",
        "artifact_lines",
        "artifact_sha256",
        "setup",
        "input_mix",
        "measured",
        "byte_identical_projection",
        "production_order_entry_authorized",
    ] {
        assert_eq!(
            first_report[field], second_report[field],
            "deterministic combined field {field} differs"
        );
    }
    assert_eq!(first_report["artifact_lines"], 35_012);
    assert_eq!(first_report["setup"]["journal_header_records"], 1);
    assert_eq!(first_report["setup"]["w0_external_observations"], 1);
    assert_eq!(
        first_report["setup"]["w0_internal_fact_acknowledgements"],
        1
    );
    assert_eq!(first_report["setup"]["w0_owner_reductions"], 2);
    assert_eq!(first_report["setup"]["w0_journal_records"], 1);
    assert_eq!(first_report["setup"]["w0_watermark_advances"], 1);
    assert_eq!(first_report["setup"]["physical_journal_lines"], 2);
    assert_eq!(first_report["input_mix"]["pm_book_observations"], 10_000);
    assert_eq!(
        first_report["input_mix"]["okx_reference_observations"],
        10_000
    );
    assert_eq!(first_report["input_mix"]["private_unique_fills"], 5_000);
    assert_eq!(first_report["input_mix"]["private_duplicate_fills"], 5_000);
    assert_eq!(first_report["measured"]["watermark_advances"], 10);
    assert_eq!(first_report["measured"]["journal_records"], 35_010);
    assert_eq!(first_report["production_order_entry_authorized"], false);
    let logical_fields = [
        "record_count",
        "last_sequence",
        "last_intent_id",
        "last_owned_observation_sequence",
        "compacted_intent_id",
        "owned_orders",
        "fill_keys",
        "unresolved_orders",
        "safety_halted",
        "requires_reconciliation",
        "fill_watermark",
        "canonical_sha256",
    ];
    let baseline_recovery = &first_report["first_recovery"];
    for report in [&first_report, &second_report] {
        assert_eq!(report["byte_identical_projection"], true);
        for field in ["first_recovery", "second_recovery"] {
            let recovery = &report[field];
            for logical_field in logical_fields {
                assert_eq!(
                    recovery[logical_field], baseline_recovery[logical_field],
                    "logical recovery field {logical_field} differs"
                );
            }
            assert_eq!(recovery["record_count"], 35_012);
            assert_eq!(recovery["last_intent_id"], 10_000);
            assert_eq!(recovery["compacted_intent_id"], 10_000);
            assert_eq!(recovery["owned_orders"], 0);
            assert_eq!(recovery["fill_keys"], 0);
            assert_eq!(recovery["unresolved_orders"], 0);
            assert_eq!(recovery["requires_reconciliation"], false);
            assert_eq!(
                recovery["allocator_measurement"],
                "recovery-window peak live delta minus the post-input-construction window baseline"
            );
            let baseline_live_delta = recovery["allocator_window_baseline_live_delta_bytes"]
                .as_i64()
                .unwrap();
            assert_eq!(
                recovery["peak_working_bytes"],
                recovery["allocator_window_peak_live_delta_bytes"]
                    .as_u64()
                    .unwrap()
                    .saturating_sub(u64::try_from(baseline_live_delta).unwrap_or_default())
            );
            assert!(recovery["peak_working_bytes"].as_u64().unwrap() <= 16 * 1_024 * 1_024);
        }
    }
}

struct ConsumePublic;

impl reap_pm_live::PmPublicLaneService for ConsumePublic {
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

fn service_all_public(run: &mut reap_pm_live::PmPublicCaptureRun, now_ns: u64) {
    let mut consumer = ConsumePublic;
    while run.public_lane_metrics().depth() != 0 {
        assert!(
            run.service_lane_turn(now_ns, &mut consumer).unwrap() > 0,
            "a nonempty public lane must make service progress"
        );
    }
}

#[tokio::test]
async fn fake_websocket_capture_verify_and_replay_are_byte_deterministic() {
    let directory = tempfile::tempdir().unwrap();
    let first_path = directory.path().join("fake-first.jsonl");
    let second_path = directory.path().join("fake-second.jsonl");

    run_fake_capture(first_path.clone()).await;
    run_fake_capture(second_path.clone()).await;

    let header = capture_header();
    let first_bytes = std::fs::read(&first_path).unwrap();
    let second_bytes = std::fs::read(&second_path).unwrap();
    assert_eq!(first_bytes, second_bytes);

    let first_verification = verify_pm_public_capture(&first_path, &header).unwrap();
    let second_verification = verify_pm_public_capture(&second_path, &header).unwrap();
    assert_eq!(first_verification, second_verification);
    assert_eq!(
        first_verification.artifact_sha256,
        "4f28d2dee68636dfee56efc6b979e63670263fae96c59e441536f57cda8bdd72"
    );
    assert_eq!(
        first_verification.schema_version,
        PM_PUBLIC_CAPTURE_SCHEMA_VERSION
    );
    assert_eq!(first_verification.raw_public_frames, 5);
    assert_eq!(first_verification.okx_raw_public_frames, 0);
    assert_eq!(first_verification.lifecycle_records, 7);
    assert_eq!(first_verification.freshness_timers, 1);
    assert!(!first_verification.production_order_entry_authorized);

    let first_projection = replay_pm_public_capture(&first_path, &header).unwrap();
    let second_projection = replay_pm_public_capture(&second_path, &header).unwrap();
    assert_eq!(first_projection, second_projection);
    assert_eq!(
        first_projection.canonical_bytes().unwrap(),
        second_projection.canonical_bytes().unwrap()
    );
    assert_eq!(
        sha256_hex(&first_projection.canonical_bytes().unwrap()),
        "e8092e6ab7063685e1fe8afad5d5dbbd61f0a4ec553b23a9a4cf22ba3517b9cf"
    );
    assert_eq!(first_projection.counters().snapshots_committed, 2);
    assert_eq!(first_projection.counters().resync_snapshots, 1);
    assert_eq!(first_projection.counters().delta_batches_committed, 1);
    assert_eq!(first_projection.counters().delta_changes_committed, 2);
    assert_eq!(first_projection.counters().delta_top_checks_confirmed, 1);
    assert_eq!(first_projection.counters().top_checks_confirmed, 1);
    assert_eq!(first_projection.counters().disconnects, 1);
    assert_eq!(first_projection.counters().reconnects, 1);
    assert_eq!(first_projection.counters().integrity_batches_coalesced, 0);
}

#[tokio::test]
async fn combined_okx_pm_disconnect_resync_and_timer_replay_is_deterministic() {
    let directory = tempfile::tempdir().unwrap();
    let first_path = directory.path().join("combined-first.jsonl");
    let second_path = directory.path().join("combined-second.jsonl");

    write_combined_capture(first_path.clone()).await;
    write_combined_capture(second_path.clone()).await;

    let first_bytes = std::fs::read(&first_path).unwrap();
    let second_bytes = std::fs::read(&second_path).unwrap();
    assert_eq!(first_bytes, second_bytes);
    assert_eq!(
        sha256_hex(&first_bytes),
        "3649d8daec0e72703ed6406c35c9b703beb730bf11923af3d4b8581cd2376212"
    );
    let header = capture_header();
    let first = replay_pm_public_capture(&first_path, &header).unwrap();
    let second = replay_pm_public_capture(&second_path, &header).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        first.canonical_bytes().unwrap(),
        second.canonical_bytes().unwrap()
    );
    assert_eq!(
        sha256_hex(&first.canonical_bytes().unwrap()),
        "1ea769add571701034d38476c885c448804ae76f03997cac16bd6f655e4a90d9"
    );
    assert_eq!(first.counters().okx_raw_public_frames, 4);
    assert_eq!(first.counters().okx_lifecycle_records, 6);
    assert_eq!(first.counters().okx_subscription_acknowledgements, 2);
    assert_eq!(first.counters().okx_references, 2);
    assert_eq!(first.counters().okx_disconnects, 1);
    assert_eq!(first.counters().okx_reconnects, 1);
    assert_eq!(first.counters().snapshots_committed, 2);
    assert_eq!(first.counters().delta_batches_committed, 2);
    assert_eq!(first.counters().delta_changes_committed, 4);
    assert_eq!(first.counters().integrity_batches_coalesced, 0);

    let events = first.logical_events();
    let delta_evidence = events
        .iter()
        .filter_map(|event| match event {
            PmReplayLogicalEvent::DeltaBatchCommitted {
                local_ingress_sequence,
                changes,
                venue_change_hashes_present,
                ordered_change_hashes_sha256,
                ..
            } => Some((
                *local_ingress_sequence,
                *changes,
                *venue_change_hashes_present,
                ordered_change_hashes_sha256.as_str(),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(delta_evidence.len(), 2);
    assert_eq!(delta_evidence[0].0, 2);
    assert_eq!(delta_evidence[1].0, 4);
    assert_eq!(delta_evidence[0].1, 2);
    assert_eq!(delta_evidence[1].1, 2);
    assert_eq!(delta_evidence[0].2, 2);
    assert_eq!(delta_evidence[1].2, 2);
    assert_ne!(
        delta_evidence[0].3, delta_evidence[1].3,
        "ordered per-change venue hashes must distinguish separate delta batches"
    );
    let okx_ack_index = position(events, |event| {
        matches!(
            event,
            PmReplayLogicalEvent::OkxSubscriptionAcknowledged { .. }
        )
    });
    let first_snapshot_index = position(events, |event| {
        matches!(
            event,
            PmReplayLogicalEvent::SnapshotCommitted {
                snapshot_revision: 1,
                ..
            }
        )
    });
    let okx_reference_index = position(events, |event| {
        matches!(event, PmReplayLogicalEvent::OkxReference { .. })
    });
    let okx_reconnect_index = position(events, |event| {
        matches!(
            event,
            PmReplayLogicalEvent::OkxReconnectScheduled {
                prior_epoch: 21,
                next_epoch: 22,
                ..
            }
        )
    });
    let fresh_okx_ack_index = position(events, |event| {
        matches!(
            event,
            PmReplayLogicalEvent::OkxSubscriptionAcknowledged { epoch: 22 }
        )
    });
    let delta_index = position(events, |event| {
        matches!(event, PmReplayLogicalEvent::DeltaBatchCommitted { .. })
    });
    assert!(okx_ack_index < first_snapshot_index);
    assert!(first_snapshot_index < okx_reference_index);
    assert!(okx_reference_index < okx_reconnect_index);
    assert!(okx_reconnect_index < fresh_okx_ack_index);
    assert!(fresh_okx_ack_index < delta_index);
}

#[tokio::test]
async fn raw_frame_and_raw_count_bounds_are_exact() {
    let directory = tempfile::tempdir().unwrap();
    let ignored = br#"[{"event_type":"last_trade_price"}]"#;
    let mut max_raw = vec![b' '; MAX_PM_RAW_PUBLIC_FRAME_BYTES];
    max_raw[..ignored.len()].copy_from_slice(ignored);

    let max_path = directory.path().join("max-frame.jsonl");
    let mut max_run = start_capture_run(max_path.clone()).await;
    max_run.record_pm_connection_started(60).await.unwrap();
    max_run.record_pm_subscription_sent(90).await.unwrap();
    assert_eq!(
        max_run
            .capture_pm_public(1_700_000_000_000_000_100, 100, &max_raw)
            .await
            .unwrap()
            .ignored_public_trades(),
        1
    );
    max_run.finish().await.unwrap();
    let verified = verify_pm_public_capture(&max_path, &capture_header()).unwrap();
    assert_eq!(
        verified.raw_payload_bytes,
        MAX_PM_RAW_PUBLIC_FRAME_BYTES as u64
    );

    let oversize_path = directory.path().join("oversize-frame.jsonl");
    let mut oversize = start_capture_run(oversize_path).await;
    oversize.record_pm_connection_started(60).await.unwrap();
    oversize.record_pm_subscription_sent(90).await.unwrap();
    let error = oversize
        .capture_pm_public(
            1_700_000_000_000_000_100,
            100,
            &vec![0_u8; MAX_PM_RAW_PUBLIC_FRAME_BYTES + 1],
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            &error,
            PmPublicCaptureRunError::PmCaptureRejected {
                source: PmCaptureWriteError::Contract(PmCaptureVerifyError::RawFrameTooLarge),
                ..
            }
        ),
        "unexpected oversize-frame error: {error:?}"
    );
    assert!(matches!(
        oversize.finish().await,
        Err(PmPublicCaptureRunError::TerminalFinish { .. })
    ));

    let count_path = directory.path().join("raw-count.jsonl");
    let mut count_run = start_capture_run(count_path.clone()).await;
    count_run.record_pm_connection_started(60).await.unwrap();
    count_run.record_pm_subscription_sent(100).await.unwrap();
    for sequence in 1..=8_192_u64 {
        let monotonic = 1_000 + sequence;
        let _ = count_run
            .capture_pm_public(1_700_000_000_000_000_000 + monotonic, monotonic, ignored)
            .await
            .unwrap();
    }
    let error = count_run
        .capture_pm_public(1_700_000_000_000_010_000, 10_000, ignored)
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        PmPublicCaptureRunError::PmCaptureRejected {
            source: PmCaptureWriteError::Contract(PmCaptureVerifyError::TooManyRawFrames),
            ..
        }
    ));
    assert!(matches!(
        count_run.finish().await,
        Err(PmPublicCaptureRunError::TerminalFinish { .. })
    ));
    let verified = verify_pm_public_capture(&count_path, &capture_header()).unwrap();
    assert_eq!(verified.raw_public_frames, 8_192);

    let record_path = directory.path().join("record-count.jsonl");
    let mut record_run = start_capture_run(record_path.clone()).await;
    record_run.record_pm_connection_started(60).await.unwrap();
    record_run.record_pm_subscription_sent(100).await.unwrap();
    for sequence in 1..=8_191_u64 {
        let monotonic = 1_000 + sequence;
        let _ = record_run
            .capture_pm_public(1_700_000_000_000_000_000 + monotonic, monotonic, ignored)
            .await
            .unwrap();
    }
    let mut snapshot = record_run
        .capture_pm_public(1_700_000_000_000_010_000, 10_000, snapshot_one().as_bytes())
        .await
        .unwrap();
    let flow = snapshot.take_snapshot_flow().unwrap();
    let delivery = snapshot.into_books().into_iter().next().unwrap();
    record_run
        .commit_then_enqueue_pm_snapshot(delivery, flow)
        .unwrap();
    for timer in 1..=8_189_u64 {
        let _ = timer;
        record_run.record_freshness_timer(10_001).await.unwrap();
    }
    let error = record_run.record_freshness_timer(10_001).await.unwrap_err();
    assert!(matches!(
        error,
        PmPublicCaptureRunError::Write(PmCaptureWriteError::Contract(
            PmCaptureVerifyError::TooManyRecords
        ))
    ));
    assert!(matches!(
        record_run.finish().await,
        Err(PmPublicCaptureRunError::TerminalFinish { .. })
    ));
    let verified = verify_pm_public_capture(&record_path, &capture_header()).unwrap();
    assert_eq!(verified.raw_public_frames, 8_192);
    assert_eq!(verified.freshness_timers, 8_189);
    assert_eq!(verified.records, MAX_PM_PUBLIC_CAPTURE_RECORDS);

    let aggregate_path = directory.path().join("raw-aggregate.jsonl");
    let mut aggregate = start_capture_run(aggregate_path.clone()).await;
    aggregate.record_pm_connection_started(60).await.unwrap();
    aggregate.record_pm_subscription_sent(100).await.unwrap();
    for sequence in 1..=32 {
        let monotonic = 20_000 + sequence;
        let _ = aggregate
            .capture_pm_public(1_700_000_000_000_000_000 + monotonic, monotonic, &max_raw)
            .await
            .unwrap();
    }
    let error = aggregate
        .capture_pm_public(1_700_000_000_000_020_033, 20_033, &[0])
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        PmPublicCaptureRunError::PmCaptureRejected {
            source: PmCaptureWriteError::Contract(PmCaptureVerifyError::RawPayloadTooLarge),
            ..
        }
    ));
    assert!(matches!(
        aggregate.finish().await,
        Err(PmPublicCaptureRunError::TerminalFinish { .. })
    ));
    let verified = verify_pm_public_capture(&aggregate_path, &capture_header()).unwrap();
    assert_eq!(
        verified.raw_payload_bytes,
        MAX_PM_PUBLIC_CAPTURE_RAW_PAYLOAD_BYTES
    );
}

#[tokio::test]
async fn pm_capture_overflow_terminals_shared_artifact_and_requires_a_fresh_run() {
    let directory = tempfile::tempdir().unwrap();
    let overflow_path = directory.path().join("overflow.jsonl");
    let mut run = start_capture_run(overflow_path).await;
    run.record_pm_connection_started(60).await.unwrap();
    run.record_pm_subscription_sent(100).await.unwrap();
    let ignored = br#"[{"event_type":"last_trade_price"}]"#;
    for sequence in 1..=MAX_PM_PUBLIC_CAPTURE_RAW_FRAMES {
        let monotonic = 1_000 + sequence;
        let batch = run
            .capture_pm_public(1_700_000_000_000_000_000 + monotonic, monotonic, ignored)
            .await
            .unwrap();
        assert_eq!(batch.ignored_public_trades(), 1);
    }
    let error = run
        .capture_pm_public(1_700_000_000_000_020_000, 20_000, snapshot_one().as_bytes())
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        PmPublicCaptureRunError::PmCaptureRejected {
            source: PmCaptureWriteError::Contract(PmCaptureVerifyError::TooManyRawFrames),
            ..
        }
    ));
    let unavailable = error.pm_unavailable().unwrap().envelope();
    assert_eq!(
        unavailable.payload().fault(),
        PmPublicSessionFault::Overflow
    );
    assert_eq!(
        unavailable.received_clock().local_wall_receive_ns(),
        1_700_000_000_000_020_000
    );
    assert_eq!(unavailable.received_clock().monotonic_receive_ns(), 20_000);
    assert!(matches!(
        run.record_pm_reconnect_scheduled(20_001).await,
        Err(PmPublicCaptureRunError::ArtifactTerminal { .. })
    ));
    assert!(matches!(
        run.finish().await,
        Err(PmPublicCaptureRunError::TerminalFinish { .. })
    ));
}

#[tokio::test]
async fn okx_capture_overflow_terminals_shared_artifact_and_requires_a_fresh_run() {
    let directory = tempfile::tempdir().unwrap();
    let overflow_path = directory.path().join("okx-overflow.jsonl");
    let mut run = start_capture_run(overflow_path).await;
    run.record_pm_connection_started(60).await.unwrap();
    run.record_okx_connection_started(70).await.unwrap();
    run.record_pm_subscription_sent(100).await.unwrap();
    run.record_okx_subscription_sent(101).await.unwrap();
    let ignored = br#"[{"event_type":"last_trade_price"}]"#;
    for sequence in 1..=MAX_PM_PUBLIC_CAPTURE_RAW_FRAMES {
        let monotonic = 1_000 + sequence;
        let _ = run
            .capture_pm_public(1_700_000_000_000_000_000 + monotonic, monotonic, ignored)
            .await
            .unwrap();
    }
    let error = run
        .capture_okx_public(1_700_000_000_000_020_000, 20_000, okx_ack().as_bytes())
        .await
        .unwrap_err();
    let run_error = error.run_error().unwrap();
    assert!(matches!(
        run_error,
        PmPublicCaptureRunError::OkxCaptureRejected {
            source: PmCaptureWriteError::Contract(PmCaptureVerifyError::TooManyRawFrames),
            ..
        }
    ));
    let unavailable = run_error.okx_unavailable().unwrap().envelope();
    assert_eq!(
        unavailable.payload().fault(),
        OkxPublicSessionFault::Overflow
    );
    assert_eq!(
        unavailable.received_clock().local_wall_receive_ns(),
        1_700_000_000_000_020_000
    );
    assert_eq!(unavailable.received_clock().monotonic_receive_ns(), 20_000);
    assert!(matches!(
        run.record_okx_reconnect_scheduled(20_001).await,
        Err(PmPublicCaptureRunError::ArtifactTerminal { .. })
    ));
    assert!(matches!(
        run.finish().await,
        Err(PmPublicCaptureRunError::TerminalFinish { .. })
    ));
}

#[tokio::test]
async fn worst_case_multi_event_frame_is_budgeted_and_replayed_without_coalescing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("max-events.jsonl");
    let mut run = start_capture_run(path).await;
    run.record_pm_connection_started(60).await.unwrap();
    run.record_pm_subscription_sent(100).await.unwrap();
    assert_eq!(
        run.capture_pm_public(1_700_000_000_000_000_110, 110, &max_ignored_trade_frame(),)
            .await
            .unwrap()
            .ignored_public_trades(),
        64
    );
    let outcome = run.finish().await.unwrap();
    let projection = outcome.projection();
    assert_eq!(projection.counters().public_trades_ignored, 64);
    assert_eq!(
        projection
            .logical_events()
            .iter()
            .filter(|event| matches!(event, PmReplayLogicalEvent::PublicTradeIgnored { .. }))
            .count(),
        64
    );
    assert_eq!(projection.counters().integrity_batches_coalesced, 0);
    assert!(projection.counters().projection_reserved_capacity_bytes <= 16 * 1024 * 1024);
    assert_eq!(
        projection.counters().projection_reserved_capacity_bytes,
        projection.counters().projection_event_capacity_bytes
            + projection.counters().projection_payload_capacity_bytes
    );
}

#[tokio::test]
async fn tick_drift_is_terminal_until_new_authoritative_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let terminal_path = directory.path().join("tick-terminal.jsonl");
    write_tick_capture(terminal_path.clone(), false).await;
    let terminal = replay_pm_public_capture(&terminal_path, &capture_header()).unwrap();
    assert_eq!(terminal.counters().tick_size_invalidations, 1);
    assert!(terminal.counters().metadata_refresh_required);

    let resumed_path = directory.path().join("tick-resumed.jsonl");
    write_tick_capture(resumed_path.clone(), true).await;
    let gated = replay_pm_public_capture(&resumed_path, &capture_header()).unwrap();
    assert_eq!(
        gated, terminal,
        "post-tick mutation is rejected before another raw record reaches the artifact"
    );
}

#[tokio::test]
async fn successful_tick_classification_immediately_terminals_and_applies_exact_invalidation() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_capture_run(directory.path().join("tick-admission-terminal.jsonl")).await;
    run.record_pm_connection_started(60).await.unwrap();
    run.record_okx_connection_started(61).await.unwrap();
    run.record_pm_subscription_sent(80).await.unwrap();
    run.record_okx_subscription_sent(81).await.unwrap();
    let mut snapshot = run
        .capture_pm_public(1_700_000_000_000_000_100, 100, snapshot_one().as_bytes())
        .await
        .unwrap();
    let flow = snapshot.take_snapshot_flow().unwrap();
    let delivery = snapshot.into_books().into_iter().next().unwrap();
    run.commit_then_enqueue_pm_snapshot(delivery, flow).unwrap();

    let mut tick = run
        .capture_pm_public(1_700_000_000_000_000_110, 110, tick_change().as_bytes())
        .await
        .unwrap();
    assert_eq!(
        run.terminal_cause(),
        Some(PmPublicCaptureTerminalCause::TickSizeChanged)
    );
    assert_eq!(
        tick.take_unavailable()
            .unwrap()
            .envelope()
            .payload()
            .fault(),
        PmPublicSessionFault::TickSizeChanged
    );
    assert!(
        tick.take_unavailable().is_none(),
        "the PM session issues exactly one unavailable occurrence"
    );
    assert!(
        run.terminal_pm_unavailable().is_none(),
        "capture-time terminalization must not duplicate the PM occurrence already in the batch"
    );
    let delivery = tick.into_books().into_iter().next().unwrap();
    assert!(run.artifact_terminal());
    assert_eq!(
        run.terminal_tick_cleanup_status(),
        reap_pm_live::PmPublicTerminalTickCleanupStatus::Pending
    );
    assert_eq!(
        run.terminal_okx_unavailable()
            .unwrap()
            .envelope()
            .payload()
            .fault(),
        OkxPublicSessionFault::InvalidTransition
    );
    let okx_clock = run
        .terminal_okx_unavailable()
        .unwrap()
        .envelope()
        .received_clock();
    assert_eq!(okx_clock.local_wall_receive_ns(), 1_700_000_000_000_000_110);
    assert_eq!(okx_clock.monotonic_receive_ns(), 110);

    assert!(matches!(
        run.record_freshness_timer(111).await,
        Err(PmPublicCaptureRunError::ArtifactTerminal {
            cause: PmPublicCaptureTerminalCause::TickSizeChanged
        })
    ));
    assert!(matches!(
        run.capture_pm_public(1_700_000_000_000_000_112, 112, bbo().as_bytes())
            .await,
        Err(PmPublicCaptureRunError::ArtifactTerminal {
            cause: PmPublicCaptureTerminalCause::TickSizeChanged
        })
    ));

    let reason = run.apply_terminal_tick_invalidation(delivery).unwrap();
    assert_eq!(
        reason,
        PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::Grid)
    );
    assert_eq!(run.pm_book_counters().tick_size_changes, 1);
    assert_eq!(
        run.terminal_tick_cleanup_status(),
        reap_pm_live::PmPublicTerminalTickCleanupStatus::Applied
    );
    assert_eq!(run.pm_book_counters().metadata_rejected, 1);
    assert_eq!(
        run.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::ArtifactTerminal)
    );
    assert_eq!(
        run.terminal_cause(),
        Some(PmPublicCaptureTerminalCause::TickSizeChanged)
    );
    assert!(matches!(
        run.finish().await,
        Err(PmPublicCaptureRunError::TerminalFinish {
            cause: PmPublicCaptureTerminalCause::TickSizeChanged,
            shutdown_error: None,
        })
    ));
}

#[tokio::test]
async fn dropped_terminal_tick_is_reported_as_incomplete_product_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_capture_run(directory.path().join("tick-cleanup-dropped.jsonl")).await;
    run.record_pm_connection_started(60).await.unwrap();
    run.record_pm_subscription_sent(80).await.unwrap();
    let mut snapshot = run
        .capture_pm_public(1_700_000_000_000_000_100, 100, snapshot_one().as_bytes())
        .await
        .unwrap();
    let flow = snapshot.take_snapshot_flow().unwrap();
    let delivery = snapshot.into_books().into_iter().next().unwrap();
    run.commit_then_enqueue_pm_snapshot(delivery, flow).unwrap();

    let mut tick = run
        .capture_pm_public(1_700_000_000_000_000_110, 110, tick_change().as_bytes())
        .await
        .unwrap();
    let _ = tick.take_unavailable().unwrap();
    drop(tick.into_books());
    assert_eq!(
        run.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::ArtifactTerminal)
    );
    assert!(run.ready_pm_book_view().is_none());
    let error = run.finish().await.unwrap_err();
    assert!(matches!(
        &error,
        PmPublicCaptureRunError::TerminalTickCleanupIncomplete {
            cleanup_status: reap_pm_live::PmPublicTerminalTickCleanupStatus::Pending,
            ..
        }
    ));
    assert_eq!(
        error.terminal_cause(),
        Some(PmPublicCaptureTerminalCause::TickSizeChanged)
    );
}

#[tokio::test]
async fn stale_freshness_timer_is_typed_replayable_and_same_epoch_snapshot_resyncs() {
    let directory = tempfile::tempdir().unwrap();
    let mut run = start_capture_run(directory.path().join("freshness-stale.jsonl")).await;
    run.record_pm_connection_started(60).await.unwrap();
    run.record_pm_subscription_sent(80).await.unwrap();
    let mut snapshot = run
        .capture_pm_public(1_700_000_000_000_000_100, 100, snapshot_one().as_bytes())
        .await
        .unwrap();
    let flow = snapshot.take_snapshot_flow().unwrap();
    let delivery = snapshot.into_books().into_iter().next().unwrap();
    run.commit_then_enqueue_pm_snapshot(delivery, flow).unwrap();

    let outcome = run.record_freshness_timer(1_101).await.unwrap();
    assert_eq!(
        outcome.unavailable_reason(),
        Some(PmPublicReadinessReason::BookStale)
    );
    assert_eq!(
        run.pm_book_readiness().reason(),
        Some(PmPublicBookReadinessReason::Reducer(
            PmPublicReadinessReason::BookStale
        ))
    );
    assert_eq!(run.pm_book_counters().freshness_checks, 1);
    assert_eq!(run.pm_book_counters().stale_invalidations, 1);
    assert_eq!(run.pm_book_counters().external_faults, 0);
    assert_eq!(run.pm_book_counters().backlog_aged_faults, 0);
    assert!(!run.artifact_terminal());

    let mut snapshot = run
        .capture_pm_public(1_700_000_000_000_001_102, 1_102, snapshot_two().as_bytes())
        .await
        .unwrap();
    let flow = snapshot.take_snapshot_flow().unwrap();
    let delivery = snapshot.into_books().into_iter().next().unwrap();
    run.commit_then_enqueue_pm_snapshot(delivery, flow).unwrap();
    service_all_public(&mut run, 1_102);
    let ready = run.ready_pm_book_view().unwrap();
    assert!(ready.readiness().is_ready());
    assert_eq!(ready.connection_epoch(), ConnectionEpoch::new(11));

    let outcome = run.finish().await.unwrap();
    assert!(outcome.projection().logical_events().iter().any(|event| {
        matches!(
            event,
            PmReplayLogicalEvent::FreshnessInvalidated {
                reason: reap_pm_live::PmReplayFreshnessInvalidation::BookStale,
                ..
            }
        )
    }));
    assert_eq!(outcome.projection().counters().freshness_checks, 1);
    assert_eq!(outcome.projection().counters().stale_invalidations, 1);
    assert_eq!(outcome.projection().counters().external_faults, 0);
    assert_eq!(outcome.projection().counters().backlog_aged_faults, 0);
}

#[tokio::test]
async fn replay_authenticates_heartbeat_timeout_against_ping_and_deadline_state() {
    let directory = tempfile::tempdir().unwrap();

    let no_ping_source = directory.path().join("no-ping-source.jsonl");
    write_explicit_disconnect_capture(no_ping_source.clone(), None, 110).await;
    let no_ping = tamper_disconnect_reason_to_heartbeat_timeout(
        &no_ping_source,
        directory.path().join("no-ping-timeout.jsonl"),
    );
    verify_pm_public_capture(&no_ping, &capture_header()).unwrap();
    assert!(matches!(
        replay_pm_public_capture(&no_ping, &capture_header()),
        Err(PmReplayError::LifecycleMismatch)
    ));

    let early_source = directory.path().join("early-source.jsonl");
    write_explicit_disconnect_capture(early_source.clone(), Some(110), 114).await;
    let early = tamper_disconnect_reason_to_heartbeat_timeout(
        &early_source,
        directory.path().join("early-timeout.jsonl"),
    );
    verify_pm_public_capture(&early, &capture_header()).unwrap();
    assert!(matches!(
        replay_pm_public_capture(&early, &capture_header()),
        Err(PmReplayError::LifecycleMismatch)
    ));

    let positive_path = directory.path().join("authenticated-timeout.jsonl");
    let mut run = start_capture_run(positive_path).await;
    run.record_pm_connection_started(60).await.unwrap();
    run.record_pm_subscription_sent(100).await.unwrap();
    run.record_pm_heartbeat_ping_sent(1_700_000_000_000_000_110, 110)
        .await
        .unwrap();
    let error = run
        .record_pm_heartbeat_ping_sent(1_700_000_000_000_000_115, 115)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PmPublicCaptureRunError::PmHeartbeat {
            source: PmPublicSessionError::HeartbeatTimeout { deadline_ns: 115 },
            ..
        }
    ));
    service_all_public(&mut run, 115);
    let outcome = run.finish().await.unwrap();
    assert_eq!(outcome.projection().counters().heartbeat_timeouts, 1);
    assert_eq!(outcome.projection().counters().external_faults, 1);
}

#[tokio::test]
async fn verifier_rejects_policy_tampering_ordering_corruption_and_partial_tail() {
    let directory = tempfile::tempdir().unwrap();
    let mut invalid_frame_run =
        start_capture_run(directory.path().join("invalid-frame.jsonl")).await;
    invalid_frame_run
        .record_pm_connection_started(60)
        .await
        .unwrap();
    invalid_frame_run
        .record_pm_subscription_sent(100)
        .await
        .unwrap();
    let error = invalid_frame_run
        .capture_pm_public(1_700_000_000_000_000_101, 101, &[])
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        PmPublicCaptureRunError::PmCaptureRejected {
            source: PmCaptureWriteError::Contract(PmCaptureVerifyError::InvalidRawFrame(_)),
            ..
        }
    ));
    let unavailable = error.pm_unavailable().unwrap().envelope();
    assert_eq!(
        unavailable.payload().fault(),
        PmPublicSessionFault::InvalidTransition
    );
    assert_eq!(unavailable.received_clock().monotonic_receive_ns(), 101);
    assert!(matches!(
        invalid_frame_run.finish().await,
        Err(PmPublicCaptureRunError::TerminalFinish { .. })
    ));

    let valid_path = directory.path().join("valid.jsonl");
    let mut run = start_capture_run(valid_path.clone()).await;
    run.record_okx_connection_started(60).await.unwrap();
    run.record_okx_subscription_sent(90).await.unwrap();
    run.capture_okx_public(1_700_000_000_000_000_100, 100, okx_ack().as_bytes())
        .await
        .unwrap();
    run.finish().await.unwrap();
    let original = std::fs::read_to_string(&valid_path).unwrap();

    let records = original
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_invalid_tamper(directory.path(), "semantic-policy", &records, |values| {
        let header = &mut values[0]["header"];
        header["session_policy"]["pm_initial_epoch"] = serde_json::json!(0);
        let scope: PmCaptureScope = serde_json::from_value(header["scope"].clone()).unwrap();
        let policy: PmCaptureSessionPolicy =
            serde_json::from_value(header["session_policy"].clone()).unwrap();
        header["configuration_sha256"] =
            serde_json::Value::String(sha256_hex(&serde_json::to_vec(&(&scope, policy)).unwrap()));
    });
    assert_invalid_tamper(directory.path(), "raw-hash", &records, |values| {
        values[3]["frame"]["raw_hash"] = serde_json::json!(1);
    });
    assert_invalid_tamper(directory.path(), "raw-base64", &records, |values| {
        values[3]["frame"]["raw_base64"] = serde_json::json!("AA==");
    });
    assert_invalid_tamper(directory.path(), "raw-length", &records, |values| {
        values[3]["frame"]["raw_length"] = serde_json::json!(1);
    });
    assert_invalid_tamper(directory.path(), "raw-sha256", &records, |values| {
        values[3]["frame"]["raw_sha256"] = serde_json::json!("0".repeat(64));
    });
    assert_invalid_tamper(directory.path(), "ingress", &records, |values| {
        values[3]["frame"]["local_ingress_sequence"] = serde_json::json!(2);
    });
    assert_invalid_tamper(directory.path(), "epoch", &records, |values| {
        values[3]["frame"]["connection_epoch"] = serde_json::json!(22);
    });
    assert_invalid_tamper(directory.path(), "monotonic", &records, |values| {
        values[3]["frame"]["monotonic_receive_ns"] = serde_json::json!(80);
    });
    assert_invalid_tamper(directory.path(), "lifecycle-epoch", &records, |values| {
        values[2]["connection_epoch"] = serde_json::json!(22);
    });
    assert_invalid_tamper(directory.path(), "sequence", &records, |values| {
        values[3]["sequence"] = serde_json::json!(5);
    });
    assert_invalid_tamper(directory.path(), "provenance", &records, |values| {
        values[0]["header"]["provenance"]["fixture_sha256"] = serde_json::json!("1".repeat(64));
    });

    let lifecycle_path = directory.path().join("lifecycle-tamper-source.jsonl");
    write_lifecycle_tamper_capture(lifecycle_path.clone()).await;
    let lifecycle_records = std::fs::read_to_string(lifecycle_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_invalid_tamper(
        directory.path(),
        "pm-disconnected-nested-field",
        &lifecycle_records,
        |values| {
            values[5]["event"]["disconnected"]["unexpected"] = serde_json::json!(true);
        },
    );
    assert_invalid_tamper(
        directory.path(),
        "okx-disconnected-nested-field",
        &lifecycle_records,
        |values| {
            values[6]["event"]["disconnected"]["unexpected"] = serde_json::json!(true);
        },
    );
    assert_invalid_tamper(
        directory.path(),
        "pm-reconnect-nested-field",
        &lifecycle_records,
        |values| {
            values[7]["event"]["reconnect_scheduled"]["unexpected"] = serde_json::json!(true);
        },
    );
    assert_invalid_tamper(
        directory.path(),
        "okx-reconnect-nested-field",
        &lifecycle_records,
        |values| {
            values[8]["event"]["reconnect_scheduled"]["unexpected"] = serde_json::json!(true);
        },
    );

    let partial_path = directory.path().join("partial.jsonl");
    std::fs::write(&partial_path, original.trim_end_matches('\n')).unwrap();
    assert!(matches!(
        verify_pm_public_capture(&partial_path, &capture_header()),
        Err(PmCaptureVerifyError::TrailingPartialRecord)
    ));

    let oversized_path = directory.path().join("encoded-over-limit.jsonl");
    let oversized = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&oversized_path)
        .unwrap();
    oversized
        .set_len(MAX_PM_PUBLIC_CAPTURE_ENCODED_BYTES + 1)
        .unwrap();
    assert!(matches!(
        verify_pm_public_capture(&oversized_path, &capture_header()),
        Err(PmCaptureVerifyError::CaptureTooLarge)
    ));
}

async fn write_explicit_disconnect_capture(
    path: PathBuf,
    ping_monotonic_ns: Option<u64>,
    disconnect_monotonic_ns: u64,
) {
    let mut run = start_capture_run(path).await;
    run.record_pm_connection_started(60).await.unwrap();
    run.record_pm_subscription_sent(100).await.unwrap();
    if let Some(ping_monotonic_ns) = ping_monotonic_ns {
        run.record_pm_heartbeat_ping_sent(
            1_700_000_000_000_000_000 + ping_monotonic_ns,
            ping_monotonic_ns,
        )
        .await
        .unwrap();
    }
    let _ = run
        .record_pm_disconnected(
            1_700_000_000_000_000_000 + disconnect_monotonic_ns,
            disconnect_monotonic_ns,
        )
        .await
        .unwrap();
    service_all_public(&mut run, disconnect_monotonic_ns);
    run.finish().await.unwrap();
}

fn tamper_disconnect_reason_to_heartbeat_timeout(source: &Path, target: PathBuf) -> PathBuf {
    let mut records = std::fs::read_to_string(source)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    let disconnected = records
        .iter_mut()
        .find(|record| record["event"].get("disconnected").is_some())
        .expect("source artifact carries one explicit disconnect");
    disconnected["event"]["disconnected"]["reason"] = serde_json::json!("heartbeat_timeout");
    let bytes = records
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n")
        + "\n";
    std::fs::write(&target, bytes).unwrap();
    target
}

async fn write_lifecycle_tamper_capture(path: PathBuf) {
    let mut run = start_capture_run(path).await;
    run.record_pm_connection_started(60).await.unwrap();
    run.record_okx_connection_started(70).await.unwrap();
    run.record_pm_subscription_sent(80).await.unwrap();
    run.record_okx_subscription_sent(90).await.unwrap();
    let _ = run
        .record_pm_disconnected(1_700_000_000_000_000_100, 100)
        .await
        .unwrap();
    let _ = run
        .record_okx_disconnected(1_700_000_000_000_000_101, 101)
        .await
        .unwrap();
    service_all_public(&mut run, 101);
    run.record_pm_reconnect_scheduled(102).await.unwrap();
    run.record_okx_reconnect_scheduled(103).await.unwrap();
    run.finish().await.unwrap();
}

async fn run_fake_capture(path: PathBuf) {
    let mut run = start_capture_run(path).await;
    let subscription = run.pm_subscription_bytes().to_vec();
    let (address, server) = spawn_fake_server(subscription.clone()).await;

    run.record_pm_connection_started(60).await.unwrap();
    let (mut socket, _) = connect_async(format!("ws://{address}")).await.unwrap();
    run.record_pm_subscription_sent(100).await.unwrap();
    socket
        .send(Message::Binary(subscription.clone().into()))
        .await
        .unwrap();
    run.record_pm_heartbeat_ping_sent(1_700_000_000_000_000_110, 110)
        .await
        .unwrap();
    socket
        .send(Message::Binary(PM_PUBLIC_PING_BYTES.to_vec().into()))
        .await
        .unwrap();

    let pong = receive_application_bytes(&mut socket).await;
    let pong_batch = run
        .capture_pm_public(1_700_000_000_000_000_111, 111, &pong)
        .await
        .unwrap();
    assert!(pong_batch.into_books().is_empty());

    let snapshot = receive_application_bytes(&mut socket).await;
    let mut snapshot_batch = run
        .capture_pm_public(1_700_000_000_000_000_120, 120, &snapshot)
        .await
        .unwrap();
    let snapshot_flow = snapshot_batch.take_snapshot_flow().unwrap();
    let snapshot_delivery = snapshot_batch.into_books().into_iter().next().unwrap();
    run.commit_then_enqueue_pm_snapshot(snapshot_delivery, snapshot_flow)
        .unwrap();

    let delta = receive_application_bytes(&mut socket).await;
    let delta_batch = run
        .capture_pm_public(1_700_000_000_000_000_130, 130, &delta)
        .await
        .unwrap();
    let delta_delivery = delta_batch.into_books().into_iter().next().unwrap();
    assert!(matches!(
        run.reduce_then_enqueue_pm_book(delta_delivery).unwrap(),
        PmBookTransition::DeltaBatchCommitted { changes: 2, .. }
    ));

    let top = receive_application_bytes(&mut socket).await;
    let top_batch = run
        .capture_pm_public(1_700_000_000_000_000_140, 140, &top)
        .await
        .unwrap();
    let top_delivery = top_batch.into_books().into_iter().next().unwrap();
    assert_eq!(
        run.reduce_then_enqueue_pm_book(top_delivery).unwrap(),
        PmBookTransition::TopConfirmed
    );
    expect_close(&mut socket).await;

    let _ = run
        .record_pm_disconnected(1_700_000_000_000_000_150, 150)
        .await
        .unwrap();
    service_all_public(&mut run, 150);
    run.record_pm_reconnect_scheduled(151).await.unwrap();
    run.record_pm_connection_started(160).await.unwrap();

    let (mut socket, _) = connect_async(format!("ws://{address}")).await.unwrap();
    run.record_pm_subscription_sent(170).await.unwrap();
    socket
        .send(Message::Binary(subscription.into()))
        .await
        .unwrap();
    let raw = receive_application_bytes(&mut socket).await;
    let mut snapshot_batch = run
        .capture_pm_public(1_700_000_000_000_000_180, 180, &raw)
        .await
        .unwrap();
    let snapshot_flow = snapshot_batch.take_snapshot_flow().unwrap();
    let snapshot_delivery = snapshot_batch.into_books().into_iter().next().unwrap();
    run.commit_then_enqueue_pm_snapshot(snapshot_delivery, snapshot_flow)
        .unwrap();
    expect_close(&mut socket).await;
    run.record_freshness_timer(190).await.unwrap();
    service_all_public(&mut run, 190);
    let outcome = run.finish().await.unwrap();
    assert_eq!(outcome.projection().counters().snapshots_committed, 2);
    server.await.unwrap();
}

async fn write_tick_capture(path: PathBuf, resume_after_tick: bool) {
    let mut run = start_capture_run(path).await;
    run.record_pm_connection_started(60).await.unwrap();
    run.record_pm_subscription_sent(100).await.unwrap();
    let mut snapshot = run
        .capture_pm_public(1_700_000_000_000_000_110, 110, snapshot_one().as_bytes())
        .await
        .unwrap();
    let flow = snapshot.take_snapshot_flow().unwrap();
    let delivery = snapshot.into_books().into_iter().next().unwrap();
    run.commit_then_enqueue_pm_snapshot(delivery, flow).unwrap();
    let mut tick = run
        .capture_pm_public(1_700_000_000_000_000_120, 120, tick_change().as_bytes())
        .await
        .unwrap();
    assert_eq!(
        tick.take_unavailable()
            .unwrap()
            .envelope()
            .payload()
            .fault(),
        PmPublicSessionFault::TickSizeChanged
    );
    let delivery = tick.into_books().into_iter().next().unwrap();
    assert_eq!(
        run.apply_terminal_tick_invalidation(delivery).unwrap(),
        PmPublicReadinessReason::MetadataDrift(PmMetadataDrift::Grid)
    );
    if resume_after_tick {
        assert!(matches!(
            run.capture_pm_public(1_700_000_000_000_000_130, 130, bbo().as_bytes())
                .await,
            Err(PmPublicCaptureRunError::ArtifactTerminal {
                cause: PmPublicCaptureTerminalCause::TickSizeChanged,
            })
        ));
    }
    assert!(matches!(
        run.finish().await,
        Err(PmPublicCaptureRunError::TerminalFinish {
            cause: PmPublicCaptureTerminalCause::TickSizeChanged,
            shutdown_error: None,
        })
    ));
}

async fn write_combined_capture(path: PathBuf) {
    let mut run = start_capture_run(path).await;
    run.record_pm_connection_started(60).await.unwrap();
    run.record_okx_connection_started(70).await.unwrap();
    run.record_pm_subscription_sent(90).await.unwrap();
    run.record_okx_subscription_sent(100).await.unwrap();
    run.capture_okx_public(1_700_000_000_000_000_105, 105, okx_ack().as_bytes())
        .await
        .unwrap();
    let mut snapshot = run
        .capture_pm_public(1_700_000_000_000_000_110, 110, snapshot_one().as_bytes())
        .await
        .unwrap();
    let flow = snapshot.take_snapshot_flow().unwrap();
    let delivery = snapshot.into_books().into_iter().next().unwrap();
    run.commit_then_enqueue_pm_snapshot(delivery, flow).unwrap();
    run.capture_okx_public(1_700_000_000_000_000_115, 115, okx_reference().as_bytes())
        .await
        .unwrap();
    let _ = run
        .record_okx_disconnected(1_700_000_000_000_000_116, 116)
        .await
        .unwrap();
    service_all_public(&mut run, 116);
    run.record_okx_reconnect_scheduled(117).await.unwrap();
    run.record_okx_connection_started(118).await.unwrap();
    run.record_okx_subscription_sent(119).await.unwrap();
    run.capture_okx_public(1_700_000_000_000_000_120, 120, okx_ack().as_bytes())
        .await
        .unwrap();
    run.capture_okx_public(1_700_000_000_000_000_121, 121, okx_reference().as_bytes())
        .await
        .unwrap();
    let delta = run
        .capture_pm_public(1_700_000_000_000_000_125, 125, delta().as_bytes())
        .await
        .unwrap()
        .into_books()
        .into_iter()
        .next()
        .unwrap();
    assert!(matches!(
        run.reduce_then_enqueue_pm_book(delta).unwrap(),
        PmBookTransition::DeltaBatchCommitted { changes: 2, .. }
    ));
    let top = run
        .capture_pm_public(1_700_000_000_000_000_126, 126, bbo().as_bytes())
        .await
        .unwrap()
        .into_books()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(
        run.reduce_then_enqueue_pm_book(top).unwrap(),
        PmBookTransition::TopConfirmed
    );
    let delta_two = run
        .capture_pm_public(1_700_000_000_000_000_127, 127, delta_two().as_bytes())
        .await
        .unwrap()
        .into_books()
        .into_iter()
        .next()
        .unwrap();
    assert!(matches!(
        run.reduce_then_enqueue_pm_book(delta_two).unwrap(),
        PmBookTransition::DeltaBatchCommitted { changes: 2, .. }
    ));
    let _ = run
        .record_pm_disconnected(1_700_000_000_000_000_130, 130)
        .await
        .unwrap();
    service_all_public(&mut run, 130);
    run.record_pm_reconnect_scheduled(131).await.unwrap();
    run.record_pm_connection_started(140).await.unwrap();
    run.record_pm_subscription_sent(150).await.unwrap();
    let mut snapshot = run
        .capture_pm_public(1_700_000_000_000_000_160, 160, snapshot_two().as_bytes())
        .await
        .unwrap();
    let flow = snapshot.take_snapshot_flow().unwrap();
    let delivery = snapshot.into_books().into_iter().next().unwrap();
    run.commit_then_enqueue_pm_snapshot(delivery, flow).unwrap();
    run.record_freshness_timer(170).await.unwrap();
    service_all_public(&mut run, 170);
    run.finish().await.unwrap();
}

async fn spawn_fake_server(
    subscription: Vec<u8>,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut first = accept_async(stream).await.unwrap();
        assert_eq!(receive_application_bytes(&mut first).await, subscription);
        assert_eq!(
            receive_application_bytes(&mut first).await,
            PM_PUBLIC_PING_BYTES
        );
        send_bytes(&mut first, PM_PUBLIC_PONG_BYTES).await;
        send_text(&mut first, snapshot_one()).await;
        send_text(&mut first, delta()).await;
        send_text(&mut first, bbo()).await;
        first.close(None).await.unwrap();

        let (stream, _) = listener.accept().await.unwrap();
        let mut second = accept_async(stream).await.unwrap();
        assert_eq!(receive_application_bytes(&mut second).await, subscription);
        send_text(&mut second, snapshot_two()).await;
        second.close(None).await.unwrap();
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
            Message::Close(_) => panic!("socket closed before expected application frame"),
            Message::Frame(_) => unreachable!("raw frame is not exposed"),
        }
    }
}

async fn expect_close<S>(socket: &mut WebSocketStream<S>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    while let Some(message) = socket.next().await {
        if matches!(message.unwrap(), Message::Close(_)) {
            return;
        }
    }
}

async fn send_bytes(socket: &mut WebSocketStream<TcpStream>, value: &[u8]) {
    socket
        .send(Message::Binary(value.to_vec().into()))
        .await
        .unwrap();
}

async fn send_text(socket: &mut WebSocketStream<TcpStream>, value: String) {
    socket.send(Message::Text(value.into())).await.unwrap();
}

async fn start_capture_run(path: PathBuf) -> reap_pm_live::PmPublicCaptureRun {
    PmPublicCapture::new(public_config())
        .unwrap()
        .start(path, authoritative(), session_policy(), provenance())
        .await
        .unwrap()
}

fn position(
    events: &[PmReplayLogicalEvent],
    predicate: impl Fn(&PmReplayLogicalEvent) -> bool,
) -> usize {
    events.iter().position(predicate).unwrap()
}

fn assert_invalid_tamper(
    directory: &Path,
    name: &str,
    original: &[serde_json::Value],
    mutate: impl FnOnce(&mut [serde_json::Value]),
) {
    let mut tampered = original.to_vec();
    mutate(&mut tampered);
    let bytes = tampered
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n")
        + "\n";
    let path = directory.join(format!("tampered-{name}.jsonl"));
    std::fs::write(&path, bytes).unwrap();
    assert!(
        matches!(
            verify_pm_public_capture(&path, &capture_header()),
            Err(PmCaptureVerifyError::InvalidRecords)
        ),
        "{name} tamper was accepted"
    );
}
