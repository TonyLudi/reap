//! Bin-private PM-T2 controlled-trial authority assembly.
//!
//! Parent-owned sealed inputs in this module are the only requests accepted
//! by the secret-owning child authority. They carry no transport capability.

mod authority;
mod runtime;

use std::fmt;

use reap_polymarket_auth::{
    AuthenticatedL2Headers, AuthenticatedOwnedCancelRequest, AuthenticatedPlaceRequest,
    FixedOrderId, L2Timestamp, PlacePublicRequestIdentity, PmClobDomain,
};
use reap_polymarket_wire::PmUnsignedClobV2Order;

/// An L2 timestamp admitted by the future in-process freshness/session join.
///
/// This wrapper has no public constructor. The current authority tests mint
/// synthetic instances inside this private parent module only.
#[derive(Clone, Copy)]
struct AuthorizedL2Timestamp(L2Timestamp);

impl AuthorizedL2Timestamp {
    const fn new(timestamp: L2Timestamp) -> Self {
        Self(timestamp)
    }

    const fn into_inner(self) -> L2Timestamp {
        self.0
    }
}

impl fmt::Debug for AuthorizedL2Timestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizedL2Timestamp(<opaque>)")
    }
}

/// Move-only parent-sealed inputs for the sole fresh proxy place attempt.
struct SealedFreshPlaceAuthentication {
    domain: PmClobDomain,
    unsigned_order: PmUnsignedClobV2Order,
    timestamp: AuthorizedL2Timestamp,
    expected_public_identity: PlacePublicRequestIdentity,
}

impl SealedFreshPlaceAuthentication {
    fn new(
        domain: PmClobDomain,
        unsigned_order: PmUnsignedClobV2Order,
        timestamp: AuthorizedL2Timestamp,
        expected_public_identity: PlacePublicRequestIdentity,
    ) -> Self {
        Self {
            domain,
            unsigned_order,
            timestamp,
            expected_public_identity,
        }
    }
}

impl fmt::Debug for SealedFreshPlaceAuthentication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SealedFreshPlaceAuthentication(<opaque>)")
    }
}

/// Move-only parent-sealed inputs for one journal-bounded exact-owned cancel.
struct SealedExactOwnedCancelAuthentication {
    order_id: FixedOrderId,
    timestamp: AuthorizedL2Timestamp,
}

/// Move-only parent-sealed inputs for the one journal/scoped exact-order
/// detail read. The route identity cannot be selected by the credential task.
struct SealedExactOwnedOrderReadAuthentication {
    order_id: FixedOrderId,
    timestamp: AuthorizedL2Timestamp,
}

impl SealedExactOwnedOrderReadAuthentication {
    const fn new(order_id: FixedOrderId, timestamp: AuthorizedL2Timestamp) -> Self {
        Self {
            order_id,
            timestamp,
        }
    }
}

impl fmt::Debug for SealedExactOwnedOrderReadAuthentication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SealedExactOwnedOrderReadAuthentication(<opaque>)")
    }
}

impl SealedExactOwnedCancelAuthentication {
    const fn new(order_id: FixedOrderId, timestamp: AuthorizedL2Timestamp) -> Self {
        Self {
            order_id,
            timestamp,
        }
    }
}

impl fmt::Debug for SealedExactOwnedCancelAuthentication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SealedExactOwnedCancelAuthentication(<opaque>)")
    }
}

/// Authenticated place carrier returned only after the task-local signer has
/// been destroyed. It grants no network operation by itself.
struct SignerDroppedAuthenticatedPlace {
    request: AuthenticatedPlaceRequest,
    timestamp: L2Timestamp,
}

impl SignerDroppedAuthenticatedPlace {
    const fn new(request: AuthenticatedPlaceRequest, timestamp: L2Timestamp) -> Self {
        Self { request, timestamp }
    }

    fn into_parts(self) -> (AuthenticatedPlaceRequest, L2Timestamp) {
        (self.request, self.timestamp)
    }
}

impl fmt::Debug for SignerDroppedAuthenticatedPlace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SignerDroppedAuthenticatedPlace([REDACTED])")
    }
}

/// Authenticated exact-owned cancel carrier with its admitted L2 time.
struct AuthenticatedExactOwnedCancel {
    request: AuthenticatedOwnedCancelRequest,
    timestamp: L2Timestamp,
}

impl AuthenticatedExactOwnedCancel {
    const fn new(request: AuthenticatedOwnedCancelRequest, timestamp: L2Timestamp) -> Self {
        Self { request, timestamp }
    }

    fn into_parts(self) -> (AuthenticatedOwnedCancelRequest, L2Timestamp) {
        (self.request, self.timestamp)
    }
}

impl fmt::Debug for AuthenticatedExactOwnedCancel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthenticatedExactOwnedCancel([REDACTED])")
    }
}

/// Fixed authenticated headers paired with the exact journal/scoped order
/// identity used in their signed GET route.
struct AuthenticatedExactOwnedOrderRead {
    headers: AuthenticatedL2Headers,
    order_id: FixedOrderId,
    timestamp: L2Timestamp,
}

impl AuthenticatedExactOwnedOrderRead {
    const fn new(
        headers: AuthenticatedL2Headers,
        order_id: FixedOrderId,
        timestamp: L2Timestamp,
    ) -> Self {
        Self {
            headers,
            order_id,
            timestamp,
        }
    }

    fn into_parts(self) -> (AuthenticatedL2Headers, FixedOrderId, L2Timestamp) {
        (self.headers, self.order_id, self.timestamp)
    }
}

impl fmt::Debug for AuthenticatedExactOwnedOrderRead {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthenticatedExactOwnedOrderRead([REDACTED])")
    }
}
