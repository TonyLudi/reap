//! Offline reviewed local-operator cooperative credential-custody profile V1.
//!
//! This additive record describes one deliberately bounded alternative that a
//! future Phase-A V5 may choose instead of a provider-signed credential lease.
//! It pins the exact canonical config, V2 policy and authorization, static V3,
//! eligibility V4, reviewed locator, and unsigned delivery binding. All pins
//! are derived from those canonical holders; no caller supplies a digest,
//! runtime observation, actor identity, current clock, proof, or permission.
//!
//! The trust model is intentionally narrow. The exact authorized Linux EUID
//! account and every same-EUID actor are assumed trusted, quiescent, and
//! cooperative for the complete reviewed window. A continuously held advisory
//! directory lease is only a coordination protocol inside that trust boundary;
//! it is not protection against a same-EUID actor that ignores the lease. A
//! future runner may hold one directory descriptor and four source descriptors,
//! retaining the three named L2 entries for crash recovery.
//!
//! Cleanup may report only a conditional observation: while the cooperative
//! lease remained held, the expected basename matched the held source, name
//! removal returned success, the directory was synchronized, and the basename
//! was then absent. Linux supplies no atomic unlink-if-name-still-identifies-
//! this-inode operation. This profile therefore attests no atomic exact-inode
//! unlink, secure erasure, provider origin, global delivery uniqueness,
//! credential currentness, revocation state, or absence of other descriptors
//! or credential copies.
//!
//! This module opens only its own protected canonical record. It never opens a
//! credential directory, acquires a lease, reads a credential, removes a name,
//! synchronizes a directory, constructs an actor/request/HMAC, writes a
//! journal, or mints authority. V4 remains byte-for-byte semantically
//! unchanged and permanently denied. A future V5 must separately select and
//! consume this alternative together with reviewer trust, live remote
//! acceptance, signer/proxy control, actor/attempt, durable burn, and
//! single-dispatch evidence. The reviewed Poly proxy control policy remains a
//! separate role and is deliberately not treated as credential custody.

use std::{fmt, path::Path};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    CanonicalReviewedPhaseAEligibilityEnvelopeV4, ONLINE_AUTHORIZATION_V2_SCHEMA_VERSION,
    ONLINE_POLICY_V2_SCHEMA_VERSION, OfflineAuthorizationState,
    REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_SCHEMA_VERSION,
    REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4_SCHEMA_VERSION,
    REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3_SCHEMA_VERSION,
    ReviewedPhaseAEligibilityEnvelopeContextV4, TrialPhase,
    config::TRIAL_CONFIG_SCHEMA_VERSION,
    fresh_credential_delivery_binding_v1::FRESH_CREDENTIAL_DELIVERY_BINDING_V1_SCHEMA_VERSION,
    protected_file::{ProtectedFileKind, read_one},
    verify_reviewed_phase_a_eligibility_envelope_v4,
};

pub const REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_V1_SCHEMA_VERSION: u32 = 1;
pub const PM_T2_REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_FILE_V1: &str =
    "pm-t2-reviewed-local-operator-cooperative-custody-profile-v1.json";

const REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_ID_V1: &str =
    "pm-t2-reviewed-local-operator-cooperative-custody-profile-v1";
const MAX_CANONICAL_REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_BYTES_V1: usize =
    128 * 1024;
const REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_V1_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.reviewed-local-operator-cooperative-custody-profile.v1\0";

/// Exact config-role pins, including the distinct trial-plan commitment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedLocalOperatorCustodyConfigPinsV1 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
    pub trial_plan_fingerprint: String,
}

