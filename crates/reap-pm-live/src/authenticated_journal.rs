//! Separate durability boundary for authenticated PM application writes.
//!
//! This family contains only non-secret commitments and typed transition
//! identities. It deliberately neither changes nor aliases the Goal F PM
//! mutation-journal schema.

#![allow(
    dead_code,
    reason = "the Phase 4 journal lands before its authenticated composition owner"
)]

mod recovery;
mod schema;
mod writer;

use std::{path::PathBuf, sync::Arc};

use reap_durable_writer::{
    DurableAcknowledgement, DurableReceipt, DurableReceiptPoll, DurableWriterConfig,
    DurableWriterRuntime, EnqueueError, start_durable_writer_with_lease,
};
use thiserror::Error;

#[allow(
    unused_imports,
    reason = "the crate-private Phase 4 recovery seam lands before coordinator integration"
)]
pub(crate) use recovery::{
    PmAuthenticatedJournalRecovery, PmAuthenticatedJournalRecoveryError,
    PmAuthenticatedPreparedWithoutGrantV1, PmAuthenticatedRecoveredResultClassificationV1,
    PmAuthenticatedRecoveredResultV1, PmAuthenticatedUnresolvedOperationKindV1,
    PmAuthenticatedUnresolvedOperationV1, PmAuthenticatedUnresolvedReasonV1,
};
use schema::{
    MAX_PM_AUTHENTICATED_JOURNAL_RECORDS, PmAuthenticatedJournalHeaderV1,
    PmAuthenticatedJournalLineV1, next_sequence,
};
#[allow(
    unused_imports,
    reason = "the crate-private Phase 4 record seam lands before coordinator integration"
)]
pub(crate) use schema::{
    PmAuthenticatedCancelPreparedV1, PmAuthenticatedCancelResultKindV1,
    PmAuthenticatedCancelResultV1, PmAuthenticatedCoordinatorIdentityV1,
    PmAuthenticatedDispatchAuthorizedV1, PmAuthenticatedJournalRecordV1,
    PmAuthenticatedJournalSchemaError, PmAuthenticatedJournalScopeV1,
    PmAuthenticatedOperationKeyV1, PmAuthenticatedPlacePreparedV1,
    PmAuthenticatedPlaceResultKindV1, PmAuthenticatedPlaceResultV1,
};
#[cfg(test)]
use writer::PmAuthenticatedJournalTestControl;
#[cfg(test)]
pub(crate) use writer::PmAuthenticatedJournalWriteLatch;
use writer::{PmAuthenticatedJournalCodec, PmAuthenticatedJournalCodecError};

const PM_AUTHENTICATED_PENDING_CAPACITY: usize = 512;

#[derive(Debug, Error)]
pub(crate) enum PmAuthenticatedJournalError {
    #[error("PM authenticated journal IO failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("PM authenticated journal schema failed: {0}")]
    Schema(#[from] PmAuthenticatedJournalSchemaError),
    #[error("PM authenticated journal recovery failed: {0}")]
    Recovery(#[from] PmAuthenticatedJournalRecoveryError),
    #[error("PM authenticated journal recovery task failed: {0}")]
    RecoveryTask(#[source] tokio::task::JoinError),
    #[error("PM authenticated journal lease failed: {0}")]
    Lease(#[from] reap_durable_writer::LeaseError),
    #[error("PM authenticated journal writer failed: {0}")]
    Writer(String),
    #[error("PM authenticated journal queue is full")]
    QueueFull,
    #[error("PM authenticated journal writer is closed")]
    WriterClosed,
    #[error("PM authenticated journal durable acknowledgement failed: {0}")]
    Durability(String),
    #[error("PM authenticated journal record bound is exhausted")]
    RecordLimit,
    #[error(
        "PM authenticated dispatch authorization requires the exact durable prepared acknowledgement"
    )]
    DispatchAuthorizationRequiresDurablePrepared,
    #[error("PM authenticated result admission requires its exact durable send grant")]
    ResultRequiresExactDurableGrant,
    #[error("PM authenticated result does not match its durable send grant")]
    ResultDoesNotMatchDurableGrant,
    #[error("PM authenticated durable proof belongs to a different journal runtime")]
    DurableProofFromDifferentRuntime,
}

/// Process-local, non-serializable identity of one held journal lease/runtime.
/// Durable transition tokens retain this marker so equal scopes and sequence
/// numbers from two different files can never be composed before a send.
struct PmAuthenticatedJournalRuntimeIdentity;

/// Failed place-result admission that retains the sole durable send grant and
/// exact classified result. Callers can quarantine or retry only the journal
/// append without reconstructing either proof.
pub(crate) struct PmAuthenticatedPlaceResultAdmissionError {
    error: PmAuthenticatedJournalError,
    retained: Box<(PmAuthenticatedPlaceSendGrant, PmAuthenticatedPlaceResultV1)>,
}

impl PmAuthenticatedPlaceResultAdmissionError {
    pub(crate) const fn error(&self) -> &PmAuthenticatedJournalError {
        &self.error
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        PmAuthenticatedJournalError,
        PmAuthenticatedPlaceSendGrant,
        PmAuthenticatedPlaceResultV1,
    ) {
        let (grant, result) = *self.retained;
        (self.error, grant, result)
    }
}

impl std::fmt::Debug for PmAuthenticatedPlaceResultAdmissionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmAuthenticatedPlaceResultAdmissionError")
            .field("error", &self.error)
            .field("grant", &self.retained.0)
            .field("result", &self.retained.1)
            .finish()
    }
}

