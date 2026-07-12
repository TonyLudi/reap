use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use reap_core::{
    FillLiquidity, NormalizedEvent, OrderIntent, OrderUpdate, Price, Quantity, RawEnvelope, Side,
    Symbol, SystemEvent, TimeMs,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum StorageRecord {
    Raw {
        account_id: Option<String>,
        envelope: RawEnvelope,
    },
    Normalized(NormalizedEvent),
    Intent {
        ts_ms: TimeMs,
        intent: OrderIntent,
    },
    IntentRejected {
        ts_ms: TimeMs,
        intent: OrderIntent,
        reason: String,
    },
    Bootstrap(BootstrapRecord),
    OrderRequest(OrderRequestRecord),
    OrderAck(OrderAckRecord),
    Order {
        account_id: Option<String>,
        update: OrderUpdate,
    },
    Fill(FillRecord),
    System(SystemEvent),
    Reconciliation(ReconciliationRecord),
}

impl StorageRecord {
    fn is_critical(&self) -> bool {
        !matches!(self, Self::Raw { .. } | Self::Normalized(_))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderRequestRecord {
    pub ts_ms: TimeMs,
    pub account_id: String,
    pub operation: OrderOperation,
    pub idempotency_key: Option<String>,
    pub client_order_id: Option<String>,
    pub exchange_order_id: Option<String>,
    pub symbol: Symbol,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderOperation {
    Submit,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderAckRecord {
    pub ts_ms: TimeMs,
    pub account_id: String,
    pub operation: OrderOperation,
    pub client_order_id: String,
    pub exchange_order_id: Option<String>,
    pub status: OrderAckStatus,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderAckStatus {
    Accepted,
    Duplicate,
    PendingReconciliation,
    Rejected,
    Ambiguous,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconciliationRecord {
    pub ts_ms: TimeMs,
    pub account_id: String,
    pub clean: bool,
    pub local_live_orders: usize,
    pub remote_live_orders: usize,
    pub remote_recent_fills: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapRecord {
    pub ts_ms: TimeMs,
    pub account_id: String,
    pub strategy_name: String,
    pub config_fingerprint: String,
    pub baseline_fill_ids: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RecoveredStorage {
    pub latest_orders: HashMap<String, OrderUpdate>,
    pub fills: Vec<FillRecord>,
    pub seen_fill_ids: HashSet<String>,
    pub baseline_fill_ids: HashMap<String, HashSet<String>>,
    pub bootstrap_identities: HashMap<String, (String, String)>,
    pub last_ts_ms: TimeMs,
    pub records: u64,
    pub ignored_truncated_tail: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillRecord {
    pub ts_ms: TimeMs,
    pub account_id: Option<String>,
    pub fill_id: String,
    pub order_id: String,
    pub symbol: Symbol,
    pub side: Side,
    pub price: Price,
    pub qty: Quantity,
    pub liquidity: FillLiquidity,
}

#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub path: PathBuf,
    pub channel_capacity: usize,
    pub flush_every_records: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageWriteOutcome {
    Queued,
    DroppedBestEffort,
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("storage writer is closed")]
    Closed,
    #[error("storage queue is full for a critical record")]
    Backpressure,
    #[error("storage IO failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("storage serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("storage recovery found an invalid record on line {line}: {message}")]
    Corrupt { line: usize, message: String },
    #[error("storage writer task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

#[derive(Clone)]
pub struct StorageSink {
    sender: mpsc::Sender<StorageRecord>,
    dropped: Arc<AtomicU64>,
    queued: Arc<AtomicUsize>,
}

impl StorageSink {
    pub async fn record(&self, record: StorageRecord) -> Result<StorageWriteOutcome, StorageError> {
        if record.is_critical() {
            self.queued.fetch_add(1, Ordering::Relaxed);
            if self.sender.send(record).await.is_err() {
                self.queued.fetch_sub(1, Ordering::Relaxed);
                return Err(StorageError::Closed);
            }
            return Ok(StorageWriteOutcome::Queued);
        }
        self.queued.fetch_add(1, Ordering::Relaxed);
        match self.sender.try_send(record) {
            Ok(()) => Ok(StorageWriteOutcome::Queued),
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.queued.fetch_sub(1, Ordering::Relaxed);
                self.dropped.fetch_add(1, Ordering::Relaxed);
                Ok(StorageWriteOutcome::DroppedBestEffort)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.queued.fetch_sub(1, Ordering::Relaxed);
                Err(StorageError::Closed)
            }
        }
    }

    pub fn try_record(&self, record: StorageRecord) -> Result<StorageWriteOutcome, StorageError> {
        let critical = record.is_critical();
        self.queued.fetch_add(1, Ordering::Relaxed);
        match self.sender.try_send(record) {
            Ok(()) => Ok(StorageWriteOutcome::Queued),
            Err(mpsc::error::TrySendError::Full(_)) if critical => {
                self.queued.fetch_sub(1, Ordering::Relaxed);
                Err(StorageError::Backpressure)
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.queued.fetch_sub(1, Ordering::Relaxed);
                self.dropped.fetch_add(1, Ordering::Relaxed);
                Ok(StorageWriteOutcome::DroppedBestEffort)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.queued.fetch_sub(1, Ordering::Relaxed);
                Err(StorageError::Closed)
            }
        }
    }

    pub fn dropped_records(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    pub fn queue_depth(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }
}

pub struct StorageRuntime {
    sink: StorageSink,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<Result<(), StorageError>>,
}

impl StorageRuntime {
    pub fn sink(&self) -> StorageSink {
        self.sink.clone()
    }

    pub async fn shutdown(self) -> Result<(), StorageError> {
        let _ = self.shutdown.send(());
        self.task.await??;
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredEnvelope {
    schema_version: u16,
    record: StorageRecord,
}

pub fn recover_jsonl(path: impl AsRef<Path>) -> Result<RecoveredStorage, StorageError> {
    let path = path.as_ref();
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RecoveredStorage::default());
        }
        Err(error) => return Err(error.into()),
    };
    let trailing_newline = text.ends_with('\n');
    let lines = text.lines().collect::<Vec<_>>();
    let mut recovered = RecoveredStorage::default();
    for (index, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let envelope: StoredEnvelope = match serde_json::from_str(line) {
            Ok(envelope) => envelope,
            Err(_) if index + 1 == lines.len() && !trailing_newline => {
                recovered.ignored_truncated_tail = true;
                break;
            }
            Err(error) => {
                return Err(StorageError::Corrupt {
                    line: index + 1,
                    message: error.to_string(),
                });
            }
        };
        if envelope.schema_version != 2 {
            return Err(StorageError::Corrupt {
                line: index + 1,
                message: format!("unsupported schema version {}", envelope.schema_version),
            });
        }
        recovered.records += 1;
        recovered.last_ts_ms = recovered
            .last_ts_ms
            .max(storage_record_ts_ms(&envelope.record));
        match envelope.record {
            StorageRecord::Bootstrap(bootstrap) => {
                recovered.bootstrap_identities.insert(
                    bootstrap.account_id.clone(),
                    (bootstrap.strategy_name, bootstrap.config_fingerprint),
                );
                recovered.baseline_fill_ids.insert(
                    bootstrap.account_id,
                    bootstrap.baseline_fill_ids.into_iter().collect(),
                );
            }
            StorageRecord::Order { update, .. } => {
                recovered
                    .latest_orders
                    .insert(update.order_id.clone(), update);
            }
            StorageRecord::Fill(fill) => {
                recovered.seen_fill_ids.insert(fill.fill_id.clone());
                recovered.fills.push(fill);
            }
            _ => {}
        }
    }
    Ok(recovered)
}

fn storage_record_ts_ms(record: &StorageRecord) -> TimeMs {
    match record {
        StorageRecord::Raw { envelope, .. } => envelope.recv_ts_ns / 1_000_000,
        StorageRecord::Normalized(event) => event.ts_ms(),
        StorageRecord::Intent { ts_ms, .. } | StorageRecord::IntentRejected { ts_ms, .. } => *ts_ms,
        StorageRecord::Bootstrap(bootstrap) => bootstrap.ts_ms,
        StorageRecord::OrderRequest(request) => request.ts_ms,
        StorageRecord::OrderAck(ack) => ack.ts_ms,
        StorageRecord::Order { update, .. } => update.ts_ms,
        StorageRecord::Fill(fill) => fill.ts_ms,
        StorageRecord::System(system) => system.ts_ms,
        StorageRecord::Reconciliation(reconciliation) => reconciliation.ts_ms,
    }
}

pub fn spawn_jsonl_storage(config: StorageConfig) -> StorageRuntime {
    let (sender, receiver) = mpsc::channel(config.channel_capacity.max(1));
    let (shutdown, shutdown_rx) = oneshot::channel();
    let dropped = Arc::new(AtomicU64::new(0));
    let queued = Arc::new(AtomicUsize::new(0));
    let sink = StorageSink {
        sender,
        dropped,
        queued: Arc::clone(&queued),
    };
    let task = tokio::spawn(run_writer(config, receiver, shutdown_rx, queued));
    StorageRuntime {
        sink,
        shutdown,
        task,
    }
}

pub async fn start_jsonl_storage(config: StorageConfig) -> Result<StorageRuntime, StorageError> {
    let file = open_storage_file(&config).await?;
    let (sender, receiver) = mpsc::channel(config.channel_capacity.max(1));
    let (shutdown, shutdown_rx) = oneshot::channel();
    let dropped = Arc::new(AtomicU64::new(0));
    let queued = Arc::new(AtomicUsize::new(0));
    let sink = StorageSink {
        sender,
        dropped,
        queued: Arc::clone(&queued),
    };
    let task = tokio::spawn(run_writer_with_file(
        config,
        file,
        receiver,
        shutdown_rx,
        queued,
    ));
    Ok(StorageRuntime {
        sink,
        shutdown,
        task,
    })
}

async fn run_writer(
    config: StorageConfig,
    receiver: mpsc::Receiver<StorageRecord>,
    shutdown: oneshot::Receiver<()>,
    queued: Arc<AtomicUsize>,
) -> Result<(), StorageError> {
    let file = open_storage_file(&config).await?;
    run_writer_with_file(config, file, receiver, shutdown, queued).await
}

async fn open_storage_file(config: &StorageConfig) -> Result<tokio::fs::File, StorageError> {
    if let Some(parent) = config.path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    Ok(tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config.path)
        .await?)
}

async fn run_writer_with_file(
    config: StorageConfig,
    mut file: tokio::fs::File,
    mut receiver: mpsc::Receiver<StorageRecord>,
    mut shutdown: oneshot::Receiver<()>,
    queued: Arc<AtomicUsize>,
) -> Result<(), StorageError> {
    let flush_every = config.flush_every_records.max(1);
    let mut since_flush = 0_usize;

    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => {
                receiver.close();
                while let Some(record) = receiver.recv().await {
                    write_record(&mut file, record).await?;
                    queued.fetch_sub(1, Ordering::Relaxed);
                }
                break;
            }
            record = receiver.recv() => {
                let Some(record) = record else { break; };
                write_record(&mut file, record).await?;
                queued.fetch_sub(1, Ordering::Relaxed);
                since_flush += 1;
                if since_flush >= flush_every {
                    file.flush().await?;
                    since_flush = 0;
                }
            }
        }
    }
    file.flush().await?;
    Ok(())
}

async fn write_record(
    file: &mut tokio::fs::File,
    record: StorageRecord,
) -> Result<(), StorageError> {
    let mut line = serde_json::to_vec(&StoredEnvelope {
        schema_version: 2,
        record,
    })?;
    line.push(b'\n');
    file.write_all(&line).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use reap_core::{ConnId, ControlEvent, OrderEvent, OrderStatus, OrderUpdate, Venue};

    use super::*;

    fn raw() -> StorageRecord {
        StorageRecord::Raw {
            account_id: None,
            envelope: RawEnvelope {
                venue: Venue::Okx,
                conn_id: ConnId::new("test"),
                channel: reap_core::Channel::Books,
                symbol: Some("BTC-USDT".to_string()),
                recv_ts_ns: 1,
                raw_hash: 2,
                payload: "{}".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn best_effort_records_drop_when_bounded_queue_is_full() {
        let (sender, _receiver) = mpsc::channel(1);
        let sink = StorageSink {
            sender,
            dropped: Arc::new(AtomicU64::new(0)),
            queued: Arc::new(AtomicUsize::new(0)),
        };
        assert_eq!(
            sink.record(raw()).await.unwrap(),
            StorageWriteOutcome::Queued
        );
        assert_eq!(
            sink.record(raw()).await.unwrap(),
            StorageWriteOutcome::DroppedBestEffort
        );
        assert_eq!(sink.dropped_records(), 1);
    }

    #[test]
    fn critical_record_backpressure_is_fail_stop_not_drop() {
        let (sender, _receiver) = mpsc::channel(1);
        let sink = StorageSink {
            sender,
            dropped: Arc::new(AtomicU64::new(0)),
            queued: Arc::new(AtomicUsize::new(0)),
        };
        sink.try_record(StorageRecord::Intent {
            ts_ms: 1,
            intent: OrderIntent::CancelOrder {
                order_id: "one".to_string(),
                reason: "test".to_string(),
            },
        })
        .unwrap();
        assert!(matches!(
            sink.try_record(StorageRecord::Intent {
                ts_ms: 2,
                intent: OrderIntent::CancelOrder {
                    order_id: "two".to_string(),
                    reason: "test".to_string(),
                },
            }),
            Err(StorageError::Backpressure)
        ));
        assert_eq!(sink.dropped_records(), 0);
    }

    #[tokio::test]
    async fn writer_persists_all_record_classes_as_jsonl() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.jsonl");
        let runtime = spawn_jsonl_storage(StorageConfig {
            path: path.clone(),
            channel_capacity: 16,
            flush_every_records: 1,
        });
        let sink = runtime.sink();
        sink.record(raw()).await.unwrap();
        sink.record(StorageRecord::Normalized(NormalizedEvent::Control(
            ControlEvent {
                ts_ms: 1,
                reason: "test".to_string(),
            },
        )))
        .await
        .unwrap();
        sink.record(StorageRecord::Intent {
            ts_ms: 1,
            intent: OrderIntent::CancelOrder {
                order_id: "order-1".to_string(),
                reason: "test".to_string(),
            },
        })
        .await
        .unwrap();
        sink.record(StorageRecord::Order {
            account_id: None,
            update: OrderUpdate {
                ts_ms: 1,
                order_id: "order-1".to_string(),
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                event: OrderEvent::New,
                status: OrderStatus::Live,
                price: 100.0,
                qty: 1.0,
                open_qty: 1.0,
                filled_qty: 0.0,
                avg_fill_price: 0.0,
                last_fill_qty: 0.0,
                last_fill_price: 0.0,
                last_fill_liquidity: None,
                reason: "test".to_string(),
            },
        })
        .await
        .unwrap();
        sink.record(StorageRecord::Fill(FillRecord {
            ts_ms: 1,
            account_id: None,
            fill_id: "fill-1".to_string(),
            order_id: "order-1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            price: 100.0,
            qty: 1.0,
            liquidity: FillLiquidity::Maker,
        }))
        .await
        .unwrap();
        runtime.shutdown().await.unwrap();

        let text = std::fs::read_to_string(path).unwrap();
        let lines = text.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 5);
        assert!(
            lines
                .iter()
                .all(|line| serde_json::from_str::<serde_json::Value>(line).is_ok())
        );
        assert!(text.contains("normalized"));
        assert!(text.contains("intent"));
        assert!(text.contains("order"));
        assert!(text.contains("fill"));
    }

    #[tokio::test]
    async fn recovery_restores_latest_orders_and_fill_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events.jsonl");
        let runtime = start_jsonl_storage(StorageConfig {
            path: path.clone(),
            channel_capacity: 16,
            flush_every_records: 1,
        })
        .await
        .unwrap();
        let sink = runtime.sink();
        sink.record(StorageRecord::Bootstrap(BootstrapRecord {
            ts_ms: 1,
            account_id: "main".to_string(),
            strategy_name: "test".to_string(),
            config_fingerprint: "fingerprint".to_string(),
            baseline_fill_ids: vec!["historical-fill".to_string()],
        }))
        .await
        .unwrap();
        let mut order = OrderUpdate {
            ts_ms: 1,
            order_id: "order-1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            event: OrderEvent::New,
            status: OrderStatus::Live,
            price: 100.0,
            qty: 1.0,
            open_qty: 1.0,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            reason: "test".to_string(),
        };
        sink.record(StorageRecord::Order {
            account_id: Some("main".to_string()),
            update: order.clone(),
        })
        .await
        .unwrap();
        order.ts_ms = 2;
        order.event = OrderEvent::Cancelled;
        order.status = OrderStatus::Cancelled;
        sink.record(StorageRecord::Order {
            account_id: Some("main".to_string()),
            update: order.clone(),
        })
        .await
        .unwrap();
        sink.record(StorageRecord::Fill(FillRecord {
            ts_ms: 2,
            account_id: Some("main".to_string()),
            fill_id: "fill-1".to_string(),
            order_id: "order-1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            price: 100.0,
            qty: 0.5,
            liquidity: FillLiquidity::Maker,
        }))
        .await
        .unwrap();
        runtime.shutdown().await.unwrap();

        let recovered = recover_jsonl(&path).unwrap();
        assert_eq!(
            recovered.latest_orders["order-1"].status,
            OrderStatus::Cancelled
        );
        assert!(recovered.seen_fill_ids.contains("fill-1"));
        assert!(recovered.baseline_fill_ids["main"].contains("historical-fill"));
        assert_eq!(recovered.last_ts_ms, 2);
    }
}
