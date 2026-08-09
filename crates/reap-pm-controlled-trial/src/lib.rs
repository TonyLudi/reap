//! Offline-only PM-T2 controlled-trial plan, authorization, and custody gate.
//!
//! This crate deliberately has no network transport, mutation execution,
//! signed-order serialization, journal writer, or production authorization.

#![forbid(unsafe_code)]

mod config;
mod consumption;
mod custody;
mod preflight;
mod protected_file;

pub use config::{
    AuthorizationApproval, AuthorizationBuildBinding, AuthorizationHostBinding,
    AuthorizationVerification, CanonicalAuthorization, CanonicalTrialConfig, PlanVerification,
    PmTrialConfigError, TrialAccount, TrialAuthorization, TrialConfig, TrialCredentialSlot,
    TrialDomain, TrialJournalBinding, TrialMarket, TrialOrder, TrialOrderType, TrialPhase,
    TrialSide, TrialTimeLimits, load_canonical_authorization, load_canonical_trial_config,
    verify_authorization, verify_plan,
};
pub use consumption::{
    AuthorizationConsumptionBindingEvidence, AuthorizationConsumptionEvidence,
    AuthorizationConsumptionState, AuthorizationConsumptionVerification,
    AuthorizationRuntimeBinding, ConsumedAuthorizationConsumption, PmAuthorizationConsumptionError,
    PreparedAuthorizationConsumption, TerminalAuthorizationConsumption, TerminalDisposition,
    claim_prepared_authorization_consumption, prepare_authorization_consumption,
    verify_authorization_consumption,
};
pub use custody::{
    CustodyInspection, CustodyPaths, CustodySummary, PmTrialCustodyError, inspect_custody,
};
pub use preflight::{
    CanonicalTrialPreflight, PmTrialPreflightError, TRIAL_PREFLIGHT_SCHEMA_VERSION,
    TrialAccountPreflight, TrialAuthorizationConsumptionLeaseState, TrialBookPreflight,
    TrialClosedOnlyEvidence, TrialCompleteCutEvidence, TrialConfiguredPositionState,
    TrialDataApiPositionPreflight, TrialEnvironmentPreflight, TrialExactDetailCutEvidence,
    TrialFinalizedChainPreflight, TrialGeoblockEvidence, TrialJournalLeaseEvidence,
    TrialMarketPreflight, TrialObservationStamp, TrialPhaseGateEvidence, TrialPreflightBinding,
    TrialPreflightEvidence, TrialPreflightWindow, TrialPrivateAccountCut,
    TrialReconciliationPreflight, TrialRiskPreflight, TrialServerTimeEvidence,
    TrialUserStreamPreflight, validate_canonical_trial_preflight,
};

/// Hard offline outcome of every command in this A0 executable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfflineAuthorizationState {
    pub production_order_entry_authorized: bool,
    pub real_order_submission_authorized: bool,
    pub place_dispatch_allowance: u8,
}

impl OfflineAuthorizationState {
    pub const DENIED: Self = Self {
        production_order_entry_authorized: false,
        real_order_submission_authorized: false,
        place_dispatch_allowance: 0,
    };
}
