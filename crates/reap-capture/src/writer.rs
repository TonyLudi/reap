use std::path::PathBuf;
use std::time::Duration;

use reap_capture_framing::{
    AdditionalWriterError, JsonlWriterError, LegacyEntryCountJsonlWriter,
    LegacyEntryCountWriterConfig,
};
use serde::Serialize;

use crate::error::{CaptureError, combine_capture_lifecycle_errors};

const WRITER_ENQUEUE_TIMEOUT: Duration = Duration::from_secs(1);
const WRITER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
const WRITER_ABORT_TIMEOUT: Duration = Duration::from_secs(1);
const WRITER_EVIDENCE_SCAN_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) type JsonlWriterStats = reap_capture_framing::JsonlWriterStats;

pub(super) struct JsonlWriter<T> {
    inner: LegacyEntryCountJsonlWriter<T>,
}

pub(super) struct JsonlWriterShutdown {
    pub(super) stats: JsonlWriterStats,
    pub(super) failure: Option<CaptureError>,
}

impl<T> JsonlWriter<T>
where
    T: Serialize + Send + 'static,
{
    pub(super) async fn start(
        name: &'static str,
        path: PathBuf,
        capacity: usize,
        flush_every_records: usize,
        fsync_every_records: usize,
    ) -> Result<Self, CaptureError> {
        // Existing Chaos capture has no historical byte ceiling. Keep its
        // typed values and serialization on the writer task through the
        // explicitly named compatibility path; new capture code must use the
        // byte-bounded framing API directly.
        let config = LegacyEntryCountWriterConfig {
            capacity,
            flush_every_records,
            fsync_every_records,
            enqueue_timeout: WRITER_ENQUEUE_TIMEOUT,
            shutdown_timeout: WRITER_SHUTDOWN_TIMEOUT,
            abort_timeout: WRITER_ABORT_TIMEOUT,
            evidence_scan_timeout: WRITER_EVIDENCE_SCAN_TIMEOUT,
        };
        let inner = LegacyEntryCountJsonlWriter::start(name, path, config)
            .await
            .map_err(map_writer_error)?;
        Ok(Self { inner })
    }

    pub(super) async fn send(&self, value: T) -> Result<(), CaptureError> {
        self.inner.send(value).await.map_err(map_writer_error)
    }

    pub(super) async fn shutdown_with_evidence(self) -> Result<JsonlWriterShutdown, CaptureError> {
        let outcome = self
            .inner
            .shutdown_with_evidence()
            .await
            .map_err(map_writer_error)?;
        Ok(JsonlWriterShutdown {
            stats: outcome.stats,
            failure: outcome.failure.map(map_writer_error),
        })
    }
}

fn map_writer_error(error: JsonlWriterError) -> CaptureError {
    match error {
        JsonlWriterError::OpenOutput { name, path, source } => {
            CaptureError::OpenOutput { name, path, source }
        }
        JsonlWriterError::Io(error) => CaptureError::Io(error),
        JsonlWriterError::Serialization(error) => CaptureError::Serialization(error),
        JsonlWriterError::Closed(name) => CaptureError::WriterClosed(name),
        JsonlWriterError::Backpressure { name, timeout_ms } => {
            CaptureError::WriterBackpressure { name, timeout_ms }
        }
        JsonlWriterError::Join(error) => CaptureError::WriterJoin(error),
        JsonlWriterError::ShutdownTimeout { name, timeout_ms } => {
            CaptureError::WriterShutdownTimeout { name, timeout_ms }
        }
        JsonlWriterError::AbortTimeout { name, timeout_ms } => {
            CaptureError::WriterAbortTimeout { name, timeout_ms }
        }
        JsonlWriterError::EvidenceTimeout { name, timeout_ms } => {
            CaptureError::WriterEvidenceTimeout { name, timeout_ms }
        }
        JsonlWriterError::Lifecycle {
            primary,
            additional,
        } => combine_capture_lifecycle_errors(
            map_writer_error(*primary),
            additional
                .into_iter()
                .map(|AdditionalWriterError { label, error }| (label, map_writer_error(*error)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use reap_core::{Level, MarketEvent, NormalizedEvent, OrderBook};

    use super::*;
    use crate::hashing::sha256_hex;

    #[tokio::test]
    async fn normalized_writer_emits_backtest_compatible_jsonl() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("normalized.jsonl");
        let writer = JsonlWriter::start("test", path.clone(), 4, 1, 0)
            .await
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let event = NormalizedEvent::from(MarketEvent::Depth(OrderBook {
            symbol: "BTC-USDT".to_string(),
            ts_ms: 1,
            bids: vec![Level::new(100.0, 1.0)],
            asks: vec![Level::new(101.0, 1.0)],
        }));
        let mut expected = serde_json::to_vec(&event).unwrap();
        expected.push(b'\n');
        writer.send(event).await.unwrap();
        let outcome = writer.shutdown_with_evidence().await.unwrap();
        assert!(outcome.failure.is_none());
        let stats = outcome.stats;
        assert_eq!(stats.records, 1);
        assert_eq!(stats.sha256.len(), 64);

        let bytes = std::fs::read(path).unwrap();
        assert_eq!(bytes, expected);
        let text = std::str::from_utf8(&bytes).unwrap();
        let decoded: NormalizedEvent = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(decoded.ts_ms(), 1);
        assert_eq!(stats.sha256, sha256_hex(text.as_bytes()));
    }

    #[test]
    fn writer_enqueue_fails_with_bounded_backpressure_evidence() {
        let error = map_writer_error(JsonlWriterError::Backpressure {
            name: "test",
            timeout_ms: 10,
        });
        assert!(matches!(
            error,
            CaptureError::WriterBackpressure {
                name: "test",
                timeout_ms: 10,
            }
        ));
    }

    #[test]
    fn writer_shutdown_timeout_aborts_task_and_recovers_partial_file_stats() {
        let error = map_writer_error(JsonlWriterError::ShutdownTimeout {
            name: "test",
            timeout_ms: 10,
        });
        assert!(matches!(
            error,
            CaptureError::WriterShutdownTimeout {
                name: "test",
                timeout_ms: 10,
            }
        ));
    }

    #[test]
    fn writer_stats_scan_counts_a_trailing_partial_record() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("partial.jsonl");
        let partial = b"first\nsecond";
        std::fs::write(&path, partial).unwrap();

        let scan = reap_capture_framing::scan_jsonl_file_legacy_unbounded(&path, |_| true).unwrap();

        assert_eq!(scan.records, 2);
        assert_eq!(scan.bytes, partial.len() as u64);
        assert!(scan.has_trailing_partial_record);
        assert_eq!(scan.sha256, sha256_hex(partial));
    }
}
