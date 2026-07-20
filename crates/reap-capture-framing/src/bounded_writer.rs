use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::sync::mpsc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinHandle;

#[cfg(feature = "legacy-reap-capture")]
use crate::encode_jsonl_frame_legacy_unbounded;
use crate::{BoundedJsonlFrameError, digest_hex, encode_jsonl_frame_bounded, sha256_hex};

#[derive(Debug, Clone)]
pub struct BoundedWriterConfig {
    pub capacity: usize,
    /// Maximum compact JSONL record size, including its trailing newline.
    pub max_frame_bytes: usize,
    /// Maximum exact encoded bytes retained across queued and in-flight items.
    pub max_queued_bytes: usize,
    pub flush_every_records: usize,
    pub fsync_every_records: usize,
    pub enqueue_timeout: Duration,
    pub shutdown_timeout: Duration,
    pub abort_timeout: Duration,
    pub evidence_scan_timeout: Duration,
}

impl Default for BoundedWriterConfig {
    fn default() -> Self {
        Self {
            capacity: 1,
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
}

/// Configuration for the explicitly unweighted compatibility writer.
///
/// New capture paths must use [`BoundedWriterConfig`] and
/// [`BoundedJsonlWriter`]. This configuration exists only for callers whose
/// historical serialization and error behavior must remain byte-for-byte
/// compatible while serialization continues on the writer task.
#[derive(Debug, Clone)]
#[cfg(feature = "legacy-reap-capture")]
pub struct LegacyEntryCountWriterConfig {
    pub capacity: usize,
    pub flush_every_records: usize,
    pub fsync_every_records: usize,
    pub enqueue_timeout: Duration,
    pub shutdown_timeout: Duration,
    pub abort_timeout: Duration,
    pub evidence_scan_timeout: Duration,
}

#[cfg(feature = "legacy-reap-capture")]
impl Default for LegacyEntryCountWriterConfig {
    fn default() -> Self {
        Self {
            capacity: 1,
            flush_every_records: 1,
            fsync_every_records: 0,
            enqueue_timeout: Duration::from_secs(1),
            shutdown_timeout: Duration::from_secs(30),
            abort_timeout: Duration::from_secs(1),
            evidence_scan_timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JsonlWriterStats {
    pub records: u64,
    pub bytes: u64,
    pub max_queue_depth: usize,
    /// Exact high-water of charged frame bytes for byte-bounded writers.
    ///
    /// The legacy entry-count compatibility writer reports `None`.
    pub max_queue_bytes: Option<usize>,
    /// Exact high-water including pre-serialization worst-case reservations.
    ///
    /// The legacy entry-count compatibility writer reports `None`.
    pub max_reserved_bytes: Option<usize>,
    pub sha256: String,
}

impl JsonlWriterStats {
    pub fn empty() -> Self {
        Self {
            sha256: sha256_hex(&[]),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueByteEvidence {
    pub current_bytes: usize,
    pub high_water_bytes: usize,
    pub current_reserved_bytes: usize,
    pub reservation_high_water_bytes: usize,
    pub limit_bytes: usize,
}

#[derive(Debug)]
pub struct AdditionalWriterError {
    pub label: &'static str,
    pub error: Box<JsonlWriterError>,
}

#[derive(Debug, Error)]
pub enum JsonlWriterError {
    #[error("failed to create new {name} output {path}: {source}")]
    OpenOutput {
        name: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("writer IO failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("writer serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("{0} writer closed unexpectedly")]
    Closed(&'static str),
    #[error("{name} writer queue remained full for {timeout_ms}ms")]
    Backpressure {
        name: &'static str,
        timeout_ms: u128,
    },
    #[error("writer task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("{name} writer did not shut down within {timeout_ms}ms")]
    ShutdownTimeout {
        name: &'static str,
        timeout_ms: u128,
    },
    #[error("{name} writer did not abort within {timeout_ms}ms")]
    AbortTimeout {
        name: &'static str,
        timeout_ms: u128,
    },
    #[error("{name} writer evidence scan exceeded {timeout_ms}ms")]
    EvidenceTimeout {
        name: &'static str,
        timeout_ms: u128,
    },
    #[error("{primary}; additional writer lifecycle failures")]
    Lifecycle {
        #[source]
        primary: Box<JsonlWriterError>,
        additional: Vec<AdditionalWriterError>,
    },
}

#[derive(Debug, Error)]
pub enum ByteBoundedWriterError {
    #[error(
        "invalid writer byte bounds: frame limit {max_frame_bytes}, queue limit {max_queued_bytes}: {reason}"
    )]
    InvalidByteBounds {
        max_frame_bytes: usize,
        max_queued_bytes: usize,
        reason: &'static str,
    },
    #[error(
        "{name} frame exceeds {limit_bytes} byte frame limit (observed at least {observed_at_least_bytes} bytes)"
    )]
    FrameTooLarge {
        name: &'static str,
        observed_at_least_bytes: usize,
        limit_bytes: usize,
    },
    #[error(
        "{name} writer byte budget remained full for {timeout_ms}ms: worst-case reservation {reservation_bytes}, queued {queued_bytes}, reserved {reserved_bytes}, limit {limit_bytes}, queue high-water {high_water_bytes}, reservation high-water {reservation_high_water_bytes}"
    )]
    ByteBackpressure {
        name: &'static str,
        reservation_bytes: usize,
        queued_bytes: usize,
        reserved_bytes: usize,
        limit_bytes: usize,
        high_water_bytes: usize,
        reservation_high_water_bytes: usize,
        timeout_ms: u128,
    },
    #[error(
        "{name} writer serialization size changed between bounded passes: measured {measured_bytes}, second pass observed {second_pass_bytes} bytes"
    )]
    SerializationSizeChanged {
        name: &'static str,
        measured_bytes: usize,
        second_pass_bytes: usize,
    },
    #[error(transparent)]
    Writer(#[from] JsonlWriterError),
}

#[derive(Debug)]
pub struct JsonlWriterShutdown {
    pub stats: JsonlWriterStats,
    pub failure: Option<JsonlWriterError>,
}

/// A statically typed JSONL writer with strict entry, per-frame, and aggregate
/// queued-byte bounds.
///
/// Both entry and byte permits remain charged until the writer has finished
/// writing the item. The bounded path performs a capped counting pass followed
/// by a fixed-capacity encoding pass, so neither a caller-supplied estimate nor
/// an unweighted item can bypass the exact serialized-frame budget.
pub struct BoundedJsonlWriter<T> {
    core: WriterCore<T>,
    entry_budget: Arc<Semaphore>,
    byte_budget: Arc<Semaphore>,
    max_frame_bytes: usize,
    max_queued_bytes: usize,
}

impl<T> std::fmt::Debug for BoundedJsonlWriter<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BoundedJsonlWriter")
            .field("name", &self.core.name)
            .field("path", &self.core.path)
            .field("max_frame_bytes", &self.max_frame_bytes)
            .field("max_queued_bytes", &self.max_queued_bytes)
            .field("config", &self.core.config)
            .finish_non_exhaustive()
    }
}

impl<T> BoundedJsonlWriter<T>
where
    T: Serialize + Send + 'static,
{
    pub async fn start(
        name: &'static str,
        path: PathBuf,
        config: BoundedWriterConfig,
    ) -> Result<Self, ByteBoundedWriterError> {
        validate_byte_bounds(&config)?;
        let queued_bytes = Arc::new(AtomicUsize::new(0));
        let max_queue_bytes = Arc::new(AtomicUsize::new(0));
        let reserved_bytes = Arc::new(AtomicUsize::new(0));
        let max_reserved_bytes = Arc::new(AtomicUsize::new(0));
        let core = WriterCore::start(
            name,
            path,
            config.capacity,
            WriterRuntimeConfig::from_bounded(&config),
            WriterQueueEvidence {
                queued_bytes: Some(Arc::clone(&queued_bytes)),
                max_queue_bytes: Some(Arc::clone(&max_queue_bytes)),
                reserved_bytes: Some(Arc::clone(&reserved_bytes)),
                max_reserved_bytes: Some(Arc::clone(&max_reserved_bytes)),
            },
        )
        .await?;
        Ok(Self {
            core,
            entry_budget: Arc::new(Semaphore::new(config.capacity.max(1))),
            byte_budget: Arc::new(Semaphore::new(config.max_queued_bytes)),
            max_frame_bytes: config.max_frame_bytes,
            max_queued_bytes: config.max_queued_bytes,
        })
    }

    pub async fn send(&self, value: T) -> Result<(), ByteBoundedWriterError> {
        self.send_with_timeout(value, self.core.config.enqueue_timeout)
            .await
    }

    pub async fn send_with_timeout(
        &self,
        value: T,
        enqueue_timeout: Duration,
    ) -> Result<(), ByteBoundedWriterError> {
        let Some(sender) = &self.core.sender else {
            return Err(JsonlWriterError::Closed(self.core.name).into());
        };

        let started = Instant::now();
        let entry_permit = match tokio::time::timeout(
            enqueue_timeout,
            Arc::clone(&self.entry_budget).acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => return Err(JsonlWriterError::Closed(self.core.name).into()),
            Err(_) => {
                return Err(JsonlWriterError::Backpressure {
                    name: self.core.name,
                    timeout_ms: enqueue_timeout.as_millis(),
                }
                .into());
            }
        };

        let remaining = enqueue_timeout.saturating_sub(started.elapsed());
        let byte_permit = match tokio::time::timeout(
            remaining,
            Arc::clone(&self.byte_budget).acquire_many_owned(self.max_frame_bytes as u32),
        )
        .await
        {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => return Err(JsonlWriterError::Closed(self.core.name).into()),
            Err(_) => {
                let queued_bytes = self
                    .core
                    .queued_bytes
                    .as_ref()
                    .expect("byte-bounded writer has byte evidence")
                    .load(Ordering::Acquire);
                let high_water_bytes = self
                    .core
                    .max_queue_bytes
                    .as_ref()
                    .expect("byte-bounded writer has byte evidence")
                    .load(Ordering::Acquire);
                let reserved_bytes = self
                    .core
                    .reserved_bytes
                    .as_ref()
                    .expect("byte-bounded writer has reservation evidence")
                    .load(Ordering::Acquire);
                let reservation_high_water_bytes = self
                    .core
                    .max_reserved_bytes
                    .as_ref()
                    .expect("byte-bounded writer has reservation evidence")
                    .load(Ordering::Acquire);
                return Err(ByteBoundedWriterError::ByteBackpressure {
                    name: self.core.name,
                    reservation_bytes: self.max_frame_bytes,
                    queued_bytes,
                    reserved_bytes,
                    limit_bytes: self.max_queued_bytes,
                    high_water_bytes,
                    reservation_high_water_bytes,
                    timeout_ms: enqueue_timeout.as_millis(),
                });
            }
        };
        let mut byte_reservation = TrackedByteReservation::new(
            byte_permit,
            self.max_frame_bytes,
            Arc::clone(
                self.core
                    .reserved_bytes
                    .as_ref()
                    .expect("byte-bounded writer has reservation evidence"),
            ),
            Arc::clone(
                self.core
                    .max_reserved_bytes
                    .as_ref()
                    .expect("byte-bounded writer has reservation evidence"),
            ),
        );

        let frame = match encode_jsonl_frame_bounded(&value, self.max_frame_bytes) {
            Ok(frame) => frame,
            Err(BoundedJsonlFrameError::Serialization(source)) => {
                return Err(JsonlWriterError::Serialization(source).into());
            }
            Err(BoundedJsonlFrameError::FrameTooLarge {
                observed_at_least_bytes,
                limit_bytes,
            }) => {
                return Err(ByteBoundedWriterError::FrameTooLarge {
                    name: self.core.name,
                    observed_at_least_bytes,
                    limit_bytes,
                });
            }
            Err(BoundedJsonlFrameError::SizeChanged {
                measured_bytes,
                second_pass_bytes,
            }) => {
                return Err(ByteBoundedWriterError::SerializationSizeChanged {
                    name: self.core.name,
                    measured_bytes,
                    second_pass_bytes,
                });
            }
        };
        let frame_bytes = frame.len();
        byte_reservation.shrink_to(frame_bytes);

        let remaining = enqueue_timeout.saturating_sub(started.elapsed());
        let channel_permit = match tokio::time::timeout(remaining, sender.reserve()).await {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => return Err(JsonlWriterError::Closed(self.core.name).into()),
            Err(_) => {
                return Err(JsonlWriterError::Backpressure {
                    name: self.core.name,
                    timeout_ms: enqueue_timeout.as_millis(),
                }
                .into());
            }
        };

        let depth = self.core.queued.fetch_add(1, Ordering::AcqRel) + 1;
        self.core.max_queue_depth.fetch_max(depth, Ordering::AcqRel);
        let queued_bytes = self
            .core
            .queued_bytes
            .as_ref()
            .expect("byte-bounded writer has byte evidence")
            .fetch_add(frame_bytes, Ordering::AcqRel)
            + frame_bytes;
        self.core
            .max_queue_bytes
            .as_ref()
            .expect("byte-bounded writer has byte evidence")
            .fetch_max(queued_bytes, Ordering::AcqRel);
        channel_permit.send(QueuedItem::byte_bounded(
            frame,
            frame_bytes,
            Arc::clone(&self.core.queued),
            Arc::clone(
                self.core
                    .queued_bytes
                    .as_ref()
                    .expect("byte-bounded writer has byte evidence"),
            ),
            entry_permit,
            byte_reservation,
        ));
        Ok(())
    }

    pub fn queue_byte_evidence(&self) -> QueueByteEvidence {
        QueueByteEvidence {
            current_bytes: self
                .core
                .queued_bytes
                .as_ref()
                .expect("byte-bounded writer has byte evidence")
                .load(Ordering::Acquire),
            high_water_bytes: self
                .core
                .max_queue_bytes
                .as_ref()
                .expect("byte-bounded writer has byte evidence")
                .load(Ordering::Acquire),
            current_reserved_bytes: self
                .core
                .reserved_bytes
                .as_ref()
                .expect("byte-bounded writer has reservation evidence")
                .load(Ordering::Acquire),
            reservation_high_water_bytes: self
                .core
                .max_reserved_bytes
                .as_ref()
                .expect("byte-bounded writer has reservation evidence")
                .load(Ordering::Acquire),
            limit_bytes: self.max_queued_bytes,
        }
    }

    pub async fn shutdown_with_evidence(
        self,
    ) -> Result<JsonlWriterShutdown, ByteBoundedWriterError> {
        let timeout = self.core.config.shutdown_timeout;
        Ok(self.core.shutdown_with_evidence_timeout(timeout).await?)
    }

    pub async fn shutdown_with_evidence_timeout(
        self,
        shutdown_timeout: Duration,
    ) -> Result<JsonlWriterShutdown, ByteBoundedWriterError> {
        Ok(self
            .core
            .shutdown_with_evidence_timeout(shutdown_timeout)
            .await?)
    }
}

/// Explicit entry-count-only compatibility path.
///
/// This type intentionally does not expose a byte high-water and must not be
/// used by new capture code. It preserves the historical typed queue and
/// writer-task serialization behavior for an existing wrapper.
#[cfg(feature = "legacy-reap-capture")]
pub struct LegacyEntryCountJsonlWriter<T> {
    core: WriterCore<T>,
}

#[cfg(feature = "legacy-reap-capture")]
impl<T> std::fmt::Debug for LegacyEntryCountJsonlWriter<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LegacyEntryCountJsonlWriter")
            .field("name", &self.core.name)
            .field("path", &self.core.path)
            .field("config", &self.core.config)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "legacy-reap-capture")]
