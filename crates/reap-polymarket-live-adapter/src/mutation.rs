//! Fixed, consume-once Polymarket mutation edge.
//!
//! The production transport is destination-closed to the reviewed
//! `clob.polymarket.com` TLS peer and selected Linux egress. It still carries
//! no credentials or order authority: callers must separately consume one
//! authenticated, purpose-closed request. Literal-loopback construction
//! remains a feature-gated evidence seam.

mod retained;
mod transport;

#[cfg(test)]
mod tests;

use thiserror::Error;

/// Construction/retention failures which occur before a network write is
/// possible. The enum deliberately carries no caller-controlled or secret
/// material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmMutationEdgeError {
    #[error("Polymarket loopback mutation configuration is invalid: {0}")]
    InvalidLoopbackConfiguration(&'static str),
    #[error("Polymarket production mutation configuration is invalid: {0}")]
    InvalidProductionConfiguration(&'static str),
    #[error("Polymarket loopback mutation HTTP client construction failed")]
    TransportBuild,
    #[error("authenticated Polymarket mutation request failed retention validation")]
    InvalidAuthenticatedRequest,
}

pub use retained::{
    PmRetainedGtcFillPlaceRequest, PmRetainedOwnedCancelRequest, PmRetainedPlaceRequest,
};
pub use transport::{
    PmCancelMutationOutcome, PmExactOwnedCancelLoopbackRole, PmExactOwnedCancelProductionRole,
    PmFixedGtcFillPlaceProductionRole, PmFixedPlaceLoopbackRole, PmFixedPlaceProductionRole,
    PmLoopbackMutationConfig, PmMutationClassification, PmMutationDiagnostic,
    PmMutationDiagnosticKind, PmPlaceMutationOutcome, PmProductionMutationConfig,
};
