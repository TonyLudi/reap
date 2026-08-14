//! Protected, canonical, offline Phase-A eligibility review envelope V4.
//!
//! This additive record binds the exact canonical V1 config/authorization,
//! V2 policy/authorization, five reviewed V1 sidecars, and the frozen static
//! V3 conjunction. It does not reinterpret V3 and it does not make any of
//! those denied structural records positive.
//!
//! The external facts required for a positive Phase-A attempt are represented
//! only by distinct closed `required_*_unavailable_v1` enum variants. There is
//! deliberately no field for a caller-selected trust digest, public key,
//! signature, lease, HTTP response, chain observation, actor generation,
//! runtime nonce, Prepared record, consumption claim, or dispatch grant.
//! In particular, `reviewer_label` is an unauthenticated display label.
//! It is not a reviewer identity or trust anchor.
//!
//! The optional draft helper derives exact pins from already-loaded canonical
//! holders. It writes nothing, signs nothing, accepts no external proof, and
//! always emits the same unavailable requirements. Loading, drafting, and
//! verifying V4 are offline and non-authorizing. They never consume the
//! retained delivery evidence/load token, open a journal, sample a clock, load
//! a credential, construct authentication material, or mint runtime, send,
//! Prepared, claim, burn, A3, recovery, or no-resend capability.

use std::{fmt, path::Path};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    CanonicalAuthorization, CanonicalFreshCredentialDeliveryBindingV1,
    CanonicalOnlineAuthorizationV2, CanonicalOnlinePolicyV2,
    CanonicalReviewedFreshCredentialSlotLocatorV1, CanonicalReviewedProductionDestinationProfileV1,
    CanonicalReviewedRemoteCredentialProofPolicyV1, CanonicalReviewedSignerProxyAccountIdentityV1,
    CanonicalReviewedStaticOnlineAuthorizationV3, CanonicalTrialConfig,
    FRESH_CREDENTIAL_DELIVERY_BINDING_V1_SCHEMA_VERSION, ONLINE_AUTHORIZATION_V2_SCHEMA_VERSION,
    ONLINE_POLICY_V2_SCHEMA_VERSION, OfflineAuthorizationState,
    REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_SCHEMA_VERSION,
    REVIEWED_PRODUCTION_DESTINATION_PROFILE_V1_SCHEMA_VERSION,
    REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_SCHEMA_VERSION,
    REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_SCHEMA_VERSION,
    REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3_SCHEMA_VERSION,
    ReviewedStaticOnlineAuthorizationContextV3, TrialPhase,
    config::{TRIAL_AUTHORIZATION_SCHEMA_VERSION, TRIAL_CONFIG_SCHEMA_VERSION},
    protected_file::{ProtectedFileKind, read_one},
    verify_reviewed_static_online_authorization_v3,
};

pub const REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4_SCHEMA_VERSION: u32 = 4;
pub const PM_T2_REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_FILE_V4: &str =
    "pm-t2-reviewed-phase-a-eligibility-envelope-v4.json";

const MAX_CANONICAL_REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_BYTES_V4: usize = 160 * 1024;
const REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.controlled-trial.reviewed-phase-a-eligibility-envelope.v4\0";

/// Exact role pin for the canonical V1 config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPhaseAConfigPinsV4 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
    pub plan_fingerprint: String,
}