/// Exact online-policy V2 role pins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedLocalOperatorCustodyOnlinePolicyPinsV1 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact online-authorization V2 role pins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedLocalOperatorCustodyOnlineAuthorizationPinsV1 {
    pub schema_version: u32,
    pub authorization_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact static V3 role pins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedLocalOperatorCustodyStaticAuthorizationPinsV1 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact denied eligibility V4 role pins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedLocalOperatorCustodyEligibilityEnvelopePinsV1 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact reviewed lexical-locator V1 role pins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedLocalOperatorCustodyLocatorPinsV1 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Exact unsigned delivery-binding V1 role pins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedLocalOperatorCustodyDeliveryPinsV1 {
    pub schema_version: u32,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedLocalOperatorCustodyRecordRoleV1 {
    #[serde(rename = "offline_local_trust_alternative_only_no_authorization_v1")]
    OfflineLocalTrustAlternativeOnlyNoAuthorizationV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedLocalOperatorCustodyPhaseScopeV1 {
    #[serde(rename = "phase_a_exactly_one_place_then_exact_cancel_only_v1")]
    PhaseAExactlyOnePlaceThenExactCancelOnlyV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedLocalOperatorSameEuidTrustV1 {
    #[serde(
        rename = "exact_authorized_euid_account_and_all_same_euid_actors_trusted_quiescent_for_complete_window_v1"
    )]
    ExactAuthorizedEuidAccountAndAllSameEuidActorsTrustedQuiescentForCompleteWindowV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedLocalOperatorDirectoryLeaseV1 {
    #[serde(rename = "continuous_advisory_directory_lease_coordination_only_v1")]
    ContinuousAdvisoryDirectoryLeaseCoordinationOnlyV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedLocalOperatorSourceDescriptorCustodyV1 {
    #[serde(rename = "one_held_directory_fd_and_four_held_credential_source_fds_v1")]
    OneHeldDirectoryFdAndFourHeldCredentialSourceFdsV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedLocalOperatorRecoveryRetentionV1 {
    #[serde(rename = "three_named_l2_entries_persist_for_recovery_until_terminal_v1")]
    ThreeNamedL2EntriesPersistForRecoveryUntilTerminalV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedLocalOperatorCleanupObservationV1 {
    #[serde(
        rename = "conditional_expected_basename_removal_directory_fsync_then_absence_observation_under_continuous_lease_v1"
    )]
    ConditionalExpectedBasenameRemovalDirectoryFsyncThenAbsenceObservationUnderContinuousLeaseV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedLocalOperatorAtomicUnlinkLimitationV1 {
    #[serde(rename = "no_atomic_unlink_if_name_still_identifies_held_inode_claim_v1")]
    NoAtomicUnlinkIfNameStillIdentifiesHeldInodeClaimV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedLocalOperatorSecureErasureLimitationV1 {
    #[serde(rename = "name_removal_and_userspace_drop_are_not_secure_erasure_v1")]
    NameRemovalAndUserspaceDropAreNotSecureErasureV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedLocalOperatorProviderOriginLimitationV1 {
    #[serde(rename = "no_credential_provider_origin_or_authorship_claim_v1")]
    NoCredentialProviderOriginOrAuthorshipClaimV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedLocalOperatorGlobalUniquenessLimitationV1 {
    #[serde(rename = "no_global_credential_delivery_uniqueness_claim_v1")]
    NoGlobalCredentialDeliveryUniquenessClaimV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedLocalOperatorCurrentnessLimitationV1 {
    #[serde(rename = "no_credential_currentness_claim_v1")]
    NoCredentialCurrentnessClaimV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedLocalOperatorRevocationLimitationV1 {
    #[serde(rename = "no_credential_or_delivery_revocation_state_claim_v1")]
    NoCredentialOrDeliveryRevocationStateClaimV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedLocalOperatorOtherCopiesLimitationV1 {
    #[serde(rename = "no_absence_of_other_descriptors_or_credential_copies_claim_v1")]
    NoAbsenceOfOtherDescriptorsOrCredentialCopiesClaimV1,
}

/// Closed trust grammar. Each field has one admissible variant; no caller can
/// widen the trust boundary or convert a limitation into evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedLocalOperatorCooperativeCustodyTrustV1 {
    pub same_euid_trust: ReviewedLocalOperatorSameEuidTrustV1,
    pub directory_lease: ReviewedLocalOperatorDirectoryLeaseV1,
    pub source_descriptor_custody: ReviewedLocalOperatorSourceDescriptorCustodyV1,
    pub recovery_retention: ReviewedLocalOperatorRecoveryRetentionV1,
    pub cleanup_observation: ReviewedLocalOperatorCleanupObservationV1,
    pub atomic_unlink_limitation: ReviewedLocalOperatorAtomicUnlinkLimitationV1,
    pub secure_erasure_limitation: ReviewedLocalOperatorSecureErasureLimitationV1,
    pub provider_origin_limitation: ReviewedLocalOperatorProviderOriginLimitationV1,
    pub global_uniqueness_limitation: ReviewedLocalOperatorGlobalUniquenessLimitationV1,
    pub currentness_limitation: ReviewedLocalOperatorCurrentnessLimitationV1,
    pub revocation_limitation: ReviewedLocalOperatorRevocationLimitationV1,
    pub other_copies_limitation: ReviewedLocalOperatorOtherCopiesLimitationV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewedLocalOperatorCustodySuccessorDispositionV1 {
    #[serde(rename = "future_v5_may_select_this_alternative_v4_remains_unchanged_denied_v1")]
    FutureV5MaySelectThisAlternativeV4RemainsUnchangedDeniedV1,
}

/// Non-secret labels copied from already canonical holders. They are not
/// current-runtime observations and the copied reviewer string is not an
/// authorship attestation.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedLocalOperatorCustodyAudienceV1 {
    pub phase: TrialPhase,
    pub online_authorization_id: String,
    pub unattested_online_authorization_reviewer_label: String,
    pub reviewed_at_utc: String,
    pub not_before_utc: String,
    pub expires_at_utc: String,
    pub cleanup_not_after_utc: String,
    pub repository_commit: String,
    pub cargo_lock_sha256: String,
    pub release_binary_sha256: String,
    pub release_binary_length: u64,
    pub uts_nodename: String,
    pub boot_id: String,
    pub nss_username: String,
    pub linux_euid: u32,
    pub chain_id: u64,
    pub signature_type: u8,
    pub wallet_profile: String,
    pub signer: String,
    pub funder: String,
    pub credential_slot_id: String,
    pub credential_slot_nonsecret_fingerprint_sha256: String,
}

impl fmt::Debug for ReviewedLocalOperatorCustodyAudienceV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "ReviewedLocalOperatorCustodyAudienceV1(<exact-build-host-account-window-labels; redacted; not-current>)",
        )
    }
}

/// Canonical offline trust-alternative record. It has no proof bytes, runtime
/// identity, source descriptor, credential value, cleanup result, or permit.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedLocalOperatorCooperativeCustodyProfileV1 {
    pub schema_version: u32,
    pub profile_id: String,
    pub record_role: ReviewedLocalOperatorCustodyRecordRoleV1,
    pub phase_scope: ReviewedLocalOperatorCustodyPhaseScopeV1,
    pub canonical_config: ReviewedLocalOperatorCustodyConfigPinsV1,
    pub online_policy_v2: ReviewedLocalOperatorCustodyOnlinePolicyPinsV1,
    pub online_authorization_v2: ReviewedLocalOperatorCustodyOnlineAuthorizationPinsV1,
    pub reviewed_static_online_authorization_v3:
        ReviewedLocalOperatorCustodyStaticAuthorizationPinsV1,
    pub reviewed_phase_a_eligibility_envelope_v4:
        ReviewedLocalOperatorCustodyEligibilityEnvelopePinsV1,
    pub reviewed_fresh_credential_slot_locator_v1: ReviewedLocalOperatorCustodyLocatorPinsV1,
    pub fresh_credential_delivery_binding_v1: ReviewedLocalOperatorCustodyDeliveryPinsV1,
    pub audience: ReviewedLocalOperatorCustodyAudienceV1,
    pub cooperative_trust: ReviewedLocalOperatorCooperativeCustodyTrustV1,
    pub successor_disposition: ReviewedLocalOperatorCustodySuccessorDispositionV1,
}

impl fmt::Debug for ReviewedLocalOperatorCooperativeCustodyProfileV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "ReviewedLocalOperatorCooperativeCustodyProfileV1(<offline-local-trust-alternative; no-runtime-proof-or-authority; redacted; denied>)",
        )
    }
}

impl ReviewedLocalOperatorCooperativeCustodyProfileV1 {
    fn validate_intrinsic(
        &self,
    ) -> Result<(), PmReviewedLocalOperatorCooperativeCustodyProfileV1Error> {
        if self.schema_version
            != REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_V1_SCHEMA_VERSION
            || self.profile_id != REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_ID_V1
            || self.record_role
                != ReviewedLocalOperatorCustodyRecordRoleV1::OfflineLocalTrustAlternativeOnlyNoAuthorizationV1
            || self.phase_scope
                != ReviewedLocalOperatorCustodyPhaseScopeV1::PhaseAExactlyOnePlaceThenExactCancelOnlyV1
            || self.successor_disposition
                != ReviewedLocalOperatorCustodySuccessorDispositionV1::FutureV5MaySelectThisAlternativeV4RemainsUnchangedDeniedV1
        {
            return Err(invalid("local-operator cooperative custody closed record role is invalid"));
        }
        validate_config_pins(&self.canonical_config)?;
        validate_pin(
            PinView {
                schema_version: self.online_policy_v2.schema_version,
                expected_schema_version: ONLINE_POLICY_V2_SCHEMA_VERSION,
                canonical_sha256: &self.online_policy_v2.canonical_sha256,
                canonical_length: self.online_policy_v2.canonical_length,
                fingerprint: &self.online_policy_v2.fingerprint,
            },
            "local-operator cooperative custody online-policy pin is invalid",
        )?;
        validate_pin(
            PinView {
                schema_version: self.online_authorization_v2.schema_version,
                expected_schema_version: ONLINE_AUTHORIZATION_V2_SCHEMA_VERSION,
                canonical_sha256: &self.online_authorization_v2.canonical_sha256,
                canonical_length: self.online_authorization_v2.canonical_length,
                fingerprint: &self.online_authorization_v2.fingerprint,
            },
            "local-operator cooperative custody online-authorization pin is invalid",
        )?;
        validate_token(&self.online_authorization_v2.authorization_id)?;
        validate_pin(
            PinView {
                schema_version: self.reviewed_static_online_authorization_v3.schema_version,
                expected_schema_version: REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3_SCHEMA_VERSION,
                canonical_sha256: &self
                    .reviewed_static_online_authorization_v3
                    .canonical_sha256,
                canonical_length: self
                    .reviewed_static_online_authorization_v3
                    .canonical_length,
                fingerprint: &self.reviewed_static_online_authorization_v3.fingerprint,
            },
            "local-operator cooperative custody static V3 pin is invalid",
        )?;
        validate_pin(
            PinView {
                schema_version: self.reviewed_phase_a_eligibility_envelope_v4.schema_version,
                expected_schema_version: REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4_SCHEMA_VERSION,
                canonical_sha256: &self
                    .reviewed_phase_a_eligibility_envelope_v4
                    .canonical_sha256,
                canonical_length: self
                    .reviewed_phase_a_eligibility_envelope_v4
                    .canonical_length,
                fingerprint: &self.reviewed_phase_a_eligibility_envelope_v4.fingerprint,
            },
            "local-operator cooperative custody eligibility V4 pin is invalid",
        )?;
        validate_pin(
            PinView {
                schema_version: self
                    .reviewed_fresh_credential_slot_locator_v1
                    .schema_version,
                expected_schema_version: REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_SCHEMA_VERSION,
                canonical_sha256: &self
                    .reviewed_fresh_credential_slot_locator_v1
                    .canonical_sha256,
                canonical_length: self
                    .reviewed_fresh_credential_slot_locator_v1
                    .canonical_length,
                fingerprint: &self.reviewed_fresh_credential_slot_locator_v1.fingerprint,
            },
            "local-operator cooperative custody locator pin is invalid",
        )?;
        validate_pin(
            PinView {
                schema_version: self.fresh_credential_delivery_binding_v1.schema_version,
                expected_schema_version: FRESH_CREDENTIAL_DELIVERY_BINDING_V1_SCHEMA_VERSION,
                canonical_sha256: &self.fresh_credential_delivery_binding_v1.canonical_sha256,
                canonical_length: self.fresh_credential_delivery_binding_v1.canonical_length,
                fingerprint: &self.fresh_credential_delivery_binding_v1.fingerprint,
            },
            "local-operator cooperative custody delivery pin is invalid",
        )?;
        self.audience.validate()
    }
}

impl ReviewedLocalOperatorCustodyAudienceV1 {
    fn validate(&self) -> Result<(), PmReviewedLocalOperatorCooperativeCustodyProfileV1Error> {
        if self.phase != TrialPhase::APlaceCancel
            || self.release_binary_length == 0
            || self.linux_euid == 0
            || self.linux_euid == u32::MAX
            || self.chain_id != 137
            || self.signature_type != 1
            || self.wallet_profile != "poly_proxy"
            || self.signer == self.funder
        {
            return Err(invalid(
                "local-operator cooperative custody audience is invalid",
            ));
        }
        validate_token(&self.online_authorization_id)?;
        validate_reference(&self.unattested_online_authorization_reviewer_label)?;
        validate_lower_hex(&self.repository_commit, 40)?;
        for digest in [
            &self.cargo_lock_sha256,
            &self.release_binary_sha256,
            &self.credential_slot_nonsecret_fingerprint_sha256,
        ] {
            validate_sha256(digest)?;
        }
        for address in [&self.signer, &self.funder] {
            validate_evm_address(address)?;
        }
        validate_token(&self.credential_slot_id)?;
        for label in [&self.uts_nodename, &self.boot_id, &self.nss_username] {
            validate_reference(label)?;
        }
        let reviewed_at = parse_utc(&self.reviewed_at_utc)?;
        let not_before = parse_utc(&self.not_before_utc)?;
        let expires_at = parse_utc(&self.expires_at_utc)?;
        let cleanup_not_after = parse_utc(&self.cleanup_not_after_utc)?;
        if reviewed_at > not_before || not_before >= expires_at || expires_at > cleanup_not_after {
            return Err(invalid(
                "local-operator cooperative custody audience time envelope is invalid",
            ));
        }
        Ok(())
    }
}

/// Exact canonical holder with only byte commitments projected.
pub struct CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1 {
    value: ReviewedLocalOperatorCooperativeCustodyProfileV1,
    canonical_bytes: Vec<u8>,
    canonical_sha256: String,
    fingerprint: String,
}

impl CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1 {
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

impl fmt::Debug for CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1(<exact-protected-canonical-bytes; no-value-path-fd-proof-runtime-or-authority-projection; redacted; denied>)",
        )
    }
}

