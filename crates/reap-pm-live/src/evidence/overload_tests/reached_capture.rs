use crate::{
    MAX_PM_PUBLIC_CAPTURE_PENDING_AGE_NS, MAX_PM_PUBLIC_CAPTURE_RAW_PAYLOAD_BYTES,
    MAX_PM_RAW_PUBLIC_FRAME_BYTES, PmCaptureVerifyError, PmCaptureWriteError, PmLaneKind,
    PmLanePolicy, PmProductPublicAgedRetryReason, PmProductPublicIngress,
    PmProductPublicIngressOutcome, PmPublicAgedLaneFaultEnactment, PmPublicBookReadinessReason,
    PmPublicCaptureRunError, PmPublicCaptureTerminalCause, SaturationAction,
};
use reap_okx_public_source::OkxPublicSessionFault;
use reap_pm_state::PmPublicReadinessReason;
use reap_polymarket_adapter::PmPublicSessionFault;

const WALL_BASE: u64 = 1_700_000_000_000_000_000;
const ONE_MIB: usize = 1024 * 1024;

fn snapshot() -> String {
    super::super::fixture::snapshot_frame()
}

fn top() -> String {
    super::super::fixture::top_frame()
}

fn repeated_top(count: usize) -> Vec<u8> {
    format!(
        "[{}]",
        std::iter::repeat_n(top(), count)
            .collect::<Vec<_>>()
            .join(",")
    )
    .into_bytes()
}

fn padded_ignored_frame() -> Vec<u8> {
    let mut frame = br#"[{"event_type":"last_trade_price"}]"#.to_vec();
    frame.resize(ONE_MIB, b' ');
    frame
}

async fn start(
    case: &str,
) -> (
    tempfile::TempDir,
    crate::PmProductRun<super::super::fixture::Phase6Model>,
) {
    let directory = tempfile::tempdir().expect("temporary evidence directory");
    let run = super::super::start_reached_overload_product(
        directory.path().join(format!("{case}-capture.jsonl")),
        directory.path().join(format!("{case}-journal.jsonl")),
    )
    .await
    .expect("fixed reached product starts");
    (directory, run)
}

async fn open_pm_capture(run: &mut PmProductPublicIngress<'_>) {
    run.record_pm_connection_started(60)
        .await
        .expect("connection start");
    run.record_pm_subscription_sent(80)
        .await
        .expect("subscription");
}

async fn queue_aged_snapshot(
    product: &mut crate::PmProductRun<super::super::fixture::Phase6Model>,
) -> (crate::PmProductRunError, u64) {
    {
        let mut ingress = product.public_ingress();
        open_pm_capture(&mut ingress).await;
        let mut batch = ingress
            .capture_pm_public(WALL_BASE + 100, 100, snapshot().as_bytes())
            .await
            .expect("snapshot capture");
        let flow = batch.take_snapshot_flow().expect("snapshot flow");
        let delivery = batch
            .into_books()
            .into_iter()
            .next()
            .expect("snapshot delivery");
        assert!(matches!(
            ingress
                .commit_then_enqueue_pm_snapshot(delivery, flow)
                .await
                .expect("one valid public item"),
            PmProductPublicIngressOutcome::Enqueued(())
        ));
    }
    let drain_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while product
        .public_capture()
        .pending_capture_record_depth_for_evidence()
        != 0
    {
        assert!(
            tokio::time::Instant::now() < drain_deadline,
            "capture writer did not drain before isolated public-age evidence"
        );
        tokio::task::yield_now().await;
    }
    let maximum = PmLanePolicy::for_lane(PmLaneKind::Public)
        .maximum_age_ns()
        .expect("public age bound");
    let now = 100 + maximum + 1;
    let error = product
        .service_turn(now)
        .expect_err("public item is aged one nanosecond past the bound");
    (error, now)
}