impl std::fmt::Display for PmAuthenticatedPlaceResultAdmissionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for PmAuthenticatedPlaceResultAdmissionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// Failed cancel-result admission retaining the exact owned-order send grant
/// and classification. It is deliberately move-only.
pub(crate) struct PmAuthenticatedCancelResultAdmissionError {
    error: PmAuthenticatedJournalError,
    retained: Box<(
        PmAuthenticatedCancelSendGrant,
        PmAuthenticatedCancelResultV1,
    )>,
}

impl PmAuthenticatedCancelResultAdmissionError {
    pub(crate) const fn error(&self) -> &PmAuthenticatedJournalError {
        &self.error
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        PmAuthenticatedJournalError,
        PmAuthenticatedCancelSendGrant,
        PmAuthenticatedCancelResultV1,
    ) {
        let (grant, result) = *self.retained;
        (self.error, grant, result)
    }
}

impl std::fmt::Debug for PmAuthenticatedCancelResultAdmissionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmAuthenticatedCancelResultAdmissionError")
            .field("error", &self.error)
            .field("grant", &self.retained.0)
            .field("result", &self.retained.1)
            .finish()
    }
}

impl std::fmt::Display for PmAuthenticatedCancelResultAdmissionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for PmAuthenticatedCancelResultAdmissionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// Exact place classification whose append was already admitted but whose
/// durability acknowledgement was not obtained. This type deliberately does
/// not expose the contained send grant, so a caller cannot reuse it to append
/// a second result or authorize another network write.
pub(crate) struct PmAuthenticatedPlaceResultUnresolved {
    sequence: u64,
    grant: PmAuthenticatedPlaceSendGrant,
    result: PmAuthenticatedPlaceResultV1,
}

impl PmAuthenticatedPlaceResultUnresolved {
    fn new(
        sequence: u64,
        grant: PmAuthenticatedPlaceSendGrant,
        result: PmAuthenticatedPlaceResultV1,
    ) -> Self {
        Self {
            sequence,
            grant,
            result,
        }
    }

    pub(crate) const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub(crate) const fn grant_sequence(&self) -> u64 {
        self.grant.grant_sequence()
    }

    pub(crate) const fn client_order(&self) -> reap_pm_core::PmClientOrderKey {
        self.grant.client_order()
    }

    pub(crate) const fn outcome(&self) -> PmAuthenticatedPlaceResultKindV1 {
        self.result.outcome()
    }

    pub(crate) const fn observed_order_id(&self) -> Option<[u8; 32]> {
        self.result.observed_order_id()
    }
}

impl std::fmt::Debug for PmAuthenticatedPlaceResultUnresolved {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmAuthenticatedPlaceResultUnresolved")
            .field("sequence", &self.sequence)
            .field("grant", &self.grant)
            .field("result", &self.result)
            .finish()
    }
}

/// Exact owned-cancel classification whose admitted append has no observed
/// durability acknowledgement. The embedded grant is intentionally sealed.
pub(crate) struct PmAuthenticatedCancelResultUnresolved {
    sequence: u64,
    grant: PmAuthenticatedCancelSendGrant,
    result: PmAuthenticatedCancelResultV1,
}

impl PmAuthenticatedCancelResultUnresolved {
    fn new(
        sequence: u64,
        grant: PmAuthenticatedCancelSendGrant,
        result: PmAuthenticatedCancelResultV1,
    ) -> Self {
        Self {
            sequence,
            grant,
            result,
        }
    }

    pub(crate) const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub(crate) const fn grant_sequence(&self) -> u64 {
        self.grant.grant_sequence()
    }

    pub(crate) const fn client_order(&self) -> reap_pm_core::PmClientOrderKey {
        self.grant.client_order()
    }

    pub(crate) const fn outcome(&self) -> PmAuthenticatedCancelResultKindV1 {
        self.result.outcome()
    }

    pub(crate) const fn observed_order_id(&self) -> Option<[u8; 32]> {
        self.result.observed_order_id()
    }
}

impl std::fmt::Debug for PmAuthenticatedCancelResultUnresolved {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmAuthenticatedCancelResultUnresolved")
            .field("sequence", &self.sequence)
            .field("grant", &self.grant)
            .field("result", &self.result)
            .finish()
    }
}

pub(crate) struct PmAuthenticatedMutationJournal {
    runtime: DurableWriterRuntime<PmAuthenticatedJournalLineV1, PmAuthenticatedJournalCodec>,
    runtime_identity: Arc<PmAuthenticatedJournalRuntimeIdentity>,
    #[cfg(test)]
    test_control: Arc<PmAuthenticatedJournalTestControl>,
    scope: PmAuthenticatedJournalScopeV1,
    next_sequence: u64,
}

