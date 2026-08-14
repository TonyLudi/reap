//! Network-free PM-T2 controlled-trial live journal families.
//!
//! This crate persists production-shaped, secret-free evidence. It has no
//! credentials, authentication, transport, signed body, CLI, or mutation
//! operation. The V1 journal acknowledgements remain evidence-only; a separate
//! durable Phase-A barrier can mint one closed, move-only dispatch grant for a
//! runner to consume at a higher layer.

#![forbid(unsafe_code)]

#[cfg(not(target_os = "linux"))]
compile_error!("reap-pm-controlled-trial-live requires Linux descriptor semantics");

mod error;
mod hash;
mod journal;
mod live_dispatch;
mod online_preflight_v2;
mod phase_a_attempt_lineage_v4;
mod protected;
mod recovery;
mod recovery_continuation;
mod schema;

pub use error::PmTrialLiveJournalError;
pub use journal::{
    PmControlledTrialLiveCancelRecoveryJournals, PmControlledTrialLiveJournals,
    PmControlledTrialLiveRecoveryJournals, PmDurableCancelDispatchAckV1,
    PmDurableCancelIntentAckV1, PmDurableCancelOutcomeBridgeAckV1, PmDurableCancelPreparedAckV1,
    PmDurableCancelResultAckV1, PmDurableIntentTerminalAckV1,
    PmDurablePhaseALiveCancelDispatchAckV1, PmDurablePhaseALiveCancelIntentAckV1,
    PmDurablePhaseALiveCancelPreparedAckV1, PmDurablePhaseAPlaceLiveResultAckV1,
    PmDurablePlaceDispatchAckV1, PmDurablePlaceIntentAckV1, PmDurablePlaceOutcomeBridgeAckV1,
    PmDurablePlacePreparedAckV1, PmDurablePlaceResultAckV1, PmDurableReconciliationAckV1,
    PmJournalOwnedVenueOrderV1, PmPendingTrialLiveJournalsV1,
    PmPhaseALiveCancelDefinitelyNotDispatchedV1, PmPhaseALiveCancelDispatchOwnerV1,
    PmPhaseALiveCancelIntentCustodyV1, PmPhaseALiveCancelMayHaveBeenDispatchedV1,
    PmPhaseALiveCancelNetworkDispatchOwnerV1, PmPhaseALiveCancelOutcomeCustodyV1,
    PmPhaseALiveCancelPreparedCustodyV1, PmPhaseALiveCancelResultCustodyV1,
    PmPhaseALiveReconciliationCustodyV1, PmPhaseAPlaceDefinitelyNotDispatchedV1,
    PmPhaseAPlaceLiveDispatchOwnerV1, PmPhaseAPlaceLiveOutcomeBridgeAckV1,
    PmPhaseAPlaceLiveOutcomeCustodyV1, PmPhaseAPlaceLiveOwnedVenueOrderV1,
    PmPhaseAPlaceLiveResultCustodyV1, PmPhaseAPlaceMayHaveBeenDispatchedV1,
    PmPhaseAPlaceNetworkDispatchOwnerV1, PmPreparedConsumedAuthorizationProofV1,
    PmRevalidatedPhaseALiveCancelDispatchOwnerV1, PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1,
};
pub use live_dispatch::{
    PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1, PmPhaseAPlaceLiveDispatchProfileV1,
};
pub use online_preflight_v2::{
    PM_PHASE_A_ONLINE_PREFLIGHT_SIDECAR_FILE_V2, PmPendingPhaseAOnlinePreflightBasisV2,
    PmPhaseAOnlinePreflightDispatchOwnerV2, PmPhaseAOnlinePreflightEvidenceManifestV2,
    PmPhaseAOnlinePreflightInspectionV2, PmPhaseAOnlinePreflightNetworkDispatchOwnerV2,
    PmPhaseAOnlinePreflightPostA3FailureReasonV2, PmPhaseAOnlinePreflightPostA3FailureV2,
    PmPhaseAOnlinePreflightV2Error, create_phase_a_online_preflight_basis_v2,
    inspect_phase_a_online_preflight_v2,
};
pub use phase_a_attempt_lineage_v4::{
    PM_PHASE_A_ATTEMPT_BURN_CLAIM_FILE_V4, PM_PHASE_A_ATTEMPT_LINEAGE_LEDGER_FILE_V4,
    PmPhaseAAttemptLineageInspectionV4, inspect_phase_a_attempt_lineage_v4,
};
pub use recovery::{
    PmPhaseALiveCancelRecoveryRequiredActionV1, PmTrialLiveRecoveryClassificationV1,
    PmTrialLiveRecoveryProjectionV1, verify_controlled_trial_live_recovery,
};
pub use recovery_continuation::{
    PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_DISPATCH_FILE_V1,
    PM_PHASE_A_LIVE_CANCEL_RECOVERY_CONTINUATION_INTENT_FILE_V1,
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