#[test]
fn public_integrity_row_reaches_product_8193_times_and_invalidates_one_epoch() {
    super::run_product_test(|| async {
        let (_directory, mut product) = start("public-integrity").await;
        let mut receive_ns = 200;
        {
            let mut ingress = product.public_ingress();
            open_pm_capture(&mut ingress).await;

            let mut first = ingress
                .capture_pm_public(WALL_BASE + 100, 100, snapshot().as_bytes())
                .await
                .expect("snapshot capture");
            let flow = first.take_snapshot_flow().expect("snapshot flow");
            let delivery = first
                .into_books()
                .into_iter()
                .next()
                .expect("snapshot delivery");
            assert!(matches!(
                ingress
                    .commit_then_enqueue_pm_snapshot(delivery, flow)
                    .await
                    .expect("snapshot commit and admission"),
                PmProductPublicIngressOutcome::Enqueued(())
            ));

            let mut remaining = 8_191;
            while remaining != 0 {
                let count = remaining.min(64);
                let deliveries = ingress
                    .capture_pm_public(WALL_BASE + receive_ns, receive_ns, &repeated_top(count))
                    .await
                    .expect("top capture")
                    .into_books();
                assert_eq!(deliveries.len(), count);
                for delivery in deliveries {
                    assert!(matches!(
                        ingress
                            .reduce_then_enqueue_pm_book(delivery)
                            .await
                            .expect("first 8192 integrity-bearing inputs"),
                        PmProductPublicIngressOutcome::Enqueued(_)
                    ));
                }
                remaining -= count;
                receive_ns += 1;
            }
        }
        let run = product.public_capture();
        assert_eq!(run.public_lane_metrics().depth(), 8_192);
        assert_eq!(run.public_lane_metrics().high_water(), 8_192);

        let outcome = {
            let mut ingress = product.public_ingress();
            let rejected = ingress
                .capture_pm_public(WALL_BASE + receive_ns, receive_ns, &repeated_top(1))
                .await
                .expect("the 8193rd valid parser input reaches the lane")
                .into_books()
                .into_iter()
                .next()
                .expect("one rejected delivery");
            ingress
                .reduce_then_enqueue_pm_book(rejected)
                .await
                .expect("the facade atomically enacts the authenticated lane failure")
        };
        let PmProductPublicIngressOutcome::ResyncRequired(enacted) = outcome else {
            panic!("8193rd integrity input must invalidate and resync");
        };
        assert_eq!(
            enacted.action(),
            SaturationAction::InvalidateStreamAndResync
        );
        assert_eq!(enacted.purged_queued_deliveries(), 8_192);
        let run = product.public_capture();
        assert_eq!(run.public_lane_metrics().rejected_full(), 1);
        assert_eq!(run.pm_book_counters().external_faults, 1);
        assert_eq!(run.pm_book_counters().overflows, 1);
        assert_eq!(run.pm_book_counters().invalidations, 1);
        assert_eq!(run.public_lane_metrics().invalidated_purged(), 8_192);
        assert_eq!(run.public_lane_metrics().high_water(), 8_192);
        let _ = product.shutdown().await;
    });
}

#[test]
fn raw_entry_capacity_failure_invalidates_capture_and_resyncs_the_pm_stream_once() {
    super::run_product_test(|| async {
        let (_directory, mut product) = start("raw-entry-terminal").await;
        let error = {
            let mut ingress = product.public_ingress();
            open_pm_capture(&mut ingress).await;
            ingress.phase6_reject_pm_capture_write_for_evidence(
                PmCaptureVerifyError::TooManyRawFrames.into(),
                WALL_BASE + 1_000,
                1_000,
            )
        };
        assert!(matches!(
            &error,
            PmPublicCaptureRunError::PmCaptureRejected {
                source: PmCaptureWriteError::Contract(PmCaptureVerifyError::TooManyRawFrames),
                ..
            }
        ));
        assert_eq!(
            error
                .pm_unavailable()
                .map(|delivery| delivery.envelope().payload().fault()),
            Some(PmPublicSessionFault::Overflow)
        );
        let run = product.public_capture();
        assert_eq!(
            run.terminal_cause(),
            Some(PmPublicCaptureTerminalCause::CaptureWriter)
        );
        assert!(run.terminal_pm_unavailable().is_none());
        assert_eq!(
            run.terminal_okx_unavailable()
                .map(|delivery| delivery.envelope().payload().fault()),
            Some(OkxPublicSessionFault::Overflow)
        );
        assert_eq!(
            run.pm_book_readiness().reason(),
            Some(PmPublicBookReadinessReason::ArtifactTerminal)
        );

        let repeated = product
            .public_ingress()
            .capture_pm_public(WALL_BASE + 1_001, 1_001, top().as_bytes())
            .await
            .expect_err("a terminal capture cannot produce a second resync");
        assert!(matches!(
            &repeated,
            PmPublicCaptureRunError::ArtifactTerminal {
                cause: PmPublicCaptureTerminalCause::CaptureWriter
            }
        ));
        assert!(repeated.pm_unavailable().is_none());
        assert!(repeated.okx_unavailable().is_none());
        let _ = product.shutdown().await;
    });
}

