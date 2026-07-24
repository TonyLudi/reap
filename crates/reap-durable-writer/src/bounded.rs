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

/// A take-once reservation for one durable record in the bounded writer queue.
///
/// Dropping an uncommitted reservation releases its queue capacity without
/// creating or accounting for a record.
///
/// The owning coordinator must commit or drop the reservation in the same
/// turn and before writer shutdown. An outstanding owned channel permit
/// participates in the receiver's bounded drain.
#[must_use = "a durable reservation must be committed or dropped"]
pub struct DurableReservation<Record> {
    permit: mpsc::OwnedPermit<PendingRecord<Record>>,
    progress: Arc<WriterProgress>,
}

/// A take-once receipt for a committed record's durability result.
///
/// Dropping the receipt never cancels the accepted record or its durable
/// write. It only gives up the caller's ability to observe the result.
#[must_use = "a durable receipt must be retained until it resolves"]
pub struct DurableReceipt {
    acknowledgement: oneshot::Receiver<Result<(), String>>,
}

/// Unforgeable evidence that one committed record completed its durable sync.
///
/// The acknowledgement is intentionally move-only. A product-specific owner
/// must keep it paired with the exact pending intent whose receipt produced it.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "a durable acknowledgement must be consumed by its owner"]
pub struct DurableAcknowledgement {
    _private: (),
}

/// The result of one nonblocking, consuming durable-receipt check.
#[must_use = "a pending receipt or completed durability result must be handled"]
pub enum DurableReceiptPoll {
    /// The writer has not produced a result; retain this same receipt.
    Pending(DurableReceipt),
    /// The record was written, flushed, and synced to durable storage.
    Acknowledged(DurableAcknowledgement),
    /// The writer produced an explicit codec, write, or sync failure.
    Failed(String),
    /// The writer disappeared without producing a durability result.
    Closed,
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