impl<T> LegacyEntryCountJsonlWriter<T>
where
    T: Serialize + Send + 'static,
{
    pub async fn start(
        name: &'static str,
        path: PathBuf,
        config: LegacyEntryCountWriterConfig,
    ) -> Result<Self, JsonlWriterError> {
        let core = WriterCore::start(
            name,
            path,
            config.capacity,
            WriterRuntimeConfig::from_legacy(&config),
            WriterQueueEvidence::default(),
        )
        .await?;
        Ok(Self { core })
    }

    pub async fn send(&self, value: T) -> Result<(), JsonlWriterError> {
        self.send_with_timeout(value, self.core.config.enqueue_timeout)
            .await
    }

    pub async fn send_with_timeout(
        &self,
        value: T,
        enqueue_timeout: Duration,
    ) -> Result<(), JsonlWriterError> {
        let depth = self.core.queued.fetch_add(1, Ordering::Relaxed) + 1;
        self.core
            .max_queue_depth
            .fetch_max(depth, Ordering::Relaxed);
        let Some(sender) = &self.core.sender else {
            self.core.queued.fetch_sub(1, Ordering::Relaxed);
            return Err(JsonlWriterError::Closed(self.core.name));
        };
        let item = QueuedItem::legacy(value, Arc::clone(&self.core.queued));
        match tokio::time::timeout(enqueue_timeout, sender.send(item)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(JsonlWriterError::Closed(self.core.name)),
            Err(_) => Err(JsonlWriterError::Backpressure {
                name: self.core.name,
                timeout_ms: enqueue_timeout.as_millis(),
            }),
        }
    }

    pub async fn shutdown_with_evidence(self) -> Result<JsonlWriterShutdown, JsonlWriterError> {
        let timeout = self.core.config.shutdown_timeout;
        self.core.shutdown_with_evidence_timeout(timeout).await
    }

    pub async fn shutdown_with_evidence_timeout(
        self,
        shutdown_timeout: Duration,
    ) -> Result<JsonlWriterShutdown, JsonlWriterError> {
        self.core
            .shutdown_with_evidence_timeout(shutdown_timeout)
            .await
    }
}