impl PmAuthenticatedMutationJournal {
    pub(crate) async fn start(
        path: PathBuf,
        expected_scope: PmAuthenticatedJournalScopeV1,
    ) -> Result<(Self, PmAuthenticatedJournalRecovery), PmAuthenticatedJournalError> {
        let lease = reap_durable_writer::DurableLease::acquire(&path)?;
        let recovery_path = lease.journal_path().to_path_buf();
        let recovery_scope = expected_scope.clone();
        let (lease, recovery) = tokio::task::spawn_blocking(move || {
            let recovery = recovery::recover_with_lease_path(&recovery_path, &recovery_scope);
            (lease, recovery)
        })
        .await
        .map_err(PmAuthenticatedJournalError::RecoveryTask)?;
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
            channel_capacity: PM_AUTHENTICATED_PENDING_CAPACITY,
            flush_every_records: 1,
        };
        #[cfg(test)]
        let test_control = Arc::new(PmAuthenticatedJournalTestControl::new());
        #[cfg(test)]
        let codec = PmAuthenticatedJournalCodec::new(existing_bytes, Arc::clone(&test_control));
        #[cfg(not(test))]
        let codec = PmAuthenticatedJournalCodec::new(existing_bytes);
        let mut runtime = start_durable_writer_with_lease(config, lease, codec)
            .await
            .map_err(map_writer_error)?;
        if empty {
            let header = PmAuthenticatedJournalLineV1::new_for_scope(
                &expected_scope,
                0,
                PmAuthenticatedJournalRecordV1::Header(PmAuthenticatedJournalHeaderV1::new(
                    expected_scope.clone(),
                )),
            );
            if let Err(error) = runtime.sink().enqueue_durable(header).await {
                let _ = runtime.stop_writer().await;
                return Err(map_enqueue_error(error));
            }
        }
        Ok((
            Self {
                runtime,
                runtime_identity: Arc::new(PmAuthenticatedJournalRuntimeIdentity),
                #[cfg(test)]
                test_control,
                scope: expected_scope,
                next_sequence,
            },
            recovery,
        ))
    }

    pub(crate) fn try_record(
        &mut self,
        record: PmAuthenticatedJournalRecordV1,
    ) -> Result<PmAuthenticatedPendingRecord, PmAuthenticatedJournalError> {
        if matches!(record, PmAuthenticatedJournalRecordV1::Header(_)) {
            return Err(PmAuthenticatedJournalSchemaError::HeaderAfterStart.into());
        }
        match record {
            PmAuthenticatedJournalRecordV1::DispatchAuthorized(_) => {
                return Err(
                    PmAuthenticatedJournalError::DispatchAuthorizationRequiresDurablePrepared,
                );
            }
            PmAuthenticatedJournalRecordV1::PlaceResult(_)
            | PmAuthenticatedJournalRecordV1::CancelResult(_) => {
                return Err(PmAuthenticatedJournalError::ResultRequiresExactDurableGrant);
            }
            PmAuthenticatedJournalRecordV1::Header(_)
            | PmAuthenticatedJournalRecordV1::PlacePrepared(_)
            | PmAuthenticatedJournalRecordV1::CancelPrepared(_) => {}
        }
        let kind = match &record {
            PmAuthenticatedJournalRecordV1::PlacePrepared(prepared) => {
                PmAuthenticatedPendingKind::Prepared(PmAuthenticatedPreparedRecord::Place(
                    *prepared,
                ))
            }
            PmAuthenticatedJournalRecordV1::CancelPrepared(prepared) => {
                PmAuthenticatedPendingKind::Prepared(PmAuthenticatedPreparedRecord::Cancel(
                    *prepared,
                ))
            }
            PmAuthenticatedJournalRecordV1::Header(_)
            | PmAuthenticatedJournalRecordV1::DispatchAuthorized(_)
            | PmAuthenticatedJournalRecordV1::PlaceResult(_)
            | PmAuthenticatedJournalRecordV1::CancelResult(_) => {
                unreachable!("restricted record paths were rejected before reservation")
            }
        };
        self.try_record_with_kind(record, kind)
    }

    #[cfg(test)]
    pub(crate) fn fail_next_write_for_test(&self) {
        self.test_control.fail_next();
    }

    #[cfg(test)]
    pub(crate) fn close_next_write_for_test(&self) {
        self.test_control.close_next();
    }

    #[cfg(test)]
    pub(crate) fn delay_next_write_for_test(&self) {
        self.test_control.delay_next();
    }

    #[cfg(test)]
    pub(crate) fn latch_next_write_for_test(&self) -> PmAuthenticatedJournalWriteLatch {
        self.test_control.latch_next_write()
    }

    /// Admits a place result only by consuming the exact durable send grant.
    /// Every failure before an append returns both move-only correlation proof
    /// and classified result to the caller.
    pub(crate) fn try_record_place_result(
        &mut self,
        grant: PmAuthenticatedPlaceSendGrant,
        result: PmAuthenticatedPlaceResultV1,
    ) -> Result<PmAuthenticatedPendingRecord, PmAuthenticatedPlaceResultAdmissionError> {
        let admission = self.try_reserve_place_result(&grant, result);
        match admission {
            Ok((sequence, receipt)) => Ok(PmAuthenticatedPendingRecord {
                sequence,
                receipt,
                runtime_identity: Arc::clone(&self.runtime_identity),
                kind: PmAuthenticatedPendingKind::PlaceResult { grant, result },
            }),
            Err(error) => Err(PmAuthenticatedPlaceResultAdmissionError {
                error,
                retained: Box::new((grant, result)),
            }),
        }
    }

    /// Admits a cancel result only by consuming the exact durable send grant.
    /// Every failure before an append returns both move-only correlation proof
    /// and classified result to the caller.
    pub(crate) fn try_record_cancel_result(
        &mut self,
        grant: PmAuthenticatedCancelSendGrant,
        result: PmAuthenticatedCancelResultV1,
    ) -> Result<PmAuthenticatedPendingRecord, PmAuthenticatedCancelResultAdmissionError> {
        let admission = self.try_reserve_cancel_result(&grant, result);
        match admission {
            Ok((sequence, receipt)) => Ok(PmAuthenticatedPendingRecord {
                sequence,
                receipt,
                runtime_identity: Arc::clone(&self.runtime_identity),
                kind: PmAuthenticatedPendingKind::CancelResult { grant, result },
            }),
            Err(error) => Err(PmAuthenticatedCancelResultAdmissionError {
                error,
                retained: Box::new((grant, result)),
            }),
        }
    }

    fn try_reserve_place_result(
        &mut self,
        grant: &PmAuthenticatedPlaceSendGrant,
        result: PmAuthenticatedPlaceResultV1,
    ) -> Result<(u64, DurableReceipt), PmAuthenticatedJournalError> {
        if !Arc::ptr_eq(&self.runtime_identity, &grant.runtime_identity) {
            return Err(PmAuthenticatedJournalError::DurableProofFromDifferentRuntime);
        }
        if result.client_order() != grant.client_order()
            || result.instrument() != grant.instrument()
            || result.grant_sequence() != grant.grant_sequence()
            || (result.outcome() == PmAuthenticatedPlaceResultKindV1::Accepted
                && result.observed_order_id() != Some(grant.expected_order_id()))
        {
            return Err(PmAuthenticatedJournalError::ResultDoesNotMatchDurableGrant);
        }
        self.try_reserve_record(PmAuthenticatedJournalRecordV1::PlaceResult(result))
    }

    fn try_reserve_cancel_result(
        &mut self,
        grant: &PmAuthenticatedCancelSendGrant,
        result: PmAuthenticatedCancelResultV1,
    ) -> Result<(u64, DurableReceipt), PmAuthenticatedJournalError> {
        if !Arc::ptr_eq(&self.runtime_identity, &grant.runtime_identity) {
            return Err(PmAuthenticatedJournalError::DurableProofFromDifferentRuntime);
        }
        if result.client_order() != grant.client_order()
            || result.instrument() != grant.instrument()
            || result.venue_order() != grant.venue_order()
            || result.grant_sequence() != grant.grant_sequence()
        {
            return Err(PmAuthenticatedJournalError::ResultDoesNotMatchDurableGrant);
        }
        self.try_reserve_record(PmAuthenticatedJournalRecordV1::CancelResult(result))
    }

    /// Admits the may-have-sent barrier only by consuming the exact durable
    /// Prepared acknowledgement. The private evidence carried through this
    /// path is later attached to the operation-typed send grant.
    pub(crate) fn try_authorize_dispatch(
        &mut self,
        prepared: PmAuthenticatedPreparedAcknowledged,
    ) -> Result<PmAuthenticatedPendingRecord, PmAuthenticatedJournalError> {
        let PmAuthenticatedPreparedAcknowledged {
            sequence: prepared_sequence,
            prepared,
            runtime_identity,
            acknowledgement,
        } = prepared;
        if !Arc::ptr_eq(&self.runtime_identity, &runtime_identity) {
            drop(acknowledgement);
            return Err(PmAuthenticatedJournalError::DurableProofFromDifferentRuntime);
        }
        let operation = prepared.operation();
        let record = PmAuthenticatedJournalRecordV1::DispatchAuthorized(
            PmAuthenticatedDispatchAuthorizedV1::from_durable_prepared(
                operation,
                prepared_sequence,
            ),
        );
        let pending = self.try_record_with_kind(
            record,
            PmAuthenticatedPendingKind::DispatchAuthorized {
                prepared_sequence,
                prepared,
            },
        );
        drop(acknowledgement);
        pending
    }

    fn try_record_with_kind(
        &mut self,
        record: PmAuthenticatedJournalRecordV1,
        kind: PmAuthenticatedPendingKind,
    ) -> Result<PmAuthenticatedPendingRecord, PmAuthenticatedJournalError> {
        let (sequence, receipt) = self.try_reserve_record(record)?;
        Ok(PmAuthenticatedPendingRecord {
            sequence,
            receipt,
            runtime_identity: Arc::clone(&self.runtime_identity),
            kind,
        })
    }

    fn try_reserve_record(
        &mut self,
        record: PmAuthenticatedJournalRecordV1,
    ) -> Result<(u64, DurableReceipt), PmAuthenticatedJournalError> {
        record.validate(&self.scope, self.next_sequence)?;
        if usize::try_from(self.next_sequence).map_or(true, |sequence| {
            sequence >= MAX_PM_AUTHENTICATED_JOURNAL_RECORDS
        }) {
            return Err(PmAuthenticatedJournalError::RecordLimit);
        }
        let sequence = self.next_sequence;
        let following_sequence = next_sequence(sequence)?;
        let reservation = self
            .runtime
            .sink()
            .try_reserve_durable()
            .map_err(map_enqueue_error)?;
        let receipt = reservation.commit(PmAuthenticatedJournalLineV1::new_for_scope(
            &self.scope,
            sequence,
            record,
        ));
        self.next_sequence = following_sequence;
        Ok((sequence, receipt))
    }

    pub(crate) async fn shutdown(self) -> Result<(), PmAuthenticatedJournalError> {
        self.runtime.shutdown().await.map_err(map_writer_error)
    }
}