#[test]
fn capture_age_failure_invalidates_capture_and_resyncs_the_pm_stream_once() {
    super::run_product_test(|| async {
        let (_directory, mut product) = start("capture-age-terminal").await;
        let error = {
            let mut ingress = product.public_ingress();
            open_pm_capture(&mut ingress).await;
            ingress.phase6_reject_pm_capture_write_for_evidence(
                PmCaptureWriteError::CaptureAged {
                    observed_age_ns: MAX_PM_PUBLIC_CAPTURE_PENDING_AGE_NS + 1,
                    maximum_age_ns: MAX_PM_PUBLIC_CAPTURE_PENDING_AGE_NS,
                },
                WALL_BASE + 1_000,
                1_000,
            )
        };
        assert!(matches!(
            &error,
            PmPublicCaptureRunError::PmCaptureRejected {
                source: PmCaptureWriteError::CaptureAged {
                    observed_age_ns,
                    maximum_age_ns,
                },
                ..
            } if *observed_age_ns == MAX_PM_PUBLIC_CAPTURE_PENDING_AGE_NS + 1
                && *maximum_age_ns == MAX_PM_PUBLIC_CAPTURE_PENDING_AGE_NS
        ));
        assert_eq!(
            error
                .pm_unavailable()
                .map(|delivery| delivery.envelope().payload().fault()),
            Some(PmPublicSessionFault::Stale)
        );
        let run = product.public_capture();
        assert_eq!(
            run.terminal_cause(),
            Some(PmPublicCaptureTerminalCause::CaptureWriter)
        );
        assert!(run.terminal_pm_unavailable().is_none());
        assert_eq!(
            run.terminal_okx_unavailable()
                .map(|delivery| delivery.envelope().payload().fault()),
            Some(OkxPublicSessionFault::Stale)
        );
        assert_eq!(
            run.pm_book_readiness().reason(),
            Some(PmPublicBookReadinessReason::ArtifactTerminal)
        );

        let repeated = product
            .public_ingress()
            .capture_pm_public(WALL_BASE + 1_001, 1_001, top().as_bytes())
            .await
            .expect_err("a terminal capture cannot produce a second resync");
        assert!(matches!(
            &repeated,
            PmPublicCaptureRunError::ArtifactTerminal {
                cause: PmPublicCaptureTerminalCause::CaptureWriter
            }
        ));
        assert!(repeated.pm_unavailable().is_none());
        assert!(repeated.okx_unavailable().is_none());
        let _ = product.shutdown().await;
    });
}

#[test]
fn raw_byte_row_reaches_product_33_times_and_terminally_closes_capture() {
    super::run_product_test(|| async {
        let (_directory, mut product) = start("raw-bytes").await;
        let frame = padded_ignored_frame();
        assert_eq!(frame.len(), MAX_PM_RAW_PUBLIC_FRAME_BYTES);
        let error = {
            let mut ingress = product.public_ingress();
            open_pm_capture(&mut ingress).await;
            for attempt in 1..=32_u64 {
                let batch = ingress
                    .capture_pm_public(WALL_BASE + 1_000 + attempt, 1_000 + attempt, &frame)
                    .await
                    .expect("first 32 one-MiB frames");
                assert_eq!(batch.ignored_public_trades(), 1);
            }
            ingress
                .capture_pm_public(WALL_BASE + 1_033, 1_033, &frame)
                .await
                .expect_err("33rd one-MiB frame exceeds the aggregate slab")
        };
        assert!(matches!(
            &error,
            PmPublicCaptureRunError::PmCaptureRejected {
                source: PmCaptureWriteError::Contract(PmCaptureVerifyError::RawPayloadTooLarge),
                ..
            }
        ));
        assert_eq!(
            error
                .pm_unavailable()
                .map(|delivery| delivery.envelope().payload().fault()),
            Some(PmPublicSessionFault::Overflow)
        );
        let run = product.public_capture();
        assert_eq!(run.accepted_pm_raw_frames_for_evidence(), 32);
        assert_eq!(
            run.terminal_cause(),
            Some(PmPublicCaptureTerminalCause::CaptureWriter)
        );
        assert!(run.terminal_pm_unavailable().is_none());
        assert_eq!(
            run.terminal_okx_unavailable()
                .map(|delivery| delivery.envelope().payload().fault()),
            Some(OkxPublicSessionFault::Overflow)
        );
        assert_eq!(
            run.pm_book_readiness().reason(),
            Some(PmPublicBookReadinessReason::ArtifactTerminal)
        );
        assert_eq!(
            32_u64 * u64::try_from(frame.len()).expect("bounded"),
            MAX_PM_PUBLIC_CAPTURE_RAW_PAYLOAD_BYTES
        );
        let _ = product.shutdown().await;
    });
}

