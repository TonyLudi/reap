//! Fixed, consume-once Polymarket mutation edge.
//!
//! Production construction intentionally does not exist while PM-T1 keeps
//! order entry disabled. The only transport configuration constructor is a
//! feature-gated literal-loopback evidence seam.

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
    #[error("Polymarket loopback mutation HTTP client construction failed")]
    TransportBuild,
    #[error("authenticated Polymarket mutation request failed retention validation")]
    InvalidAuthenticatedRequest,
}

pub use retained::{PmRetainedOwnedCancelRequest, PmRetainedPlaceRequest};
pub use transport::{
    PmCancelMutationOutcome, PmExactOwnedCancelLoopbackRole, PmFixedPlaceLoopbackRole,
    PmLoopbackMutationConfig, PmMutationClassification, PmMutationDiagnostic,
    PmMutationDiagnosticKind, PmPlaceMutationOutcome,
};