fn map_writer_error(
    error: reap_durable_writer::WriterError<PmAuthenticatedJournalCodecError>,
) -> PmAuthenticatedJournalError {
    PmAuthenticatedJournalError::Writer(error.to_string())
}

fn map_enqueue_error(error: EnqueueError) -> PmAuthenticatedJournalError {
    match error {
        EnqueueError::Full => PmAuthenticatedJournalError::QueueFull,
        EnqueueError::Closed => PmAuthenticatedJournalError::WriterClosed,
        EnqueueError::Durability(message) => PmAuthenticatedJournalError::Durability(message),
    }
}

pub(crate) struct PmAuthenticatedPendingRecord {
    sequence: u64,
    receipt: DurableReceipt,
    runtime_identity: Arc<PmAuthenticatedJournalRuntimeIdentity>,
    kind: PmAuthenticatedPendingKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PmAuthenticatedPreparedRecord {
    Place(PmAuthenticatedPlacePreparedV1),
    Cancel(PmAuthenticatedCancelPreparedV1),
}

impl PmAuthenticatedPreparedRecord {
    const fn operation(self) -> PmAuthenticatedOperationKeyV1 {
        match self {
            Self::Place(prepared) => prepared.operation,
            Self::Cancel(prepared) => prepared.operation,
        }
    }
}

#[derive(Debug)]
enum PmAuthenticatedPendingKind {
    Prepared(PmAuthenticatedPreparedRecord),
    DispatchAuthorized {
        prepared_sequence: u64,
        prepared: PmAuthenticatedPreparedRecord,
    },
    PlaceResult {
        grant: PmAuthenticatedPlaceSendGrant,
        result: PmAuthenticatedPlaceResultV1,
    },
    CancelResult {
        grant: PmAuthenticatedCancelSendGrant,
        result: PmAuthenticatedCancelResultV1,
    },
}

impl PmAuthenticatedPendingRecord {
    /// Converts an admitted place-result receipt that the caller has timed out
    /// observing into sealed reconciliation evidence. Dropping the receipt
    /// does not cancel the append; recovery remains authoritative.
    #[allow(
        clippy::result_large_err,
        reason = "Err(Self) retains the exact move-only pending receipt; boxing would change the take-once recovery API"
    )]
    pub(crate) fn into_unresolved_place_result(
        self,
    ) -> Result<PmAuthenticatedPlaceResultUnresolved, Self> {
        let Self {
            sequence,
            receipt,
            runtime_identity,
            kind,
        } = self;
        match kind {
            PmAuthenticatedPendingKind::PlaceResult { grant, result } => {
                drop(receipt);
                drop(runtime_identity);
                Ok(PmAuthenticatedPlaceResultUnresolved::new(
                    sequence, grant, result,
                ))
            }
            kind => Err(Self {
                sequence,
                receipt,
                runtime_identity,
                kind,
            }),
        }
    }

    /// Cancel counterpart to `into_unresolved_place_result`.
    #[allow(
        clippy::result_large_err,
        reason = "Err(Self) retains the exact move-only pending receipt; boxing would change the take-once recovery API"
    )]
    pub(crate) fn into_unresolved_cancel_result(
        self,
    ) -> Result<PmAuthenticatedCancelResultUnresolved, Self> {
        let Self {
            sequence,
            receipt,
            runtime_identity,
            kind,
        } = self;
        match kind {
            PmAuthenticatedPendingKind::CancelResult { grant, result } => {
                drop(receipt);
                drop(runtime_identity);
                Ok(PmAuthenticatedCancelResultUnresolved::new(
                    sequence, grant, result,
                ))
            }
            kind => Err(Self {
                sequence,
                receipt,
                runtime_identity,
                kind,
            }),
        }
    }

    pub(crate) fn poll(self) -> PmAuthenticatedReceiptPoll {
        let Self {
            sequence,
            receipt,
            runtime_identity,
            kind,
        } = self;
        match receipt.try_result() {
            DurableReceiptPoll::Pending(receipt) => PmAuthenticatedReceiptPoll::Pending(Self {
                sequence,
                receipt,
                runtime_identity,
                kind,
            }),
            DurableReceiptPoll::Acknowledged(acknowledgement) => match kind {
                PmAuthenticatedPendingKind::Prepared(prepared) => {
                    PmAuthenticatedReceiptPoll::PreparedAcknowledged(
                        PmAuthenticatedPreparedAcknowledged {
                            sequence,
                            prepared,
                            runtime_identity,
                            acknowledgement,
                        },
                    )
                }
                PmAuthenticatedPendingKind::DispatchAuthorized {
                    prepared_sequence,
                    prepared,
                } => PmAuthenticatedReceiptPoll::SendGranted(
                    PmAuthenticatedSendGrant::from_durable_dispatch(
                        sequence,
                        prepared_sequence,
                        prepared,
                        runtime_identity,
                        acknowledgement,
                    ),
                ),
                PmAuthenticatedPendingKind::PlaceResult { grant, result } => {
                    drop(runtime_identity);
                    PmAuthenticatedReceiptPoll::PlaceResultAcknowledged(
                        PmAuthenticatedPlaceResultAcknowledged {
                            sequence,
                            grant,
                            result,
                            acknowledgement,
                        },
                    )
                }
                PmAuthenticatedPendingKind::CancelResult { grant, result } => {
                    drop(runtime_identity);
                    PmAuthenticatedReceiptPoll::CancelResultAcknowledged(
                        PmAuthenticatedCancelResultAcknowledged {
                            sequence,
                            grant,
                            result,
                            acknowledgement,
                        },
                    )
                }
            },
            DurableReceiptPoll::Failed(message) => match kind {
                PmAuthenticatedPendingKind::PlaceResult { grant, result } => {
                    drop(runtime_identity);
                    PmAuthenticatedReceiptPoll::PlaceResultFailed {
                        message,
                        unresolved: PmAuthenticatedPlaceResultUnresolved::new(
                            sequence, grant, result,
                        ),
                    }
                }
                PmAuthenticatedPendingKind::CancelResult { grant, result } => {
                    drop(runtime_identity);
                    PmAuthenticatedReceiptPoll::CancelResultFailed {
                        message,
                        unresolved: PmAuthenticatedCancelResultUnresolved::new(
                            sequence, grant, result,
                        ),
                    }
                }
                PmAuthenticatedPendingKind::Prepared(_)
                | PmAuthenticatedPendingKind::DispatchAuthorized { .. } => {
                    PmAuthenticatedReceiptPoll::Failed(message)
                }
            },
            DurableReceiptPoll::Closed => match kind {
                PmAuthenticatedPendingKind::PlaceResult { grant, result } => {
                    drop(runtime_identity);
                    PmAuthenticatedReceiptPoll::PlaceResultClosed(
                        PmAuthenticatedPlaceResultUnresolved::new(sequence, grant, result),
                    )
                }
                PmAuthenticatedPendingKind::CancelResult { grant, result } => {
                    drop(runtime_identity);
                    PmAuthenticatedReceiptPoll::CancelResultClosed(
                        PmAuthenticatedCancelResultUnresolved::new(sequence, grant, result),
                    )
                }
                PmAuthenticatedPendingKind::Prepared(_)
                | PmAuthenticatedPendingKind::DispatchAuthorized { .. } => {
                    PmAuthenticatedReceiptPoll::Closed
                }
            },
        }
    }
}