/// Exact canonical-holder context. The nested V4 context supplies all holders
/// needed to reverify V4 while this profile pins only its seven distinct roles.
pub struct ReviewedLocalOperatorCooperativeCustodyProfileContextV1<'a> {
    pub phase_a_eligibility_context: ReviewedPhaseAEligibilityEnvelopeContextV4<'a>,
    pub reviewed_phase_a_eligibility_envelope_v4: &'a CanonicalReviewedPhaseAEligibilityEnvelopeV4,
}

/// Exhaustive structural report. Only immutable pin/grammar facts are true;
/// every runtime, trust-realization, cleanup, proof, and authority fact is
/// false.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewedLocalOperatorCooperativeCustodyProfileVerificationV1 {
    pub schema_version: u32,
    pub canonical_config_fingerprint: String,
    pub online_policy_v2_fingerprint: String,
    pub online_authorization_v2_fingerprint: String,
    pub reviewed_static_online_authorization_v3_fingerprint: String,
    pub reviewed_phase_a_eligibility_envelope_v4_fingerprint: String,
    pub reviewed_fresh_credential_slot_locator_v1_fingerprint: String,
    pub fresh_credential_delivery_binding_v1_fingerprint: String,
    pub reviewed_local_operator_cooperative_custody_profile_v1_fingerprint: String,
    pub exact_canonical_config_pin_structurally_valid: bool,
    pub exact_online_policy_v2_pin_structurally_valid: bool,
    pub exact_online_authorization_v2_pin_structurally_valid: bool,
    pub exact_reviewed_static_online_authorization_v3_pin_structurally_valid: bool,
    pub exact_reviewed_phase_a_eligibility_envelope_v4_pin_structurally_valid: bool,
    pub exact_reviewed_fresh_credential_slot_locator_v1_pin_structurally_valid: bool,
    pub exact_fresh_credential_delivery_binding_v1_pin_structurally_valid: bool,
    pub exact_build_host_account_window_labels_match_canonical_holders: bool,
    pub closed_local_operator_trust_grammar_structurally_valid: bool,
    pub v4_reverified_unchanged_and_denied: bool,
    pub future_v5_selection_label_structurally_valid: bool,
    pub profile_reviewer_authorship_attested: bool,
    pub source_owned_current_time_checked: bool,
    pub exact_linux_euid_observed_at_runtime: bool,
    pub all_same_euid_actors_trusted_and_quiescent_observed: bool,
    pub advisory_directory_lease_acquired: bool,
    pub advisory_directory_lease_continuously_held: bool,
    pub held_directory_descriptor_opened: bool,
    pub four_held_credential_source_descriptors_opened: bool,
    pub loaded_linux_objects_match_delivery_binding: bool,
    pub private_key_and_l2_credentials_loaded_and_bound: bool,
    pub named_l2_entries_retained_for_recovery: bool,
    pub conditional_basename_identity_checked: bool,
    pub basename_removal_performed: bool,
    pub directory_fsync_performed: bool,
    pub post_fsync_basename_absence_observed: bool,
    pub atomic_unlink_if_inode_attested: bool,
    pub secure_erasure_attested: bool,
    pub credential_provider_origin_attested: bool,
    pub globally_unique_credential_delivery_attested: bool,
    pub credential_currentness_attested: bool,
    pub credential_or_delivery_unrevoked_attested: bool,
    pub no_other_descriptors_or_credential_copies_attested: bool,
    pub selected_actor_generation_bound: bool,
    pub source_owned_runtime_attempt_bound: bool,
    pub same_holder_live_remote_acceptance_proof_verified: bool,
    pub signer_proxy_control_proof_verified: bool,
    pub durable_attempt_burn_and_no_resend_established: bool,
    pub fixed_egress_single_dispatch_owner_minted: bool,
    pub authenticated_request_or_hmac_constructed: bool,
    pub network_dispatch_performed: bool,
    pub credential_mutation_authority_attested: bool,
    #[serde(flatten)]
    pub authorization: OfflineAuthorizationState,
}

