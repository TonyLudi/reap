//! Typed failures crossing the active public capture-role boundary.

use reap_okx_public_source::OkxPublicSessionError;
use reap_polymarket_adapter::PmPublicSessionError;
use thiserror::Error;

use super::{PmPublicBookReduceError, PmPublicSnapshotCommitError};
use crate::capture::PmCaptureVerifyError;
use crate::public_routes::{
    OkxPublicUnavailableDelivery, PmPublicRouteError, PmPublicUnavailableDelivery,
};

#[derive(Debug, Error)]
#[allow(
    clippy::large_enum_variant,
    reason = "snapshot failure retains exact move-only unavailable evidence on the bounded fail-closed path"
)]
pub(crate) enum PmCaptureSnapshotCommitFailure {
    #[error("PM snapshot commit failed: {source}")]
    Commit {
        #[source]
        source: PmPublicSnapshotCommitError,
        unavailable: Option<PmPublicUnavailableDelivery>,
    },
    #[error(transparent)]
    Route(#[from] PmPublicRouteError),
}

#[derive(Debug, Error)]
#[allow(
    clippy::large_enum_variant,
    reason = "book-reduce failure retains exact move-only unavailable evidence on the bounded fail-closed path"
)]
pub(crate) enum PmCaptureBookReduceFailure {
    #[error("PM routed book update reduction failed: {source}")]
    Reduce {
        #[source]
        source: PmPublicBookReduceError,
        unavailable: Option<PmPublicUnavailableDelivery>,
    },
    #[error(transparent)]
    Route(#[from] PmPublicRouteError),
}

#[derive(Debug, Error)]
#[allow(
    clippy::large_enum_variant,
    reason = "classification failure retains exact move-only unavailable evidence on the bounded fail-closed path"
)]
pub(crate) enum PmCaptureRoleIngressError {
    #[error(transparent)]
    PmSession(#[from] PmPublicSessionError),
    #[error(transparent)]
    OkxSession(#[from] OkxPublicSessionError),
    #[error("captured OKX public payload is not UTF-8 text")]
    OkxRawNotUtf8 {
        unavailable: Option<OkxPublicUnavailableDelivery>,
    },
    #[error(transparent)]
    Route(#[from] PmPublicRouteError),
}

#[derive(Debug, Error)]
pub(crate) enum PmCaptureRoleStartError {
    #[error(transparent)]
    Header(#[from] PmCaptureVerifyError),
    #[error(transparent)]
    PmSession(#[from] PmPublicSessionError),
    #[error(transparent)]
    OkxSession(#[from] OkxPublicSessionError),
    #[error(transparent)]
    MetadataContract(#[from] reap_pm_state::PmMetadataContractError),
    #[error(transparent)]
    Route(#[from] PmPublicRouteError),
}