#[derive(Debug, Clone)]
struct WriterRuntimeConfig {
    flush_every_records: usize,
    fsync_every_records: usize,
    enqueue_timeout: Duration,
    shutdown_timeout: Duration,
    abort_timeout: Duration,
    evidence_scan_timeout: Duration,
}

impl WriterRuntimeConfig {
    fn from_bounded(config: &BoundedWriterConfig) -> Self {
        Self {
            flush_every_records: config.flush_every_records,
            fsync_every_records: config.fsync_every_records,
            enqueue_timeout: config.enqueue_timeout,
            shutdown_timeout: config.shutdown_timeout,
            abort_timeout: config.abort_timeout,
            evidence_scan_timeout: config.evidence_scan_timeout,
        }
    }

    #[cfg(feature = "legacy-reap-capture")]
    fn from_legacy(config: &LegacyEntryCountWriterConfig) -> Self {
        Self {
            flush_every_records: config.flush_every_records,
            fsync_every_records: config.fsync_every_records,
            enqueue_timeout: config.enqueue_timeout,
            shutdown_timeout: config.shutdown_timeout,
            abort_timeout: config.abort_timeout,
            evidence_scan_timeout: config.evidence_scan_timeout,
        }
    }
}

struct WriterCore<T> {
    name: &'static str,
    path: PathBuf,
    sender: Option<mpsc::Sender<QueuedItem<T>>>,
    task: JoinHandle<Result<String, JsonlWriterError>>,
    queued: Arc<AtomicUsize>,
    max_queue_depth: Arc<AtomicUsize>,
    queued_bytes: Option<Arc<AtomicUsize>>,
    max_queue_bytes: Option<Arc<AtomicUsize>>,
    reserved_bytes: Option<Arc<AtomicUsize>>,
    max_reserved_bytes: Option<Arc<AtomicUsize>>,
    records: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
    config: WriterRuntimeConfig,
}

