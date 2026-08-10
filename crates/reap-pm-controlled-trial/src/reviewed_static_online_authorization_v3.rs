//! Protected, canonical, static online-authorization conjunction V3.
//!
//! This additive controlled-trial record binds nine exact canonical holders:
//! V1 config and authorization, V2 online policy and authorization, and the
//! five reviewed V1 sidecars for destinations, credential location/delivery,
//! account identity, and the remote credential-proof policy. Verification is
//! offline and read-only. It borrows every holder, consumes none, and never
//! opens a journal, creates a file, samples a current clock, loads a
//! credential, constructs authentication material, or mints an actor/dispatch
//! capability.
//!
//! V1 authorization does not retain its accepted canonical bytes. Its loader
//! nevertheless proved that the input bytes exactly equal compact
//! `serde_json` encoding, so this module deterministically reconstructs that
//! encoding from the canonical holder to obtain its exact raw SHA-256 and
//! length. The existing domain fingerprint remains the holder's fingerprint.
//!
//! The four unavailable positive contracts below are distinct closed enums;
//! no designated caller digest, key, signature, lease, or substitute-contract
//! field is accepted. The actor and consumption-lineage fields are closed
//! reviewed labels only. No PID, TID, pointer, runtime nonce, generation value,
//! or code/runtime correspondence is recorded or checked here. A future
//! separately versioned Prepared lineage must consume the retained delivery
//! evidence and load token, bind a selected actor generation, prove live
//! freshness, and durably burn the attempt before any positive use.
//!
//! Every live, provider, lease, loaded-holder, actor, Prepared, consumption,
//! A3, send, and mutation-authority result in this module is permanently
//! false. The overall authorization result is always DENIED.
//!
//! Static review ordering compares only the V1 and V2 authorization review
//! times. Exact pins do not prove that any of the five prerequisite sidecars
//! existed when this V3 record was reviewed.

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
    CanonicalTrialConfig, FRESH_CREDENTIAL_DELIVERY_BINDING_V1_SCHEMA_VERSION,
    ONLINE_AUTHORIZATION_V2_SCHEMA_VERSION, ONLINE_POLICY_V2_SCHEMA_VERSION,
    OfflineAuthorizationState, REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_SCHEMA_VERSION,
    REVIEWED_PRODUCTION_DESTINATION_PROFILE_V1_SCHEMA_VERSION,
    REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_SCHEMA_VERSION,
    REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_SCHEMA_VERSION, TrialPhase,
    config::{TRIAL_AUTHORIZATION_SCHEMA_VERSION, TRIAL_CONFIG_SCHEMA_VERSION},
    fresh_credential_delivery_binding_v1::verify_fresh_credential_delivery_binding_v1,
    online_policy_v2::validate_online_authorization_contract_v2,
    protected_file::{ProtectedFileKind, read_one},
    reviewed_fresh_credential_slot_locator_v1::verify_reviewed_fresh_credential_slot_locator_v1,
    verify_authorization, verify_reviewed_production_destination_profile_v1,
    verify_reviewed_remote_credential_proof_policy_v1,
    verify_reviewed_signer_proxy_account_identity_v1,
};

pub const REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3_SCHEMA_VERSION: u32 = 3;
pub const PM_T2_REVIEWED_STATIC_ONLINE_AUTHORIZATION_FILE_V3: &str =
    "pm-t2-reviewed-static-online-authorization-v3.json";

const MAX_CANONICAL_REVIEWED_STATIC_ONLINE_AUTHORIZATION_BYTES_V3: usize = 128 * 1024;
const REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.controlled-trial.reviewed-static-online-authorization.v3\0";

/// Exact role pin for the canonical V1 config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedStaticOnlineAuthorizationConfigPinsV3 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
    pub plan_fingerprint: String,
}

