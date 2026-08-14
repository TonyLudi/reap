//! Narrow Polymarket CLOB V2 authentication for the fixed PM-T1 profile.
//!
//! This crate owns synthetic-secret-safe EOA order signing, fixed-purpose L2
//! request authentication, and once-only place/cancel bytes. A separate
//! consume-only L1 request can ask a provisioning transport to derive an
//! existing API-key tuple. Bounded caller-input parsing can consume that tuple
//! into a local-equality holder with already-staged L2 credentials. That
//! holder can be consumed into one exact closed-only HMAC request while the
//! original L2 holder remains retained, but this crate has no response source,
//! transport, remote-acceptance proof, or credential-provisioning runtime. It
//! deliberately has no network, async runtime, strategy, state, generic
//! signing, or arbitrary request capability.

#![forbid(unsafe_code)]

mod credential_slot;
mod error;
mod identity;
mod l1_credential_derivation;
mod l1_credential_derivation_response;
mod l2;
mod legacy_type1_proxy;
mod order;
mod secret;
mod user_frame;

pub use credential_slot::{AuthenticatedJournalCredentialSlotFingerprint, CredentialSlotId};
pub use error::PmAuthError;
pub use identity::{
    EoaAddress, ExpectedOrderId, FixedOrderId, LegacyType1ProxyAddress,
    OwnedCancelSemanticRequestCommitment, PlaceSemanticRequestCommitment, PmClobDomain,
    RuntimeExactBodyCommitment,
};
pub use l1_credential_derivation::{
    AuthenticatedL1CredentialDerivationRequest, L1CredentialDerivationNonce,
    L1CredentialDerivationRequestSink, L1CredentialDerivationTimestamp,
};
pub use l1_credential_derivation_response::{
    L1CredentialDerivationMatchedClosedOnlyDispatch,
    L1CredentialDerivationMatchedClosedOnlyRequest, L1CredentialDerivationMatchedL2Credentials,
    L1CredentialDerivationResponseInput, MAX_L1_CREDENTIAL_DERIVATION_RESPONSE_BYTES,
};
pub use l2::{
    AuthenticatedCancelAllRequest, AuthenticatedGtcFillPlaceRequest, AuthenticatedL2Headers,
    AuthenticatedOrderHeartbeatRequest, AuthenticatedOwnedCancelRequest, AuthenticatedPlaceRequest,
    AuthenticatedUserSubscription, AuthenticatedUserSubscriptionSink, FixedCancelAllRequestSink,
    FixedClosedOnlyRequestSink, FixedGtcFillPlaceRequestSink, FixedOrderHeartbeatRequestSink,
    FixedOwnedCancelRequestSink, FixedPlaceRequestSink, L2HeaderSink, L2Timestamp,
};
pub use legacy_type1_proxy::{
    POLYMARKET_LEGACY_TYPE1_PROXY_CHAIN_ID, derive_legacy_type1_proxy_address,
    legacy_type1_proxy_address_matches,
};
pub use order::{
    PlacePublicRequestIdentity, SerializedGtcFillPlaceRequest, SerializedOwnedCancelRequest,
    SerializedPlaceRequest, SignedClobV2Order, SignedGtcFillClobV2Order,
    derive_gtc_fill_place_public_request_identity, derive_owned_cancel_semantic_request_commitment,
    derive_place_public_request_identity,
};
pub use secret::{EoaPrivateKeyInput, FixedEoaSigner, L2CredentialInput, L2Credentials};
pub use user_frame::CredentialOwnedUserFrame;
