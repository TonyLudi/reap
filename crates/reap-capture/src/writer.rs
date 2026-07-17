use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::error::{CaptureError, combine_capture_lifecycle_errors};
use crate::hashing::{digest_hex, sha256_hex};

const WRITER_ENQUEUE_TIMEOUT: Duration = Duration::from_secs(1);
const WRITER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
const WRITER_ABORT_TIMEOUT: Duration = Duration::from_secs(1);
const WRITER_EVIDENCE_SCAN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Default)]
pub(super) struct JsonlWriterStats {
    pub(super) records: u64,
    pub(super) bytes: u64,
    pub(super) max_queue_depth: usize,
    pub(super) sha256: String,
}

impl JsonlWriterStats {
    pub(super) fn empty() -> Self {
        Self {
            sha256: sha256_hex(&[]),
            ..Self::default()
        }
    }
}

pub(super) struct JsonlWriter<T> {
    name: &'static str,
    path: PathBuf,
    sender: Option<mpsc::Sender<T>>,
    task: JoinHandle<Result<String, CaptureError>>,
    queued: Arc<AtomicUsize>,
    max_queue_depth: Arc<AtomicUsize>,
    records: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
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
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options
            .open(&path)
            .await
            .map_err(|source| CaptureError::OpenOutput {
                name,
                path: path.clone(),
                source,
            })?;
        sync_parent_directory(&path).await?;
        let (sender, receiver) = mpsc::channel(capacity.max(1));
        let queued = Arc::new(AtomicUsize::new(0));
        let max_queue_depth = Arc::new(AtomicUsize::new(0));
        let records = Arc::new(AtomicU64::new(0));
        let bytes = Arc::new(AtomicU64::new(0));
        let task = tokio::spawn(run_jsonl_writer(
            file,
            receiver,
            flush_every_records,
            fsync_every_records,
            Arc::clone(&queued),
            Arc::clone(&records),
            Arc::clone(&bytes),
        ));
        Ok(Self {
            name,
            path,
            sender: Some(sender),
            task,
            queued,
            max_queue_depth,
            records,
            bytes,
        })
    }

    pub(super) async fn send(&self, value: T) -> Result<(), CaptureError> {
        self.send_with_timeout(value, WRITER_ENQUEUE_TIMEOUT).await
    }

    async fn send_with_timeout(
        &self,
        value: T,
        enqueue_timeout: Duration,
    ) -> Result<(), CaptureError> {
        let depth = self.queued.fetch_add(1, Ordering::Relaxed) + 1;
        self.max_queue_depth.fetch_max(depth, Ordering::Relaxed);
        let Some(sender) = &self.sender else {
            self.queued.fetch_sub(1, Ordering::Relaxed);
            return Err(CaptureError::WriterClosed(self.name));
        };
        match tokio::time::timeout(enqueue_timeout, sender.send(value)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => {
                self.queued.fetch_sub(1, Ordering::Relaxed);
                Err(CaptureError::WriterClosed(self.name))
            }
            Err(_) => {
                self.queued.fetch_sub(1, Ordering::Relaxed);
                Err(CaptureError::WriterBackpressure {
                    name: self.name,
                    timeout_ms: enqueue_timeout.as_millis(),
                })
            }
        }
    }

    pub(super) async fn shutdown_with_evidence(self) -> Result<JsonlWriterShutdown, CaptureError> {
        self.shutdown_with_evidence_timeout(WRITER_SHUTDOWN_TIMEOUT)
            .await
    }

    async fn shutdown_with_evidence_timeout(
        mut self,
        shutdown_timeout: Duration,
    ) -> Result<JsonlWriterShutdown, CaptureError> {
        drop(self.sender.take());
        let task_result = tokio::time::timeout(shutdown_timeout, &mut self.task).await;
        match task_result {
            Ok(Ok(Ok(sha256))) => Ok(JsonlWriterShutdown {
                stats: JsonlWriterStats {
                    records: self.records.load(Ordering::Relaxed),
                    bytes: self.bytes.load(Ordering::Relaxed),
                    max_queue_depth: self.max_queue_depth.load(Ordering::Relaxed),
                    sha256,
                },
                failure: None,
            }),
            result => {
                let failure = match result {
                    Ok(Ok(Ok(_))) => unreachable!("successful writer outcome handled above"),
                    Ok(Ok(Err(error))) => error,
                    Ok(Err(error)) => CaptureError::WriterJoin(error),
                    Err(_) => {
                        self.task.abort();
                        let failure = CaptureError::WriterShutdownTimeout {
                            name: self.name,
                            timeout_ms: shutdown_timeout.as_millis(),
                        };
                        if tokio::time::timeout(WRITER_ABORT_TIMEOUT, &mut self.task)
                            .await
                            .is_err()
                        {
                            return Err(combine_capture_lifecycle_errors(
                                failure,
                                vec![(
                                    "abort stalled writer task",
                                    CaptureError::WriterAbortTimeout {
                                        name: self.name,
                                        timeout_ms: WRITER_ABORT_TIMEOUT.as_millis(),
                                    },
                                )],
                            ));
                        }
                        failure
                    }
                };
                let scan = tokio::time::timeout(
                    WRITER_EVIDENCE_SCAN_TIMEOUT,
                    scan_jsonl_writer_stats(
                        &self.path,
                        self.max_queue_depth.load(Ordering::Relaxed),
                    ),
                )
                .await;
                let stats = match scan {
                    Ok(Ok(stats)) => stats,
                    Ok(Err(scan_error)) => {
                        return Err(combine_capture_lifecycle_errors(
                            failure,
                            vec![("scan failed writer output", scan_error)],
                        ));
                    }
                    Err(_) => {
                        return Err(combine_capture_lifecycle_errors(
                            failure,
                            vec![(
                                "scan failed writer output",
                                CaptureError::WriterEvidenceTimeout {
                                    name: self.name,
                                    timeout_ms: WRITER_EVIDENCE_SCAN_TIMEOUT.as_millis(),
                                },
                            )],
                        ));
                    }
                };
                Ok(JsonlWriterShutdown {
                    stats,
                    failure: Some(failure),
                })
            }
        }
    }
}