/// Purely draft the exact non-authorizing record from canonical holders.
pub fn draft_non_authorizing_reviewed_local_operator_cooperative_custody_profile_v1(
    context: &ReviewedLocalOperatorCooperativeCustodyProfileContextV1<'_>,
) -> Result<
    ReviewedLocalOperatorCooperativeCustodyProfileV1,
    PmReviewedLocalOperatorCooperativeCustodyProfileV1Error,
> {
    let v4 = verify_reviewed_phase_a_eligibility_envelope_v4(
        &context.phase_a_eligibility_context,
        context.reviewed_phase_a_eligibility_envelope_v4,
    )
    .map_err(|_| invalid("local-operator cooperative custody bound V4 is invalid"))?;
    if v4.authorization != OfflineAuthorizationState::DENIED
        || v4.credential_mutation_authority_attested
        || v4.place_dispatch_owner_or_grant_minted
        || v4.network_dispatch_performed
        || v4.placement_resumption_allowed
    {
        return Err(invalid(
            "local-operator cooperative custody requires unchanged denied V4 evidence",
        ));
    }

    let joined = &context.phase_a_eligibility_context;
    let config = joined.v1_config;
    let policy = joined.online_policy_v2;
    let authorization = joined.online_authorization_v2;
    let static_v3 = joined.reviewed_static_online_authorization_v3;
    let locator = joined.reviewed_fresh_credential_slot_locator_v1;
    let delivery = joined.fresh_credential_delivery_binding_v1;
    let account = &config.value().account;
    let slot = &config.value().credential_slot;
    let online = authorization.value();

    Ok(ReviewedLocalOperatorCooperativeCustodyProfileV1 {
        schema_version: REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_V1_SCHEMA_VERSION,
        profile_id: REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_ID_V1.to_owned(),
        record_role:
            ReviewedLocalOperatorCustodyRecordRoleV1::OfflineLocalTrustAlternativeOnlyNoAuthorizationV1,
        phase_scope:
            ReviewedLocalOperatorCustodyPhaseScopeV1::PhaseAExactlyOnePlaceThenExactCancelOnlyV1,
        canonical_config: ReviewedLocalOperatorCustodyConfigPinsV1 {
            schema_version: config.value().schema_version,
            canonical_sha256: config.canonical_sha256().to_owned(),
            canonical_length: config.canonical_length(),
            fingerprint: config.fingerprint().to_owned(),
            trial_plan_fingerprint: config.plan_fingerprint().to_owned(),
        },
        online_policy_v2: ReviewedLocalOperatorCustodyOnlinePolicyPinsV1 {
            schema_version: policy.value().schema_version,
            canonical_sha256: policy.canonical_sha256().to_owned(),
            canonical_length: policy.canonical_length(),
            fingerprint: policy.fingerprint().to_owned(),
        },
        online_authorization_v2: ReviewedLocalOperatorCustodyOnlineAuthorizationPinsV1 {
            schema_version: online.schema_version,
            authorization_id: online.authorization_id.clone(),
            canonical_sha256: authorization.canonical_sha256().to_owned(),
            canonical_length: authorization.canonical_length(),
            fingerprint: authorization.fingerprint().to_owned(),
        },
        reviewed_static_online_authorization_v3:
            ReviewedLocalOperatorCustodyStaticAuthorizationPinsV1 {
                schema_version: REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3_SCHEMA_VERSION,
                canonical_sha256: static_v3.canonical_sha256().to_owned(),
                canonical_length: static_v3.canonical_length(),
                fingerprint: static_v3.fingerprint().to_owned(),
            },
        reviewed_phase_a_eligibility_envelope_v4:
            ReviewedLocalOperatorCustodyEligibilityEnvelopePinsV1 {
                schema_version: REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4_SCHEMA_VERSION,
                canonical_sha256: context
                    .reviewed_phase_a_eligibility_envelope_v4
                    .canonical_sha256()
                    .to_owned(),
                canonical_length: context
                    .reviewed_phase_a_eligibility_envelope_v4
                    .canonical_length(),
                fingerprint: context
                    .reviewed_phase_a_eligibility_envelope_v4
                    .fingerprint()
                    .to_owned(),
            },
        reviewed_fresh_credential_slot_locator_v1:
            ReviewedLocalOperatorCustodyLocatorPinsV1 {
                schema_version: REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_SCHEMA_VERSION,
                canonical_sha256: locator.canonical_sha256().to_owned(),
                canonical_length: locator.canonical_length(),
                fingerprint: locator.fingerprint().to_owned(),
            },
        fresh_credential_delivery_binding_v1: ReviewedLocalOperatorCustodyDeliveryPinsV1 {
            schema_version: FRESH_CREDENTIAL_DELIVERY_BINDING_V1_SCHEMA_VERSION,
            canonical_sha256: delivery.canonical_sha256().to_owned(),
            canonical_length: delivery.canonical_length(),
            fingerprint: delivery.fingerprint().to_owned(),
        },
        audience: ReviewedLocalOperatorCustodyAudienceV1 {
            phase: online.phase,
            online_authorization_id: online.authorization_id.clone(),
            unattested_online_authorization_reviewer_label: online.issuing_reviewer.clone(),
            reviewed_at_utc: online.reviewed_at_utc.clone(),
            not_before_utc: online.not_before_utc.clone(),
            expires_at_utc: online.expires_at_utc.clone(),
            cleanup_not_after_utc: online.cleanup_not_after_utc.clone(),
            repository_commit: online.build.repository_commit.clone(),
            cargo_lock_sha256: online.build.cargo_lock_sha256.clone(),
            release_binary_sha256: online.build.release_binary_sha256.clone(),
            release_binary_length: online.build.release_binary_length,
            uts_nodename: online.host.uts_nodename.clone(),
            boot_id: online.host.boot_id.clone(),
            nss_username: online.host.nss_username.clone(),
            linux_euid: online.host.linux_euid,
            chain_id: account.chain_id,
            signature_type: account.signature_type,
            wallet_profile: account.wallet_profile.clone(),
            signer: account.signer.clone(),
            funder: account.funder.clone(),
            credential_slot_id: slot.slot_id.clone(),
            credential_slot_nonsecret_fingerprint_sha256: slot
                .nonsecret_fingerprint_sha256
                .clone(),
        },
        cooperative_trust: closed_trust_grammar(),
        successor_disposition:
            ReviewedLocalOperatorCustodySuccessorDispositionV1::FutureV5MaySelectThisAlternativeV4RemainsUnchangedDeniedV1,
    })
}