#[derive(Default)]
struct WriterQueueEvidence {
    queued_bytes: Option<Arc<AtomicUsize>>,
    max_queue_bytes: Option<Arc<AtomicUsize>>,
    reserved_bytes: Option<Arc<AtomicUsize>>,
    max_reserved_bytes: Option<Arc<AtomicUsize>>,
}

impl<T> WriterCore<T>
where
    T: Serialize + Send + 'static,
{
    async fn start(
        name: &'static str,
        path: PathBuf,
        capacity: usize,
        config: WriterRuntimeConfig,
        evidence: WriterQueueEvidence,
    ) -> Result<Self, JsonlWriterError> {
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
            .map_err(|source| JsonlWriterError::OpenOutput {
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
            config.flush_every_records,
            config.fsync_every_records,
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
            queued_bytes: evidence.queued_bytes,
            max_queue_bytes: evidence.max_queue_bytes,
            reserved_bytes: evidence.reserved_bytes,
            max_reserved_bytes: evidence.max_reserved_bytes,
            records,
            bytes,
            config,
        })
    }

    async fn shutdown_with_evidence_timeout(
        mut self,
        shutdown_timeout: Duration,
    ) -> Result<JsonlWriterShutdown, JsonlWriterError> {
        drop(self.sender.take());
        let task_result = tokio::time::timeout(shutdown_timeout, &mut self.task).await;
        match task_result {
            Ok(Ok(Ok(sha256))) => Ok(JsonlWriterShutdown {
                stats: JsonlWriterStats {
                    records: self.records.load(Ordering::Relaxed),
                    bytes: self.bytes.load(Ordering::Relaxed),
                    max_queue_depth: self.max_queue_depth.load(Ordering::Relaxed),
                    max_queue_bytes: self
                        .max_queue_bytes
                        .as_ref()
                        .map(|value| value.load(Ordering::Acquire)),
                    max_reserved_bytes: self
                        .max_reserved_bytes
                        .as_ref()
                        .map(|value| value.load(Ordering::Acquire)),
                    sha256,
                },
                failure: None,
            }),
            result => {
                let failure = match result {
                    Ok(Ok(Ok(_))) => unreachable!("successful writer outcome handled above"),
                    Ok(Ok(Err(error))) => error,
                    Ok(Err(error)) => JsonlWriterError::Join(error),
                    Err(_) => {
                        self.task.abort();
                        let failure = JsonlWriterError::ShutdownTimeout {
                            name: self.name,
                            timeout_ms: shutdown_timeout.as_millis(),
                        };
                        if tokio::time::timeout(self.config.abort_timeout, &mut self.task)
                            .await
                            .is_err()
                        {
                            return Err(JsonlWriterError::Lifecycle {
                                primary: Box::new(failure),
                                additional: vec![AdditionalWriterError {
                                    label: "abort stalled writer task",
                                    error: Box::new(JsonlWriterError::AbortTimeout {
                                        name: self.name,
                                        timeout_ms: self.config.abort_timeout.as_millis(),
                                    }),
                                }],
                            });
                        }
                        failure
                    }
                };
                let scan = tokio::time::timeout(
                    self.config.evidence_scan_timeout,
                    scan_jsonl_writer_stats(
                        &self.path,
                        self.max_queue_depth.load(Ordering::Relaxed),
                        self.max_queue_bytes
                            .as_ref()
                            .map(|value| value.load(Ordering::Acquire)),
                        self.max_reserved_bytes
                            .as_ref()
                            .map(|value| value.load(Ordering::Acquire)),
                    ),
                )
                .await;
                let stats = match scan {
                    Ok(Ok(stats)) => stats,
                    Ok(Err(scan_error)) => {
                        return Err(JsonlWriterError::Lifecycle {
                            primary: Box::new(failure),
                            additional: vec![AdditionalWriterError {
                                label: "scan failed writer output",
                                error: Box::new(scan_error),
                            }],
                        });
                    }
                    Err(_) => {
                        return Err(JsonlWriterError::Lifecycle {
                            primary: Box::new(failure),
                            additional: vec![AdditionalWriterError {
                                label: "scan failed writer output",
                                error: Box::new(JsonlWriterError::EvidenceTimeout {
                                    name: self.name,
                                    timeout_ms: self.config.evidence_scan_timeout.as_millis(),
                                }),
                            }],
                        });
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

struct QueuedItem<T> {
    payload: QueuedPayload<T>,
    accounting: QueueAccounting,
}

enum QueuedPayload<T> {
    #[cfg(feature = "legacy-reap-capture")]
    SerializeOnWriter(T),
    EncodedJsonl(Vec<u8>, std::marker::PhantomData<fn() -> T>),
}

impl<T> QueuedItem<T> {
    #[cfg(feature = "legacy-reap-capture")]
    fn legacy(value: T, queued: Arc<AtomicUsize>) -> Self {
        Self {
            payload: QueuedPayload::SerializeOnWriter(value),
            accounting: QueueAccounting {
                queued,
                queued_bytes: None,
                frame_bytes: 0,
                entry_permit: None,
                byte_reservation: None,
                released: false,
            },
        }
    }

    fn byte_bounded(
        frame: Vec<u8>,
        frame_bytes: usize,
        queued: Arc<AtomicUsize>,
        queued_bytes: Arc<AtomicUsize>,
        entry_permit: OwnedSemaphorePermit,
        byte_reservation: TrackedByteReservation,
    ) -> Self {
        Self {
            payload: QueuedPayload::EncodedJsonl(frame, std::marker::PhantomData),
            accounting: QueueAccounting {
                queued,
                queued_bytes: Some(queued_bytes),
                frame_bytes,
                entry_permit: Some(entry_permit),
                byte_reservation: Some(byte_reservation),
                released: false,
            },
        }
    }

    fn release_accounting(&mut self) {
        self.accounting.release();
    }
}

struct QueueAccounting {
    queued: Arc<AtomicUsize>,
    queued_bytes: Option<Arc<AtomicUsize>>,
    frame_bytes: usize,
    entry_permit: Option<OwnedSemaphorePermit>,
    byte_reservation: Option<TrackedByteReservation>,
    released: bool,
}

impl QueueAccounting {
    fn release(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        self.queued.fetch_sub(1, Ordering::AcqRel);
        if let Some(queued_bytes) = &self.queued_bytes {
            queued_bytes.fetch_sub(self.frame_bytes, Ordering::AcqRel);
        }
        drop(self.byte_reservation.take());
        drop(self.entry_permit.take());
    }
}

impl Drop for QueueAccounting {
    fn drop(&mut self) {
        self.release();
    }
}

struct TrackedByteReservation {
    permit: Option<OwnedSemaphorePermit>,
    charged_bytes: usize,
    reserved_bytes: Arc<AtomicUsize>,
}

impl TrackedByteReservation {
    fn new(
        permit: OwnedSemaphorePermit,
        charged_bytes: usize,
        reserved_bytes: Arc<AtomicUsize>,
        max_reserved_bytes: Arc<AtomicUsize>,
    ) -> Self {
        let current = reserved_bytes.fetch_add(charged_bytes, Ordering::AcqRel) + charged_bytes;
        max_reserved_bytes.fetch_max(current, Ordering::AcqRel);
        Self {
            permit: Some(permit),
            charged_bytes,
            reserved_bytes,
        }
    }

    fn shrink_to(&mut self, retained_bytes: usize) {
        assert!(retained_bytes <= self.charged_bytes);
        let surplus = self.charged_bytes - retained_bytes;
        if surplus == 0 {
            return;
        }
        let surplus_permit = self
            .permit
            .as_mut()
            .and_then(|permit| permit.split(surplus))
            .expect("tracked byte permit covers its charged surplus");
        self.reserved_bytes.fetch_sub(surplus, Ordering::AcqRel);
        self.charged_bytes = retained_bytes;
        drop(surplus_permit);
    }
}

impl Drop for TrackedByteReservation {
    fn drop(&mut self) {
        self.reserved_bytes
            .fetch_sub(self.charged_bytes, Ordering::AcqRel);
        drop(self.permit.take());
    }
}

fn validate_byte_bounds(config: &BoundedWriterConfig) -> Result<(), ByteBoundedWriterError> {
    let reason = if config.max_frame_bytes == 0 {
        Some("frame limit must be positive")
    } else if config.max_queued_bytes == 0 {
        Some("queue byte limit must be positive")
    } else if config.max_frame_bytes > config.max_queued_bytes {
        Some("frame limit cannot exceed queue byte limit")
    } else if config.max_queued_bytes > u32::MAX as usize {
        Some("queue byte limit exceeds semaphore accounting range")
    } else {
        None
    };
    match reason {
        Some(reason) => Err(ByteBoundedWriterError::InvalidByteBounds {
            max_frame_bytes: config.max_frame_bytes,
            max_queued_bytes: config.max_queued_bytes,
            reason,
        }),
        None => Ok(()),
    }
}

async fn scan_jsonl_writer_stats(
    path: &Path,
    max_queue_depth: usize,
    max_queue_bytes: Option<usize>,
    max_reserved_bytes: Option<usize>,
) -> Result<JsonlWriterStats, JsonlWriterError> {
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
        max_queue_bytes,
        max_reserved_bytes,
        sha256: digest_hex(hasher.finalize()),
    })
}

async fn run_jsonl_writer<T>(
    file: tokio::fs::File,
    mut receiver: mpsc::Receiver<QueuedItem<T>>,
    flush_every_records: usize,
    fsync_every_records: usize,
    records: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
) -> Result<String, JsonlWriterError>
where
    T: Serialize,
{
    let flush_every_records = flush_every_records.max(1);
    let mut writer = BufWriter::new(file);
    let mut hasher = Sha256::new();
    let mut since_flush = 0_usize;
    let mut since_sync = 0_usize;
    while let Some(mut item) = receiver.recv().await {
        let frame = match &mut item.payload {
            #[cfg(feature = "legacy-reap-capture")]
            QueuedPayload::SerializeOnWriter(value) => encode_jsonl_frame_legacy_unbounded(value)?,
            QueuedPayload::EncodedJsonl(frame, _) => std::mem::take(frame),
        };
        writer.write_all(&frame).await?;
        hasher.update(&frame);
        let frame_bytes = frame.len() as u64;
        drop(frame);
        item.release_accounting();
        records.fetch_add(1, Ordering::Relaxed);
        bytes.fetch_add(frame_bytes, Ordering::Relaxed);
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
async fn sync_parent_directory(path: &Path) -> Result<(), JsonlWriterError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    tokio::fs::File::open(parent).await?.sync_all().await?;
    Ok(())
}

#[cfg(not(unix))]
async fn sync_parent_directory(_path: &Path) -> Result<(), JsonlWriterError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Condvar, Mutex};

    use serde::Serializer;

    use super::*;

    #[derive(Debug, Clone)]
    struct ChargedRecord(Arc<str>);

    impl Serialize for ChargedRecord {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_str(&self.0)
        }
    }

    fn charged_record(frame_bytes: usize) -> ChargedRecord {
        assert!(frame_bytes >= 3);
        ChargedRecord(Arc::from("x".repeat(frame_bytes - 3)))
    }

    fn stalled_bounded_writer<T>(
        path: PathBuf,
        capacity: usize,
        max_frame_bytes: usize,
        max_queued_bytes: usize,
    ) -> BoundedJsonlWriter<T>
    where
        T: Serialize + Send + 'static,
    {
        std::fs::write(&path, []).unwrap();
        let (sender, receiver) = mpsc::channel(capacity);
        let queued = Arc::new(AtomicUsize::new(0));
        let max_queue_depth = Arc::new(AtomicUsize::new(0));
        let queued_bytes = Arc::new(AtomicUsize::new(0));
        let max_queue_bytes = Arc::new(AtomicUsize::new(0));
        let reserved_bytes = Arc::new(AtomicUsize::new(0));
        let max_reserved_bytes = Arc::new(AtomicUsize::new(0));
        let task: JoinHandle<Result<String, JsonlWriterError>> = tokio::spawn(async move {
            let _receiver = receiver;
            std::future::pending().await
        });
        BoundedJsonlWriter {
            core: WriterCore {
                name: "test",
                path,
                sender: Some(sender),
                task,
                queued,
                max_queue_depth,
                queued_bytes: Some(queued_bytes),
                max_queue_bytes: Some(max_queue_bytes),
                reserved_bytes: Some(reserved_bytes),
                max_reserved_bytes: Some(max_reserved_bytes),
                records: Arc::new(AtomicU64::new(0)),
                bytes: Arc::new(AtomicU64::new(0)),
                config: WriterRuntimeConfig::from_bounded(&BoundedWriterConfig::default()),
            },
            entry_budget: Arc::new(Semaphore::new(capacity)),
            byte_budget: Arc::new(Semaphore::new(max_queued_bytes)),
            max_frame_bytes,
            max_queued_bytes,
        }
    }

    #[tokio::test]
    async fn byte_budget_accepts_exact_limit_and_reports_exact_high_water() {
        const MIB: usize = 1024 * 1024;
        const QUEUE_BYTES: usize = 32 * MIB;
        const OVERSIZE_BYTES: usize = MIB + 1;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("stalled.jsonl");
        let writer = stalled_bounded_writer::<ChargedRecord>(path, 8_192, MIB, QUEUE_BYTES);

        let error = writer
            .send(charged_record(OVERSIZE_BYTES))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ByteBoundedWriterError::FrameTooLarge {
                observed_at_least_bytes: OVERSIZE_BYTES,
                limit_bytes: MIB,
                ..
            }
        ));
        assert_eq!(writer.queue_byte_evidence().current_bytes, 0);
        assert_eq!(writer.queue_byte_evidence().current_reserved_bytes, 0);

        let exact_frame = charged_record(MIB);
        for _ in 0..32 {
            writer.send(exact_frame.clone()).await.unwrap();
        }
        assert_eq!(
            writer.queue_byte_evidence(),
            QueueByteEvidence {
                current_bytes: QUEUE_BYTES,
                high_water_bytes: QUEUE_BYTES,
                current_reserved_bytes: QUEUE_BYTES,
                reservation_high_water_bytes: QUEUE_BYTES,
                limit_bytes: QUEUE_BYTES,
            }
        );

        let error = writer
            .send_with_timeout(charged_record(4), Duration::from_millis(10))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ByteBoundedWriterError::ByteBackpressure {
                reservation_bytes: MIB,
                queued_bytes: QUEUE_BYTES,
                reserved_bytes: QUEUE_BYTES,
                limit_bytes: QUEUE_BYTES,
                high_water_bytes: QUEUE_BYTES,
                reservation_high_water_bytes: QUEUE_BYTES,
                timeout_ms: 10,
                ..
            }
        ));
        assert_eq!(writer.queue_byte_evidence().high_water_bytes, QUEUE_BYTES);
        writer.core.task.abort();
    }

    struct SerializationLatch {
        released: Mutex<bool>,
        wake: Condvar,
        active: AtomicUsize,
        max_active: AtomicUsize,
    }

    impl SerializationLatch {
        fn new() -> Self {
            Self {
                released: Mutex::new(false),
                wake: Condvar::new(),
                active: AtomicUsize::new(0),
                max_active: AtomicUsize::new(0),
            }
        }

        fn wait(&self) {
            let active = self.active.fetch_add(1, Ordering::AcqRel) + 1;
            self.max_active.fetch_max(active, Ordering::AcqRel);
            let mut released = self.released.lock().unwrap();
            while !*released {
                released = self.wake.wait(released).unwrap();
            }
            self.active.fetch_sub(1, Ordering::AcqRel);
        }

        fn release(&self) {
            *self.released.lock().unwrap() = true;
            self.wake.notify_all();
        }
    }

    struct BlockingRecord {
        latch: Arc<SerializationLatch>,
        calls: Arc<AtomicUsize>,
    }

    impl Serialize for BlockingRecord {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            if self.calls.fetch_add(1, Ordering::AcqRel) == 0 {
                self.latch.wait();
            }
            serializer.serialize_str("x")
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn worst_case_reservations_bound_concurrent_serialization() {
        const MAX_FRAME: usize = 64;
        const BYTE_LIMIT: usize = 2 * MAX_FRAME;
        let directory = tempfile::tempdir().unwrap();
        let writer = Arc::new(stalled_bounded_writer::<BlockingRecord>(
            directory.path().join("concurrent.jsonl"),
            3,
            MAX_FRAME,
            BYTE_LIMIT,
        ));
        let latch = Arc::new(SerializationLatch::new());
        let first_calls = Arc::new(AtomicUsize::new(0));
        let second_calls = Arc::new(AtomicUsize::new(0));
        let third_calls = Arc::new(AtomicUsize::new(0));

        let first = {
            let writer = Arc::clone(&writer);
            let latch = Arc::clone(&latch);
            let calls = Arc::clone(&first_calls);
            tokio::spawn(async move { writer.send(BlockingRecord { latch, calls }).await })
        };
        let second = {
            let writer = Arc::clone(&writer);
            let latch = Arc::clone(&latch);
            let calls = Arc::clone(&second_calls);
            tokio::spawn(async move { writer.send(BlockingRecord { latch, calls }).await })
        };

        tokio::time::timeout(Duration::from_secs(1), async {
            while latch.active.load(Ordering::Acquire) != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let third = {
            let writer = Arc::clone(&writer);
            let latch = Arc::clone(&latch);
            let calls = Arc::clone(&third_calls);
            tokio::spawn(async move { writer.send(BlockingRecord { latch, calls }).await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(third_calls.load(Ordering::Acquire), 0);
        assert_eq!(
            writer.queue_byte_evidence().current_reserved_bytes,
            BYTE_LIMIT
        );

        latch.release();
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        third.await.unwrap().unwrap();

        assert_eq!(latch.max_active.load(Ordering::Acquire), 2);
        assert_eq!(first_calls.load(Ordering::Acquire), 2);
        assert_eq!(second_calls.load(Ordering::Acquire), 2);
        assert_eq!(third_calls.load(Ordering::Acquire), 2);
        assert_eq!(
            writer.queue_byte_evidence(),
            QueueByteEvidence {
                current_bytes: 12,
                high_water_bytes: 12,
                current_reserved_bytes: 12,
                reservation_high_water_bytes: BYTE_LIMIT,
                limit_bytes: BYTE_LIMIT,
            }
        );
        let writer = Arc::try_unwrap(writer).ok().unwrap();
        writer.core.task.abort();
    }

    #[cfg(feature = "legacy-reap-capture")]
    #[tokio::test]
    async fn legacy_enqueue_timeout_preserves_entry_depth_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("stalled.jsonl");
        std::fs::write(&path, []).unwrap();
        let (sender, _receiver) = mpsc::channel::<QueuedItem<u64>>(1);
        let queued = Arc::new(AtomicUsize::new(1));
        let max_queue_depth = Arc::new(AtomicUsize::new(1));
        let task: JoinHandle<Result<String, JsonlWriterError>> =
            tokio::spawn(std::future::pending());
        let writer = LegacyEntryCountJsonlWriter {
            core: WriterCore {
                name: "test",
                path,
                sender: Some(sender.clone()),
                task,
                queued: Arc::clone(&queued),
                max_queue_depth: Arc::clone(&max_queue_depth),
                queued_bytes: None,
                max_queue_bytes: None,
                reserved_bytes: None,
                max_reserved_bytes: None,
                records: Arc::new(AtomicU64::new(0)),
                bytes: Arc::new(AtomicU64::new(0)),
                config: WriterRuntimeConfig::from_legacy(&LegacyEntryCountWriterConfig::default()),
            },
        };
        sender
            .send(QueuedItem::legacy(1, Arc::clone(&queued)))
            .await
            .unwrap();

        let error = writer
            .send_with_timeout(2, Duration::from_millis(10))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            JsonlWriterError::Backpressure {
                name: "test",
                timeout_ms: 10,
            }
        ));
        assert_eq!(queued.load(Ordering::Relaxed), 1);
        assert_eq!(max_queue_depth.load(Ordering::Relaxed), 2);
        writer.core.task.abort();
    }

    #[cfg(feature = "legacy-reap-capture")]
    #[tokio::test]
    async fn shutdown_timeout_recovers_partial_file_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("partial.jsonl");
        let partial = b"{\"record\":1}\n{\"record\":2}";
        std::fs::write(&path, partial).unwrap();
        let (sender, _receiver) = mpsc::channel::<QueuedItem<u64>>(1);
        let task: JoinHandle<Result<String, JsonlWriterError>> =
            tokio::spawn(std::future::pending());
        let writer = LegacyEntryCountJsonlWriter {
            core: WriterCore {
                name: "test",
                path,
                sender: Some(sender),
                task,
                queued: Arc::new(AtomicUsize::new(0)),
                max_queue_depth: Arc::new(AtomicUsize::new(3)),
                queued_bytes: None,
                max_queue_bytes: None,
                reserved_bytes: None,
                max_reserved_bytes: None,
                records: Arc::new(AtomicU64::new(0)),
                bytes: Arc::new(AtomicU64::new(0)),
                config: WriterRuntimeConfig::from_legacy(&LegacyEntryCountWriterConfig::default()),
            },
        };

        let outcome = writer
            .shutdown_with_evidence_timeout(Duration::from_millis(10))
            .await
            .unwrap();

        assert!(matches!(
            outcome.failure,
            Some(JsonlWriterError::ShutdownTimeout {
                name: "test",
                timeout_ms: 10,
            })
        ));
        assert_eq!(outcome.stats.records, 2);
        assert_eq!(outcome.stats.bytes, partial.len() as u64);
        assert_eq!(outcome.stats.max_queue_depth, 3);
        assert_eq!(outcome.stats.max_queue_bytes, None);
        assert_eq!(outcome.stats.max_reserved_bytes, None);
        assert_eq!(outcome.stats.sha256, sha256_hex(partial));
    }
}