async fn scan_jsonl_writer_stats(
    path: &Path,
    max_queue_depth: usize,
) -> Result<JsonlWriterStats, CaptureError> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut bytes = 0_u64;
    let mut records = 0_u64;
    let mut last_byte = None;
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let chunk = &buffer[..read];
        bytes = bytes.saturating_add(read as u64);
        records =
            records.saturating_add(chunk.iter().filter(|byte| **byte == b'\n').count() as u64);
        last_byte = chunk.last().copied();
        hasher.update(chunk);
    }
    if bytes > 0 && last_byte != Some(b'\n') {
        records = records.saturating_add(1);
    }
    Ok(JsonlWriterStats {
        records,
        bytes,
        max_queue_depth,
        sha256: digest_hex(hasher.finalize()),
    })
}

async fn run_jsonl_writer<T>(
    file: tokio::fs::File,
    mut receiver: mpsc::Receiver<T>,
    flush_every_records: usize,
    fsync_every_records: usize,
    queued: Arc<AtomicUsize>,
    records: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
) -> Result<String, CaptureError>
where
    T: Serialize,
{
    let flush_every_records = flush_every_records.max(1);
    let mut writer = BufWriter::new(file);
    let mut hasher = Sha256::new();
    let mut since_flush = 0_usize;
    let mut since_sync = 0_usize;
    while let Some(value) = receiver.recv().await {
        let mut line = serde_json::to_vec(&value)?;
        line.push(b'\n');
        writer.write_all(&line).await?;
        hasher.update(&line);
        queued.fetch_sub(1, Ordering::Relaxed);
        records.fetch_add(1, Ordering::Relaxed);
        bytes.fetch_add(line.len() as u64, Ordering::Relaxed);
        since_flush += 1;
        since_sync += 1;
        if since_flush >= flush_every_records {
            writer.flush().await?;
            since_flush = 0;
        }
        if fsync_every_records > 0 && since_sync >= fsync_every_records {
            writer.flush().await?;
            writer.get_ref().sync_data().await?;
            since_sync = 0;
        }
    }
    writer.flush().await?;
    writer.get_ref().sync_data().await?;
    Ok(digest_hex(hasher.finalize()))
}