/// Exact role pin for the canonical V1 authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPhaseAV1AuthorizationPinsV4 {
    pub schema_version: u32,
    pub authorization_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact role pin for the canonical online policy V2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPhaseAOnlinePolicyPinsV4 {
    pub schema_version: u32,
    pub policy_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact role pin for the canonical online authorization V2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPhaseAOnlineAuthorizationPinsV4 {
    pub schema_version: u32,
    pub authorization_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact role pin for the reviewed destination profile V1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPhaseADestinationPinsV4 {
    pub schema_version: u32,
    pub profile_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact role pin for the reviewed fresh credential-slot locator V1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPhaseALocatorPinsV4 {
    pub schema_version: u32,
    pub locator_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact role pin for the unsigned credential delivery binding V1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPhaseADeliveryPinsV4 {
    pub schema_version: u32,
    pub binding_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact role pin for the reviewed signer/proxy identity V1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPhaseAAccountIdentityPinsV4 {
    pub schema_version: u32,
    pub identity_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact role pin for the reviewed remote credential-proof policy V1.
///
/// Its move-only holder exposes no policy ID. SHA-256, exact length, and its
/// domain-separated fingerprint commit the complete canonical record,
/// including the private ID, without accepting an uncorrelated caller label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPhaseARemoteProofPolicyPinsV4 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact role pin for the frozen static conjunction V3.
///
/// The static V3 holder intentionally exposes no authorization ID. Its three
/// exact byte commitments bind that hidden field without adding a projection
/// to the frozen type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPhaseAStaticAuthorizationPinsV4 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedPhaseARecordRoleV4 {
    #[serde(rename = "offline_eligibility_conjunction_only_no_authorization_v1")]
    OfflineEligibilityConjunctionOnlyNoAuthorizationV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedPhaseAScopeV4 {
    #[serde(rename = "phase_a_exactly_one_place_then_exact_cancel_only_v1")]
    PhaseAExactlyOnePlaceThenExactCancelOnlyV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedPhaseAReviewerTrustAnchorStatusV4 {
    #[serde(rename = "required_external_reviewer_trust_anchor_unavailable_v1")]
    RequiredExternalReviewerTrustAnchorUnavailableV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedPhaseACredentialProviderTrustRootStatusV4 {
    #[serde(rename = "required_authenticated_provider_trust_root_unavailable_v1")]
    RequiredAuthenticatedProviderTrustRootUnavailableV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedPhaseACredentialDeliveryLeaseProofStatusV4 {
    #[serde(rename = "required_provider_signed_attempt_audience_lease_unavailable_v1")]
    RequiredProviderSignedAttemptAudienceLeaseUnavailableV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedPhaseARemoteAcceptanceContractStatusV4 {
    #[serde(rename = "required_authoritative_remote_acceptance_contract_unavailable_v1")]
    RequiredAuthoritativeRemoteAcceptanceContractUnavailableV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedPhaseARemoteCredentialProofStatusV4 {
    #[serde(rename = "required_same_holder_live_remote_acceptance_proof_unavailable_v1")]
    RequiredSameHolderLiveRemoteAcceptanceProofUnavailableV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedPhaseASignerProxyContractStatusV4 {
    #[serde(rename = "required_authoritative_signer_proxy_control_contract_unavailable_v1")]
    RequiredAuthoritativeSignerProxyControlContractUnavailableV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedPhaseASignerProxyProofStatusV4 {
    #[serde(rename = "required_account_specific_current_unrevoked_control_proof_unavailable_v1")]
    RequiredAccountSpecificCurrentUnrevokedControlProofUnavailableV1,
}

/// Closed negative requirements. No key, digest, signature, lease, response,
/// or chain claim can be substituted for one of these variants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPhaseAUnavailableExternalEvidenceV4 {
    pub reviewer_trust_anchor_status: ReviewedPhaseAReviewerTrustAnchorStatusV4,
    pub credential_provider_trust_root_status: ReviewedPhaseACredentialProviderTrustRootStatusV4,
    pub provider_signed_credential_delivery_lease_proof_status:
        ReviewedPhaseACredentialDeliveryLeaseProofStatusV4,
    pub authoritative_remote_credential_acceptance_contract_status:
        ReviewedPhaseARemoteAcceptanceContractStatusV4,
    pub same_holder_live_remote_credential_acceptance_proof_status:
        ReviewedPhaseARemoteCredentialProofStatusV4,
    pub authoritative_signer_proxy_control_contract_status:
        ReviewedPhaseASignerProxyContractStatusV4,
    pub account_specific_signer_proxy_control_proof_status: ReviewedPhaseASignerProxyProofStatusV4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedPhaseAActorRuntimeAttemptStatusV4 {
    #[serde(rename = "required_future_selected_actor_prepared_lineage_unavailable_v1")]
    RequiredFutureSelectedActorPreparedLineageUnavailableV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedPhaseADeliveryCustodyJoinStatusV4 {
    #[serde(rename = "required_future_retained_evidence_load_token_join_unavailable_v1")]
    RequiredFutureRetainedEvidenceLoadTokenJoinUnavailableV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedPhaseADurableAttemptLineageStatusV4 {
    #[serde(rename = "required_future_create_new_claim_a3_lineage_unavailable_v1")]
    RequiredFutureCreateNewClaimA3LineageUnavailableV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedPhaseASingleDispatchStatusV4 {
    #[serde(rename = "required_future_selected_egress_single_dispatch_owner_unavailable_v1")]
    RequiredFutureSelectedEgressSingleDispatchOwnerUnavailableV1,
}

/// Closed negative runtime requirements. These labels are not actor,
/// Prepared, claim, burn, network, or no-resend evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPhaseAUnavailableRuntimeLineageV4 {
    pub selected_actor_generation_and_source_owned_attempt_status:
        ReviewedPhaseAActorRuntimeAttemptStatusV4,
    pub retained_delivery_evidence_and_load_token_join_status:
        ReviewedPhaseADeliveryCustodyJoinStatusV4,
    pub durable_attempt_claim_and_final_conjunct_status:
        ReviewedPhaseADurableAttemptLineageStatusV4,
    pub fixed_egress_single_dispatch_owner_status: ReviewedPhaseASingleDispatchStatusV4,
}

/// Serializable offline draft/record. Arbitrary ID and reviewer-label strings
/// cannot be proven secret-free; operators must never place secrets or
/// secret-derived material in them.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPhaseAEligibilityEnvelopeV4 {
    pub schema_version: u32,
    pub eligibility_record_id: String,
    pub reviewer_label: String,
    pub reviewed_at_utc: String,
    pub not_before_utc: String,
    pub expires_at_utc: String,
    pub cleanup_not_after_utc: String,
    pub record_role: ReviewedPhaseARecordRoleV4,
    pub phase_scope: ReviewedPhaseAScopeV4,
    pub v1_config: ReviewedPhaseAConfigPinsV4,
    pub v1_authorization: ReviewedPhaseAV1AuthorizationPinsV4,
    pub online_policy_v2: ReviewedPhaseAOnlinePolicyPinsV4,
    pub online_authorization_v2: ReviewedPhaseAOnlineAuthorizationPinsV4,
    pub reviewed_production_destination_v1: ReviewedPhaseADestinationPinsV4,
    pub reviewed_fresh_credential_slot_locator_v1: ReviewedPhaseALocatorPinsV4,
    pub fresh_credential_delivery_binding_v1: ReviewedPhaseADeliveryPinsV4,
    pub reviewed_signer_proxy_account_identity_v1: ReviewedPhaseAAccountIdentityPinsV4,
    pub reviewed_remote_credential_proof_policy_v1: ReviewedPhaseARemoteProofPolicyPinsV4,
    pub reviewed_static_online_authorization_v3: ReviewedPhaseAStaticAuthorizationPinsV4,
    pub required_unavailable_external_evidence: ReviewedPhaseAUnavailableExternalEvidenceV4,
    pub required_unavailable_runtime_lineage: ReviewedPhaseAUnavailableRuntimeLineageV4,
}

impl fmt::Debug for ReviewedPhaseAEligibilityEnvelopeV4 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "ReviewedPhaseAEligibilityEnvelopeV4(<ten-holder-offline-eligibility; external-and-runtime-requirements-unavailable; denied>)",
        )
    }
}

struct ValidatedPhaseATimesV4 {
    reviewed_at: DateTime<Utc>,
    not_before: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    cleanup_not_after: DateTime<Utc>,
}

impl ReviewedPhaseAEligibilityEnvelopeV4 {
    fn validate_intrinsic(
        &self,
    ) -> Result<ValidatedPhaseATimesV4, PmReviewedPhaseAEligibilityEnvelopeV4Error> {
        if self.schema_version != REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4_SCHEMA_VERSION {
            return Err(invalid(
                "unsupported reviewed Phase-A eligibility envelope V4 schema",
            ));
        }
        validate_token(
            &self.eligibility_record_id,
            128,
            "reviewed Phase-A eligibility record ID is invalid",
        )?;
        validate_reference(
            &self.reviewer_label,
            "reviewed Phase-A reviewer label is invalid",
        )?;
        self.v1_config.validate()?;
        self.v1_authorization.validate()?;
        self.online_policy_v2.validate()?;
        self.online_authorization_v2.validate()?;
        self.reviewed_production_destination_v1.validate()?;
        self.reviewed_fresh_credential_slot_locator_v1.validate()?;
        self.fresh_credential_delivery_binding_v1.validate()?;
        self.reviewed_signer_proxy_account_identity_v1.validate()?;
        self.reviewed_remote_credential_proof_policy_v1.validate()?;
        self.reviewed_static_online_authorization_v3.validate()?;

        let times = ValidatedPhaseATimesV4 {
            reviewed_at: parse_utc(&self.reviewed_at_utc)?,
            not_before: parse_utc(&self.not_before_utc)?,
            expires_at: parse_utc(&self.expires_at_utc)?,
            cleanup_not_after: parse_utc(&self.cleanup_not_after_utc)?,
        };
        if times.reviewed_at > times.not_before
            || times.not_before >= times.expires_at
            || times.expires_at > times.cleanup_not_after
        {
            return Err(invalid(
                "reviewed Phase-A eligibility envelope V4 time envelope is invalid",
            ));
        }
        Ok(times)
    }
}

macro_rules! impl_exact_role_pin_validation {
    ($type_name:ty, $expected_schema:expr, $role_id:ident, $message:literal) => {
        impl $type_name {
            fn validate(&self) -> Result<(), PmReviewedPhaseAEligibilityEnvelopeV4Error> {
                validate_exact_pin(
                    ExactPinViewV4 {
                        schema_version: self.schema_version,
                        expected_schema_version: $expected_schema,
                        role_id: Some(&self.$role_id),
                        canonical_sha256: &self.canonical_sha256,
                        canonical_length: self.canonical_length,
                        fingerprint: &self.fingerprint,
                    },
                    $message,
                )
            }
        }
    };
}

impl_exact_role_pin_validation!(
    ReviewedPhaseAV1AuthorizationPinsV4,
    TRIAL_AUTHORIZATION_SCHEMA_VERSION,
    authorization_id,
    "reviewed Phase-A V1 authorization pin is invalid"
);
impl_exact_role_pin_validation!(
    ReviewedPhaseAOnlinePolicyPinsV4,
    ONLINE_POLICY_V2_SCHEMA_VERSION,
    policy_id,
    "reviewed Phase-A online policy V2 pin is invalid"
);
impl_exact_role_pin_validation!(
    ReviewedPhaseAOnlineAuthorizationPinsV4,
    ONLINE_AUTHORIZATION_V2_SCHEMA_VERSION,
    authorization_id,
    "reviewed Phase-A online authorization V2 pin is invalid"
);
impl_exact_role_pin_validation!(
    ReviewedPhaseADestinationPinsV4,
    REVIEWED_PRODUCTION_DESTINATION_PROFILE_V1_SCHEMA_VERSION,
    profile_id,
    "reviewed Phase-A destination pin is invalid"
);
impl_exact_role_pin_validation!(
    ReviewedPhaseALocatorPinsV4,
    REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_SCHEMA_VERSION,
    locator_id,
    "reviewed Phase-A locator pin is invalid"
);
impl_exact_role_pin_validation!(
    ReviewedPhaseADeliveryPinsV4,
    FRESH_CREDENTIAL_DELIVERY_BINDING_V1_SCHEMA_VERSION,
    binding_id,
    "reviewed Phase-A delivery pin is invalid"
);
impl_exact_role_pin_validation!(
    ReviewedPhaseAAccountIdentityPinsV4,
    REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_SCHEMA_VERSION,
    identity_id,
    "reviewed Phase-A account-identity pin is invalid"
);

impl ReviewedPhaseAConfigPinsV4 {
    fn validate(&self) -> Result<(), PmReviewedPhaseAEligibilityEnvelopeV4Error> {
        validate_exact_pin(
            ExactPinViewV4 {
                schema_version: self.schema_version,
                expected_schema_version: TRIAL_CONFIG_SCHEMA_VERSION,
                role_id: None,
                canonical_sha256: &self.canonical_sha256,
                canonical_length: self.canonical_length,
                fingerprint: &self.fingerprint,
            },
            "reviewed Phase-A V1 config pin is invalid",
        )?;
        validate_sha256(&self.plan_fingerprint)
    }
}

impl ReviewedPhaseARemoteProofPolicyPinsV4 {
    fn validate(&self) -> Result<(), PmReviewedPhaseAEligibilityEnvelopeV4Error> {
        validate_exact_pin(
            ExactPinViewV4 {
                schema_version: self.schema_version,
                expected_schema_version: REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_SCHEMA_VERSION,
                role_id: None,
                canonical_sha256: &self.canonical_sha256,
                canonical_length: self.canonical_length,
                fingerprint: &self.fingerprint,
            },
            "reviewed Phase-A remote proof-policy pin is invalid",
        )
    }
}

impl ReviewedPhaseAStaticAuthorizationPinsV4 {
    fn validate(&self) -> Result<(), PmReviewedPhaseAEligibilityEnvelopeV4Error> {
        validate_exact_pin(
            ExactPinViewV4 {
                schema_version: self.schema_version,
                expected_schema_version: REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3_SCHEMA_VERSION,
                role_id: None,
                canonical_sha256: &self.canonical_sha256,
                canonical_length: self.canonical_length,
                fingerprint: &self.fingerprint,
            },
            "reviewed Phase-A static authorization V3 pin is invalid",
        )
    }
}

/// Move-only, non-serializable, non-projecting holder of protected canonical
/// V4 bytes. It is structural evidence, never an authorization capability.
pub struct CanonicalReviewedPhaseAEligibilityEnvelopeV4 {
    value: ReviewedPhaseAEligibilityEnvelopeV4,
    canonical_bytes: Vec<u8>,
    canonical_sha256: String,
    fingerprint: String,
}

impl CanonicalReviewedPhaseAEligibilityEnvelopeV4 {
    #[must_use]
    pub fn canonical_sha256(&self) -> &str {
        &self.canonical_sha256
    }

    #[must_use]
    pub fn canonical_length(&self) -> u64 {
        self.canonical_bytes.len() as u64
    }

    #[must_use]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

impl fmt::Debug for CanonicalReviewedPhaseAEligibilityEnvelopeV4 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "CanonicalReviewedPhaseAEligibilityEnvelopeV4(<exact-protected-canonical-bytes; no-value-proof-actor-runtime-or-authority-projection; redacted; denied>)",
        )
    }
}

/// Borrowed ten-holder context. All inputs are canonical holders; there is no
/// caller-supplied hash, verification boolean, proof, actor, clock, or token.
pub struct ReviewedPhaseAEligibilityEnvelopeContextV4<'a> {
    pub v1_config: &'a CanonicalTrialConfig,
    pub v1_authorization: &'a CanonicalAuthorization,
    pub online_policy_v2: &'a CanonicalOnlinePolicyV2,
    pub online_authorization_v2: &'a CanonicalOnlineAuthorizationV2,
    pub reviewed_production_destination_v1: &'a CanonicalReviewedProductionDestinationProfileV1,
    pub reviewed_fresh_credential_slot_locator_v1:
        &'a CanonicalReviewedFreshCredentialSlotLocatorV1,
    pub fresh_credential_delivery_binding_v1: &'a CanonicalFreshCredentialDeliveryBindingV1,
    pub reviewed_signer_proxy_account_identity_v1:
        &'a CanonicalReviewedSignerProxyAccountIdentityV1,
    pub reviewed_remote_credential_proof_policy_v1:
        &'a CanonicalReviewedRemoteCredentialProofPolicyV1,
    pub reviewed_static_online_authorization_v3: &'a CanonicalReviewedStaticOnlineAuthorizationV3,
}

/// Human-entered labels and time envelope for pure non-authorizing drafting.
/// All exact pins and all closed unavailable requirements are derived by the
/// crate rather than accepted from the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewedPhaseAEligibilityEnvelopeDraftInputsV4 {
    pub eligibility_record_id: String,
    pub reviewer_label: String,
    pub reviewed_at_utc: String,
    pub not_before_utc: String,
    pub expires_at_utc: String,
    pub cleanup_not_after_utc: String,
}

/// Offline structural display. Exact pin checks alone are true. Every trust,
/// live, eligibility, transition, and mutation fact remains false.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewedPhaseAEligibilityEnvelopeVerificationV4 {
    pub schema_version: u32,
    pub v1_config_fingerprint: String,
    pub v1_authorization_fingerprint: String,
    pub online_policy_v2_fingerprint: String,
    pub online_authorization_v2_fingerprint: String,
    pub reviewed_production_destination_v1_fingerprint: String,
    pub reviewed_fresh_credential_slot_locator_v1_fingerprint: String,
    pub fresh_credential_delivery_binding_v1_fingerprint: String,
    pub reviewed_signer_proxy_account_identity_v1_fingerprint: String,
    pub reviewed_remote_credential_proof_policy_v1_fingerprint: String,
    pub reviewed_static_online_authorization_v3_fingerprint: String,
    pub reviewed_phase_a_eligibility_envelope_v4_fingerprint: String,
    pub exact_v1_config_pin_structurally_valid: bool,
    pub exact_v1_authorization_pin_structurally_valid: bool,
    pub exact_online_policy_v2_pin_structurally_valid: bool,
    pub exact_online_authorization_v2_pin_structurally_valid: bool,
    pub exact_reviewed_production_destination_v1_pin_structurally_valid: bool,
    pub exact_reviewed_fresh_credential_slot_locator_v1_pin_structurally_valid: bool,
    pub exact_fresh_credential_delivery_binding_v1_pin_structurally_valid: bool,
    pub exact_reviewed_signer_proxy_account_identity_v1_pin_structurally_valid: bool,
    pub exact_reviewed_remote_credential_proof_policy_v1_pin_structurally_valid: bool,
    pub exact_reviewed_static_online_authorization_v3_pin_structurally_valid: bool,
    pub exact_phase_a_scope_and_v2_time_envelope_structurally_valid: bool,
    pub frozen_static_v3_reverified_as_denied_structural_evidence: bool,
    pub unavailable_external_requirements_structurally_valid: bool,
    pub unavailable_runtime_requirements_structurally_valid: bool,
    pub static_v3_existed_at_v4_review_time_attested: bool,
    pub reviewer_trust_anchor_available: bool,
    pub reviewer_authorship_attested: bool,
    pub credential_provider_trust_root_available: bool,
    pub credential_provider_trust_root_authenticated: bool,
    pub credential_provider_signature_verified: bool,
    pub credential_delivery_lease_verified: bool,
    pub credential_delivery_lease_current_and_unrevoked: bool,
    pub delivery_generation_attested: bool,
    pub authoritative_remote_credential_acceptance_contract_available: bool,
    pub same_holder_live_remote_credential_acceptance_proof_verified: bool,
    pub live_credential_tuple_accepted_by_provider: bool,
    pub authoritative_signer_proxy_control_contract_available: bool,
    pub account_specific_signer_proxy_control_proof_verified: bool,
    pub signer_controls_proxy_current_and_unrevoked_attested: bool,
    pub source_owned_current_time_checked: bool,
    pub v1_authorization_current: bool,
    pub online_authorization_v2_current: bool,
    pub static_online_authorization_v3_current: bool,
    pub reviewed_phase_a_eligibility_envelope_v4_current: bool,
    pub retained_delivery_evidence_joined: bool,
    pub fresh_credential_delivery_load_token_consumed: bool,
    pub loaded_credentials_match_delivery_binding: bool,
    pub same_loaded_credential_holder_attested: bool,
    pub selected_egress_actor_started: bool,
    pub selected_actor_generation_bound: bool,
    pub source_owned_runtime_attempt_bound: bool,
    pub runtime_attempt_fresh: bool,
    pub positive_external_proof_bundle_complete: bool,
    pub offline_phase_a_eligibility_established: bool,
    pub positive_runtime_lineage_implemented: bool,
    pub durable_preparation_record_created: bool,
    pub atomic_consumption_claim_created: bool,
    pub authorization_burn_performed: bool,
    pub final_a3_conjunct_durable: bool,
    pub burn_and_no_resend_established: bool,
    pub authenticated_request_or_hmac_constructed: bool,
    pub signed_order_body_constructed: bool,
    pub place_dispatch_owner_or_grant_minted: bool,
    pub network_dispatch_performed: bool,
    pub placement_resumption_allowed: bool,
    pub credential_mutation_authority_attested: bool,
    #[serde(flatten)]
    pub authorization: OfflineAuthorizationState,
}

/// Pure offline drafting from canonical holders. The resulting record keeps
/// all external and runtime requirements unavailable and authorizes nothing.
pub fn draft_non_authorizing_reviewed_phase_a_eligibility_envelope_v4(
    context: &ReviewedPhaseAEligibilityEnvelopeContextV4<'_>,
    inputs: ReviewedPhaseAEligibilityEnvelopeDraftInputsV4,
) -> Result<ReviewedPhaseAEligibilityEnvelopeV4, PmReviewedPhaseAEligibilityEnvelopeV4Error> {
    let metadata = verify_denied_context(context)?;
    let v1_authorization_bytes =
        serde_json::to_vec(context.v1_authorization.value()).map_err(|_| {
            invalid("reviewed Phase-A V1 authorization cannot be reconstructed canonically")
        })?;

    let record = ReviewedPhaseAEligibilityEnvelopeV4 {
        schema_version: REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4_SCHEMA_VERSION,
        eligibility_record_id: inputs.eligibility_record_id,
        reviewer_label: inputs.reviewer_label,
        reviewed_at_utc: inputs.reviewed_at_utc,
        not_before_utc: inputs.not_before_utc,
        expires_at_utc: inputs.expires_at_utc,
        cleanup_not_after_utc: inputs.cleanup_not_after_utc,
        record_role: ReviewedPhaseARecordRoleV4::OfflineEligibilityConjunctionOnlyNoAuthorizationV1,
        phase_scope: ReviewedPhaseAScopeV4::PhaseAExactlyOnePlaceThenExactCancelOnlyV1,
        v1_config: ReviewedPhaseAConfigPinsV4 {
            schema_version: context.v1_config.value().schema_version,
            canonical_sha256: context.v1_config.canonical_sha256().to_owned(),
            canonical_length: context.v1_config.canonical_length(),
            fingerprint: context.v1_config.fingerprint().to_owned(),
            plan_fingerprint: context.v1_config.plan_fingerprint().to_owned(),
        },
        v1_authorization: ReviewedPhaseAV1AuthorizationPinsV4 {
            schema_version: context.v1_authorization.value().schema_version,
            authorization_id: context.v1_authorization.value().authorization_id.clone(),
            canonical_sha256: hash_bytes(&[], &v1_authorization_bytes),
            canonical_length: v1_authorization_bytes.len() as u64,
            fingerprint: context.v1_authorization.fingerprint().to_owned(),
        },
        online_policy_v2: ReviewedPhaseAOnlinePolicyPinsV4 {
            schema_version: context.online_policy_v2.value().schema_version,
            policy_id: context.online_policy_v2.value().policy_id.clone(),
            canonical_sha256: context.online_policy_v2.canonical_sha256().to_owned(),
            canonical_length: context.online_policy_v2.canonical_length(),
            fingerprint: context.online_policy_v2.fingerprint().to_owned(),
        },
        online_authorization_v2: ReviewedPhaseAOnlineAuthorizationPinsV4 {
            schema_version: context.online_authorization_v2.value().schema_version,
            authorization_id: context
                .online_authorization_v2
                .value()
                .authorization_id
                .clone(),
            canonical_sha256: context
                .online_authorization_v2
                .canonical_sha256()
                .to_owned(),
            canonical_length: context.online_authorization_v2.canonical_length(),
            fingerprint: context.online_authorization_v2.fingerprint().to_owned(),
        },
        reviewed_production_destination_v1: ReviewedPhaseADestinationPinsV4 {
            schema_version: metadata.destination_schema_version,
            profile_id: metadata.destination_id,
            canonical_sha256: context
                .reviewed_production_destination_v1
                .canonical_sha256()
                .to_owned(),
            canonical_length: context
                .reviewed_production_destination_v1
                .canonical_length(),
            fingerprint: context
                .reviewed_production_destination_v1
                .fingerprint()
                .to_owned(),
        },
        reviewed_fresh_credential_slot_locator_v1: ReviewedPhaseALocatorPinsV4 {
            schema_version: metadata.locator_schema_version,
            locator_id: metadata.locator_id,
            canonical_sha256: context
                .reviewed_fresh_credential_slot_locator_v1
                .canonical_sha256()
                .to_owned(),
            canonical_length: context
                .reviewed_fresh_credential_slot_locator_v1
                .canonical_length(),
            fingerprint: context
                .reviewed_fresh_credential_slot_locator_v1
                .fingerprint()
                .to_owned(),
        },
        fresh_credential_delivery_binding_v1: ReviewedPhaseADeliveryPinsV4 {
            schema_version: metadata.delivery_schema_version,
            binding_id: metadata.delivery_id,
            canonical_sha256: context
                .fresh_credential_delivery_binding_v1
                .canonical_sha256()
                .to_owned(),
            canonical_length: context
                .fresh_credential_delivery_binding_v1
                .canonical_length(),
            fingerprint: context
                .fresh_credential_delivery_binding_v1
                .fingerprint()
                .to_owned(),
        },
        reviewed_signer_proxy_account_identity_v1: ReviewedPhaseAAccountIdentityPinsV4 {
            schema_version: metadata.identity_schema_version,
            identity_id: metadata.identity_id,
            canonical_sha256: context
                .reviewed_signer_proxy_account_identity_v1
                .canonical_sha256()
                .to_owned(),
            canonical_length: context
                .reviewed_signer_proxy_account_identity_v1
                .canonical_length(),
            fingerprint: context
                .reviewed_signer_proxy_account_identity_v1
                .fingerprint()
                .to_owned(),
        },
        reviewed_remote_credential_proof_policy_v1: ReviewedPhaseARemoteProofPolicyPinsV4 {
            schema_version: REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_SCHEMA_VERSION,
            canonical_sha256: context
                .reviewed_remote_credential_proof_policy_v1
                .canonical_sha256()
                .to_owned(),
            canonical_length: context
                .reviewed_remote_credential_proof_policy_v1
                .canonical_length(),
            fingerprint: context
                .reviewed_remote_credential_proof_policy_v1
                .fingerprint()
                .to_owned(),
        },
        reviewed_static_online_authorization_v3: ReviewedPhaseAStaticAuthorizationPinsV4 {
            schema_version: REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3_SCHEMA_VERSION,
            canonical_sha256: context
                .reviewed_static_online_authorization_v3
                .canonical_sha256()
                .to_owned(),
            canonical_length: context
                .reviewed_static_online_authorization_v3
                .canonical_length(),
            fingerprint: context
                .reviewed_static_online_authorization_v3
                .fingerprint()
                .to_owned(),
        },
        required_unavailable_external_evidence: unavailable_external_evidence(),
        required_unavailable_runtime_lineage: unavailable_runtime_lineage(),
    };
    let times = record.validate_intrinsic()?;
    validate_phase_and_time_binding(context, &times, &record)?;
    Ok(record)
}

/// Load exact compact JSON from one protected 0600 file without creating an
/// authoring, trust, runtime, or mutation capability.
pub fn load_canonical_reviewed_phase_a_eligibility_envelope_v4(
    path: &Path,
) -> Result<CanonicalReviewedPhaseAEligibilityEnvelopeV4, PmReviewedPhaseAEligibilityEnvelopeV4Error>
{
    let bytes = read_one(
        path,
        ProtectedFileKind::ReviewedPhaseAEligibilityEnvelopeV4,
        MAX_CANONICAL_REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_BYTES_V4,
    )
    .map_err(|_| {
        invalid("reviewed Phase-A eligibility envelope V4 protection or stability check failed")
    })?;
    let value: ReviewedPhaseAEligibilityEnvelopeV4 = parse_exact_canonical(&bytes)?;
    let _ = value.validate_intrinsic()?;
    let canonical_bytes = bytes.to_vec();
    Ok(CanonicalReviewedPhaseAEligibilityEnvelopeV4 {
        canonical_sha256: hash_bytes(&[], &canonical_bytes),
        fingerprint: hash_bytes(
            REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4_FINGERPRINT_DOMAIN,
            &canonical_bytes,
        ),
        canonical_bytes,
        value,
    })
}

/// Verify exact static pins and closed unavailable requirements. The output
/// is a denied report and cannot be transformed into runtime authority.
pub fn verify_reviewed_phase_a_eligibility_envelope_v4(
    context: &ReviewedPhaseAEligibilityEnvelopeContextV4<'_>,
    reviewed: &CanonicalReviewedPhaseAEligibilityEnvelopeV4,
) -> Result<
    ReviewedPhaseAEligibilityEnvelopeVerificationV4,
    PmReviewedPhaseAEligibilityEnvelopeV4Error,
> {
    let expected = draft_non_authorizing_reviewed_phase_a_eligibility_envelope_v4(
        context,
        ReviewedPhaseAEligibilityEnvelopeDraftInputsV4 {
            eligibility_record_id: reviewed.value.eligibility_record_id.clone(),
            reviewer_label: reviewed.value.reviewer_label.clone(),
            reviewed_at_utc: reviewed.value.reviewed_at_utc.clone(),
            not_before_utc: reviewed.value.not_before_utc.clone(),
            expires_at_utc: reviewed.value.expires_at_utc.clone(),
            cleanup_not_after_utc: reviewed.value.cleanup_not_after_utc.clone(),
        },
    )?;
    if expected != reviewed.value {
        return Err(invalid(
            "reviewed Phase-A eligibility envelope V4 exact pin or closed requirement mismatched",
        ));
    }

    Ok(ReviewedPhaseAEligibilityEnvelopeVerificationV4 {
        schema_version: reviewed.value.schema_version,
        v1_config_fingerprint: context.v1_config.fingerprint().to_owned(),
        v1_authorization_fingerprint: context.v1_authorization.fingerprint().to_owned(),
        online_policy_v2_fingerprint: context.online_policy_v2.fingerprint().to_owned(),
        online_authorization_v2_fingerprint: context
            .online_authorization_v2
            .fingerprint()
            .to_owned(),
        reviewed_production_destination_v1_fingerprint: context
            .reviewed_production_destination_v1
            .fingerprint()
            .to_owned(),
        reviewed_fresh_credential_slot_locator_v1_fingerprint: context
            .reviewed_fresh_credential_slot_locator_v1
            .fingerprint()
            .to_owned(),
        fresh_credential_delivery_binding_v1_fingerprint: context
            .fresh_credential_delivery_binding_v1
            .fingerprint()
            .to_owned(),
        reviewed_signer_proxy_account_identity_v1_fingerprint: context
            .reviewed_signer_proxy_account_identity_v1
            .fingerprint()
            .to_owned(),
        reviewed_remote_credential_proof_policy_v1_fingerprint: context
            .reviewed_remote_credential_proof_policy_v1
            .fingerprint()
            .to_owned(),
        reviewed_static_online_authorization_v3_fingerprint: context
            .reviewed_static_online_authorization_v3
            .fingerprint()
            .to_owned(),
        reviewed_phase_a_eligibility_envelope_v4_fingerprint: reviewed.fingerprint.clone(),
        exact_v1_config_pin_structurally_valid: true,
        exact_v1_authorization_pin_structurally_valid: true,
        exact_online_policy_v2_pin_structurally_valid: true,
        exact_online_authorization_v2_pin_structurally_valid: true,
        exact_reviewed_production_destination_v1_pin_structurally_valid: true,
        exact_reviewed_fresh_credential_slot_locator_v1_pin_structurally_valid: true,
        exact_fresh_credential_delivery_binding_v1_pin_structurally_valid: true,
        exact_reviewed_signer_proxy_account_identity_v1_pin_structurally_valid: true,
        exact_reviewed_remote_credential_proof_policy_v1_pin_structurally_valid: true,
        exact_reviewed_static_online_authorization_v3_pin_structurally_valid: true,
        exact_phase_a_scope_and_v2_time_envelope_structurally_valid: true,
        frozen_static_v3_reverified_as_denied_structural_evidence: true,
        unavailable_external_requirements_structurally_valid: true,
        unavailable_runtime_requirements_structurally_valid: true,
        static_v3_existed_at_v4_review_time_attested: false,
        reviewer_trust_anchor_available: false,
        reviewer_authorship_attested: false,
        credential_provider_trust_root_available: false,
        credential_provider_trust_root_authenticated: false,
        credential_provider_signature_verified: false,
        credential_delivery_lease_verified: false,
        credential_delivery_lease_current_and_unrevoked: false,
        delivery_generation_attested: false,
        authoritative_remote_credential_acceptance_contract_available: false,
        same_holder_live_remote_credential_acceptance_proof_verified: false,
        live_credential_tuple_accepted_by_provider: false,
        authoritative_signer_proxy_control_contract_available: false,
        account_specific_signer_proxy_control_proof_verified: false,
        signer_controls_proxy_current_and_unrevoked_attested: false,
        source_owned_current_time_checked: false,
        v1_authorization_current: false,
        online_authorization_v2_current: false,
        static_online_authorization_v3_current: false,
        reviewed_phase_a_eligibility_envelope_v4_current: false,
        retained_delivery_evidence_joined: false,
        fresh_credential_delivery_load_token_consumed: false,
        loaded_credentials_match_delivery_binding: false,
        same_loaded_credential_holder_attested: false,
        selected_egress_actor_started: false,
        selected_actor_generation_bound: false,
        source_owned_runtime_attempt_bound: false,
        runtime_attempt_fresh: false,
        positive_external_proof_bundle_complete: false,
        offline_phase_a_eligibility_established: false,
        positive_runtime_lineage_implemented: false,
        durable_preparation_record_created: false,
        atomic_consumption_claim_created: false,
        authorization_burn_performed: false,
        final_a3_conjunct_durable: false,
        burn_and_no_resend_established: false,
        authenticated_request_or_hmac_constructed: false,
        signed_order_body_constructed: false,
        place_dispatch_owner_or_grant_minted: false,
        network_dispatch_performed: false,
        placement_resumption_allowed: false,
        credential_mutation_authority_attested: false,
        authorization: OfflineAuthorizationState::DENIED,
    })
}

#[derive(Debug, Error)]
pub enum PmReviewedPhaseAEligibilityEnvelopeV4Error {
    #[error("controlled-trial reviewed Phase-A eligibility envelope V4 is invalid: {0}")]
    Invalid(&'static str),
}

struct VerifiedContextMetadataV4 {
    destination_schema_version: u32,
    destination_id: String,
    locator_schema_version: u32,
    locator_id: String,
    delivery_schema_version: u32,
    delivery_id: String,
    identity_schema_version: u32,
    identity_id: String,
}

fn verify_denied_context(
    context: &ReviewedPhaseAEligibilityEnvelopeContextV4<'_>,
) -> Result<VerifiedContextMetadataV4, PmReviewedPhaseAEligibilityEnvelopeV4Error> {
    let v3_context = ReviewedStaticOnlineAuthorizationContextV3 {
        v1_config: context.v1_config,
        v1_authorization: context.v1_authorization,
        online_policy_v2: context.online_policy_v2,
        online_authorization_v2: context.online_authorization_v2,
        reviewed_production_destination_v1: context.reviewed_production_destination_v1,
        reviewed_fresh_credential_slot_locator_v1: context
            .reviewed_fresh_credential_slot_locator_v1,
        fresh_credential_delivery_binding_v1: context.fresh_credential_delivery_binding_v1,
        reviewed_signer_proxy_account_identity_v1: context
            .reviewed_signer_proxy_account_identity_v1,
        reviewed_remote_credential_proof_policy_v1: context
            .reviewed_remote_credential_proof_policy_v1,
    };
    let v3 = verify_reviewed_static_online_authorization_v3(
        &v3_context,
        context.reviewed_static_online_authorization_v3,
    )
    .map_err(|_| invalid("reviewed Phase-A bound static V3 conjunction is invalid"))?;
    if v3.authorization != OfflineAuthorizationState::DENIED
        || !v3.exact_v1_config_pin_structurally_valid
        || !v3.exact_v1_authorization_pin_structurally_valid
        || !v3.exact_online_policy_v2_pin_structurally_valid
        || !v3.exact_online_authorization_v2_pin_structurally_valid
        || !v3.exact_reviewed_production_destination_v1_pin_structurally_valid
        || !v3.exact_reviewed_fresh_credential_slot_locator_v1_pin_structurally_valid
        || !v3.exact_fresh_credential_delivery_binding_v1_pin_structurally_valid
        || !v3.exact_reviewed_signer_proxy_account_identity_v1_pin_structurally_valid
        || !v3.exact_reviewed_remote_credential_proof_policy_v1_pin_structurally_valid
        || !v3.component_verifiers_denied_and_negative_facts_checked
        || !v3.unavailable_positive_contracts_structurally_valid
        || v3.reviewer_authorship_attested
        || v3.credential_provider_trust_root_available
        || v3.credential_delivery_lease_verified
        || v3.authoritative_remote_credential_acceptance_contract_available
        || v3.authoritative_signer_proxy_control_contract_available
        || v3.live_credential_tuple_accepted_by_provider
        || v3.signer_controls_proxy_attested
        || v3.actor_started
        || v3.runtime_attempt_commitment_present
        || v3.prospective_v3_prepared_implemented
        || v3.atomic_consumption_claim_created
        || v3.burn_and_no_resend_established
        || v3.place_dispatch_owner_or_grant_minted
        || v3.network_dispatch_performed
        || v3.credential_mutation_authority_attested
    {
        return Err(invalid(
            "reviewed Phase-A static V3 input is not denied structural evidence",
        ));
    }

    let destination = crate::verify_reviewed_production_destination_profile_v1(
        context.v1_config,
        context.online_policy_v2,
        context.online_authorization_v2,
        context.reviewed_production_destination_v1,
    )
    .map_err(|_| invalid("reviewed Phase-A destination context is invalid"))?;
    let locator = crate::verify_reviewed_fresh_credential_slot_locator_v1(
        context.v1_config,
        context.online_policy_v2,
        context.online_authorization_v2,
        context.reviewed_fresh_credential_slot_locator_v1,
    )
    .map_err(|_| invalid("reviewed Phase-A locator context is invalid"))?;
    let delivery =
        crate::fresh_credential_delivery_binding_v1::verify_fresh_credential_delivery_binding_v1(
            context.v1_config,
            context.online_policy_v2,
            context.online_authorization_v2,
            context.reviewed_fresh_credential_slot_locator_v1,
            context.fresh_credential_delivery_binding_v1,
        )
        .map_err(|_| invalid("reviewed Phase-A delivery context is invalid"))?;
    let identity = crate::verify_reviewed_signer_proxy_account_identity_v1(
        context.v1_config,
        context.online_policy_v2,
        context.online_authorization_v2,
        context.reviewed_signer_proxy_account_identity_v1,
    )
    .map_err(|_| invalid("reviewed Phase-A account-identity context is invalid"))?;

    if destination.authorization != OfflineAuthorizationState::DENIED
        || locator.authorization != OfflineAuthorizationState::DENIED
        || delivery.authorization != OfflineAuthorizationState::DENIED
        || identity.authorization != OfflineAuthorizationState::DENIED
    {
        return Err(invalid(
            "reviewed Phase-A component context gained authorization",
        ));
    }

    Ok(VerifiedContextMetadataV4 {
        destination_schema_version: destination.schema_version,
        destination_id: destination.profile_id,
        locator_schema_version: locator.schema_version,
        locator_id: locator.locator_id,
        delivery_schema_version: delivery.schema_version,
        delivery_id: delivery.binding_id,
        identity_schema_version: identity.schema_version,
        identity_id: identity.identity_id,
    })
}

fn validate_phase_and_time_binding(
    context: &ReviewedPhaseAEligibilityEnvelopeContextV4<'_>,
    times: &ValidatedPhaseATimesV4,
    record: &ReviewedPhaseAEligibilityEnvelopeV4,
) -> Result<(), PmReviewedPhaseAEligibilityEnvelopeV4Error> {
    let online = context.online_authorization_v2.value();
    let online_reviewed_at = parse_utc(&online.reviewed_at_utc)?;
    if context.v1_authorization.value().phase != TrialPhase::APlaceCancel
        || online.phase != TrialPhase::APlaceCancel
        || record.not_before_utc != online.not_before_utc
        || record.expires_at_utc != online.expires_at_utc
        || record.cleanup_not_after_utc != online.cleanup_not_after_utc
        || times.reviewed_at < online_reviewed_at
        || times.reviewed_at > times.not_before
        || times.not_before != parse_utc(&online.not_before_utc)?
        || times.expires_at != parse_utc(&online.expires_at_utc)?
        || times.cleanup_not_after != parse_utc(&online.cleanup_not_after_utc)?
    {
        return Err(invalid(
            "reviewed Phase-A scope or time envelope differs from online authorization V2",
        ));
    }
    Ok(())
}

fn unavailable_external_evidence() -> ReviewedPhaseAUnavailableExternalEvidenceV4 {
    ReviewedPhaseAUnavailableExternalEvidenceV4 {
        reviewer_trust_anchor_status:
            ReviewedPhaseAReviewerTrustAnchorStatusV4::RequiredExternalReviewerTrustAnchorUnavailableV1,
        credential_provider_trust_root_status:
            ReviewedPhaseACredentialProviderTrustRootStatusV4::RequiredAuthenticatedProviderTrustRootUnavailableV1,
        provider_signed_credential_delivery_lease_proof_status:
            ReviewedPhaseACredentialDeliveryLeaseProofStatusV4::RequiredProviderSignedAttemptAudienceLeaseUnavailableV1,
        authoritative_remote_credential_acceptance_contract_status:
            ReviewedPhaseARemoteAcceptanceContractStatusV4::RequiredAuthoritativeRemoteAcceptanceContractUnavailableV1,
        same_holder_live_remote_credential_acceptance_proof_status:
            ReviewedPhaseARemoteCredentialProofStatusV4::RequiredSameHolderLiveRemoteAcceptanceProofUnavailableV1,
        authoritative_signer_proxy_control_contract_status:
            ReviewedPhaseASignerProxyContractStatusV4::RequiredAuthoritativeSignerProxyControlContractUnavailableV1,
        account_specific_signer_proxy_control_proof_status:
            ReviewedPhaseASignerProxyProofStatusV4::RequiredAccountSpecificCurrentUnrevokedControlProofUnavailableV1,
    }
}

fn unavailable_runtime_lineage() -> ReviewedPhaseAUnavailableRuntimeLineageV4 {
    ReviewedPhaseAUnavailableRuntimeLineageV4 {
        selected_actor_generation_and_source_owned_attempt_status:
            ReviewedPhaseAActorRuntimeAttemptStatusV4::RequiredFutureSelectedActorPreparedLineageUnavailableV1,
        retained_delivery_evidence_and_load_token_join_status:
            ReviewedPhaseADeliveryCustodyJoinStatusV4::RequiredFutureRetainedEvidenceLoadTokenJoinUnavailableV1,
        durable_attempt_claim_and_final_conjunct_status:
            ReviewedPhaseADurableAttemptLineageStatusV4::RequiredFutureCreateNewClaimA3LineageUnavailableV1,
        fixed_egress_single_dispatch_owner_status:
            ReviewedPhaseASingleDispatchStatusV4::RequiredFutureSelectedEgressSingleDispatchOwnerUnavailableV1,
    }
}

struct ExactPinViewV4<'a> {
    schema_version: u32,
    expected_schema_version: u32,
    role_id: Option<&'a str>,
    canonical_sha256: &'a str,
    canonical_length: u64,
    fingerprint: &'a str,
}

fn validate_exact_pin(
    pin: ExactPinViewV4<'_>,
    message: &'static str,
) -> Result<(), PmReviewedPhaseAEligibilityEnvelopeV4Error> {
    if pin.schema_version != pin.expected_schema_version || pin.canonical_length == 0 {
        return Err(invalid(message));
    }
    if let Some(role_id) = pin.role_id {
        validate_token(role_id, 128, message)?;
    }
    validate_sha256(pin.canonical_sha256)?;
    validate_sha256(pin.fingerprint)
}

fn parse_exact_canonical<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
) -> Result<T, PmReviewedPhaseAEligibilityEnvelopeV4Error> {
    let value: T = serde_json::from_slice(bytes).map_err(|_| {
        invalid("reviewed Phase-A eligibility envelope JSON is malformed, duplicated, unknown, or trailing")
    })?;
    let canonical = serde_json::to_vec(&value).map_err(|_| {
        invalid("reviewed Phase-A eligibility envelope cannot be serialized canonically")
    })?;
    if canonical != bytes {
        return Err(invalid(
            "reviewed Phase-A eligibility envelope bytes are not exact canonical compact JSON",
        ));
    }
    Ok(value)
}

fn parse_utc(value: &str) -> Result<DateTime<Utc>, PmReviewedPhaseAEligibilityEnvelopeV4Error> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid("reviewed Phase-A eligibility envelope timestamp is invalid"))?
        .with_timezone(&Utc);
    if parsed.to_rfc3339_opts(SecondsFormat::Secs, true) != value {
        return Err(invalid(
            "reviewed Phase-A eligibility envelope timestamp is not canonical UTC seconds",
        ));
    }
    Ok(parsed)
}

fn validate_sha256(value: &str) -> Result<(), PmReviewedPhaseAEligibilityEnvelopeV4Error> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(
            "reviewed Phase-A eligibility envelope SHA-256 value is invalid",
        ));
    }
    Ok(())
}

fn validate_token(
    value: &str,
    maximum: usize,
    message: &'static str,
) -> Result<(), PmReviewedPhaseAEligibilityEnvelopeV4Error> {
    if value.is_empty()
        || value.len() > maximum
        || value.bytes().any(|byte| {
            !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/'))
        })
    {
        return Err(invalid(message));
    }
    Ok(())
}

fn validate_reference(
    value: &str,
    message: &'static str,
) -> Result<(), PmReviewedPhaseAEligibilityEnvelopeV4Error> {
    if value.is_empty()
        || value.len() > 512
        || value.bytes().any(|byte| !(0x21..=0x7e).contains(&byte))
    {
        return Err(invalid(message));
    }
    Ok(())
}

fn hash_bytes(domain: &[u8], bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn invalid(message: &'static str) -> PmReviewedPhaseAEligibilityEnvelopeV4Error {
    PmReviewedPhaseAEligibilityEnvelopeV4Error::Invalid(message)
}
