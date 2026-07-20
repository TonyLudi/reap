use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use reap_durable_writer::{
    DeliveryClass, DurableLease, DurableWriterConfig, EnqueueOutcome, JournalCodec, LeaseError,
    start_durable_writer, start_durable_writer_with_lease,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct Record {
    sequence: u64,
}

#[derive(Clone)]
struct JsonCodec {
    calls: Arc<AtomicUsize>,
}

impl JournalCodec<Record> for JsonCodec {
    type Error = serde_json::Error;

    fn encode(&self, record: &Record, output: &mut Vec<u8>) -> Result<(), Self::Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        serde_json::to_writer(output, record)
    }
}

#[tokio::test]
async fn static_codec_runs_on_writer_and_exact_lines_are_durable() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("journal.jsonl");
    let calls = Arc::new(AtomicUsize::new(0));
    let mut runtime = start_durable_writer(
        DurableWriterConfig {
            path: path.clone(),
            channel_capacity: 2,
            flush_every_records: 2,
        },
        JsonCodec {
            calls: Arc::clone(&calls),
        },
    )
    .await
    .unwrap();
    let sink = runtime.sink();
    assert_eq!(
        sink.enqueue(Record { sequence: 1 }, DeliveryClass::Critical)
            .await
            .unwrap(),
        EnqueueOutcome::Queued
    );
    sink.enqueue_durable(Record { sequence: 2 }).await.unwrap();
    let snapshot = sink.progress_snapshot();
    assert_eq!(snapshot.records_enqueued, 2);
    assert_eq!(snapshot.records_written, 2);
    assert_eq!(snapshot.durable_sync_completions, 1);
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    runtime.stop_writer().await.unwrap();

    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"{\"sequence\":1}\n{\"sequence\":2}\n"
    );
}

#[test]
fn lease_is_canonical_private_and_exclusive_with_legacy_lock_shape() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("journal.jsonl");
    let lease = DurableLease::acquire(&path).unwrap();
    assert!(lease.journal_path().is_absolute());
    assert_eq!(lease.lock_path().file_name().unwrap(), "journal.jsonl.lock");
    let lock = std::fs::read_to_string(lease.lock_path()).unwrap();
    assert!(lock.starts_with(&format!("pid={} acquired_at_ms=", std::process::id())));
    assert!(lock.ends_with('\n'));
    assert!(matches!(
        DurableLease::acquire(&path),
        Err(LeaseError::AlreadyLocked { .. })
    ));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(lease.lock_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

struct InfallibleCodec;

impl JournalCodec<u8> for InfallibleCodec {
    type Error = Infallible;

    fn encode(&self, record: &u8, output: &mut Vec<u8>) -> Result<(), Self::Error> {
        output.extend_from_slice(record.to_string().as_bytes());
        Ok(())
    }
}

#[tokio::test]
async fn supplied_lease_must_match_the_configured_journal() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.jsonl");
    let second = directory.path().join("second.jsonl");
    let lease = DurableLease::acquire(&first).unwrap();
    let error = start_durable_writer_with_lease::<u8, _>(
        DurableWriterConfig {
            path: second,
            channel_capacity: 1,
            flush_every_records: 1,
        },
        lease,
        InfallibleCodec,
    )
    .await
    .err()
    .expect("mismatched lease must fail");
    assert!(
        error
            .to_string()
            .contains("does not match configured journal")
    );
}