#[cfg(unix)]
async fn sync_parent_directory(path: &Path) -> Result<(), CaptureError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    tokio::fs::File::open(parent).await?.sync_all().await?;
    Ok(())
}

#[cfg(not(unix))]
async fn sync_parent_directory(_path: &Path) -> Result<(), CaptureError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    use reap_core::{Level, MarketEvent, NormalizedEvent, OrderBook};
    use tokio::task::JoinHandle;

    use super::*;

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
        writer.send(event).await.unwrap();
        let outcome = writer.shutdown_with_evidence().await.unwrap();
        assert!(outcome.failure.is_none());
        let stats = outcome.stats;
        assert_eq!(stats.records, 1);
        assert_eq!(stats.sha256.len(), 64);

        let text = std::fs::read_to_string(path).unwrap();
        let decoded: NormalizedEvent = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(decoded.ts_ms(), 1);
        assert_eq!(stats.sha256, sha256_hex(text.as_bytes()));
    }

    #[tokio::test]
    async fn writer_enqueue_fails_with_bounded_backpressure_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("stalled.jsonl");
        std::fs::write(&path, []).unwrap();
        let (sender, _receiver) = mpsc::channel::<u64>(1);
        sender.send(1).await.unwrap();
        let queued = Arc::new(AtomicUsize::new(1));
        let max_queue_depth = Arc::new(AtomicUsize::new(1));
        let task: JoinHandle<Result<String, CaptureError>> = tokio::spawn(std::future::pending());
        let writer = JsonlWriter {
            name: "test",
            path,
            sender: Some(sender),
            task,
            queued: Arc::clone(&queued),
            max_queue_depth: Arc::clone(&max_queue_depth),
            records: Arc::new(AtomicU64::new(0)),
            bytes: Arc::new(AtomicU64::new(0)),
        };

        let error = tokio::time::timeout(
            Duration::from_secs(1),
            writer.send_with_timeout(2, Duration::from_millis(10)),
        )
        .await
        .unwrap()
        .unwrap_err();

        assert!(matches!(
            error,
            CaptureError::WriterBackpressure {
                name: "test",
                timeout_ms: 10,
            }
        ));
        assert_eq!(queued.load(Ordering::Relaxed), 1);
        assert_eq!(max_queue_depth.load(Ordering::Relaxed), 2);
        writer.task.abort();
    }

    #[tokio::test]
    async fn writer_shutdown_timeout_aborts_task_and_recovers_partial_file_stats() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("partial.jsonl");
        let partial = b"{\"record\":1}\n{\"record\":2}";
        std::fs::write(&path, partial).unwrap();
        let (sender, _receiver) = mpsc::channel::<u64>(1);
        let task: JoinHandle<Result<String, CaptureError>> = tokio::spawn(std::future::pending());
        let writer = JsonlWriter {
            name: "test",
            path,
            sender: Some(sender),
            task,
            queued: Arc::new(AtomicUsize::new(0)),
            max_queue_depth: Arc::new(AtomicUsize::new(3)),
            records: Arc::new(AtomicU64::new(0)),
            bytes: Arc::new(AtomicU64::new(0)),
        };

        let outcome = tokio::time::timeout(
            Duration::from_secs(1),
            writer.shutdown_with_evidence_timeout(Duration::from_millis(10)),
        )
        .await
        .unwrap()
        .unwrap();

        assert!(matches!(
            outcome.failure,
            Some(CaptureError::WriterShutdownTimeout {
                name: "test",
                timeout_ms: 10,
            })
        ));
        assert_eq!(outcome.stats.records, 2);
        assert_eq!(outcome.stats.bytes, partial.len() as u64);
        assert_eq!(outcome.stats.max_queue_depth, 3);
        assert_eq!(outcome.stats.sha256, sha256_hex(partial));
    }

    #[tokio::test]
    async fn writer_stats_scan_counts_a_trailing_partial_record() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("partial.jsonl");
        let partial = b"first\nsecond";
        std::fs::write(&path, partial).unwrap();

        let stats = scan_jsonl_writer_stats(&path, 7).await.unwrap();

        assert_eq!(stats.records, 2);
        assert_eq!(stats.bytes, partial.len() as u64);
        assert_eq!(stats.max_queue_depth, 7);
        assert_eq!(stats.sha256, sha256_hex(partial));
    }
}