pub(crate) enum PmAuthenticatedReceiptPoll {
    Pending(PmAuthenticatedPendingRecord),
    PreparedAcknowledged(PmAuthenticatedPreparedAcknowledged),
    SendGranted(PmAuthenticatedSendGrant),
    PlaceResultAcknowledged(PmAuthenticatedPlaceResultAcknowledged),
    CancelResultAcknowledged(PmAuthenticatedCancelResultAcknowledged),
    PlaceResultFailed {
        message: String,
        unresolved: PmAuthenticatedPlaceResultUnresolved,
    },
    CancelResultFailed {
        message: String,
        unresolved: PmAuthenticatedCancelResultUnresolved,
    },
    PlaceResultClosed(PmAuthenticatedPlaceResultUnresolved),
    CancelResultClosed(PmAuthenticatedCancelResultUnresolved),
    Failed(String),
    Closed,
}

/// Move-only proof that one exact prepared request commitment is durable.
/// It is the only production constructor for a dispatch-authorization record.
pub(crate) struct PmAuthenticatedPreparedAcknowledged {
    sequence: u64,
    prepared: PmAuthenticatedPreparedRecord,
    runtime_identity: Arc<PmAuthenticatedJournalRuntimeIdentity>,
    acknowledgement: DurableAcknowledgement,
}

