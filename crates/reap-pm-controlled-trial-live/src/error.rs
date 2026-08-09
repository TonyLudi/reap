use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmTrialLiveJournalError {
    #[error("controlled-trial live journal binding is invalid")]
    InvalidBinding,
    #[error("controlled-trial live journal transition is invalid")]
    InvalidTransition,
    #[error("controlled-trial live journal record is malformed, reordered, or foreign")]
    InvalidRecord,
    #[error("controlled-trial live journal has a torn or ambiguous tail")]
    AmbiguousTail,
    #[error("controlled-trial live journal file already exists")]
    AlreadyExists,
    #[error("controlled-trial live journal file is absent")]
    Absent,
    #[error("controlled-trial live journal is already exclusively leased")]
    AlreadyLeased,
    #[error("controlled-trial live journal protection or descriptor stability check failed")]
    Protection,
    #[error("controlled-trial live journal exceeds a closed bound")]
    BoundExceeded,
    #[error("controlled-trial live journal durable append or synchronization failed")]
    Durability,
    #[error("controlled-trial live journal acknowledgement belongs to another runtime")]
    ForeignAcknowledgement,
    #[error("controlled-trial live journal recovery classification forbids this operation")]
    RecoveryOperationForbidden,
}
