//! Network-free PM-T2 controlled-trial live journal families.
//!
//! This crate persists production-shaped, secret-free evidence. It has no
//! credentials, authentication, transport, signed body, CLI, or mutation
//! capability. Durable dispatch acknowledgements are not network send grants.

#![forbid(unsafe_code)]

#[cfg(not(target_os = "linux"))]
compile_error!("reap-pm-controlled-trial-live requires Linux descriptor semantics");

mod error;
mod hash;
mod journal;
mod protected;
mod recovery;
mod schema;

pub use error::PmTrialLiveJournalError;
pub use journal::{
    PmControlledTrialLiveJournals, PmControlledTrialLiveRecoveryJournals,
    PmDurableCancelDispatchAckV1, PmDurableCancelIntentAckV1, PmDurableCancelOutcomeBridgeAckV1,
    PmDurableCancelPreparedAckV1, PmDurableCancelResultAckV1, PmDurableIntentTerminalAckV1,
    PmDurablePlaceDispatchAckV1, PmDurablePlaceIntentAckV1, PmDurablePlaceOutcomeBridgeAckV1,
    PmDurablePlacePreparedAckV1, PmDurablePlaceResultAckV1, PmDurableReconciliationAckV1,
    PmJournalOwnedVenueOrderV1, PmPendingTrialLiveJournalsV1,
    PmPreparedConsumedAuthorizationProofV1,
};
pub use recovery::{
    PmTrialLiveRecoveryClassificationV1, PmTrialLiveRecoveryProjectionV1,
    verify_controlled_trial_live_recovery,
};
pub use schema::{
    PM_TRIAL_LIVE_DISPATCH_FILE_V1, PM_TRIAL_LIVE_INTENT_FILE_V1, PmCancelDispatchClassV1,
    PmCancelPreparationV1, PmCancelPreparationViewV1, PmCancelResultKindV1,
    PmIntentTerminalDispositionV1, PmPlacePreparationV1, PmPlacePreparationViewV1,
    PmPlaceResultKindV1, PmReconciliationOrderStateV1, PmTrialLivePreflightBindingV1,
};

pub const PRODUCTION_ORDER_ENTRY_AUTHORIZED: bool = false;
pub const REAL_ORDER_SUBMISSION_AUTHORIZED: bool = false;
pub const PLACE_DISPATCH_ALLOWANCE: u8 = 0;