    /// Reserves one bounded writer slot without waiting.
    ///
    /// A successful reservation changes no record-progress counters until
    /// [`DurableReservation::commit`] consumes it.
    pub fn try_reserve_durable(&self) -> Result<DurableReservation<Record>, EnqueueError> {
        match self.sender.clone().try_reserve_owned() {
            Ok(permit) => Ok(DurableReservation {
                permit,
                progress: Arc::clone(&self.progress),
            }),
            Err(mpsc::error::TrySendError::Full(_)) => Err(EnqueueError::Full),
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

impl<Record> DurableReservation<Record> {
    /// Commits the record into the already-reserved queue slot.
    ///
    /// This operation is synchronous and infallible because the reservation
    /// owns the required channel capacity. If the writer disappears before
    /// reporting durability, the receipt becomes [`DurableReceiptPoll::Closed`]
    /// once the accepted pending record is discarded.
    pub fn commit(self, record: Record) -> DurableReceipt {
        let Self { permit, progress } = self;
        let (durable_ack, acknowledgement) = oneshot::channel();
        progress.record_enqueued();
        permit.send(PendingRecord::queued(
            record,
            Some(durable_ack),
            Arc::clone(&progress),
        ));
        DurableReceipt { acknowledgement }
    }
}

impl DurableReceipt {
    /// Checks for a durability result without waiting and consumes this
    /// receipt.
    ///
    /// If the result is not ready, [`DurableReceiptPoll::Pending`] returns the
    /// same receipt so its owner can retain it for a later coordinator turn.
    pub fn try_result(mut self) -> DurableReceiptPoll {
        match self.acknowledgement.try_recv() {
            Ok(Ok(())) => DurableReceiptPoll::Acknowledged(DurableAcknowledgement { _private: () }),
            Ok(Err(message)) => DurableReceiptPoll::Failed(message),
            Err(oneshot::error::TryRecvError::Empty) => DurableReceiptPoll::Pending(self),
            Err(oneshot::error::TryRecvError::Closed) => DurableReceiptPoll::Closed,
        }
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

    #[tokio::test]
    async fn durable_reservation_is_nonblocking_bounded_and_drop_is_unaccounted() {
        let (sender, receiver) = mpsc::channel(1);
        let progress = Arc::new(WriterProgress::new(1));
        let sink = BoundedSink {
            sender,
            progress: Arc::clone(&progress),
        };

        let reservation = sink.try_reserve_durable().unwrap();
        assert!(matches!(
            sink.try_reserve_durable(),
            Err(EnqueueError::Full)
        ));
        assert_eq!(
            sink.try_enqueue(1_u8, DeliveryClass::Critical),
            Err(EnqueueError::Full)
        );
        let snapshot = sink.progress_snapshot();
        assert_eq!(snapshot.records_enqueued, 0);
        assert_eq!(snapshot.records_outstanding, 0);
        assert_eq!(snapshot.queue_depth, 0);
        assert_eq!(snapshot.queue_high_water, 0);
        assert_eq!(snapshot.dropped_records, 0);

        drop(reservation);
        let replacement = sink.try_reserve_durable().unwrap();
        drop(replacement);
        drop(receiver);
        assert!(matches!(
            sink.try_reserve_durable(),
            Err(EnqueueError::Closed)
        ));
    }

    #[tokio::test]
    async fn durable_commit_and_receipt_preserve_existing_progress_semantics() {
        let (sender, mut receiver) = mpsc::channel(1);
        let progress = Arc::new(WriterProgress::new(1));
        let sink = BoundedSink {
            sender,
            progress: Arc::clone(&progress),
        };

        let receipt = sink.try_reserve_durable().unwrap().commit(7_u8);
        let snapshot = sink.progress_snapshot();
        assert_eq!(snapshot.records_enqueued, 1);
        assert_eq!(snapshot.records_outstanding, 1);
        assert_eq!(snapshot.queue_depth, 1);
        assert_eq!(snapshot.queue_high_water, 1);
        assert_eq!(snapshot.dropped_records, 0);

        let receipt = match receipt.try_result() {
            DurableReceiptPoll::Pending(receipt) => receipt,
            _ => panic!("an unsignalled receipt must remain pending"),
        };
        let mut pending = receiver.recv().await.unwrap();
        pending.mark_received();
        assert_eq!(pending.record, 7);
        pending.durable_ack.take().unwrap().send(Ok(())).unwrap();
        drop(pending);
        progress.record_completed();

        assert!(matches!(
            receipt.try_result(),
            DurableReceiptPoll::Acknowledged(_)
        ));
        let snapshot = sink.progress_snapshot();
        assert_eq!(snapshot.records_enqueued, 1);
        assert_eq!(snapshot.records_outstanding, 0);
        assert_eq!(snapshot.queue_depth, 0);
        assert_eq!(snapshot.durable_sync_completions, 0);
    }

    #[tokio::test]
    async fn durable_receipt_distinguishes_failure_closed_and_abandoned_observation() {
        let (sender, mut receiver) = mpsc::channel(1);
        let progress = Arc::new(WriterProgress::new(1));
        let sink = BoundedSink {
            sender,
            progress: Arc::clone(&progress),
        };

        let failed = sink.try_reserve_durable().unwrap().commit(1_u8);
        let mut pending = receiver.recv().await.unwrap();
        pending.mark_received();
        pending
            .durable_ack
            .take()
            .unwrap()
            .send(Err("exact durable failure".to_string()))
            .unwrap();
        drop(pending);
        progress.record_completed();
        match failed.try_result() {
            DurableReceiptPoll::Failed(message) => {
                assert_eq!(message, "exact durable failure");
            }
            _ => panic!("an explicit writer failure must remain distinct"),
        }

        let closed = sink.try_reserve_durable().unwrap().commit(2_u8);
        let mut pending = receiver.recv().await.unwrap();
        pending.mark_received();
        drop(pending.durable_ack.take().unwrap());
        drop(pending);
        progress.record_completed();
        assert!(matches!(closed.try_result(), DurableReceiptPoll::Closed));

        let abandoned = sink.try_reserve_durable().unwrap().commit(3_u8);
        drop(abandoned);
        let mut pending = receiver.recv().await.unwrap();
        pending.mark_received();
        assert_eq!(pending.record, 3);
        assert!(pending.durable_ack.take().unwrap().send(Ok(())).is_err());
        drop(pending);
        progress.record_completed();
        let snapshot = sink.progress_snapshot();
        assert_eq!(snapshot.records_enqueued, 3);
        assert_eq!(snapshot.records_outstanding, 0);
        assert_eq!(snapshot.dropped_records, 0);
    }

    #[tokio::test]
    async fn close_after_reservation_is_reported_by_the_committed_receipt() {
        let (sender, receiver) = mpsc::channel(1);
        let progress = Arc::new(WriterProgress::new(1));
        let sink = BoundedSink {
            sender,
            progress: Arc::clone(&progress),
        };

        let reservation = sink.try_reserve_durable().unwrap();
        drop(receiver);
        let receipt = reservation.commit(4_u8);
        drop(sink);
        assert!(matches!(receipt.try_result(), DurableReceiptPoll::Closed));
        let snapshot = progress.snapshot();
        assert_eq!(snapshot.records_enqueued, 1);
        assert_eq!(snapshot.records_outstanding, 1);
        assert_eq!(snapshot.queue_depth, 0);
        assert_eq!(snapshot.dropped_records, 1);
    }
}
