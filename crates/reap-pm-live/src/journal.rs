//! PM-specific mutation journal and bounded recovery.
//!
//! This schema deliberately shares only leased writer mechanics with the
//! existing Chaos journal. Its tuple family, version, scope fingerprint and
//! record union are separate, so neither product can interpret the other's
//! durable bytes as mutation authority.

mod evidence_hash;
mod recovery;
mod schema;
mod writer;

use std::path::{Path, PathBuf};

use reap_durable_writer::{
    DurableAcknowledgement, DurableReceipt, DurableReceiptPoll, DurableWriterConfig,
    DurableWriterRuntime, EnqueueError, start_durable_writer_with_lease,
};
use thiserror::Error;

use evidence_hash::PmSealedJournalSegment;
pub(crate) use recovery::{
    PmJournalRecoveredObservationV1, PmJournalRecoveredOrderV1, PmJournalRecoveredPlaceV1,
};
pub use recovery::{PmJournalRecovery, recover_pm_mutation_journal};
pub(crate) use schema::PmJournalLineV1;
pub(crate) use schema::derive_pm_journal_client_order_from_fingerprint;
use schema::next_sequence;
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
use writer::{PmJournalCodec, PmJournalCodecError};

pub(crate) const PM_JOURNAL_PENDING_CAPACITY: usize = 1_024;
const PM_SEALED_JOURNAL_RECORD_KINDS: usize = 9;

#[derive(Debug, Error)]
pub enum PmJournalError {
    #[error("PM mutation journal IO failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("PM mutation journal schema failed: {0}")]
    Schema(#[from] schema::PmJournalSchemaError),
    #[error("PM mutation journal recovery failed: {0}")]
    Recovery(#[from] recovery::PmJournalRecoveryError),
    #[error("PM mutation journal recovery task failed: {0}")]
    RecoveryTask(#[source] tokio::task::JoinError),
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
    runtime: PmJournalRuntime,
    scope: PmJournalScopeV1,
    next_sequence: u64,
}

enum PmJournalRuntime {
    Durable(DurableWriterRuntime<PmJournalLineV1, PmJournalCodec>),
    SealedEvidence(Box<PmSealedJournalLedger>),
}

impl PmMutationJournal {
    pub(crate) async fn start(
        path: PathBuf,
        expected_scope: PmJournalScopeV1,
    ) -> Result<(Self, PmJournalRecovery), PmJournalError> {
        let lease = reap_durable_writer::DurableLease::acquire(&path)?;
        let recovery_path = lease.journal_path().to_path_buf();
        let recovery_scope = expected_scope.clone();
        // Recovery is a cold, bounded synchronous decode. Keep its derived
        // fixed-record serde frames off the product-start poll stack while
        // moving the already-acquired lease with it, so cancellation can
        // never expose an unleased recovery window.
        let (lease, recovery) = tokio::task::spawn_blocking(move || {
            let recovery = recovery::recover_with_lease_path(&recovery_path, &recovery_scope);
            (lease, recovery)
        })
        .await
        .map_err(PmJournalError::RecoveryTask)?;
        let recovery = recovery?;
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
                runtime: PmJournalRuntime::Durable(runtime),
                scope: expected_scope,
                next_sequence,
            },
            recovery,
        ))
    }

    /// Constructs the one benchmark-only journal used by the fixed opaque
    /// action-path runner. It is deliberately not a backend selector: the
    /// caller supplies neither records nor sequence/acknowledgement facts.
    pub(crate) fn start_sealed_evidence(expected_scope: PmJournalScopeV1) -> Self {
        let ledger = PmSealedJournalLedger::new(&expected_scope);
        Self {
            runtime: PmJournalRuntime::SealedEvidence(Box::new(ledger)),
            scope: expected_scope,
            next_sequence: 1,
        }
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
        let sequence = self.next_sequence;
        let receipt = match &mut self.runtime {
            PmJournalRuntime::Durable(runtime) => {
                let reservation = runtime
                    .sink()
                    .try_reserve_durable()
                    .map_err(map_enqueue_error)?;
                PmJournalPendingReceipt::Durable(reservation.commit(PmJournalLineV1::new(
                    self.scope.fingerprint(),
                    sequence,
                    record,
                )))
            }
            PmJournalRuntime::SealedEvidence(ledger) => {
                ledger.commit(sequence, &record);
                PmJournalPendingReceipt::SealedEvidence(PmSealedJournalReceipt { _private: () })
            }
        };
        self.next_sequence = next_sequence(sequence)?;
        Ok(PmPendingJournalRecord { sequence, receipt })
    }