impl PmAuthenticatedPreparedAcknowledged {
    pub(crate) const fn sequence(&self) -> u64 {
        self.sequence
    }
}

/// Exact operation-typed, move-only proof that a may-have-sent grant is durable.
pub(crate) enum PmAuthenticatedSendGrant {
    Place(PmAuthenticatedPlaceSendGrant),
    Cancel(PmAuthenticatedCancelSendGrant),
}

impl PmAuthenticatedSendGrant {
    fn from_durable_dispatch(
        grant_sequence: u64,
        prepared_sequence: u64,
        prepared: PmAuthenticatedPreparedRecord,
        runtime_identity: Arc<PmAuthenticatedJournalRuntimeIdentity>,
        acknowledgement: DurableAcknowledgement,
    ) -> Self {
        match prepared {
            PmAuthenticatedPreparedRecord::Place(prepared) => {
                Self::Place(PmAuthenticatedPlaceSendGrant {
                    prepared,
                    prepared_sequence,
                    grant_sequence,
                    runtime_identity,
                    acknowledgement,
                })
            }
            PmAuthenticatedPreparedRecord::Cancel(prepared) => {
                Self::Cancel(PmAuthenticatedCancelSendGrant {
                    prepared,
                    prepared_sequence,
                    grant_sequence,
                    runtime_identity,
                    acknowledgement,
                })
            }
        }
    }
}