pub fn load_canonical_reviewed_local_operator_cooperative_custody_profile_v1(
    path: &Path,
) -> Result<
    CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1,
    PmReviewedLocalOperatorCooperativeCustodyProfileV1Error,
> {
    let bytes = read_one(
        path,
        ProtectedFileKind::ReviewedLocalOperatorCooperativeCustodyProfileV1,
        MAX_CANONICAL_REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_BYTES_V1,
    )
    .map_err(|_| {
        invalid("local-operator cooperative custody profile protection or stability check failed")
    })?;
    let value: ReviewedLocalOperatorCooperativeCustodyProfileV1 = parse_exact_canonical(&bytes)?;
    value.validate_intrinsic()?;
    let canonical_bytes = bytes.to_vec();
    Ok(CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1 {
        canonical_sha256: hash_bytes(&[], &canonical_bytes),
        fingerprint: hash_bytes(
            REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_V1_FINGERPRINT_DOMAIN,
            &canonical_bytes,
        ),
        canonical_bytes,
        value,
    })
}

pub fn verify_reviewed_local_operator_cooperative_custody_profile_v1(
    context: &ReviewedLocalOperatorCooperativeCustodyProfileContextV1<'_>,
    reviewed: &CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1,
) -> Result<
    ReviewedLocalOperatorCooperativeCustodyProfileVerificationV1,
    PmReviewedLocalOperatorCooperativeCustodyProfileV1Error,