    pub(crate) async fn shutdown(self) -> Result<(), PmJournalError> {
        if let PmJournalRuntime::Durable(runtime) = self.runtime {
            runtime.shutdown().await.map_err(map_writer_error)?;
        }
        Ok(())
    }

    pub(crate) fn sealed_evidence_projection(&self) -> Option<PmSealedJournalProjection> {
        match &self.runtime {
            PmJournalRuntime::Durable(_) => None,
            PmJournalRuntime::SealedEvidence(ledger) => Some(ledger.projection()),
        }
    }

    pub(crate) fn begin_sealed_evidence_segment(&mut self) -> bool {
        match &mut self.runtime {
            PmJournalRuntime::Durable(_) => false,
            PmJournalRuntime::SealedEvidence(ledger) => {
                ledger.begin_segment();
                true
            }
        }
    }

    pub(crate) fn reserved_capacity_bytes(&self) -> usize {
        match &self.runtime {
            PmJournalRuntime::Durable(_) => 0,
            PmJournalRuntime::SealedEvidence(_) => std::mem::size_of::<PmSealedJournalLedger>(),
        }
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
    receipt: PmJournalPendingReceipt,
}

enum PmJournalPendingReceipt {
    Durable(DurableReceipt),
    SealedEvidence(PmSealedJournalReceipt),
}

struct PmSealedJournalReceipt {
    _private: (),
}

impl PmPendingJournalRecord {
    #[cfg(test)]
    pub(crate) const fn phase6_pending_for_age_evidence() -> Self {
        Self {
            sequence: 1,
            receipt: PmJournalPendingReceipt::SealedEvidence(PmSealedJournalReceipt {
                _private: (),
            }),
        }
    }

    pub(crate) fn poll(self) -> PmJournalReceiptPoll {
        let sequence = self.sequence;
        match self.receipt {
            PmJournalPendingReceipt::Durable(receipt) => match receipt.try_result() {
                DurableReceiptPoll::Pending(receipt) => PmJournalReceiptPoll::Pending(Self {
                    sequence,
                    receipt: PmJournalPendingReceipt::Durable(receipt),
                }),
                DurableReceiptPoll::Acknowledged(acknowledgement) => {
                    PmJournalReceiptPoll::Acknowledged(PmJournalAcknowledged {
                        sequence,
                        acknowledgement: PmJournalAcknowledgement::Durable(acknowledgement),
                    })
                }
                DurableReceiptPoll::Failed(message) => PmJournalReceiptPoll::Failed(message),
                DurableReceiptPoll::Closed => PmJournalReceiptPoll::Closed,
            },
            PmJournalPendingReceipt::SealedEvidence(receipt) => {
                PmJournalReceiptPoll::Acknowledged(PmJournalAcknowledged {
                    sequence,
                    acknowledgement: PmJournalAcknowledgement::SealedEvidence(receipt),
                })
            }
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
    acknowledgement: PmJournalAcknowledgement,
}

enum PmJournalAcknowledgement {
    Durable(DurableAcknowledgement),
    SealedEvidence(PmSealedJournalReceipt),
}

impl PmJournalAcknowledged {
    pub(crate) fn consume(self) -> u64 {
        let Self {
            sequence,
            acknowledgement,
        } = self;
        match acknowledgement {
            PmJournalAcknowledgement::Durable(acknowledgement) => drop(acknowledgement),
            PmJournalAcknowledgement::SealedEvidence(_receipt) => {}
        }
        sequence
    }
}

/// Fixed read-only projection of the private benchmark ledger. The ledger
/// never retains a caller-owned record and cannot mint an acknowledgement
/// outside the normal pending-receipt path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PmSealedJournalProjection {
    record_count: u64,
    records_by_kind: PmSealedJournalRecordCounts,
    last_sequence: u64,
    segment_record_count: u64,
    segment_records_by_kind: PmSealedJournalRecordCounts,
    segment_hash: [u8; 32],
    segment_valid: bool,
}

impl PmSealedJournalProjection {
    pub(crate) const fn record_count(self) -> u64 {
        self.record_count
    }

