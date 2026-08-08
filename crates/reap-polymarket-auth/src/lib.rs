//! Narrow Polymarket CLOB V2 authentication for the fixed PM-T1 profile.
//!
//! This crate owns synthetic-secret-safe EOA order signing, fixed-purpose L2
//! request authentication, and once-only place/cancel bytes. It deliberately
//! has no network, async runtime, strategy, state, generic signing, arbitrary
//! request, or API-key provisioning capability.

#![forbid(unsafe_code)]

mod credential_slot;
mod error;
mod identity;
mod l2;
mod order;
mod secret;
mod user_frame;

pub use credential_slot::{AuthenticatedJournalCredentialSlotFingerprint, CredentialSlotId};
pub use error::PmAuthError;
pub use identity::{
    EoaAddress, ExpectedOrderId, FixedOrderId, OwnedCancelSemanticRequestCommitment,
    PlaceSemanticRequestCommitment, PmClobDomain, RuntimeExactBodyCommitment,
};
pub use l2::{
    AuthenticatedL2Headers, AuthenticatedOwnedCancelRequest, AuthenticatedPlaceRequest,
    AuthenticatedUserSubscription, AuthenticatedUserSubscriptionSink, FixedOwnedCancelRequestSink,
    FixedPlaceRequestSink, L2HeaderSink, L2Timestamp,
};
pub use order::{SerializedOwnedCancelRequest, SerializedPlaceRequest, SignedClobV2Order};
pub use secret::{EoaPrivateKeyInput, FixedEoaSigner, L2CredentialInput, L2Credentials};
pub use user_frame::CredentialOwnedUserFrame;
