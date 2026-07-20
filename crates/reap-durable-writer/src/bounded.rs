use std::sync::Arc;

use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

use crate::progress::{WriterProgress, WriterProgressSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryClass {
    Critical,
    BestEffort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueOutcome {
    Queued,
    DroppedBestEffort,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EnqueueError {
    #[error("durable writer is closed")]
    Closed,
    #[error("durable writer queue is full")]
    Full,
    #[error("durable write failed: {0}")]
    Durability(String),
}

pub struct BoundedSink<Record> {
    pub(crate) sender: mpsc::Sender<PendingRecord<Record>>,
    pub(crate) progress: Arc<WriterProgress>,
}

impl<Record> Clone for BoundedSink<Record> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            progress: Arc::clone(&self.progress),
        }
    }
}

impl<Record> BoundedSink<Record> {
    pub async fn enqueue(
        &self,
        record: Record,
        class: DeliveryClass,
    ) -> Result<EnqueueOutcome, EnqueueError> {
        match class {
            DeliveryClass::Critical => {
                let permit = self
                    .sender
                    .reserve()
                    .await
                    .map_err(|_| EnqueueError::Closed)?;
                self.progress.record_enqueued();
                permit.send(PendingRecord::queued(
                    record,
                    None,
                    Arc::clone(&self.progress),
                ));
                Ok(EnqueueOutcome::Queued)
            }
            DeliveryClass::BestEffort => self.try_enqueue(record, class),
        }
    }

    pub fn try_enqueue(
        &self,
        record: Record,
        class: DeliveryClass,
    ) -> Result<EnqueueOutcome, EnqueueError> {
        match self.sender.try_reserve() {
            Ok(permit) => {
                self.progress.record_enqueued();
                permit.send(PendingRecord::queued(
                    record,
                    None,
                    Arc::clone(&self.progress),
                ));
                Ok(EnqueueOutcome::Queued)
            }
            Err(mpsc::error::TrySendError::Full(_)) if class == DeliveryClass::Critical => {
                Err(EnqueueError::Full)
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.progress.record_dropped();
                Ok(EnqueueOutcome::DroppedBestEffort)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(EnqueueError::Closed),
        }
    }

    pub async fn enqueue_durable(&self, record: Record) -> Result<(), EnqueueError> {
        let (durable_ack, acknowledgement) = oneshot::channel();
        let permit = self
            .sender
            .reserve()
            .await
            .map_err(|_| EnqueueError::Closed)?;
        self.progress.record_enqueued();
        permit.send(PendingRecord::queued(
            record,
            Some(durable_ack),
            Arc::clone(&self.progress),
        ));
        match acknowledgement.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(message)) => Err(EnqueueError::Durability(message)),
            Err(_) => Err(EnqueueError::Closed),
        }
    }

    #[must_use]
    pub fn dropped_records(&self) -> u64 {
        self.progress.snapshot().dropped_records
    }

    #[must_use]
    pub fn records_outstanding(&self) -> usize {
        self.progress.snapshot().records_outstanding
    }

    #[must_use]
    pub fn progress_snapshot(&self) -> WriterProgressSnapshot {
        self.progress.snapshot()
    }
}

pub(crate) struct PendingRecord<Record> {
    pub(crate) record: Record,
    pub(crate) durable_ack: Option<oneshot::Sender<Result<(), String>>>,
    pub(crate) queue_drop_guard: Option<QueueDropGuard>,
}

impl<Record> PendingRecord<Record> {
    fn queued(
        record: Record,
        durable_ack: Option<oneshot::Sender<Result<(), String>>>,
        progress: Arc<WriterProgress>,
    ) -> Self {
        Self {
            record,
            durable_ack,
            queue_drop_guard: Some(QueueDropGuard {
                progress,
                armed: true,
            }),
        }
    }

    pub(crate) fn mark_received(&mut self) {
        if let Some(guard) = &mut self.queue_drop_guard
            && guard.armed
        {
            guard.progress.record_received();
            guard.armed = false;
        }
    }
}

pub(crate) struct QueueDropGuard {
    progress: Arc<WriterProgress>,
    armed: bool,
}

impl Drop for QueueDropGuard {
    fn drop(&mut self) {
        if self.armed {
            self.progress.record_received();
            self.progress.record_dropped();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounded_full_closed_and_drop_accounting_are_explicit() {
        let (sender, mut receiver) = mpsc::channel(1);
        let progress = Arc::new(WriterProgress::new(1));
        let sink = BoundedSink {
            sender,
            progress: Arc::clone(&progress),
        };

        assert_eq!(
            sink.try_enqueue(1_u8, DeliveryClass::Critical),
            Ok(EnqueueOutcome::Queued)
        );
        assert_eq!(
            sink.try_enqueue(2_u8, DeliveryClass::Critical),
            Err(EnqueueError::Full)
        );
        assert_eq!(
            sink.try_enqueue(3_u8, DeliveryClass::BestEffort),
            Ok(EnqueueOutcome::DroppedBestEffort)
        );
        let snapshot = sink.progress_snapshot();
        assert_eq!(snapshot.queue_capacity, 1);
        assert_eq!(snapshot.queue_depth, 1);
        assert_eq!(snapshot.queue_high_water, 1);
        assert_eq!(snapshot.dropped_records, 1);

        let mut pending = receiver.recv().await.unwrap();
        pending.mark_received();
        drop(pending);
        progress.record_completed();
        drop(receiver);
        assert_eq!(
            sink.try_enqueue(4_u8, DeliveryClass::Critical),
            Err(EnqueueError::Closed)
        );
    }

    #[tokio::test]
    async fn channel_depth_and_outstanding_writes_remain_distinct() {
        let (sender, mut receiver) = mpsc::channel(1);
        let progress = Arc::new(WriterProgress::new(1));
        let sink = BoundedSink {
            sender,
            progress: Arc::clone(&progress),
        };

        sink.enqueue(1_u8, DeliveryClass::Critical).await.unwrap();
        let mut first = receiver.recv().await.unwrap();
        sink.enqueue(2_u8, DeliveryClass::Critical).await.unwrap();

        let snapshot = sink.progress_snapshot();
        assert_eq!(snapshot.queue_depth, 1);
        assert_eq!(snapshot.records_outstanding, 2);
        first.mark_received();
        drop(first);
        progress.record_completed();
        let mut second = receiver.recv().await.unwrap();
        second.mark_received();
        drop(second);
        progress.record_completed();
        assert_eq!(sink.progress_snapshot().records_outstanding, 0);
    }

    #[tokio::test]
    async fn receiver_drop_accounts_for_accepted_unwritten_records() {
        let (sender, receiver) = mpsc::channel(1);
        let progress = Arc::new(WriterProgress::new(1));
        let sink = BoundedSink {
            sender,
            progress: Arc::clone(&progress),
        };
        sink.enqueue(1_u8, DeliveryClass::Critical).await.unwrap();
        drop(receiver);
        let snapshot = sink.progress_snapshot();
        assert_eq!(snapshot.queue_depth, 0);
        assert_eq!(snapshot.records_outstanding, 1);
        assert_eq!(snapshot.dropped_records, 1);
    }
}