> {
    let expected =
        draft_non_authorizing_reviewed_local_operator_cooperative_custody_profile_v1(context)?;
    if reviewed.value != expected {
        return Err(invalid(
            "local-operator cooperative custody exact pin, audience, or closed grammar mismatched",
        ));
    }
    let joined = &context.phase_a_eligibility_context;
    Ok(
        ReviewedLocalOperatorCooperativeCustodyProfileVerificationV1 {
            schema_version: reviewed.value.schema_version,
            canonical_config_fingerprint: joined.v1_config.fingerprint().to_owned(),
            online_policy_v2_fingerprint: joined.online_policy_v2.fingerprint().to_owned(),
            online_authorization_v2_fingerprint: joined
                .online_authorization_v2
                .fingerprint()
                .to_owned(),
            reviewed_static_online_authorization_v3_fingerprint: joined
                .reviewed_static_online_authorization_v3
                .fingerprint()
                .to_owned(),
            reviewed_phase_a_eligibility_envelope_v4_fingerprint: context
                .reviewed_phase_a_eligibility_envelope_v4
                .fingerprint()
                .to_owned(),
            reviewed_fresh_credential_slot_locator_v1_fingerprint: joined
                .reviewed_fresh_credential_slot_locator_v1
                .fingerprint()
                .to_owned(),
            fresh_credential_delivery_binding_v1_fingerprint: joined
                .fresh_credential_delivery_binding_v1
                .fingerprint()
                .to_owned(),
            reviewed_local_operator_cooperative_custody_profile_v1_fingerprint: reviewed
                .fingerprint
                .clone(),
            exact_canonical_config_pin_structurally_valid: true,
            exact_online_policy_v2_pin_structurally_valid: true,
            exact_online_authorization_v2_pin_structurally_valid: true,
            exact_reviewed_static_online_authorization_v3_pin_structurally_valid: true,
            exact_reviewed_phase_a_eligibility_envelope_v4_pin_structurally_valid: true,
            exact_reviewed_fresh_credential_slot_locator_v1_pin_structurally_valid: true,
            exact_fresh_credential_delivery_binding_v1_pin_structurally_valid: true,
            exact_build_host_account_window_labels_match_canonical_holders: true,
            closed_local_operator_trust_grammar_structurally_valid: true,
            v4_reverified_unchanged_and_denied: true,
            future_v5_selection_label_structurally_valid: true,
            profile_reviewer_authorship_attested: false,
            source_owned_current_time_checked: false,
            exact_linux_euid_observed_at_runtime: false,
            all_same_euid_actors_trusted_and_quiescent_observed: false,
            advisory_directory_lease_acquired: false,
            advisory_directory_lease_continuously_held: false,
            held_directory_descriptor_opened: false,
            four_held_credential_source_descriptors_opened: false,
            loaded_linux_objects_match_delivery_binding: false,
            private_key_and_l2_credentials_loaded_and_bound: false,
            named_l2_entries_retained_for_recovery: false,
            conditional_basename_identity_checked: false,
            basename_removal_performed: false,
            directory_fsync_performed: false,
            post_fsync_basename_absence_observed: false,
            atomic_unlink_if_inode_attested: false,
            secure_erasure_attested: false,
            credential_provider_origin_attested: false,
            globally_unique_credential_delivery_attested: false,
            credential_currentness_attested: false,
            credential_or_delivery_unrevoked_attested: false,
            no_other_descriptors_or_credential_copies_attested: false,
            selected_actor_generation_bound: false,
            source_owned_runtime_attempt_bound: false,
            same_holder_live_remote_acceptance_proof_verified: false,
            signer_proxy_control_proof_verified: false,
            durable_attempt_burn_and_no_resend_established: false,
            fixed_egress_single_dispatch_owner_minted: false,
            authenticated_request_or_hmac_constructed: false,
            network_dispatch_performed: false,
            credential_mutation_authority_attested: false,
            authorization: OfflineAuthorizationState::DENIED,
        },
    )
}

