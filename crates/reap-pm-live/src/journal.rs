//! PM-specific mutation journal and bounded recovery.
//!
//! This schema deliberately shares only leased writer mechanics with the
//! existing Chaos journal. Its tuple family, version, scope fingerprint and
//! record union are separate, so neither product can interpret the other's
//! durable bytes as mutation authority.

mod recovery;
mod schema;
mod writer;

use std::path::{Path, PathBuf};

use reap_durable_writer::{
    DurableAcknowledgement, DurableReceipt, DurableReceiptPoll, DurableWriterConfig,
    DurableWriterRuntime, EnqueueError, start_durable_writer_with_lease,
};
use thiserror::Error;

pub(crate) use recovery::{
    PmJournalRecoveredObservationV1, PmJournalRecoveredOrderV1, PmJournalRecoveredPlaceV1,
};
pub use recovery::{PmJournalRecovery, recover_pm_mutation_journal};
pub use schema::{
    MAX_PM_JOURNAL_BYTES, MAX_PM_JOURNAL_LINE_BYTES, MAX_PM_JOURNAL_RECORDS,
    PM_MUTATION_JOURNAL_FAMILY, PM_MUTATION_JOURNAL_VERSION, PmJournalCancelIntentV1,
    PmJournalCancelOutcomeV1, PmJournalCancelReasonV1, PmJournalCancelRejectReasonV1,
    PmJournalCancelResultV1, PmJournalFillAppliedV1, PmJournalFillCursorV1,
    PmJournalFillDeliveryV1, PmJournalFillFeeV1, PmJournalFillKeyV1, PmJournalFillOccurrenceV1,
    PmJournalFillRoleV1, PmJournalFillSettlementV1, PmJournalFillSourceV1, PmJournalFillV1,
    PmJournalFillWatermarkV1, PmJournalFingerprintV1, PmJournalHeaderV1, PmJournalImmediateFillsV1,
    PmJournalOrderProgressSourceV1, PmJournalOrderTerminalV1, PmJournalPlaceOutcomeV1,
    PmJournalPlaceRejectReasonV1, PmJournalPlaceResultV1, PmJournalQuoteIntentV1,
    PmJournalQuoteProfileV1, PmJournalRecordV1, PmJournalSafetyHaltV1, PmJournalSafetyReasonV1,
    PmJournalSchemaError, PmJournalScopeV1, PmJournalSideV1, PmJournalTerminalStatusV1,
};
use schema::{PmJournalLineV1, next_sequence};
use writer::{PmJournalCodec, PmJournalCodecError};

pub(crate) const PM_JOURNAL_PENDING_CAPACITY: usize = 1_024;