    pub(crate) const fn records_by_kind(self) -> PmSealedJournalRecordCounts {
        self.records_by_kind
    }

    pub(crate) const fn last_sequence(self) -> u64 {
        self.last_sequence
    }

    pub(crate) const fn segment_record_count(self) -> u64 {
        self.segment_record_count
    }

    pub(crate) const fn segment_records_by_kind(self) -> PmSealedJournalRecordCounts {
        self.segment_records_by_kind
    }

    pub(crate) const fn segment_hash(self) -> [u8; 32] {
        self.segment_hash
    }

    pub(crate) const fn segment_valid(self) -> bool {
        self.segment_valid
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PmSealedJournalRecordCounts {
    pub(crate) headers: u64,
    pub(crate) quote_intents: u64,
    pub(crate) place_results: u64,
    pub(crate) cancel_intents: u64,
    pub(crate) cancel_results: u64,
    pub(crate) fills_applied: u64,
    pub(crate) order_terminals: u64,
    pub(crate) safety_halts: u64,
    pub(crate) fill_watermark_advances: u64,
}

impl From<[u64; PM_SEALED_JOURNAL_RECORD_KINDS]> for PmSealedJournalRecordCounts {
    fn from(counts: [u64; PM_SEALED_JOURNAL_RECORD_KINDS]) -> Self {
        Self {
            headers: counts[0],
            quote_intents: counts[1],
            place_results: counts[2],
            cancel_intents: counts[3],
            cancel_results: counts[4],
            fills_applied: counts[5],
            order_terminals: counts[6],
            safety_halts: counts[7],
            fill_watermark_advances: counts[8],
        }
    }
}

struct PmSealedJournalLedger {
    record_count: u64,
    records_by_kind: [u64; PM_SEALED_JOURNAL_RECORD_KINDS],
    last_sequence: u64,
    segment: PmSealedJournalSegment,
}

impl PmSealedJournalLedger {
    fn new(scope: &PmJournalScopeV1) -> Self {
        let mut ledger = Self {
            record_count: 0,
            records_by_kind: [0; PM_SEALED_JOURNAL_RECORD_KINDS],
            last_sequence: 0,
            segment: PmSealedJournalSegment::new(scope.account(), scope.fingerprint()),
        };
        ledger.commit(
            0,
            &PmJournalRecordV1::Header(PmJournalHeaderV1::new(scope.clone())),
        );
        ledger
    }

    fn begin_segment(&mut self) {
        self.segment.reset();
    }

    fn commit(&mut self, sequence: u64, record: &PmJournalRecordV1) {
        let index = sealed_record_index(record);
        self.record_count = self.record_count.saturating_add(1);
        self.records_by_kind[index] = self.records_by_kind[index].saturating_add(1);
        self.last_sequence = sequence;
        self.segment.observe(sequence, index, record);
    }

    fn projection(&self) -> PmSealedJournalProjection {
        PmSealedJournalProjection {
            record_count: self.record_count,
            records_by_kind: self.records_by_kind.into(),
            last_sequence: self.last_sequence,
            segment_record_count: self.segment.record_count(),
            segment_records_by_kind: self.segment.records_by_kind().into(),
            segment_hash: self.segment.hash(),
            segment_valid: self.segment.valid(),
        }
    }
}

const fn sealed_record_index(record: &PmJournalRecordV1) -> usize {
    match record {
        PmJournalRecordV1::Header(_) => 0,
        PmJournalRecordV1::QuoteIntent(_) => 1,
        PmJournalRecordV1::PlaceResult(_) => 2,
        PmJournalRecordV1::CancelIntent(_) => 3,
        PmJournalRecordV1::CancelResult(_) => 4,
        PmJournalRecordV1::FillApplied(_) => 5,
        PmJournalRecordV1::OrderTerminal(_) => 6,
        PmJournalRecordV1::SafetyHalt(_) => 7,
        PmJournalRecordV1::FillWatermarkAdvanced(_) => 8,
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