/// Exact role pin for the canonical V1 authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedStaticOnlineAuthorizationV1AuthorizationPinsV3 {
    pub schema_version: u32,
    pub authorization_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact role pin for the canonical online policy V2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedStaticOnlineAuthorizationOnlinePolicyPinsV3 {
    pub schema_version: u32,
    pub policy_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact role pin for the canonical online authorization V2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedStaticOnlineAuthorizationOnlineAuthorizationPinsV3 {
    pub schema_version: u32,
    pub authorization_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact role pin for the reviewed production destination profile V1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedStaticOnlineAuthorizationDestinationPinsV3 {
    pub schema_version: u32,
    pub profile_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact role pin for the reviewed fresh credential-slot locator V1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedStaticOnlineAuthorizationLocatorPinsV3 {
    pub schema_version: u32,
    pub locator_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact role pin for the unsigned fresh credential delivery binding V1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedStaticOnlineAuthorizationDeliveryPinsV3 {
    pub schema_version: u32,
    pub binding_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact role pin for the reviewed signer/proxy account identity V1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedStaticOnlineAuthorizationAccountIdentityPinsV3 {
    pub schema_version: u32,
    pub identity_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact role pin for the reviewed remote credential-proof policy V1.
///
/// Its current canonical holder intentionally exposes no policy ID. Exact
/// SHA-256, length, and domain fingerprint commit all canonical bytes,
/// including that private ID, without accepting an uncorrelated caller label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedStaticOnlineAuthorizationRemoteProofPolicyPinsV3 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedCredentialProviderTrustRootStatusV3 {
    #[serde(rename = "unavailable_in_frozen_sources_v1")]
    UnavailableInFrozenSourcesV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedCredentialDeliveryLeaseProtocolStatusV3 {
    #[serde(rename = "unavailable_in_frozen_sources_v1")]
    UnavailableInFrozenSourcesV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedRemoteCredentialAcceptanceContractStatusV3 {
    #[serde(rename = "unavailable_in_frozen_sources_v1")]
    UnavailableInFrozenSourcesV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedSignerProxyControlContractStatusV3 {
    #[serde(rename = "unattested_reviewed_labels_only_v1")]
    UnattestedReviewedLabelsOnlyV1,
}

/// Four distinct unavailable positive contracts. There is deliberately no
/// digest, key, signature, lease token, or substitute proof field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedStaticOnlineAuthorizationUnavailablePositiveContractsV3 {
    pub credential_provider_trust_root_status: ReviewedCredentialProviderTrustRootStatusV3,
    pub authenticated_credential_delivery_lease_protocol_status:
        ReviewedCredentialDeliveryLeaseProtocolStatusV3,
    pub authoritative_remote_credential_acceptance_contract_status:
        ReviewedRemoteCredentialAcceptanceContractStatusV3,
    pub authoritative_signer_proxy_control_contract_status:
        ReviewedSignerProxyControlContractStatusV3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedSelectedActorProfileV3 {
    #[serde(rename = "shutdown_only_selected_egress_current_thread_local_set_v1")]
    ShutdownOnlySelectedEgressCurrentThreadLocalSetV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedActorGenerationSchemeV3 {
    #[serde(rename = "process_id_thread_id_and_rc_pointer_identity_v1")]
    ProcessIdThreadIdAndRcPointerIdentityV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedActorGenerationAllocationV3 {
    #[serde(rename = "before_runtime_local_set_and_all_actor_local_construction_v1")]
    BeforeRuntimeLocalSetAndAllActorLocalConstructionV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedActorReadinessV3 {
    #[serde(rename = "task_entry_ack_after_generation_membership_revalidation_v1")]
    TaskEntryAckAfterGenerationMembershipRevalidationV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedActorCommandSetV3 {
    #[serde(rename = "shutdown_only_v1")]
    ShutdownOnlyV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedActorTerminalRequirementV3 {
    #[serde(
        rename = "shutdown_requested_no_abort_task_joined_clean_credentials_dropped_staged_files_removed_generation_revalidated_v1"
    )]
    ShutdownRequestedNoAbortTaskJoinedCleanCredentialsDroppedStagedFilesRemovedGenerationRevalidatedV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedActorRuntimeAttemptCommitmentLocationV3 {
    #[serde(rename = "future_prepared_only_absent_from_static_v3")]
    FuturePreparedOnlyAbsentFromStaticV3,
}

/// Closed reviewed actor profile labels. They are not implementation evidence
/// and contain no concrete process/thread/pointer/generation identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedStaticOnlineAuthorizationSelectedActorProfileV3 {
    pub profile: ReviewedSelectedActorProfileV3,
    pub generation_scheme: ReviewedActorGenerationSchemeV3,
    pub generation_allocation: ReviewedActorGenerationAllocationV3,
    pub readiness: ReviewedActorReadinessV3,
    pub command_set: ReviewedActorCommandSetV3,
    pub terminal_requirement: ReviewedActorTerminalRequirementV3,
    pub runtime_attempt_commitment_location: ReviewedActorRuntimeAttemptCommitmentLocationV3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedPreparedCreationOrderV3 {
    #[serde(rename = "online_v2_prepared_then_v1_prepared_v1")]
    OnlineV2PreparedThenV1PreparedV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedBasisAndBurnOrderV3 {
    #[serde(
        rename = "basis_after_existing_place_and_both_consumption_prepared_then_v2_claim_then_v1_claim_then_v1_a3_then_v2_a3_conjunct_v1"
    )]
    BasisAfterExistingPlaceAndBothConsumptionPreparedThenV2ClaimThenV1ClaimThenV1A3ThenV2A3ConjunctV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedCrashRecoveryProfileV3 {
    #[serde(rename = "existing_v1_lifecycle_only_no_placement_resume_v1")]
    ExistingV1LifecycleOnlyNoPlacementResumeV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedStaticV3RuntimeStateV3 {
    #[serde(rename = "no_prepared_claim_burn_recovery_or_dispatch_v1")]
    NoPreparedClaimBurnRecoveryOrDispatchV1,
}

/// Frozen labels for the unchanged V1/V2 consumption and recovery design.
/// This module neither observes nor executes any of these transitions.
/// The V2 attempt value in that design is only the existing Basis
/// fingerprint, never a fresh runtime commitment.
/// In that existing design, a one-record ledger plus its durable claim is
/// already burned even when no later `Consumed` line exists; an absent or
/// failed claim never proves that an attempt is unburned. V2 has no
/// placement-reopen path. V1 may reopen only to consume or recovery-cancel,
/// always with zero placement resumption. Those runtime facts and every
/// eligibility decision remain unchecked here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedStaticOnlineAuthorizationFrozenConsumptionLineageV3 {
    pub prepared_creation_order: ReviewedPreparedCreationOrderV3,
    pub basis_and_burn_order: ReviewedBasisAndBurnOrderV3,
    pub crash_recovery: ReviewedCrashRecoveryProfileV3,
    pub static_v3_runtime_state: ReviewedStaticV3RuntimeStateV3,
}

/// One reviewer-labeled static conjunction. The schema defines no designated
/// secret, credential-value, cryptographic-signature, lease-token, live
/// observation, actor-identity, request, or journal-state fields. Arbitrary ID
/// and reviewer-label strings cannot be proven secret-free; callers and
/// operators must never place secrets or secret-derived material in them.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedStaticOnlineAuthorizationV3 {
    pub schema_version: u32,
    pub static_authorization_id: String,
    pub reviewer_label: String,
    pub reviewed_at_utc: String,
    pub not_before_utc: String,
    pub expires_at_utc: String,
    pub cleanup_not_after_utc: String,
    pub v1_config: ReviewedStaticOnlineAuthorizationConfigPinsV3,
    pub v1_authorization: ReviewedStaticOnlineAuthorizationV1AuthorizationPinsV3,
    pub online_policy_v2: ReviewedStaticOnlineAuthorizationOnlinePolicyPinsV3,
    pub online_authorization_v2: ReviewedStaticOnlineAuthorizationOnlineAuthorizationPinsV3,
    pub reviewed_production_destination_v1: ReviewedStaticOnlineAuthorizationDestinationPinsV3,
    pub reviewed_fresh_credential_slot_locator_v1: ReviewedStaticOnlineAuthorizationLocatorPinsV3,
    pub fresh_credential_delivery_binding_v1: ReviewedStaticOnlineAuthorizationDeliveryPinsV3,
    pub reviewed_signer_proxy_account_identity_v1:
        ReviewedStaticOnlineAuthorizationAccountIdentityPinsV3,
    pub reviewed_remote_credential_proof_policy_v1:
        ReviewedStaticOnlineAuthorizationRemoteProofPolicyPinsV3,
    pub unavailable_positive_contracts:
        ReviewedStaticOnlineAuthorizationUnavailablePositiveContractsV3,
    pub selected_actor_profile: ReviewedStaticOnlineAuthorizationSelectedActorProfileV3,
    pub frozen_v1_v2_consumption_lineage:
        ReviewedStaticOnlineAuthorizationFrozenConsumptionLineageV3,
}

impl fmt::Debug for ReviewedStaticOnlineAuthorizationV3 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "ReviewedStaticOnlineAuthorizationV3(<reviewed-nine-holder-static-conjunction; unavailable-positive-contracts; denied>)",
        )
    }
}

struct ValidatedStaticTimesV3 {
    reviewed_at: DateTime<Utc>,
    not_before: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    cleanup_not_after: DateTime<Utc>,
}

impl ReviewedStaticOnlineAuthorizationV3 {
    fn validate_intrinsic(
        &self,
    ) -> Result<ValidatedStaticTimesV3, PmReviewedStaticOnlineAuthorizationV3Error> {
        if self.schema_version != REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3_SCHEMA_VERSION {
            return Err(invalid(
                "unsupported reviewed static online-authorization V3 schema",
            ));
        }
        validate_token(
            &self.static_authorization_id,
            128,
            "reviewed static online-authorization ID is invalid",
        )?;
        validate_reference(
            &self.reviewer_label,
            "reviewed static online-authorization reviewer label is invalid",
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

        let reviewed_at = parse_utc(&self.reviewed_at_utc)?;
        let not_before = parse_utc(&self.not_before_utc)?;
        let expires_at = parse_utc(&self.expires_at_utc)?;
        let cleanup_not_after = parse_utc(&self.cleanup_not_after_utc)?;
        if reviewed_at > not_before || not_before >= expires_at || expires_at > cleanup_not_after {
            return Err(invalid(
                "reviewed static online-authorization time envelope is invalid",
            ));
        }
        Ok(ValidatedStaticTimesV3 {
            reviewed_at,
            not_before,
            expires_at,
            cleanup_not_after,
        })
    }
}

macro_rules! validate_role_pin {
    ($type:ty, $schema:expr, $id_field:ident) => {
        impl $type {
            fn validate(&self) -> Result<(), PmReviewedStaticOnlineAuthorizationV3Error> {
                validate_exact_pin(
                    ExactPinViewV3 {
                        schema_version: self.schema_version,
                        expected_schema_version: $schema,
                        role_id: Some(&self.$id_field),
                        canonical_sha256: &self.canonical_sha256,
                        canonical_length: self.canonical_length,
                        fingerprint: &self.fingerprint,
                    },
                    "reviewed static online-authorization artifact pin is invalid",
                )
            }
        }
    };
}

validate_role_pin!(
    ReviewedStaticOnlineAuthorizationV1AuthorizationPinsV3,
    TRIAL_AUTHORIZATION_SCHEMA_VERSION,
    authorization_id
);
validate_role_pin!(
    ReviewedStaticOnlineAuthorizationOnlinePolicyPinsV3,
    ONLINE_POLICY_V2_SCHEMA_VERSION,
    policy_id
);
validate_role_pin!(
    ReviewedStaticOnlineAuthorizationOnlineAuthorizationPinsV3,
    ONLINE_AUTHORIZATION_V2_SCHEMA_VERSION,
    authorization_id
);
validate_role_pin!(
    ReviewedStaticOnlineAuthorizationDestinationPinsV3,
    REVIEWED_PRODUCTION_DESTINATION_PROFILE_V1_SCHEMA_VERSION,
    profile_id
);
validate_role_pin!(
    ReviewedStaticOnlineAuthorizationLocatorPinsV3,
    REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_SCHEMA_VERSION,
    locator_id
);
validate_role_pin!(
    ReviewedStaticOnlineAuthorizationDeliveryPinsV3,
    FRESH_CREDENTIAL_DELIVERY_BINDING_V1_SCHEMA_VERSION,
    binding_id
);
validate_role_pin!(
    ReviewedStaticOnlineAuthorizationAccountIdentityPinsV3,
    REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_SCHEMA_VERSION,
    identity_id
);

impl ReviewedStaticOnlineAuthorizationConfigPinsV3 {
    fn validate(&self) -> Result<(), PmReviewedStaticOnlineAuthorizationV3Error> {
        validate_exact_pin(
            ExactPinViewV3 {
                schema_version: self.schema_version,
                expected_schema_version: TRIAL_CONFIG_SCHEMA_VERSION,
                role_id: None,
                canonical_sha256: &self.canonical_sha256,
                canonical_length: self.canonical_length,
                fingerprint: &self.fingerprint,
            },
            "reviewed static online-authorization V1 config pin is invalid",
        )?;
        validate_sha256(&self.plan_fingerprint)
    }
}

impl ReviewedStaticOnlineAuthorizationRemoteProofPolicyPinsV3 {
    fn validate(&self) -> Result<(), PmReviewedStaticOnlineAuthorizationV3Error> {
        validate_exact_pin(
            ExactPinViewV3 {
                schema_version: self.schema_version,
                expected_schema_version: REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_SCHEMA_VERSION,
                role_id: None,
                canonical_sha256: &self.canonical_sha256,
                canonical_length: self.canonical_length,
                fingerprint: &self.fingerprint,
            },
            "reviewed static online-authorization remote proof-policy pin is invalid",
        )
    }
}

/// Move-only, non-serializable and non-projecting holder of exact protected
/// canonical V3 bytes.
pub struct CanonicalReviewedStaticOnlineAuthorizationV3 {
    value: ReviewedStaticOnlineAuthorizationV3,
    canonical_bytes: Vec<u8>,
    canonical_sha256: String,
    fingerprint: String,
}

impl CanonicalReviewedStaticOnlineAuthorizationV3 {
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

impl fmt::Debug for CanonicalReviewedStaticOnlineAuthorizationV3 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "CanonicalReviewedStaticOnlineAuthorizationV3(<exact-protected-canonical-bytes; no-value-id-address-path-actor-or-lineage-projection; redacted; denied>)",
        )
    }
}

/// Borrowed static conjunction of all nine exact canonical holders.
/// Constructing or verifying this context consumes nothing and grants no
/// capability.
pub struct ReviewedStaticOnlineAuthorizationContextV3<'a> {
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
}

/// Offline structural display. Only exact static conjunction facts are true;
/// every live, authority, transition, and mutation fact is false.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewedStaticOnlineAuthorizationVerificationV3 {
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
    pub exact_v1_config_pin_structurally_valid: bool,
    pub exact_v1_authorization_pin_structurally_valid: bool,
    pub exact_online_policy_v2_pin_structurally_valid: bool,
    pub exact_online_authorization_v2_pin_structurally_valid: bool,
    pub exact_reviewed_production_destination_v1_pin_structurally_valid: bool,
    pub exact_reviewed_fresh_credential_slot_locator_v1_pin_structurally_valid: bool,
    pub exact_fresh_credential_delivery_binding_v1_pin_structurally_valid: bool,
    pub exact_reviewed_signer_proxy_account_identity_v1_pin_structurally_valid: bool,
    pub exact_reviewed_remote_credential_proof_policy_v1_pin_structurally_valid: bool,
    pub v1_authorization_exact_canonical_bytes_reconstructed_and_pinned: bool,
    pub v1_authorization_structurally_valid_at_own_not_before: bool,
    pub online_v2_contract_structurally_valid_without_current_clock: bool,
    pub exact_v1_v2_common_phase_build_host_tuple_structurally_valid: bool,
    pub online_v2_window_nested_within_v1: bool,
    pub static_window_exactly_matches_online_authorization_v2: bool,
    pub static_review_order_against_v1_and_v2_structurally_valid: bool,
    pub prerequisite_artifacts_existed_at_static_review_time_attested: bool,
    pub component_verifiers_denied_and_negative_facts_checked: bool,
    pub unavailable_positive_contracts_structurally_valid: bool,
    pub selected_actor_profile_labels_structurally_valid: bool,
    pub frozen_v1_v2_consumption_lineage_labels_structurally_valid: bool,
    pub reviewer_authorship_attested: bool,
    pub credential_provider_trust_root_available: bool,
    pub credential_provider_trust_root_authenticated: bool,
    pub credential_provider_authorship_attested: bool,
    pub authenticated_credential_delivery_lease_protocol_available: bool,
    pub credential_provider_signature_verified: bool,
    pub credential_delivery_lease_verified: bool,
    pub credential_delivery_lease_current_and_unrevoked: bool,
    pub delivery_generation_attested: bool,
    pub rotation_generation_attested: bool,
    pub authoritative_remote_credential_acceptance_contract_available: bool,
    pub authoritative_signer_proxy_control_contract_available: bool,
    pub official_source_bytes_loaded_and_hash_verified: bool,
    pub official_source_publisher_authorship_attested: bool,
    pub remote_api_key_owner_attested: bool,
    pub live_credential_tuple_accepted_by_provider: bool,
    pub signer_controls_proxy_attested: bool,
    pub signer_proxy_relationship_current_and_unrevoked_attested: bool,
    pub signer_on_chain_eoa_status_verified: bool,
    pub proxy_on_chain_contract_status_verified: bool,
    pub on_chain_account_state_checked: bool,
    pub on_chain_finality_checked: bool,
    pub proxy_factory_semantics_verified: bool,
    pub credential_tuple_current_and_unrevoked_attested: bool,
    pub source_owned_current_time_checked: bool,
    pub v1_authorization_current: bool,
    pub online_authorization_v2_current: bool,
    pub static_online_authorization_v3_current: bool,
    pub retained_delivery_evidence_joined: bool,
    pub fresh_credential_delivery_load_token_consumed: bool,
    pub retained_delivery_evidence_and_load_token_joined: bool,
    pub same_loaded_credential_holder_attested: bool,
    pub delivery_and_remote_proof_same_source_generation_attested: bool,
    pub globally_unique_credential_delivery_attested: bool,
    pub loaded_credentials_match_delivery_binding: bool,
    pub protected_credential_directory_and_four_files_checked: bool,
    pub private_key_derived_signer_matches_config_checked: bool,
    pub l2_credentials_match_configured_signer_checked: bool,
    pub runner_actor_profile_implementation_checked: bool,
    pub actor_started: bool,
    pub selected_actor_bound: bool,
    pub selected_actor_generation_bound: bool,
    pub selected_actor_generation_membership_revalidated: bool,
    pub actor_runtime_attempt_commitment_bound: bool,
    pub runtime_attempt_commitment_present: bool,
    pub runtime_attempt_commitment_source_owned: bool,
    pub runtime_attempt_commitment_fresh: bool,
    pub product_clock_owner_bound: bool,
    pub server_time_proof_authenticated_and_fresh: bool,
    pub preflight_collected_and_validated: bool,
    pub v1_prepared_consumption_inspected: bool,
    pub v2_prepared_consumption_inspected: bool,
    pub prospective_v3_prepared_implemented: bool,
    pub prospective_v3_prepared_created: bool,
    pub v1_durable_prepared_observed: bool,
    pub v2_durable_prepared_observed: bool,
    pub v1_claim_absence_reserved: bool,
    pub v2_claim_absence_reserved: bool,
    pub artifact_file_created_or_written: bool,
    pub v3_create_new_performed: bool,
    pub v3_file_fsynced: bool,
    pub v3_parent_directory_fsynced: bool,
    pub journal_opened_for_append: bool,
    pub parent_directory_snapshot_refreshed: bool,
    pub basis_inspected: bool,
    pub basis_durable: bool,
    pub online_v2_claim_inspected: bool,
    pub online_v2_claim_durable: bool,
    pub v1_claim_inspected: bool,
    pub v1_claim_durable: bool,
    pub v1_authorization_consumption_checked: bool,
    pub online_authorization_v2_consumption_checked: bool,
    pub static_online_authorization_v3_consumption_checked: bool,
    pub atomic_consumption_claim_created: bool,
    pub v1_authorization_burn_performed: bool,
    pub online_authorization_v2_burn_performed: bool,
    pub v3_authorization_burn_performed: bool,
    pub burn_and_no_resend_established: bool,
    pub recovery_state_checked: bool,
    pub v1_recovery_reopen_eligibility_established: bool,
    pub v2_recovery_reopen_eligibility_established: bool,
    pub placement_resumption_allowed: bool,
    pub v1_a3_created: bool,
    pub v1_a3_checked: bool,
    pub v1_a3_durable: bool,
    pub online_v2_a3_conjunct_created: bool,
    pub online_v2_a3_conjunct_checked: bool,
    pub online_v2_a3_conjunct_durable: bool,
    pub authenticated_request_or_hmac_constructed: bool,
    pub signed_order_body_constructed: bool,
    pub place_dispatch_owner_or_grant_minted: bool,
    pub network_dispatch_performed: bool,
    pub pid_tid_or_rc_pointer_value_recorded: bool,
    pub randomness_or_runtime_nonce_sampled: bool,
    pub v1_config_reverse_pins_static_authorization_v3: bool,
    pub v1_authorization_reverse_pins_static_authorization_v3: bool,
    pub online_policy_v2_reverse_pins_static_authorization_v3: bool,
    pub online_authorization_v2_reverse_pins_static_authorization_v3: bool,
    pub reviewed_destination_reverse_pins_static_authorization_v3: bool,
    pub reviewed_locator_reverse_pins_static_authorization_v3: bool,
    pub fresh_delivery_reverse_pins_static_authorization_v3: bool,
    pub reviewed_identity_reverse_pins_static_authorization_v3: bool,
    pub remote_proof_policy_reverse_pins_static_authorization_v3: bool,
    pub static_authorization_fingerprint_pinned_by_prospective_v3_prepared: bool,
    pub durable_static_authorization_consumption_recorded: bool,
    pub credential_mutation_authority_attested: bool,
    #[serde(flatten)]
    pub authorization: OfflineAuthorizationState,
}

pub fn load_canonical_reviewed_static_online_authorization_v3(
    path: &Path,
) -> Result<CanonicalReviewedStaticOnlineAuthorizationV3, PmReviewedStaticOnlineAuthorizationV3Error>
{
    let bytes = read_one(
        path,
        ProtectedFileKind::ReviewedStaticOnlineAuthorizationV3,
        MAX_CANONICAL_REVIEWED_STATIC_ONLINE_AUTHORIZATION_BYTES_V3,
    )
    .map_err(|_| {
        invalid("reviewed static online-authorization protection or stability check failed")
    })?;
    let value: ReviewedStaticOnlineAuthorizationV3 = parse_exact_canonical(&bytes)?;
    let _ = value.validate_intrinsic()?;
    let canonical_bytes = bytes.to_vec();
    Ok(CanonicalReviewedStaticOnlineAuthorizationV3 {
        canonical_sha256: hash_bytes(&[], &canonical_bytes),
        fingerprint: hash_bytes(
            REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3_FINGERPRINT_DOMAIN,
            &canonical_bytes,
        ),
        canonical_bytes,
        value,
    })
}

/// Verify the exact nine-holder static conjunction without a current clock,
/// filesystem mutation, journal access, credential load, or capability mint.
pub fn verify_reviewed_static_online_authorization_v3(
    context: &ReviewedStaticOnlineAuthorizationContextV3<'_>,
    reviewed: &CanonicalReviewedStaticOnlineAuthorizationV3,
) -> Result<
    ReviewedStaticOnlineAuthorizationVerificationV3,
    PmReviewedStaticOnlineAuthorizationV3Error,
> {
    let static_times = reviewed.value.validate_intrinsic()?;
    let v1_value = context.v1_authorization.value();
    let v1_reviewed_at = parse_utc(&v1_value.reviewed_at_utc)?;
    let v1_not_before = parse_utc(&v1_value.not_before_utc)?;
    let v1_expires_at = parse_utc(&v1_value.expires_at_utc)?;
    let v1_cleanup_not_after = parse_utc(&v1_value.cleanup_not_after_utc)?;
    let v1_verification = verify_authorization(
        context.v1_config,
        context.v1_authorization,
        v1_not_before,
    )
    .map_err(|_| {
        invalid("reviewed static online-authorization V1 authorization is structurally invalid")
    })?;
    if v1_verification.authorization != OfflineAuthorizationState::DENIED
        || !v1_verification.exact_bindings_structurally_valid
        || !v1_verification.within_short_lived_window_at_verification
        || v1_verification.authorization_consumption_checked
    {
        return Err(invalid(
            "reviewed static online-authorization V1 verification is not denied structural evidence",
        ));
    }

    let v1_authorization_bytes = serde_json::to_vec(v1_value).map_err(|_| {
        invalid(
            "reviewed static online-authorization cannot reconstruct canonical V1 authorization",
        )
    })?;
    let v1_authorization_sha256 = hash_bytes(&[], &v1_authorization_bytes);
    let v1_authorization_length = v1_authorization_bytes.len() as u64;

    let v2_times = validate_online_authorization_contract_v2(
        context.v1_config,
        context.online_policy_v2,
        context.online_authorization_v2,
    )
    .map_err(|_| {
        invalid("reviewed static online-authorization bound online V2 contract is invalid")
    })?;

    if !common_v1_v2_tuple_matches(context) {
        return Err(invalid(
            "reviewed static online-authorization V1/V2 phase, build, or host tuple differs",
        ));
    }
    if v2_times.not_before < v1_not_before
        || v2_times.expires_at > v1_expires_at
        || v2_times.cleanup_not_after > v1_cleanup_not_after
    {
        return Err(invalid(
            "reviewed static online-authorization V2 window is outside V1",
        ));
    }
    if static_times.not_before != v2_times.not_before
        || static_times.expires_at != v2_times.expires_at
        || static_times.cleanup_not_after != v2_times.cleanup_not_after
    {
        return Err(invalid(
            "reviewed static online-authorization window differs from online authorization V2",
        ));
    }
    let latest_upstream_review = std::cmp::max(v1_reviewed_at, v2_times.reviewed_at);
    if static_times.reviewed_at < latest_upstream_review
        || static_times.reviewed_at > static_times.not_before
    {
        return Err(invalid(
            "reviewed static online-authorization review ordering is invalid",
        ));
    }

    let destination_verification = verify_reviewed_production_destination_profile_v1(
        context.v1_config,
        context.online_policy_v2,
        context.online_authorization_v2,
        context.reviewed_production_destination_v1,
    )
    .map_err(|_| invalid("reviewed static online-authorization destination is invalid"))?;
    let locator_verification = verify_reviewed_fresh_credential_slot_locator_v1(
        context.v1_config,
        context.online_policy_v2,
        context.online_authorization_v2,
        context.reviewed_fresh_credential_slot_locator_v1,
    )
    .map_err(|_| invalid("reviewed static online-authorization locator is invalid"))?;
    let delivery_verification = verify_fresh_credential_delivery_binding_v1(
        context.v1_config,
        context.online_policy_v2,
        context.online_authorization_v2,
        context.reviewed_fresh_credential_slot_locator_v1,
        context.fresh_credential_delivery_binding_v1,
    )
    .map_err(|_| invalid("reviewed static online-authorization delivery binding is invalid"))?;
    let identity_verification = verify_reviewed_signer_proxy_account_identity_v1(
        context.v1_config,
        context.online_policy_v2,
        context.online_authorization_v2,
        context.reviewed_signer_proxy_account_identity_v1,
    )
    .map_err(|_| invalid("reviewed static online-authorization account identity is invalid"))?;
    let remote_context = crate::ReviewedRemoteCredentialProofPolicyContextV1 {
        config: context.v1_config,
        online_policy: context.online_policy_v2,
        online_authorization: context.online_authorization_v2,
        reviewed_destination: context.reviewed_production_destination_v1,
        reviewed_fresh_credential_locator: context.reviewed_fresh_credential_slot_locator_v1,
        fresh_credential_delivery: context.fresh_credential_delivery_binding_v1,
        reviewed_signer_proxy_identity: context.reviewed_signer_proxy_account_identity_v1,
    };
    let remote_verification = verify_reviewed_remote_credential_proof_policy_v1(
        &remote_context,
        context.reviewed_remote_credential_proof_policy_v1,
    )
    .map_err(|_| invalid("reviewed static online-authorization remote proof policy is invalid"))?;

    if !component_verifications_are_denied(
        &destination_verification,
        &locator_verification,
        &delivery_verification,
        &identity_verification,
        &remote_verification,
    ) {
        return Err(invalid(
            "reviewed static online-authorization component verification gained a live or authority fact",
        ));
    }

    let verified_roles = VerifiedRoleMetadataV3 {
        destination: VerifiedRoleV3 {
            schema_version: destination_verification.schema_version,
            role_id: &destination_verification.profile_id,
        },
        locator: VerifiedRoleV3 {
            schema_version: locator_verification.schema_version,
            role_id: &locator_verification.locator_id,
        },
        delivery: VerifiedRoleV3 {
            schema_version: delivery_verification.schema_version,
            role_id: &delivery_verification.binding_id,
        },
        identity: VerifiedRoleV3 {
            schema_version: identity_verification.schema_version,
            role_id: &identity_verification.identity_id,
        },
        remote_schema_version: remote_verification.schema_version,
    };
    if !pins_match(
        context,
        &reviewed.value,
        &v1_authorization_sha256,
        v1_authorization_length,
        &verified_roles,
    ) {
        return Err(invalid(
            "reviewed static online-authorization exact role pin mismatched",
        ));
    }

    Ok(ReviewedStaticOnlineAuthorizationVerificationV3 {
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
        reviewed_static_online_authorization_v3_fingerprint: reviewed.fingerprint.clone(),
        exact_v1_config_pin_structurally_valid: true,
        exact_v1_authorization_pin_structurally_valid: true,
        exact_online_policy_v2_pin_structurally_valid: true,
        exact_online_authorization_v2_pin_structurally_valid: true,
        exact_reviewed_production_destination_v1_pin_structurally_valid: true,
        exact_reviewed_fresh_credential_slot_locator_v1_pin_structurally_valid: true,
        exact_fresh_credential_delivery_binding_v1_pin_structurally_valid: true,
        exact_reviewed_signer_proxy_account_identity_v1_pin_structurally_valid: true,
        exact_reviewed_remote_credential_proof_policy_v1_pin_structurally_valid: true,
        v1_authorization_exact_canonical_bytes_reconstructed_and_pinned: true,
        v1_authorization_structurally_valid_at_own_not_before: true,
        online_v2_contract_structurally_valid_without_current_clock: true,
        exact_v1_v2_common_phase_build_host_tuple_structurally_valid: true,
        online_v2_window_nested_within_v1: true,
        static_window_exactly_matches_online_authorization_v2: true,
        static_review_order_against_v1_and_v2_structurally_valid: true,
        prerequisite_artifacts_existed_at_static_review_time_attested: false,
        component_verifiers_denied_and_negative_facts_checked: true,
        unavailable_positive_contracts_structurally_valid: true,
        selected_actor_profile_labels_structurally_valid: true,
        frozen_v1_v2_consumption_lineage_labels_structurally_valid: true,
        reviewer_authorship_attested: false,
        credential_provider_trust_root_available: false,
        credential_provider_trust_root_authenticated: false,
        credential_provider_authorship_attested: false,
        authenticated_credential_delivery_lease_protocol_available: false,
        credential_provider_signature_verified: false,
        credential_delivery_lease_verified: false,
        credential_delivery_lease_current_and_unrevoked: false,
        delivery_generation_attested: false,
        rotation_generation_attested: false,
        authoritative_remote_credential_acceptance_contract_available: false,
        authoritative_signer_proxy_control_contract_available: false,
        official_source_bytes_loaded_and_hash_verified: false,
        official_source_publisher_authorship_attested: false,
        remote_api_key_owner_attested: false,
        live_credential_tuple_accepted_by_provider: false,
        signer_controls_proxy_attested: false,
        signer_proxy_relationship_current_and_unrevoked_attested: false,
        signer_on_chain_eoa_status_verified: false,
        proxy_on_chain_contract_status_verified: false,
        on_chain_account_state_checked: false,
        on_chain_finality_checked: false,
        proxy_factory_semantics_verified: false,
        credential_tuple_current_and_unrevoked_attested: false,
        source_owned_current_time_checked: false,
        v1_authorization_current: false,
        online_authorization_v2_current: false,
        static_online_authorization_v3_current: false,
        retained_delivery_evidence_joined: false,
        fresh_credential_delivery_load_token_consumed: false,
        retained_delivery_evidence_and_load_token_joined: false,
        same_loaded_credential_holder_attested: false,
        delivery_and_remote_proof_same_source_generation_attested: false,
        globally_unique_credential_delivery_attested: false,
        loaded_credentials_match_delivery_binding: false,
        protected_credential_directory_and_four_files_checked: false,
        private_key_derived_signer_matches_config_checked: false,
        l2_credentials_match_configured_signer_checked: false,
        runner_actor_profile_implementation_checked: false,
        actor_started: false,
        selected_actor_bound: false,
        selected_actor_generation_bound: false,
        selected_actor_generation_membership_revalidated: false,
        actor_runtime_attempt_commitment_bound: false,
        runtime_attempt_commitment_present: false,
        runtime_attempt_commitment_source_owned: false,
        runtime_attempt_commitment_fresh: false,
        product_clock_owner_bound: false,
        server_time_proof_authenticated_and_fresh: false,
        preflight_collected_and_validated: false,
        v1_prepared_consumption_inspected: false,
        v2_prepared_consumption_inspected: false,
        prospective_v3_prepared_implemented: false,
        prospective_v3_prepared_created: false,
        v1_durable_prepared_observed: false,
        v2_durable_prepared_observed: false,
        v1_claim_absence_reserved: false,
        v2_claim_absence_reserved: false,
        artifact_file_created_or_written: false,
        v3_create_new_performed: false,
        v3_file_fsynced: false,
        v3_parent_directory_fsynced: false,
        journal_opened_for_append: false,
        parent_directory_snapshot_refreshed: false,
        basis_inspected: false,
        basis_durable: false,
        online_v2_claim_inspected: false,
        online_v2_claim_durable: false,
        v1_claim_inspected: false,
        v1_claim_durable: false,
        v1_authorization_consumption_checked: false,
        online_authorization_v2_consumption_checked: false,
        static_online_authorization_v3_consumption_checked: false,
        atomic_consumption_claim_created: false,
        v1_authorization_burn_performed: false,
        online_authorization_v2_burn_performed: false,
        v3_authorization_burn_performed: false,
        burn_and_no_resend_established: false,
        recovery_state_checked: false,
        v1_recovery_reopen_eligibility_established: false,
        v2_recovery_reopen_eligibility_established: false,
        placement_resumption_allowed: false,
        v1_a3_created: false,
        v1_a3_checked: false,
        v1_a3_durable: false,
        online_v2_a3_conjunct_created: false,
        online_v2_a3_conjunct_checked: false,
        online_v2_a3_conjunct_durable: false,
        authenticated_request_or_hmac_constructed: false,
        signed_order_body_constructed: false,
        place_dispatch_owner_or_grant_minted: false,
        network_dispatch_performed: false,
        pid_tid_or_rc_pointer_value_recorded: false,
        randomness_or_runtime_nonce_sampled: false,
        v1_config_reverse_pins_static_authorization_v3: false,
        v1_authorization_reverse_pins_static_authorization_v3: false,
        online_policy_v2_reverse_pins_static_authorization_v3: false,
        online_authorization_v2_reverse_pins_static_authorization_v3: false,
        reviewed_destination_reverse_pins_static_authorization_v3: false,
        reviewed_locator_reverse_pins_static_authorization_v3: false,
        fresh_delivery_reverse_pins_static_authorization_v3: false,
        reviewed_identity_reverse_pins_static_authorization_v3: false,
        remote_proof_policy_reverse_pins_static_authorization_v3: false,
        static_authorization_fingerprint_pinned_by_prospective_v3_prepared: false,
        durable_static_authorization_consumption_recorded: false,
        credential_mutation_authority_attested: false,
        authorization: OfflineAuthorizationState::DENIED,
    })
}

#[derive(Debug, Error)]
pub enum PmReviewedStaticOnlineAuthorizationV3Error {
    #[error("controlled-trial reviewed static online-authorization V3 is invalid: {0}")]
    Invalid(&'static str),
}

fn common_v1_v2_tuple_matches(context: &ReviewedStaticOnlineAuthorizationContextV3<'_>) -> bool {
    let v1 = context.v1_authorization.value();
    let v2 = context.online_authorization_v2.value();
    v1.phase == TrialPhase::APlaceCancel
        && v2.phase == TrialPhase::APlaceCancel
        && v1.build.repository_commit == v2.build.repository_commit
        && v1.build.cargo_lock_sha256 == v2.build.cargo_lock_sha256
        && v1.build.release_binary_sha256 == v2.build.release_binary_sha256
        && v1.build.release_binary_length == v2.build.release_binary_length
        && v1.host.host_identity == v2.host.uts_nodename
        && v1.host.boot_identity == v2.host.boot_id
        && v1.host.runtime_user == v2.host.nss_username
        && v1.host.egress_identity == v2.host.egress.authorized_geoblock_reported_public_ip
}

fn pins_match(
    context: &ReviewedStaticOnlineAuthorizationContextV3<'_>,
    value: &ReviewedStaticOnlineAuthorizationV3,
    v1_authorization_sha256: &str,
    v1_authorization_length: u64,
    verified: &VerifiedRoleMetadataV3<'_>,
) -> bool {
    value.v1_config.schema_version == context.v1_config.value().schema_version
        && value.v1_config.canonical_sha256 == context.v1_config.canonical_sha256()
        && value.v1_config.canonical_length == context.v1_config.canonical_length()
        && value.v1_config.fingerprint == context.v1_config.fingerprint()
        && value.v1_config.plan_fingerprint == context.v1_config.plan_fingerprint()
        && value.v1_authorization.schema_version == context.v1_authorization.value().schema_version
        && value.v1_authorization.authorization_id
            == context.v1_authorization.value().authorization_id
        && value.v1_authorization.canonical_sha256 == v1_authorization_sha256
        && value.v1_authorization.canonical_length == v1_authorization_length
        && value.v1_authorization.fingerprint == context.v1_authorization.fingerprint()
        && value.online_policy_v2.schema_version == context.online_policy_v2.value().schema_version
        && value.online_policy_v2.policy_id == context.online_policy_v2.value().policy_id
        && value.online_policy_v2.canonical_sha256 == context.online_policy_v2.canonical_sha256()
        && value.online_policy_v2.canonical_length == context.online_policy_v2.canonical_length()
        && value.online_policy_v2.fingerprint == context.online_policy_v2.fingerprint()
        && value.online_authorization_v2.schema_version
            == context.online_authorization_v2.value().schema_version
        && value.online_authorization_v2.authorization_id
            == context.online_authorization_v2.value().authorization_id
        && value.online_authorization_v2.canonical_sha256
            == context.online_authorization_v2.canonical_sha256()
        && value.online_authorization_v2.canonical_length
            == context.online_authorization_v2.canonical_length()
        && value.online_authorization_v2.fingerprint
            == context.online_authorization_v2.fingerprint()
        && role_pin_matches(
            ExactRolePinV3 {
                schema_version: value.reviewed_production_destination_v1.schema_version,
                role_id: Some(&value.reviewed_production_destination_v1.profile_id),
                canonical_sha256: &value.reviewed_production_destination_v1.canonical_sha256,
                canonical_length: value.reviewed_production_destination_v1.canonical_length,
                fingerprint: &value.reviewed_production_destination_v1.fingerprint,
            },
            ExactArtifactV3 {
                schema_version: verified.destination.schema_version,
                role_id: Some(verified.destination.role_id),
                canonical_sha256: context
                    .reviewed_production_destination_v1
                    .canonical_sha256(),
                canonical_length: context
                    .reviewed_production_destination_v1
                    .canonical_length(),
                fingerprint: context.reviewed_production_destination_v1.fingerprint(),
            },
        )
        && role_pin_matches(
            ExactRolePinV3 {
                schema_version: value
                    .reviewed_fresh_credential_slot_locator_v1
                    .schema_version,
                role_id: Some(&value.reviewed_fresh_credential_slot_locator_v1.locator_id),
                canonical_sha256: &value
                    .reviewed_fresh_credential_slot_locator_v1
                    .canonical_sha256,
                canonical_length: value
                    .reviewed_fresh_credential_slot_locator_v1
                    .canonical_length,
                fingerprint: &value.reviewed_fresh_credential_slot_locator_v1.fingerprint,
            },
            ExactArtifactV3 {
                schema_version: verified.locator.schema_version,
                role_id: Some(verified.locator.role_id),
                canonical_sha256: context
                    .reviewed_fresh_credential_slot_locator_v1
                    .canonical_sha256(),
                canonical_length: context
                    .reviewed_fresh_credential_slot_locator_v1
                    .canonical_length(),
                fingerprint: context
                    .reviewed_fresh_credential_slot_locator_v1
                    .fingerprint(),
            },
        )
        && role_pin_matches(
            ExactRolePinV3 {
                schema_version: value.fresh_credential_delivery_binding_v1.schema_version,
                role_id: Some(&value.fresh_credential_delivery_binding_v1.binding_id),
                canonical_sha256: &value.fresh_credential_delivery_binding_v1.canonical_sha256,
                canonical_length: value.fresh_credential_delivery_binding_v1.canonical_length,
                fingerprint: &value.fresh_credential_delivery_binding_v1.fingerprint,
            },
            ExactArtifactV3 {
                schema_version: verified.delivery.schema_version,
                role_id: Some(verified.delivery.role_id),
                canonical_sha256: context
                    .fresh_credential_delivery_binding_v1
                    .canonical_sha256(),
                canonical_length: context
                    .fresh_credential_delivery_binding_v1
                    .canonical_length(),
                fingerprint: context.fresh_credential_delivery_binding_v1.fingerprint(),
            },
        )
        && role_pin_matches(
            ExactRolePinV3 {
                schema_version: value
                    .reviewed_signer_proxy_account_identity_v1
                    .schema_version,
                role_id: Some(&value.reviewed_signer_proxy_account_identity_v1.identity_id),
                canonical_sha256: &value
                    .reviewed_signer_proxy_account_identity_v1
                    .canonical_sha256,
                canonical_length: value
                    .reviewed_signer_proxy_account_identity_v1
                    .canonical_length,
                fingerprint: &value.reviewed_signer_proxy_account_identity_v1.fingerprint,
            },
            ExactArtifactV3 {
                schema_version: verified.identity.schema_version,
                role_id: Some(verified.identity.role_id),
                canonical_sha256: context
                    .reviewed_signer_proxy_account_identity_v1
                    .canonical_sha256(),
                canonical_length: context
                    .reviewed_signer_proxy_account_identity_v1
                    .canonical_length(),
                fingerprint: context
                    .reviewed_signer_proxy_account_identity_v1
                    .fingerprint(),
            },
        )
        && role_pin_matches(
            ExactRolePinV3 {
                schema_version: value
                    .reviewed_remote_credential_proof_policy_v1
                    .schema_version,
                role_id: None,
                canonical_sha256: &value
                    .reviewed_remote_credential_proof_policy_v1
                    .canonical_sha256,
                canonical_length: value
                    .reviewed_remote_credential_proof_policy_v1
                    .canonical_length,
                fingerprint: &value.reviewed_remote_credential_proof_policy_v1.fingerprint,
            },
            ExactArtifactV3 {
                schema_version: verified.remote_schema_version,
                role_id: None,
                canonical_sha256: context
                    .reviewed_remote_credential_proof_policy_v1
                    .canonical_sha256(),
                canonical_length: context
                    .reviewed_remote_credential_proof_policy_v1
                    .canonical_length(),
                fingerprint: context
                    .reviewed_remote_credential_proof_policy_v1
                    .fingerprint(),
            },
        )
}

fn component_verifications_are_denied(
    destination: &crate::ReviewedProductionDestinationProfileVerificationV1,
    locator: &crate::ReviewedFreshCredentialSlotLocatorVerificationV1,
    delivery: &crate::FreshCredentialDeliveryBindingVerificationV1,
    identity: &crate::ReviewedSignerProxyAccountIdentityVerificationV1,
    remote: &crate::ReviewedRemoteCredentialProofPolicyVerificationV1,
) -> bool {
    let destination_ok = {
        let crate::ReviewedProductionDestinationProfileVerificationV1 {
            schema_version: _,
            profile_id: _,
            config_fingerprint: _,
            online_policy_fingerprint: _,
            online_authorization_fingerprint: _,
            reviewed_destination_profile_fingerprint: _,
            exact_v2_bindings_structurally_valid,
            fixed_six_destination_profile_structurally_valid,
            live_dns_observation_checked,
            destination_nat_equivalence_checked,
            authorization_consumption_checked,
            authorization,
        } = destination;
        *exact_v2_bindings_structurally_valid
            && *fixed_six_destination_profile_structurally_valid
            && !*live_dns_observation_checked
            && !*destination_nat_equivalence_checked
            && !*authorization_consumption_checked
            && *authorization == OfflineAuthorizationState::DENIED
    };
    let locator_ok = {
        let crate::ReviewedFreshCredentialSlotLocatorVerificationV1 {
            schema_version: _,
            locator_id: _,
            config_fingerprint: _,
            online_policy_fingerprint: _,
            online_authorization_fingerprint: _,
            reviewed_fresh_credential_slot_locator_fingerprint: _,
            exact_v2_bindings_structurally_valid,
            canonical_credential_slot_binding_structurally_valid,
            fixed_fresh_credential_locator_structurally_valid,
            source_owned_current_time_checked,
            protected_credential_directory_and_four_files_checked,
            loaded_bundle_matches_credential_slot_generation,
            remote_api_key_owner_attested,
            locator_fingerprint_pinned_by_v2,
            reviewer_authorship_attested,
            load_token_consumption_durably_recorded,
            authorization_consumption_checked,
            authorization,
        } = locator;
        *exact_v2_bindings_structurally_valid
            && *canonical_credential_slot_binding_structurally_valid
            && *fixed_fresh_credential_locator_structurally_valid
            && !*source_owned_current_time_checked
            && !*protected_credential_directory_and_four_files_checked
            && !*loaded_bundle_matches_credential_slot_generation
            && !*remote_api_key_owner_attested
            && !*locator_fingerprint_pinned_by_v2
            && !*reviewer_authorship_attested
            && !*load_token_consumption_durably_recorded
            && !*authorization_consumption_checked
            && *authorization == OfflineAuthorizationState::DENIED
    };
    let delivery_ok = {
        let crate::FreshCredentialDeliveryBindingVerificationV1 {
            schema_version: _,
            binding_id: _,
            config_fingerprint: _,
            online_policy_fingerprint: _,
            online_authorization_fingerprint: _,
            reviewed_fresh_credential_slot_locator_fingerprint: _,
            fresh_credential_delivery_binding_fingerprint: _,
            exact_reviewed_locator_pins_structurally_valid,
            unattested_provider_generation_labels_structurally_valid,
            unattested_linux_object_metadata_labels_structurally_valid,
            unattested_validity_labels_nested_within_v2,
            source_owned_current_time_checked,
            protected_credential_directory_and_four_files_checked,
            loaded_linux_objects_match_unattested_binding,
            same_loaded_holder_attested,
            globally_unique_delivery_attested,
            provider_authorship_attested,
            provider_signature_verified,
            provider_lease_fresh_and_unrevoked,
            rotation_generation_attested,
            delivery_freshness_attested,
            loaded_bundle_matches_credential_slot_generation,
            remote_api_key_owner_attested,
            locator_fingerprint_pinned_by_v2,
            delivery_binding_fingerprint_pinned_by_v2,
            delivery_consumption_durably_recorded,
            authorization_consumption_checked,
            credential_mutation_authority_attested,
            authorization,
        } = delivery;
        *exact_reviewed_locator_pins_structurally_valid
            && *unattested_provider_generation_labels_structurally_valid
            && *unattested_linux_object_metadata_labels_structurally_valid
            && *unattested_validity_labels_nested_within_v2
            && !*source_owned_current_time_checked
            && !*protected_credential_directory_and_four_files_checked
            && !*loaded_linux_objects_match_unattested_binding
            && !*same_loaded_holder_attested
            && !*globally_unique_delivery_attested
            && !*provider_authorship_attested
            && !*provider_signature_verified
            && !*provider_lease_fresh_and_unrevoked
            && !*rotation_generation_attested
            && !*delivery_freshness_attested
            && !*loaded_bundle_matches_credential_slot_generation
            && !*remote_api_key_owner_attested
            && !*locator_fingerprint_pinned_by_v2
            && !*delivery_binding_fingerprint_pinned_by_v2
            && !*delivery_consumption_durably_recorded
            && !*authorization_consumption_checked
            && !*credential_mutation_authority_attested
            && *authorization == OfflineAuthorizationState::DENIED
    };
    let identity_ok = {
        let crate::ReviewedSignerProxyAccountIdentityVerificationV1 {
            schema_version: _,
            identity_id: _,
            config_fingerprint: _,
            online_policy_fingerprint: _,
            online_authorization_fingerprint: _,
            reviewed_signer_proxy_account_identity_fingerprint: _,
            exact_config_policy_authorization_pins_structurally_valid,
            exact_official_source_manifest_pin_structurally_valid,
            official_source_manifest_sha256_matches_config_label,
            exact_claimed_account_tuple_matches_config,
            identity_id_matches_config_evidence_reference_label,
            source_reference_matches_config_label,
            official_source_manifest_bytes_loaded_and_hash_verified,
            reviewed_account_evidence_bytes_loaded_and_hash_verified,
            official_source_manifest_publisher_authorship_attested,
            reviewer_authorship_attested,
            source_authorship_attested,
            issuer_signature_verified,
            evidence_source_tls_and_server_identity_verified,
            signer_on_chain_eoa_status_verified,
            proxy_on_chain_contract_status_verified,
            on_chain_account_state_checked,
            on_chain_finality_checked,
            proxy_factory_semantics_verified,
            signer_controls_proxy_attested,
            signer_proxy_relationship_current,
            signer_proxy_relationship_unrevoked,
            account_specific_evidence_reference_resolved_and_authenticated,
            source_owned_current_time_checked,
            remote_api_key_owner_attested,
            private_key_derived_signer_matches_config_checked,
            l2_credentials_match_configured_signer_checked,
            identity_fingerprint_pinned_by_online_authorization_v2,
            identity_fingerprint_pinned_by_v3,
            identity_consumption_durably_recorded,
            authorization_consumption_checked,
            credential_mutation_authority_attested,
            authorization,
        } = identity;
        *exact_config_policy_authorization_pins_structurally_valid
            && *exact_official_source_manifest_pin_structurally_valid
            && *official_source_manifest_sha256_matches_config_label
            && *exact_claimed_account_tuple_matches_config
            && *identity_id_matches_config_evidence_reference_label
            && *source_reference_matches_config_label
            && !*official_source_manifest_bytes_loaded_and_hash_verified
            && !*reviewed_account_evidence_bytes_loaded_and_hash_verified
            && !*official_source_manifest_publisher_authorship_attested
            && !*reviewer_authorship_attested
            && !*source_authorship_attested
            && !*issuer_signature_verified
            && !*evidence_source_tls_and_server_identity_verified
            && !*signer_on_chain_eoa_status_verified
            && !*proxy_on_chain_contract_status_verified
            && !*on_chain_account_state_checked
            && !*on_chain_finality_checked
            && !*proxy_factory_semantics_verified
            && !*signer_controls_proxy_attested
            && !*signer_proxy_relationship_current
            && !*signer_proxy_relationship_unrevoked
            && !*account_specific_evidence_reference_resolved_and_authenticated
            && !*source_owned_current_time_checked
            && !*remote_api_key_owner_attested
            && !*private_key_derived_signer_matches_config_checked
            && !*l2_credentials_match_configured_signer_checked
            && !*identity_fingerprint_pinned_by_online_authorization_v2
            && !*identity_fingerprint_pinned_by_v3
            && !*identity_consumption_durably_recorded
            && !*authorization_consumption_checked
            && !*credential_mutation_authority_attested
            && *authorization == OfflineAuthorizationState::DENIED
    };
    let remote_ok = {
        let crate::ReviewedRemoteCredentialProofPolicyVerificationV1 {
            schema_version: _,
            config_fingerprint: _,
            online_policy_fingerprint: _,
            online_authorization_fingerprint: _,
            reviewed_destination_fingerprint: _,
            reviewed_fresh_credential_locator_fingerprint: _,
            fresh_credential_delivery_fingerprint: _,
            reviewed_signer_proxy_identity_fingerprint: _,
            reviewed_remote_credential_proof_policy_fingerprint: _,
            exact_config_policy_authorization_pins_structurally_valid,
            exact_destination_locator_delivery_identity_pins_structurally_valid,
            exact_official_source_manifest_and_entries_structurally_valid,
            exact_closed_only_protocol_policy_structurally_valid,
            validity_envelope_nested_within_online_authorization_v2,
            selected_peer_and_local_egress_labels_match_bound_records,
            official_source_manifest_bytes_loaded_and_hash_verified,
            api_authentication_source_bytes_loaded_and_hash_verified,
            manage_orders_source_bytes_loaded_and_hash_verified,
            official_source_publisher_authorship_attested,
            official_source_manifest_publisher_authorship_attested,
            api_authentication_source_publisher_authorship_attested,
            manage_orders_source_publisher_authorship_attested,
            reviewer_authorship_attested,
            remote_api_key_owner_attested,
            authoritative_authentication_acceptance_contract_available,
            api_key_mismatch_rejected_before_http_200_attested,
            l2_secret_mismatch_rejected_before_http_200_attested,
            passphrase_mismatch_rejected_before_http_200_attested,
            poly_address_mismatch_rejected_before_http_200_attested,
            timestamp_mismatch_rejected_before_http_200_attested,
            authentication_precedes_closed_only_handler_attested,
            response_not_shared_or_cache_derived_attested,
            strict_http_200_implies_live_credential_tuple_acceptance_attested,
            credential_provider_authorship_attested,
            credential_delivery_generation_attested,
            same_loaded_credential_holder_attested,
            post_load_same_holder_runtime_conjunction_attested,
            credential_delivery_and_remote_proof_same_source_generation_attested,
            globally_unique_credential_delivery_attested,
            rotation_generation_attested,
            protected_credential_directory_and_four_objects_checked,
            loaded_credentials_match_delivery_binding,
            request_l2_tuple_from_same_loaded_credential_holder_checked,
            selected_actor_generation_bound,
            product_clock_owner_bound,
            retained_delivery_evidence_and_load_token_joined_for_selected_actor,
            private_key_derived_signer_matches_config_checked,
            l2_credentials_match_configured_signer_checked,
            signer_controls_proxy_attested,
            signer_proxy_relationship_current_and_unrevoked_attested,
            server_time_sample_received,
            server_time_proof_authenticated_and_fresh,
            source_owned_current_time_checked,
            server_time_sample_to_dispatch_freshness_checked,
            server_time_and_closed_only_same_peer_pairing_checked,
            proof_observation_freshness_checked,
            response_receive_freshness_checked,
            poly_address_header_from_configured_signer_produced,
            sensitive_request_headers_produced,
            request_query_body_and_content_type_absence_enforced,
            request_accept_application_json_header_produced,
            accept_encoding_identity_header_produced,
            hmac_preimage_produced,
            hmac_signature_produced,
            fixed_local_egress_selected_and_checked,
            fixed_reviewed_peer_selected_and_checked,
            network_namespace_and_interface_selected_and_checked,
            tunnel_or_gateway_profile_checked,
            live_dns_answer_checked,
            dnssec_checked,
            dns_ttl_freshness_checked,
            destination_nat_equivalence_checked,
            authorized_public_ip_checked,
            connect_and_request_timeouts_enforced,
            authenticated_dispatch_performed_once,
            redirect_retry_proxy_and_fallback_absence_enforced,
            response_received,
            connected_peer_checked_before_status_and_body,
            tls_server_identity_verified,
            http_status_200_checked,
            response_content_type_checked,
            response_content_encoding_checked,
            response_body_length_and_exact_schema_checked,
            closed_only_boolean_observed,
            closed_only_false_readiness_checked,
            closed_only_true_hard_block_checked,
            ambiguous_outcome_durable_burn_performed,
            live_credential_tuple_accepted_by_provider,
            credential_tuple_current_and_unrevoked_attested,
            online_authorization_v2_reverse_pins_remote_policy,
            reviewed_destination_reverse_pins_remote_policy,
            reviewed_locator_reverse_pins_remote_policy,
            fresh_delivery_reverse_pins_remote_policy,
            reviewed_identity_reverse_pins_remote_policy,
            remote_policy_fingerprint_pinned_by_online_authorization_v2,
            remote_policy_fingerprint_pinned_by_v3,
            remote_policy_consumption_durably_recorded,
            authorization_consumption_checked,
            credential_mutation_authority_attested,
            authorization,
        } = remote;
        *exact_config_policy_authorization_pins_structurally_valid
            && *exact_destination_locator_delivery_identity_pins_structurally_valid
            && *exact_official_source_manifest_and_entries_structurally_valid
            && *exact_closed_only_protocol_policy_structurally_valid
            && *validity_envelope_nested_within_online_authorization_v2
            && *selected_peer_and_local_egress_labels_match_bound_records
            && !*official_source_manifest_bytes_loaded_and_hash_verified
            && !*api_authentication_source_bytes_loaded_and_hash_verified
            && !*manage_orders_source_bytes_loaded_and_hash_verified
            && !*official_source_publisher_authorship_attested
            && !*official_source_manifest_publisher_authorship_attested
            && !*api_authentication_source_publisher_authorship_attested
            && !*manage_orders_source_publisher_authorship_attested
            && !*reviewer_authorship_attested
            && !*remote_api_key_owner_attested
            && !*authoritative_authentication_acceptance_contract_available
            && !*api_key_mismatch_rejected_before_http_200_attested
            && !*l2_secret_mismatch_rejected_before_http_200_attested
            && !*passphrase_mismatch_rejected_before_http_200_attested
            && !*poly_address_mismatch_rejected_before_http_200_attested
            && !*timestamp_mismatch_rejected_before_http_200_attested
            && !*authentication_precedes_closed_only_handler_attested
            && !*response_not_shared_or_cache_derived_attested
            && !*strict_http_200_implies_live_credential_tuple_acceptance_attested
            && !*credential_provider_authorship_attested
            && !*credential_delivery_generation_attested
            && !*same_loaded_credential_holder_attested
            && !*post_load_same_holder_runtime_conjunction_attested
            && !*credential_delivery_and_remote_proof_same_source_generation_attested
            && !*globally_unique_credential_delivery_attested
            && !*rotation_generation_attested
            && !*protected_credential_directory_and_four_objects_checked
            && !*loaded_credentials_match_delivery_binding
            && !*request_l2_tuple_from_same_loaded_credential_holder_checked
            && !*selected_actor_generation_bound
            && !*product_clock_owner_bound
            && !*retained_delivery_evidence_and_load_token_joined_for_selected_actor
            && !*private_key_derived_signer_matches_config_checked
            && !*l2_credentials_match_configured_signer_checked
            && !*signer_controls_proxy_attested
            && !*signer_proxy_relationship_current_and_unrevoked_attested
            && !*server_time_sample_received
            && !*server_time_proof_authenticated_and_fresh
            && !*source_owned_current_time_checked
            && !*server_time_sample_to_dispatch_freshness_checked
            && !*server_time_and_closed_only_same_peer_pairing_checked
            && !*proof_observation_freshness_checked
            && !*response_receive_freshness_checked
            && !*poly_address_header_from_configured_signer_produced
            && !*sensitive_request_headers_produced
            && !*request_query_body_and_content_type_absence_enforced
            && !*request_accept_application_json_header_produced
            && !*accept_encoding_identity_header_produced
            && !*hmac_preimage_produced
            && !*hmac_signature_produced
            && !*fixed_local_egress_selected_and_checked
            && !*fixed_reviewed_peer_selected_and_checked
            && !*network_namespace_and_interface_selected_and_checked
            && !*tunnel_or_gateway_profile_checked
            && !*live_dns_answer_checked
            && !*dnssec_checked
            && !*dns_ttl_freshness_checked
            && !*destination_nat_equivalence_checked
            && !*authorized_public_ip_checked
            && !*connect_and_request_timeouts_enforced
            && !*authenticated_dispatch_performed_once
            && !*redirect_retry_proxy_and_fallback_absence_enforced
            && !*response_received
            && !*connected_peer_checked_before_status_and_body
            && !*tls_server_identity_verified
            && !*http_status_200_checked
            && !*response_content_type_checked
            && !*response_content_encoding_checked
            && !*response_body_length_and_exact_schema_checked
            && !*closed_only_boolean_observed
            && !*closed_only_false_readiness_checked
            && !*closed_only_true_hard_block_checked
            && !*ambiguous_outcome_durable_burn_performed
            && !*live_credential_tuple_accepted_by_provider
            && !*credential_tuple_current_and_unrevoked_attested
            && !*online_authorization_v2_reverse_pins_remote_policy
            && !*reviewed_destination_reverse_pins_remote_policy
            && !*reviewed_locator_reverse_pins_remote_policy
            && !*fresh_delivery_reverse_pins_remote_policy
            && !*reviewed_identity_reverse_pins_remote_policy
            && !*remote_policy_fingerprint_pinned_by_online_authorization_v2
            && !*remote_policy_fingerprint_pinned_by_v3
            && !*remote_policy_consumption_durably_recorded
            && !*authorization_consumption_checked
            && !*credential_mutation_authority_attested
            && *authorization == OfflineAuthorizationState::DENIED
    };
    destination_ok && locator_ok && delivery_ok && identity_ok && remote_ok
}

struct ExactPinViewV3<'a> {
    schema_version: u32,
    expected_schema_version: u32,
    role_id: Option<&'a str>,
    canonical_sha256: &'a str,
    canonical_length: u64,
    fingerprint: &'a str,
}

struct VerifiedRoleV3<'a> {
    schema_version: u32,
    role_id: &'a str,
}

struct VerifiedRoleMetadataV3<'a> {
    destination: VerifiedRoleV3<'a>,
    locator: VerifiedRoleV3<'a>,
    delivery: VerifiedRoleV3<'a>,
    identity: VerifiedRoleV3<'a>,
    remote_schema_version: u32,
}

struct ExactRolePinV3<'a> {
    schema_version: u32,
    role_id: Option<&'a str>,
    canonical_sha256: &'a str,
    canonical_length: u64,
    fingerprint: &'a str,
}

struct ExactArtifactV3<'a> {
    schema_version: u32,
    role_id: Option<&'a str>,
    canonical_sha256: &'a str,
    canonical_length: u64,
    fingerprint: &'a str,
}

fn validate_exact_pin(
    pin: ExactPinViewV3<'_>,
    message: &'static str,
) -> Result<(), PmReviewedStaticOnlineAuthorizationV3Error> {
    if pin.schema_version != pin.expected_schema_version || pin.canonical_length == 0 {
        return Err(invalid(message));
    }
    if let Some(role_id) = pin.role_id {
        validate_token(role_id, 128, message)?;
    }
    validate_sha256(pin.canonical_sha256)?;
    validate_sha256(pin.fingerprint)
}

fn role_pin_matches(pin: ExactRolePinV3<'_>, artifact: ExactArtifactV3<'_>) -> bool {
    pin.schema_version == artifact.schema_version
        && pin.role_id == artifact.role_id
        && pin.canonical_sha256 == artifact.canonical_sha256
        && pin.canonical_length == artifact.canonical_length
        && pin.fingerprint == artifact.fingerprint
}

fn parse_exact_canonical<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
) -> Result<T, PmReviewedStaticOnlineAuthorizationV3Error> {
    let value: T = serde_json::from_slice(bytes).map_err(|_| {
        invalid(
            "reviewed static online-authorization JSON is malformed, duplicated, unknown, or trailing",
        )
    })?;
    let canonical = serde_json::to_vec(&value).map_err(|_| {
        invalid("reviewed static online-authorization cannot be serialized canonically")
    })?;
    if canonical != bytes {
        return Err(invalid(
            "reviewed static online-authorization bytes are not exact canonical compact JSON",
        ));
    }
    Ok(value)
}

fn parse_utc(value: &str) -> Result<DateTime<Utc>, PmReviewedStaticOnlineAuthorizationV3Error> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid("reviewed static online-authorization timestamp is invalid"))?
        .with_timezone(&Utc);
    if parsed.to_rfc3339_opts(SecondsFormat::Secs, true) != value {
        return Err(invalid(
            "reviewed static online-authorization timestamp is not canonical UTC seconds",
        ));
    }
    Ok(parsed)
}

fn validate_sha256(value: &str) -> Result<(), PmReviewedStaticOnlineAuthorizationV3Error> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(
            "reviewed static online-authorization SHA-256 value is invalid",
        ));
    }
    Ok(())
}

fn validate_token(
    value: &str,
    maximum: usize,
    message: &'static str,
) -> Result<(), PmReviewedStaticOnlineAuthorizationV3Error> {
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
) -> Result<(), PmReviewedStaticOnlineAuthorizationV3Error> {
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

fn invalid(message: &'static str) -> PmReviewedStaticOnlineAuthorizationV3Error {
    PmReviewedStaticOnlineAuthorizationV3Error::Invalid(message)
}