/// Non-forgeable place-only send grant. Construction requires the durable
/// writer acknowledgement for the exact correlated operation and sequences.
pub(crate) struct PmAuthenticatedPlaceSendGrant {
    prepared: PmAuthenticatedPlacePreparedV1,
    prepared_sequence: u64,
    grant_sequence: u64,
    runtime_identity: Arc<PmAuthenticatedJournalRuntimeIdentity>,
    acknowledgement: DurableAcknowledgement,
}

impl PmAuthenticatedPlaceSendGrant {
    pub(crate) const fn prepared_sequence(&self) -> u64 {
        self.prepared_sequence
    }

    pub(crate) const fn grant_sequence(&self) -> u64 {
        self.grant_sequence
    }

    pub(crate) const fn client_order(&self) -> reap_pm_core::PmClientOrderKey {
        self.prepared.operation.coordinator().client_order
    }

    pub(crate) const fn instrument(&self) -> reap_pm_core::PmInstrumentId {
        self.prepared.operation.coordinator().instrument
    }

    pub(crate) const fn prior_goal_f_sequence(&self) -> u64 {
        self.prepared.prior_intent_sequence
    }

    pub(crate) const fn request_commitment(&self) -> [u8; 32] {
        self.prepared.request_commitment.bytes()
    }

    pub(crate) const fn expected_order_id(&self) -> [u8; 32] {
        self.prepared.expected_order_id.bytes()
    }

    pub(crate) const fn l2_timestamp_seconds(&self) -> u64 {
        self.prepared.l2_timestamp_seconds
    }

    pub(crate) fn matches_retained_request(
        &self,
        semantic_request_commitment: [u8; 32],
        expected_order_id: [u8; 32],
        l2_timestamp_seconds: u64,
    ) -> bool {
        self.prepared.semantic_request_commitment.bytes() == semantic_request_commitment
            && self.prepared.expected_order_id.bytes() == expected_order_id
            && self.prepared.l2_timestamp_seconds == l2_timestamp_seconds
    }
}

impl std::fmt::Debug for PmAuthenticatedPlaceSendGrant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmAuthenticatedPlaceSendGrant")
            .field("operation", &self.prepared.operation)
            .field("prepared_sequence", &self.prepared_sequence)
            .field("grant_sequence", &self.grant_sequence)
            .field("durable_acknowledgement", &"[REDACTED]")
            .finish()
    }
}

/// Non-forgeable cancel-only send grant with the exact owned venue identity.
pub(crate) struct PmAuthenticatedCancelSendGrant {
    prepared: PmAuthenticatedCancelPreparedV1,
    prepared_sequence: u64,
    grant_sequence: u64,
    runtime_identity: Arc<PmAuthenticatedJournalRuntimeIdentity>,
    acknowledgement: DurableAcknowledgement,
}

impl PmAuthenticatedCancelSendGrant {
    pub(crate) const fn prepared_sequence(&self) -> u64 {
        self.prepared_sequence
    }

    pub(crate) const fn grant_sequence(&self) -> u64 {
        self.grant_sequence
    }

    pub(crate) const fn client_order(&self) -> reap_pm_core::PmClientOrderKey {
        self.prepared.operation.coordinator().client_order
    }

    pub(crate) const fn instrument(&self) -> reap_pm_core::PmInstrumentId {
        self.prepared.operation.coordinator().instrument
    }

    pub(crate) fn venue_order(&self) -> reap_pm_core::PmVenueOrderKey {
        match self.prepared.operation {
            PmAuthenticatedOperationKeyV1::Cancel { venue_order, .. } => venue_order,
            PmAuthenticatedOperationKeyV1::Place { .. } => {
                unreachable!("cancel grant constructor validates operation kind")
            }
        }
    }

    pub(crate) const fn prior_goal_f_sequence(&self) -> u64 {
        self.prepared.prior_cancel_sequence
    }

    pub(crate) const fn request_commitment(&self) -> [u8; 32] {
        self.prepared.request_commitment.bytes()
    }

    pub(crate) const fn fixed_order_id(&self) -> [u8; 32] {
        self.prepared.fixed_order_id.bytes()
    }

    pub(crate) const fn l2_timestamp_seconds(&self) -> u64 {
        self.prepared.l2_timestamp_seconds
    }

    pub(crate) fn matches_retained_request(
        &self,
        semantic_request_commitment: [u8; 32],
        fixed_order_id: [u8; 32],
        l2_timestamp_seconds: u64,
    ) -> bool {
        self.prepared.semantic_request_commitment.bytes() == semantic_request_commitment
            && self.prepared.fixed_order_id.bytes() == fixed_order_id
            && self.prepared.l2_timestamp_seconds == l2_timestamp_seconds
    }
}

