//! One-way terminal safety transition for the PM mutation owner.
//!
//! A healthy journal edge admits the exact safety fact before the in-memory
//! terminal latch becomes visible. If that admission is impossible, the owner
//! still fails closed but exposes an explicitly unjournalable cause; it never
//! claims that recovery will reproduce a fact the durable writer did not
//! accept.

use super::{PmMutationError, PmMutationHalt, PmMutationOwner, PmPersistenceError};
use crate::journal::{
    PmJournalError, PmJournalRecordV1, PmJournalSafetyHaltV1, PmJournalSafetyReasonV1,
};

/// Stable classification of why a terminal safety fact could not be admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmTerminalSafetyAdmissionFailure {
    InvalidMonotonicTime,
    DurableConsequenceFull,
    PersistenceFull,
    JournalQueueFull,
    JournalWriterClosed,
    JournalDurabilityFailed,
    JournalRecordLimit,
    JournalRejected,
    InternalInvariant,
}

/// Exact result of one take-once terminal safety transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmTerminalSafetyTransition {
    Admitted {
        reason: PmJournalSafetyReasonV1,
    },
    AlreadyEntered {
        halt: PmMutationHalt,
    },
    Unjournalable {
        reason: PmJournalSafetyReasonV1,
        failure: PmTerminalSafetyAdmissionFailure,
    },
}

impl PmMutationOwner {
    /// Enters terminal safety exactly once.
    ///
    /// Revision invalidation happens before any admission attempt, preventing
    /// new or prepared quote authority from crossing the fixture boundary.
    /// When storage is healthy, the journal and its bounded receipt/consequence
    /// carriers accept the fact before the live terminal latch is installed.
    /// Owned cancels remain serviceable through all three safety provenance
    /// variants.
    pub(crate) fn enter_terminal_safety(
        &mut self,
        reason: PmJournalSafetyReasonV1,
        monotonic_service_ns: u64,
    ) -> PmTerminalSafetyTransition {
        self.invalidate_revisions();
        if let Some(
            halt @ (PmMutationHalt::LiveSafetyHalt(_)
            | PmMutationHalt::RecoveredSafetyHalt(_)
            | PmMutationHalt::UnjournalableSafetyHalt(_)),
        ) = self.halt
        {
            return PmTerminalSafetyTransition::AlreadyEntered { halt };
        }

        let record = PmJournalRecordV1::SafetyHalt(PmJournalSafetyHaltV1 {
            account: self.account_scope().handle(),
            reason,
        });
        match self.record_fact(record, monotonic_service_ns) {
            Ok(()) => {
                self.halt = Some(PmMutationHalt::LiveSafetyHalt(reason));
                PmTerminalSafetyTransition::Admitted { reason }
            }
            Err(error) => {
                let failure = classify_admission_failure(&error);
                self.halt = Some(PmMutationHalt::UnjournalableSafetyHalt(reason));
                PmTerminalSafetyTransition::Unjournalable { reason, failure }
            }
        }
    }
}

fn classify_admission_failure(error: &PmMutationError) -> PmTerminalSafetyAdmissionFailure {
    match error {
        PmMutationError::DurableConsequenceSaturated => {
            PmTerminalSafetyAdmissionFailure::DurableConsequenceFull
        }
        PmMutationError::Persistence(PmPersistenceError::InvalidMonotonicTime) => {
            PmTerminalSafetyAdmissionFailure::InvalidMonotonicTime
        }
        PmMutationError::Persistence(PmPersistenceError::Full) => {
            PmTerminalSafetyAdmissionFailure::PersistenceFull
        }
        PmMutationError::Journal(PmJournalError::QueueFull) => {
            PmTerminalSafetyAdmissionFailure::JournalQueueFull
        }
        PmMutationError::Journal(PmJournalError::WriterClosed) => {
            PmTerminalSafetyAdmissionFailure::JournalWriterClosed
        }
        PmMutationError::Journal(PmJournalError::Durability(_)) => {
            PmTerminalSafetyAdmissionFailure::JournalDurabilityFailed
        }
        PmMutationError::Journal(
            PmJournalError::RecordLimit | PmJournalError::SequenceExhausted,
        ) => PmTerminalSafetyAdmissionFailure::JournalRecordLimit,
        PmMutationError::Journal(
            PmJournalError::Schema(_)
            | PmJournalError::Io(_)
            | PmJournalError::Recovery(_)
            | PmJournalError::Lease(_)
            | PmJournalError::Writer(_),
        )
        | PmMutationError::JournalSchema(_) => PmTerminalSafetyAdmissionFailure::JournalRejected,
        _ => PmTerminalSafetyAdmissionFailure::InternalInvariant,
    }
}
