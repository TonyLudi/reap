use std::time::Duration;

use reap_capture_framing::{
    BoundedJsonlWriter, BoundedWriterConfig, ByteBoundedWriterError, sha256_hex,
};
use serde::Serialize;

#[derive(Serialize)]
struct Record {
    sequence: u64,
}

fn config() -> BoundedWriterConfig {
    BoundedWriterConfig {
        capacity: 2,
        max_frame_bytes: 1024 * 1024,
        max_queued_bytes: 32 * 1024 * 1024,
        flush_every_records: 1,
        fsync_every_records: 0,
        enqueue_timeout: Duration::from_secs(1),
        shutdown_timeout: Duration::from_secs(30),
        abort_timeout: Duration::from_secs(1),
        evidence_scan_timeout: Duration::from_secs(5),
    }
}

#[tokio::test]
async fn writer_is_create_new_private_bounded_and_byte_exact() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("capture.jsonl");
    let writer = BoundedJsonlWriter::start("raw", path.clone(), config())
        .await
        .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    writer.send(Record { sequence: 1 }).await.unwrap();
    writer.send(Record { sequence: 2 }).await.unwrap();
    let outcome = writer.shutdown_with_evidence().await.unwrap();
    assert!(outcome.failure.is_none());

    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes, b"{\"sequence\":1}\n{\"sequence\":2}\n");
    assert_eq!(outcome.stats.records, 2);
    assert_eq!(outcome.stats.bytes, bytes.len() as u64);
    assert_eq!(outcome.stats.sha256, sha256_hex(&bytes));
    assert!(outcome.stats.max_queue_depth <= 2);
    assert!(
        outcome
            .stats
            .max_queue_bytes
            .is_some_and(|bytes| bytes <= 32)
    );
    assert!(
        outcome
            .stats
            .max_reserved_bytes
            .is_some_and(|bytes| bytes <= 32 * 1024 * 1024)
    );

    let error = BoundedJsonlWriter::<Record>::start("raw", path, config())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ByteBoundedWriterError::Writer(reap_capture_framing::JsonlWriterError::OpenOutput { .. })
    ));
}

#[tokio::test]
async fn empty_writer_has_the_canonical_empty_hash() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("empty.jsonl");
    let writer = BoundedJsonlWriter::<Record>::start("raw", path, config())
        .await
        .unwrap();

    let outcome = writer.shutdown_with_evidence().await.unwrap();

    assert!(outcome.failure.is_none());
    assert_eq!(outcome.stats.records, 0);
    assert_eq!(outcome.stats.bytes, 0);
    assert_eq!(outcome.stats.max_queue_bytes, Some(0));
    assert_eq!(outcome.stats.max_reserved_bytes, Some(0));
    assert_eq!(outcome.stats.sha256, sha256_hex(&[]));
}

#[tokio::test]
async fn oversize_frame_fails_before_it_can_enter_the_queue() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("oversize.jsonl");
    let mut limits = config();
    limits.max_frame_bytes = 8;
    limits.max_queued_bytes = 32;
    let writer = BoundedJsonlWriter::<Record>::start("raw", path, limits)
        .await
        .unwrap();

    let error = writer.send(Record { sequence: 1 }).await.unwrap_err();

    assert!(matches!(
        error,
        ByteBoundedWriterError::FrameTooLarge {
            observed_at_least_bytes: 9,
            limit_bytes: 8,
            ..
        }
    ));
    assert_eq!(writer.queue_byte_evidence().current_bytes, 0);
    assert_eq!(writer.queue_byte_evidence().high_water_bytes, 0);
    writer.shutdown_with_evidence().await.unwrap();
}