fn closed_trust_grammar() -> ReviewedLocalOperatorCooperativeCustodyTrustV1 {
    ReviewedLocalOperatorCooperativeCustodyTrustV1 {
        same_euid_trust:
            ReviewedLocalOperatorSameEuidTrustV1::ExactAuthorizedEuidAccountAndAllSameEuidActorsTrustedQuiescentForCompleteWindowV1,
        directory_lease:
            ReviewedLocalOperatorDirectoryLeaseV1::ContinuousAdvisoryDirectoryLeaseCoordinationOnlyV1,
        source_descriptor_custody:
            ReviewedLocalOperatorSourceDescriptorCustodyV1::OneHeldDirectoryFdAndFourHeldCredentialSourceFdsV1,
        recovery_retention:
            ReviewedLocalOperatorRecoveryRetentionV1::ThreeNamedL2EntriesPersistForRecoveryUntilTerminalV1,
        cleanup_observation:
            ReviewedLocalOperatorCleanupObservationV1::ConditionalExpectedBasenameRemovalDirectoryFsyncThenAbsenceObservationUnderContinuousLeaseV1,
        atomic_unlink_limitation:
            ReviewedLocalOperatorAtomicUnlinkLimitationV1::NoAtomicUnlinkIfNameStillIdentifiesHeldInodeClaimV1,
        secure_erasure_limitation:
            ReviewedLocalOperatorSecureErasureLimitationV1::NameRemovalAndUserspaceDropAreNotSecureErasureV1,
        provider_origin_limitation:
            ReviewedLocalOperatorProviderOriginLimitationV1::NoCredentialProviderOriginOrAuthorshipClaimV1,
        global_uniqueness_limitation:
            ReviewedLocalOperatorGlobalUniquenessLimitationV1::NoGlobalCredentialDeliveryUniquenessClaimV1,
        currentness_limitation:
            ReviewedLocalOperatorCurrentnessLimitationV1::NoCredentialCurrentnessClaimV1,
        revocation_limitation:
            ReviewedLocalOperatorRevocationLimitationV1::NoCredentialOrDeliveryRevocationStateClaimV1,
        other_copies_limitation:
            ReviewedLocalOperatorOtherCopiesLimitationV1::NoAbsenceOfOtherDescriptorsOrCredentialCopiesClaimV1,
    }
}

