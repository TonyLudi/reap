//! Private-monitor error taxonomy and fail-closed ingress classification.

use reap_pm_core::{EnvelopeError, PmAggregateError};
use reap_pm_state::{
    PmPrivateExternalIngressFailure, PmPrivateStateError, PmUnresolvedFillStateError,
};
use reap_polymarket_adapter::{
    PmAccountPositionRoleError, PmPrivateNormalizationError, PmReconciliationContractError,
};
use thiserror::Error;

use super::PmPrivateBatchApply;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPrivateMonitorInputError {
    #[error("completion belongs to another requested connection epoch")]
    CompletionEpochMismatch,
    #[error("completion snapshot differs from the requested snapshot")]
    CompletionSnapshotMismatch,
    #[error("request/completion causal boundary is invalid: {0}")]
    Boundary(#[from] PmAggregateError),
    #[error("completion service time is invalid: {0}")]
    ServiceClock(#[from] EnvelopeError),
    #[error("live account input is not an exact collateral plus conditional asset pair")]
    AccountAssetKindMismatch,
}

#[derive(Debug, Error)]
pub enum PmPrivateMonitorError {
    #[error("private fixture batch failed after visible partial progress {applied:?}: {source}")]
    PrivateBatchPartial {
        applied: PmPrivateBatchApply,
        #[source]
        source: Box<PmPrivateMonitorError>,
    },
    #[error(transparent)]
    PrivateNormalization(#[from] PmPrivateNormalizationError),
    #[error(transparent)]
    Reconciliation(#[from] PmReconciliationContractError),
    #[error(transparent)]
    Account(#[from] PmAccountPositionRoleError),
    #[error(transparent)]
    Envelope(#[from] EnvelopeError),
    #[error(transparent)]
    State(#[from] PmPrivateStateError),
    #[error(transparent)]
    UnresolvedFill(#[from] PmUnresolvedFillStateError),
    #[error(transparent)]
    Input(#[from] PmPrivateMonitorInputError),
    #[error("serviced private delivery belongs to another role owner")]
    PrivateDeliveryOwnerMismatch,
    #[error("serviced reconciliation delivery belongs to another role owner")]
    ReconciliationDeliveryOwnerMismatch,
    #[error("serviced account delivery belongs to another role owner")]
    AccountDeliveryOwnerMismatch,
    #[error("serviced delivery differs from the monitor's exact account/instrument scope")]
    DeliveryScopeMismatch,
    #[error("fixture read belongs to an epoch that is not active on the private role")]
    PrivateEpochMismatch,
    #[error("live conditional account response names another token")]
    ConditionalTokenMismatch,
    #[error("live HTTP dependency does not match its purpose-bound fault lane")]
    HttpDependencyLaneMismatch,
    #[error("complete open-order fixture row lacks an exact venue identity")]
    OpenOrderMissingVenueIdentity,
    #[error("private fixture batch counter overflowed")]
    BatchCounterOverflow,
    #[error("one private fixture batch repeats an order or exact fill identity")]
    DuplicateBatchIdentity,
}

pub(super) fn classify_monitor_error(
    error: &PmPrivateMonitorError,
) -> PmPrivateExternalIngressFailure {
    match error {
        PmPrivateMonitorError::PrivateNormalization(_) => {
            PmPrivateExternalIngressFailure::Normalization
        }
        PmPrivateMonitorError::Envelope(_) => PmPrivateExternalIngressFailure::Service,
        PmPrivateMonitorError::Reconciliation(PmReconciliationContractError::Normalization(_)) => {
            PmPrivateExternalIngressFailure::Normalization
        }
        PmPrivateMonitorError::Reconciliation(
            PmReconciliationContractError::WrongSource
            | PmReconciliationContractError::SourceAccountMismatch
            | PmReconciliationContractError::RequestedOrderAccountMismatch
            | PmReconciliationContractError::CursorAccountScopeMismatch
            | PmReconciliationContractError::InstrumentMismatch,
        )
        | PmPrivateMonitorError::Account(
            PmAccountPositionRoleError::WrongSource
            | PmAccountPositionRoleError::SourceAccountMismatch
            | PmAccountPositionRoleError::DomainChainMismatch
            | PmAccountPositionRoleError::SignerFunderMismatch
            | PmAccountPositionRoleError::AccountScopeMismatch
            | PmAccountPositionRoleError::SpenderAccountMismatch
            | PmAccountPositionRoleError::SpenderChainMismatch
            | PmAccountPositionRoleError::InstrumentMismatch,
        )
        | PmPrivateMonitorError::PrivateDeliveryOwnerMismatch
        | PmPrivateMonitorError::ReconciliationDeliveryOwnerMismatch
        | PmPrivateMonitorError::AccountDeliveryOwnerMismatch
        | PmPrivateMonitorError::DeliveryScopeMismatch
        | PmPrivateMonitorError::PrivateEpochMismatch
        | PmPrivateMonitorError::ConditionalTokenMismatch
        | PmPrivateMonitorError::Input(PmPrivateMonitorInputError::AccountAssetKindMismatch) => {
            PmPrivateExternalIngressFailure::Scope
        }
        PmPrivateMonitorError::PrivateBatchPartial { source, .. } => classify_monitor_error(source),
        PmPrivateMonitorError::Reconciliation(_)
        | PmPrivateMonitorError::Account(_)
        | PmPrivateMonitorError::State(_)
        | PmPrivateMonitorError::UnresolvedFill(_)
        | PmPrivateMonitorError::Input(_)
        | PmPrivateMonitorError::HttpDependencyLaneMismatch
        | PmPrivateMonitorError::OpenOrderMissingVenueIdentity
        | PmPrivateMonitorError::BatchCounterOverflow
        | PmPrivateMonitorError::DuplicateBatchIdentity => {
            PmPrivateExternalIngressFailure::Contract
        }
    }
}