impl std::fmt::Debug for PmAuthenticatedCancelSendGrant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmAuthenticatedCancelSendGrant")
            .field("operation", &self.prepared.operation)
            .field("prepared_sequence", &self.prepared_sequence)
            .field("grant_sequence", &self.grant_sequence)
            .field("durable_acknowledgement", &"[REDACTED]")
            .finish()
    }
}

/// Consumed place-send proof available only after its exact result is durable.
/// This distinct type cannot be reused to admit another result record.
pub(crate) struct PmAuthenticatedPlaceCompletionGrant {
    grant: PmAuthenticatedPlaceSendGrant,
}

impl PmAuthenticatedPlaceCompletionGrant {
    pub(crate) const fn prepared_sequence(&self) -> u64 {
        self.grant.prepared_sequence()
    }

    pub(crate) const fn grant_sequence(&self) -> u64 {
        self.grant.grant_sequence()
    }

    pub(crate) const fn client_order(&self) -> reap_pm_core::PmClientOrderKey {
        self.grant.client_order()
    }

    pub(crate) const fn instrument(&self) -> reap_pm_core::PmInstrumentId {
        self.grant.instrument()
    }

    pub(crate) const fn prior_goal_f_sequence(&self) -> u64 {
        self.grant.prior_goal_f_sequence()
    }

    pub(crate) const fn request_commitment(&self) -> [u8; 32] {
        self.grant.request_commitment()
    }

    pub(crate) const fn expected_order_id(&self) -> [u8; 32] {
        self.grant.expected_order_id()
    }
}

impl std::fmt::Debug for PmAuthenticatedPlaceCompletionGrant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmAuthenticatedPlaceCompletionGrant")
            .field("grant", &self.grant)
            .finish()
    }
}

/// Consumed cancel-send proof available only after its exact result is
/// durable. It cannot authorize a second cancel-result append.
pub(crate) struct PmAuthenticatedCancelCompletionGrant {
    grant: PmAuthenticatedCancelSendGrant,
}

impl PmAuthenticatedCancelCompletionGrant {
    pub(crate) const fn prepared_sequence(&self) -> u64 {
        self.grant.prepared_sequence()
    }

    pub(crate) const fn grant_sequence(&self) -> u64 {
        self.grant.grant_sequence()
    }

    pub(crate) const fn client_order(&self) -> reap_pm_core::PmClientOrderKey {
        self.grant.client_order()
    }

    pub(crate) const fn instrument(&self) -> reap_pm_core::PmInstrumentId {
        self.grant.instrument()
    }

    pub(crate) fn venue_order(&self) -> reap_pm_core::PmVenueOrderKey {
        self.grant.venue_order()
    }

    pub(crate) const fn prior_goal_f_sequence(&self) -> u64 {
        self.grant.prior_goal_f_sequence()
    }

    pub(crate) const fn request_commitment(&self) -> [u8; 32] {
        self.grant.request_commitment()
    }

    pub(crate) const fn fixed_order_id(&self) -> [u8; 32] {
        self.grant.fixed_order_id()
    }
}

impl std::fmt::Debug for PmAuthenticatedCancelCompletionGrant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmAuthenticatedCancelCompletionGrant")
            .field("grant", &self.grant)
            .finish()
    }
}

/// Move-only proof that this exact place classification is durable.
pub(crate) struct PmAuthenticatedPlaceResultAcknowledged {
    sequence: u64,
    grant: PmAuthenticatedPlaceSendGrant,
    result: PmAuthenticatedPlaceResultV1,
    acknowledgement: DurableAcknowledgement,
}

impl PmAuthenticatedPlaceResultAcknowledged {
    pub(crate) const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        PmAuthenticatedPlaceCompletionGrant,
        u64,
        PmAuthenticatedPlaceResultV1,
    ) {
        let Self {
            sequence,
            grant,
            result,
            acknowledgement,
        } = self;
        drop(acknowledgement);
        (
            PmAuthenticatedPlaceCompletionGrant { grant },
            sequence,
            result,
        )
    }
}

impl std::fmt::Debug for PmAuthenticatedPlaceResultAcknowledged {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmAuthenticatedPlaceResultAcknowledged")
            .field("sequence", &self.sequence)
            .field("result", &self.result)
            .field("durable_acknowledgement", &"[REDACTED]")
            .finish()
    }
}

/// Move-only proof that this exact cancel classification is durable.
pub(crate) struct PmAuthenticatedCancelResultAcknowledged {
    sequence: u64,
    grant: PmAuthenticatedCancelSendGrant,
    result: PmAuthenticatedCancelResultV1,
    acknowledgement: DurableAcknowledgement,
}

impl PmAuthenticatedCancelResultAcknowledged {
    pub(crate) const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        PmAuthenticatedCancelCompletionGrant,
        u64,
        PmAuthenticatedCancelResultV1,
    ) {
        let Self {
            sequence,
            grant,
            result,
            acknowledgement,
        } = self;
        drop(acknowledgement);
        (
            PmAuthenticatedCancelCompletionGrant { grant },
            sequence,
            result,
        )
    }
}

impl std::fmt::Debug for PmAuthenticatedCancelResultAcknowledged {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmAuthenticatedCancelResultAcknowledged")
            .field("sequence", &self.sequence)
            .field("result", &self.result)
            .field("durable_acknowledgement", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
#[path = "authenticated_journal/tests.rs"]
mod tests;