#[derive(Debug, Error)]
pub enum PmReviewedLocalOperatorCooperativeCustodyProfileV1Error {
    #[error(
        "controlled-trial reviewed local-operator cooperative custody profile V1 is invalid: {0}"
    )]
    Invalid(&'static str),
}

fn invalid(message: &'static str) -> PmReviewedLocalOperatorCooperativeCustodyProfileV1Error {
    PmReviewedLocalOperatorCooperativeCustodyProfileV1Error::Invalid(message)
}

struct PinView<'a> {
    schema_version: u32,
    expected_schema_version: u32,
    canonical_sha256: &'a str,
    canonical_length: u64,
    fingerprint: &'a str,
}

fn validate_config_pins(
    pins: &ReviewedLocalOperatorCustodyConfigPinsV1,
) -> Result<(), PmReviewedLocalOperatorCooperativeCustodyProfileV1Error> {
    validate_pin(
        PinView {
            schema_version: pins.schema_version,
            expected_schema_version: TRIAL_CONFIG_SCHEMA_VERSION,
            canonical_sha256: &pins.canonical_sha256,
            canonical_length: pins.canonical_length,
            fingerprint: &pins.fingerprint,
        },
        "local-operator cooperative custody config pin is invalid",
    )?;
    validate_sha256(&pins.trial_plan_fingerprint)
}

fn validate_pin(
    pin: PinView<'_>,
    message: &'static str,
) -> Result<(), PmReviewedLocalOperatorCooperativeCustodyProfileV1Error> {
    if pin.schema_version != pin.expected_schema_version || pin.canonical_length == 0 {
        return Err(invalid(message));
    }
    validate_sha256(pin.canonical_sha256).map_err(|_| invalid(message))?;
    validate_sha256(pin.fingerprint).map_err(|_| invalid(message))
}

fn parse_exact_canonical<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
) -> Result<T, PmReviewedLocalOperatorCooperativeCustodyProfileV1Error> {
    let value: T = serde_json::from_slice(bytes).map_err(|_| {
        invalid("local-operator cooperative custody JSON is malformed, duplicated, unknown, or trailing")
    })?;
    let canonical = serde_json::to_vec(&value).map_err(|_| {
        invalid("local-operator cooperative custody record cannot be serialized canonically")
    })?;
    if canonical != bytes {
        return Err(invalid(
            "local-operator cooperative custody bytes are not exact canonical compact JSON",
        ));
    }
    Ok(value)
}

fn parse_utc(
    value: &str,
) -> Result<DateTime<Utc>, PmReviewedLocalOperatorCooperativeCustodyProfileV1Error> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid("local-operator cooperative custody timestamp is invalid"))?
        .with_timezone(&Utc);
    if parsed.to_rfc3339_opts(chrono::SecondsFormat::Secs, true) != value {
        return Err(invalid(
            "local-operator cooperative custody timestamp is not canonical UTC seconds",
        ));
    }
    Ok(parsed)
}

fn validate_token(
    value: &str,
) -> Result<(), PmReviewedLocalOperatorCooperativeCustodyProfileV1Error> {
    if value.is_empty()
        || value.len() > 128
        || value.bytes().any(|byte| {
            !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/'))
        })
    {
        return Err(invalid(
            "local-operator cooperative custody token label is invalid",
        ));
    }
    Ok(())
}

fn validate_reference(
    value: &str,
) -> Result<(), PmReviewedLocalOperatorCooperativeCustodyProfileV1Error> {
    if value.is_empty()
        || value.len() > 256
        || value.bytes().any(|byte| !(0x20..=0x7e).contains(&byte))
    {
        return Err(invalid(
            "local-operator cooperative custody reference label is invalid",
        ));
    }
    Ok(())
}

fn validate_sha256(
    value: &str,
) -> Result<(), PmReviewedLocalOperatorCooperativeCustodyProfileV1Error> {
    validate_lower_hex(value, 64)
}

fn validate_lower_hex(
    value: &str,
    expected_length: usize,
) -> Result<(), PmReviewedLocalOperatorCooperativeCustodyProfileV1Error> {
    if value.len() != expected_length
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(
            "local-operator cooperative custody hexadecimal label is invalid",
        ));
    }
    Ok(())
}

fn validate_evm_address(
    value: &str,
) -> Result<(), PmReviewedLocalOperatorCooperativeCustodyProfileV1Error> {
    if value.len() != 42
        || !value.starts_with("0x")
        || value[2..].bytes().any(|byte| !byte.is_ascii_hexdigit())
    {
        return Err(invalid(
            "local-operator cooperative custody account address label is invalid",
        ));
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
