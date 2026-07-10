use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use reap_core::{
    FillLiquidity, NormalizedEvent, OrderIntent, OrderUpdate, Price, Quantity, RawEnvelope, Side,
    Symbol, TimeMs,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum StorageRecord {
    Raw(RawEnvelope),
    Normalized(NormalizedEvent),
    Intent { ts_ms: TimeMs, intent: OrderIntent },
    Order(OrderUpdate),
    Fill(FillRecord),
}

impl StorageRecord {
    fn is_critical(&self) -> bool {
        matches!(self, Self::Intent { .. } | Self::Order(_) | Self::Fill(_))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillRecord {
    pub ts_ms: TimeMs,
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
    #[error("storage IO failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("storage serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
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

async fn run_writer(
    config: StorageConfig,
    mut receiver: mpsc::Receiver<StorageRecord>,
    mut shutdown: oneshot::Receiver<()>,
    queued: Arc<AtomicUsize>,
) -> Result<(), StorageError> {
    if let Some(parent) = config.path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config.path)
        .await?;
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
    #[derive(Serialize)]
    struct Envelope {
        schema_version: u16,
        record: StorageRecord,
    }
    let mut line = serde_json::to_vec(&Envelope {
        schema_version: 1,
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
        StorageRecord::Raw(RawEnvelope {
            venue: Venue::Okx,
            conn_id: ConnId::new("test"),
            channel: reap_core::Channel::Books,
            symbol: Some("BTC-USDT".to_string()),
            recv_ts_ns: 1,
            raw_hash: 2,
            payload: "{}".to_string(),
        })
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
        sink.record(StorageRecord::Order(OrderUpdate {
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
        }))
        .await
        .unwrap();
        sink.record(StorageRecord::Fill(FillRecord {
            ts_ms: 1,
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
}