#[test]
fn oversize_raw_row_reaches_product_once_and_accepts_zero_frames() {
    super::run_product_test(|| async {
        let (_directory, mut product) = start("raw-oversize").await;
        let frame = vec![b' '; MAX_PM_RAW_PUBLIC_FRAME_BYTES + 1];
        let error = {
            let mut ingress = product.public_ingress();
            open_pm_capture(&mut ingress).await;
            ingress
                .capture_pm_public(WALL_BASE + 1_000, 1_000, &frame)
                .await
                .expect_err("the only oversize attempt is rejected")
        };
        assert!(matches!(
            &error,
            PmPublicCaptureRunError::PmCaptureRejected {
                source: PmCaptureWriteError::Contract(PmCaptureVerifyError::RawFrameTooLarge),
                ..
            }
        ));
        assert_eq!(
            error
                .pm_unavailable()
                .map(|delivery| delivery.envelope().payload().fault()),
            Some(PmPublicSessionFault::Overflow)
        );
        let run = product.public_capture();
        assert_eq!(run.accepted_pm_raw_frames_for_evidence(), 0);
        assert_eq!(
            run.terminal_cause(),
            Some(PmPublicCaptureTerminalCause::CaptureWriter)
        );
        assert!(run.terminal_pm_unavailable().is_none());
        assert_eq!(
            run.terminal_okx_unavailable()
                .map(|delivery| delivery.envelope().payload().fault()),
            Some(OkxPublicSessionFault::Overflow)
        );
        assert_eq!(
            run.pm_book_readiness().reason(),
            Some(PmPublicBookReadinessReason::ArtifactTerminal)
        );
        let _ = product.shutdown().await;
    });
}

#[test]
fn public_age_at_one_nanosecond_past_bound_resyncs_without_global_halt() {
    super::run_product_test(|| async {
        let (_directory, mut product) = start("public-aged").await;
        let (error, now) = queue_aged_snapshot(&mut product).await;
        assert_eq!(
            error.saturation_action(),
            Some(SaturationAction::InvalidateStreamAndResync)
        );
        let enacted = product
            .enact_public_lane_aged(error, WALL_BASE + now, now)
            .await
            .expect("owning product enacts its exact aged failure");
        let PmPublicAgedLaneFaultEnactment::Polymarket {
            unavailable_fault,
            reducer_reason,
            purged_queued_deliveries,
        } = enacted
        else {
            panic!("PM snapshot age must enact PM resync");
        };
        assert_eq!(unavailable_fault, PmPublicSessionFault::Stale);
        assert_eq!(reducer_reason, PmPublicReadinessReason::BookStale);
        assert_eq!(purged_queued_deliveries, 1);
        assert_eq!(product.halt(), None);
        assert_eq!(product.mutation_halt(), None);
        let _ = product.shutdown().await;
    });
}

#[test]
fn sibling_product_cannot_consume_public_age_authority_and_owner_can_retry() {
    super::run_product_test(|| async {
        let (_owner_directory, mut owner) = start("public-aged-owner").await;
        let (_sibling_directory, mut sibling) = start("public-aged-sibling").await;
        let (failure, now) = queue_aged_snapshot(&mut owner).await;

        let rejected = sibling
            .enact_public_lane_aged(failure, WALL_BASE + now, now)
            .await
            .expect_err("sibling product cannot consume the move-only aged proof");
        assert_eq!(
            rejected.retry_reason(),
            Some(PmProductPublicAgedRetryReason::FailureNotOwnedOrStale)
        );
        let failure = rejected
            .into_run_failure()
            .expect("sibling rejection returns the exact run failure");
        let enacted = owner
            .enact_public_lane_aged(failure, WALL_BASE + now, now)
            .await
            .expect("the owning product can retry the returned proof");
        assert!(matches!(
            enacted,
            PmPublicAgedLaneFaultEnactment::Polymarket {
                purged_queued_deliveries: 1,
                ..
            }
        ));
        assert_eq!(owner.halt(), None);
        assert_eq!(owner.mutation_halt(), None);
        assert_eq!(sibling.halt(), None);
        assert_eq!(sibling.mutation_halt(), None);
        let _ = owner.shutdown().await;
        let _ = sibling.shutdown().await;
    });
}
