//! Take-once, backend-neutral boundaries for durable PM mutations.
//!
//! These values contain the exact fixed-profile request admitted by the
//! coordinator, but no journal receipt, signer, credential, transport, or
//! generic execution capability. Constructors remain inside the coordinator;
//! a backend can only consume a dispatch that crossed the durable intent
//! barrier.

use std::num::NonZeroU64;

use reap_pm_core::{
    PmAccountScope, PmClientOrderKey, PmInstrumentHandle, PmInstrumentId, PmOrderSide, PmPrice,
    PmQuantity, PmVenueOrderKey,
};
use reap_polymarket_adapter::{PmExactOwnedCancelRequest, PmGtcPostOnlyPlaceRequest};

/// One exact, already-durable GTC/post-only place dispatch.
///
/// Deliberately neither `Clone` nor `Copy`: moving it into one backend is the
/// dispatch authority transition.
#[derive(Debug, PartialEq, Eq)]
pub struct PmPreparedPlaceDispatch {
    journal_sequence: NonZeroU64,
    request: PmGtcPostOnlyPlaceRequest,
}

impl PmPreparedPlaceDispatch {
    pub(super) const fn new(
        journal_sequence: NonZeroU64,
        request: PmGtcPostOnlyPlaceRequest,
    ) -> Self {
        Self {
            journal_sequence,
            request,
        }
    }

    #[must_use]
    pub const fn journal_sequence(&self) -> u64 {
        self.journal_sequence.get()
    }

    #[must_use]
    pub const fn account_scope(&self) -> PmAccountScope {
        self.request.account_scope()
    }

    #[must_use]
    pub const fn instrument(&self) -> PmInstrumentHandle {
        self.request.instrument()
    }

    #[must_use]
    pub const fn instrument_id(&self) -> PmInstrumentId {
        self.request.instrument_id()
    }

    #[must_use]
    pub const fn client_order(&self) -> PmClientOrderKey {
        self.request.client_order()
    }

    #[must_use]
    pub const fn side(&self) -> PmOrderSide {
        self.request.side()
    }

    #[must_use]
    pub const fn price(&self) -> PmPrice {
        self.request.price()
    }

    #[must_use]
    pub const fn quantity(&self) -> PmQuantity {
        self.request.quantity()
    }

    /// Consumes the durable dispatch into its fixed backend request.
    #[must_use]
    pub fn into_request(self) -> PmGtcPostOnlyPlaceRequest {
        self.request
    }
}

/// One exact, already-durable cancel dispatch for a journal-proven order.
///
/// Deliberately neither `Clone` nor `Copy`.
#[derive(Debug, PartialEq, Eq)]
pub struct PmPreparedCancelDispatch {
    journal_sequence: NonZeroU64,
    request: PmExactOwnedCancelRequest,
}

impl PmPreparedCancelDispatch {
    pub(super) const fn new(
        journal_sequence: NonZeroU64,
        request: PmExactOwnedCancelRequest,
    ) -> Self {
        Self {
            journal_sequence,
            request,
        }
    }

    #[must_use]
    pub const fn journal_sequence(&self) -> u64 {
        self.journal_sequence.get()
    }

    #[must_use]
    pub const fn account_scope(&self) -> PmAccountScope {
        self.request.account_scope()
    }

    #[must_use]
    pub const fn instrument(&self) -> PmInstrumentHandle {
        self.request.instrument()
    }

    #[must_use]
    pub const fn instrument_id(&self) -> PmInstrumentId {
        self.request.instrument_id()
    }

    #[must_use]
    pub const fn client_order(&self) -> PmClientOrderKey {
        self.request.client_order()
    }

    #[must_use]
    pub const fn venue_order(&self) -> PmVenueOrderKey {
        self.request.venue_order()
    }

    /// Consumes the durable dispatch into its exact-owned backend request.
    #[must_use]
    pub fn into_request(self) -> PmExactOwnedCancelRequest {
        self.request
    }
}