#[derive(Debug, Error)]
pub enum PmJournalError {
    #[error("PM mutation journal IO failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("PM mutation journal schema failed: {0}")]
    Schema(#[from] schema::PmJournalSchemaError),
    #[error("PM mutation journal recovery failed: {0}")]
    Recovery(#[from] recovery::PmJournalRecoveryError),
    #[error("PM mutation journal lease failed: {0}")]
    Lease(#[from] reap_durable_writer::LeaseError),
    #[error("PM mutation journal writer failed: {0}")]
    Writer(String),
    #[error("PM mutation journal queue is full")]
    QueueFull,
    #[error("PM mutation journal writer is closed")]
    WriterClosed,
    #[error("PM mutation journal durable acknowledgement failed: {0}")]
    Durability(String),
    #[error("PM mutation journal sequence is exhausted")]
    SequenceExhausted,
    #[error("PM mutation journal record bound is exhausted")]
    RecordLimit,
}

/// The leased PM journal writer paired with its checked recovery cut.
///
/// Construction recovers while holding the exact lease and checks the header
/// before opening the append writer. The runtime is crate-private because
/// accepting a record is part of the product's mutation authority.
pub(crate) struct PmMutationJournal {
    runtime: DurableWriterRuntime<PmJournalLineV1, PmJournalCodec>,
    scope: PmJournalScopeV1,
    next_sequence: u64,
}

impl PmMutationJournal {
    pub(crate) async fn start(
        path: PathBuf,
        expected_scope: PmJournalScopeV1,
    ) -> Result<(Self, PmJournalRecovery), PmJournalError> {
        let lease = reap_durable_writer::DurableLease::acquire(&path)?;
        let recovery = recovery::recover_with_lease_path(lease.journal_path(), &expected_scope)?;
        let existing_bytes = std::fs::metadata(lease.journal_path())
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let empty = recovery.record_count() == 0;
        let next_sequence = if empty {
            1
        } else {
            next_sequence(recovery.last_sequence())?
        };
        let config = DurableWriterConfig {
            path,
            channel_capacity: PM_JOURNAL_PENDING_CAPACITY,
            flush_every_records: 1,
        };
        let mut runtime =
            start_durable_writer_with_lease(config, lease, PmJournalCodec::new(existing_bytes))
                .await
                .map_err(map_writer_error)?;
        if empty {
            let header = PmJournalLineV1::new(
                expected_scope.fingerprint(),
                0,
                PmJournalRecordV1::Header(PmJournalHeaderV1::new(expected_scope.clone())),
            );
            if let Err(error) = runtime.sink().enqueue_durable(header).await {
                let _ = runtime.stop_writer().await;
                return Err(map_enqueue_error(error));
            }
        }
        Ok((
            Self {
                runtime,
                scope: expected_scope,
                next_sequence,
            },
            recovery,
        ))
    }

    pub(crate) fn try_record(
        &mut self,
        record: PmJournalRecordV1,
    ) -> Result<PmPendingJournalRecord, PmJournalError> {
        if matches!(
            record,
            PmJournalRecordV1::Header(_)
                | PmJournalRecordV1::QuoteIntent(_)
                | PmJournalRecordV1::CancelIntent(_)
        ) {
            return Err(schema::PmJournalSchemaError::WrongRecordPath.into());
        }
        record.validate(&self.scope)?;
        self.try_record_inner(record)
    }

    pub(crate) fn try_quote_intent(
        &mut self,
        intent: schema::PmJournalQuoteIntentV1,
    ) -> Result<PmPendingQuoteIntent, PmJournalError> {
        let pending = self.try_record_inner(PmJournalRecordV1::QuoteIntent(intent))?;
        Ok(PmPendingQuoteIntent { pending, intent })
    }

    pub(crate) fn try_cancel_intent(
        &mut self,
        intent: schema::PmJournalCancelIntentV1,
    ) -> Result<PmPendingCancelIntent, PmJournalError> {
        let pending = self.try_record_inner(PmJournalRecordV1::CancelIntent(intent))?;
        Ok(PmPendingCancelIntent { pending, intent })
    }

    fn try_record_inner(
        &mut self,
        record: PmJournalRecordV1,
    ) -> Result<PmPendingJournalRecord, PmJournalError> {
        record.validate(&self.scope)?;
        if usize::try_from(self.next_sequence)
            .map_or(true, |sequence| sequence >= MAX_PM_JOURNAL_RECORDS)
        {
            return Err(PmJournalError::RecordLimit);
        }
        let reservation = self
            .runtime
            .sink()
            .try_reserve_durable()
            .map_err(map_enqueue_error)?;
        let sequence = self.next_sequence;
        self.next_sequence = next_sequence(sequence)?;
        let receipt = reservation.commit(PmJournalLineV1::new(
            self.scope.fingerprint(),
            sequence,
            record,
        ));
        Ok(PmPendingJournalRecord { sequence, receipt })
    }

    pub(crate) async fn shutdown(self) -> Result<(), PmJournalError> {
        self.runtime.shutdown().await.map_err(map_writer_error)?;
        Ok(())
    }
}

fn map_writer_error(
    error: reap_durable_writer::WriterError<PmJournalCodecError>,
) -> PmJournalError {
    PmJournalError::Writer(error.to_string())
}

fn map_enqueue_error(error: EnqueueError) -> PmJournalError {
    match error {
        EnqueueError::Full => PmJournalError::QueueFull,
        EnqueueError::Closed => PmJournalError::WriterClosed,
        EnqueueError::Durability(message) => PmJournalError::Durability(message),
    }
}

/// Move-only correlation between one exact record and its durability result.
pub(crate) struct PmPendingJournalRecord {
    sequence: u64,
    receipt: DurableReceipt,
}

impl PmPendingJournalRecord {
    pub(crate) fn poll(self) -> PmJournalReceiptPoll {
        let sequence = self.sequence;
        match self.receipt.try_result() {
            DurableReceiptPoll::Pending(receipt) => {
                PmJournalReceiptPoll::Pending(Self { sequence, receipt })
            }
            DurableReceiptPoll::Acknowledged(acknowledgement) => {
                PmJournalReceiptPoll::Acknowledged(PmJournalAcknowledged {
                    sequence,
                    acknowledgement,
                })
            }
            DurableReceiptPoll::Failed(message) => PmJournalReceiptPoll::Failed(message),
            DurableReceiptPoll::Closed => PmJournalReceiptPoll::Closed,
        }
    }
}

pub(crate) enum PmJournalReceiptPoll {
    Pending(PmPendingJournalRecord),
    Acknowledged(PmJournalAcknowledged),
    Failed(String),
    Closed,
}

/// Product-local evidence that the exact paired PM record reached durable
/// storage. It remains move-only and cannot be forged outside this module.
pub(crate) struct PmJournalAcknowledged {
    sequence: u64,
    acknowledgement: DurableAcknowledgement,
}

impl PmJournalAcknowledged {
    pub(crate) fn consume(self) -> u64 {
        let Self {
            sequence,
            acknowledgement,
        } = self;
        drop(acknowledgement);
        sequence
    }
}

/// Exact quote intent retained alongside its move-only writer receipt.
pub(crate) struct PmPendingQuoteIntent {
    pending: PmPendingJournalRecord,
    intent: schema::PmJournalQuoteIntentV1,
}

impl PmPendingQuoteIntent {
    pub(crate) fn poll(self) -> PmQuoteIntentReceiptPoll {
        let Self { pending, intent } = self;
        match pending.poll() {
            PmJournalReceiptPoll::Pending(pending) => {
                PmQuoteIntentReceiptPoll::Pending(Self { pending, intent })
            }
            PmJournalReceiptPoll::Acknowledged(acknowledged) => {
                let sequence = acknowledged.consume();
                PmQuoteIntentReceiptPoll::Acknowledged(PmQuoteIntentDurablyAcknowledged {
                    sequence,
                    intent,
                    _private: (),
                })
            }
            PmJournalReceiptPoll::Failed(message) => PmQuoteIntentReceiptPoll::Failed(message),
            PmJournalReceiptPoll::Closed => PmQuoteIntentReceiptPoll::Closed,
        }
    }
}

pub(crate) enum PmQuoteIntentReceiptPoll {
    Pending(PmPendingQuoteIntent),
    Acknowledged(PmQuoteIntentDurablyAcknowledged),
    Failed(String),
    Closed,
}

/// Take-once durable evidence for one exact quote intent.
pub(crate) struct PmQuoteIntentDurablyAcknowledged {
    sequence: u64,
    intent: schema::PmJournalQuoteIntentV1,
    _private: (),
}

impl PmQuoteIntentDurablyAcknowledged {
    pub(crate) const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub(crate) const fn intent(&self) -> schema::PmJournalQuoteIntentV1 {
        self.intent
    }
}

/// Exact cancel intent retained alongside its move-only writer receipt.
pub(crate) struct PmPendingCancelIntent {
    pending: PmPendingJournalRecord,
    intent: schema::PmJournalCancelIntentV1,
}

impl PmPendingCancelIntent {
    pub(crate) fn poll(self) -> PmCancelIntentReceiptPoll {
        let Self { pending, intent } = self;
        match pending.poll() {
            PmJournalReceiptPoll::Pending(pending) => {
                PmCancelIntentReceiptPoll::Pending(Self { pending, intent })
            }
            PmJournalReceiptPoll::Acknowledged(acknowledged) => {
                let sequence = acknowledged.consume();
                PmCancelIntentReceiptPoll::Acknowledged(PmCancelIntentDurablyAcknowledged {
                    sequence,
                    intent,
                    _private: (),
                })
            }
            PmJournalReceiptPoll::Failed(message) => PmCancelIntentReceiptPoll::Failed(message),
            PmJournalReceiptPoll::Closed => PmCancelIntentReceiptPoll::Closed,
        }
    }
}

pub(crate) enum PmCancelIntentReceiptPoll {
    Pending(PmPendingCancelIntent),
    Acknowledged(PmCancelIntentDurablyAcknowledged),
    Failed(String),
    Closed,
}

/// Take-once durable evidence for one exact cancel intent.
pub(crate) struct PmCancelIntentDurablyAcknowledged {
    sequence: u64,
    intent: schema::PmJournalCancelIntentV1,
    _private: (),
}

impl PmCancelIntentDurablyAcknowledged {
    pub(crate) const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub(crate) const fn intent(&self) -> schema::PmJournalCancelIntentV1 {
        self.intent
    }
}

#[allow(dead_code, reason = "used by the Phase 5 journal contract tests")]
fn journal_exists(path: &Path) -> bool {
    path.exists()
}
