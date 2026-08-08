use reap_polymarket_auth::PmAuthError;
use reap_polymarket_wire::{PmLiveWireError, PmWireError};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmLiveAdapterError {
    #[error("Polymarket live HTTP configuration is invalid: {0}")]
    InvalidConfiguration(&'static str),
    #[error("Polymarket live HTTP client construction failed")]
    TransportBuild,
    #[error("Polymarket live HTTP request timed out")]
    RequestTimeout,
    #[error("Polymarket live HTTP request failed")]
    RequestFailed,
    #[error("Polymarket live HTTP endpoint returned redirect status {status}")]
    Redirect { status: u16 },
    #[error("Polymarket live HTTP endpoint returned non-200 status {status}")]
    UnexpectedStatus { status: u16 },
    #[error("Polymarket live HTTP response exceeds {limit} bytes")]
    ResponseBodyTooLarge { limit: usize },
    #[error("Polymarket live HTTP response body failed while streaming")]
    ResponseBodyRead,
    #[error("Polymarket public wire validation failed: {0}")]
    Wire(#[from] PmWireError),
    #[error("authenticated Polymarket wire validation failed: {0}")]
    PrivateWire(#[from] PmLiveWireError),
    #[error("Polymarket fixed-purpose request authentication failed: {0}")]
    Auth(#[from] PmAuthError),
    #[error("authenticated Polymarket response belongs to another credential owner")]
    CredentialOwnerMismatch,
    #[error("exact Polymarket order response does not match the requested order ID")]
    ExactOrderIdentityMismatch,
    #[error("exact Polymarket order response does not match the configured EOA")]
    ExactOrderMakerMismatch,
    #[error("exact Polymarket order response does not match the configured instrument")]
    ExactOrderScopeMismatch,
    #[error("authenticated Polymarket pagination cursor repeated before terminal completion")]
    PaginationCursorCycle,
    #[error("authenticated Polymarket cut exceeded its fixed page bound")]
    PaginationPageLimit,
    #[error("authenticated Polymarket cut exceeded its fixed total-row bound")]
    PaginationRowLimit,
    #[error("fixed authenticated Polymarket header material was invalid")]
    InvalidAuthenticatedHeaders,
    #[error("private Polymarket credential authority is unavailable")]
    CredentialAuthorityClosed,
    #[error("private Polymarket credential authority task failed")]
    CredentialAuthorityTaskFailed,
    #[error("authenticated Polymarket user subscription was invalid")]
    InvalidUserSubscription,
    #[error("shared Polymarket product clock failed")]
    ProductClock,
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum PmRestBookDeliveryError<E> {
    #[error(transparent)]
    Http(#[from] PmLiveAdapterError),
    #[error("native Polymarket REST book sink rejected the response: {0}")]
    Sink(E),
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum PmPublicMetadataDeliveryError<E> {
    #[error(transparent)]
    Http(#[from] PmLiveAdapterError),
    #[error("native Polymarket public metadata sink rejected the atomic pair: {0}")]
    Sink(E),
}
