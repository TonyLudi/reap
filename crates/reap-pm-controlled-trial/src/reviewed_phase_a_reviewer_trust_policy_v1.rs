//! Protected canonical Phase-A reviewer trust policy prerequisite V1.
//!
//! This record names the exact offline reviewer roles and Ed25519 verification
//! keys that a future Phase-A V5 may bind into a separately authenticated,
//! short-lived authorization record. Loading and verifying this policy proves
//! only its protected local-file custody, exact canonical bytes, closed schema,
//! bounded lifetime policy, and structural public-key encoding.
//!
//! A listed public key is not evidence of reviewer identity, role assignment,
//! key custody, key currentness, key revocation state, independent human
//! review, or possession of the corresponding private key. This module has no
//! private-key input, signing, signature verification, draft helper, clock,
//! runtime actor, request, permit, network, or mutation capability. Its only
//! authorization result is [`OfflineAuthorizationState::DENIED`] with zero
//! place-dispatch allowance.

use std::{collections::BTreeSet, fmt, path::Path};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::{
    OfflineAuthorizationState,
    protected_file::{ProtectedFileKind, read_one},
};

pub const REVIEWED_PHASE_A_REVIEWER_TRUST_POLICY_V1_SCHEMA_VERSION: u32 = 1;
pub const PM_T2_REVIEWED_PHASE_A_REVIEWER_TRUST_POLICY_FILE_V1: &str =
    "pm-t2-reviewed-phase-a-reviewer-trust-policy-v1.json";
pub const MAX_PHASE_A_AUTHORIZATION_TTL_SECONDS_V1: u64 = 900;
pub const MAX_PHASE_A_REVIEWER_APPROVERS_V1: usize = 8;

const REVIEWED_PHASE_A_REVIEWER_TRUST_POLICY_ID_V1: &str =
    "pm-t2-reviewed-phase-a-reviewer-trust-policy-v1";
const MIN_PHASE_A_REVIEWER_APPROVERS_V1: usize = 1;
const REQUIRED_PHASE_A_REVIEWER_QUORUM_V1: u8 = 1;
const MAX_CANONICAL_REVIEWED_PHASE_A_REVIEWER_TRUST_POLICY_BYTES_V1: usize = 64 * 1024;
const REVIEWED_PHASE_A_REVIEWER_TRUST_POLICY_V1_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.controlled-trial.reviewed-phase-a-reviewer-trust-policy.v1\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ReviewedPhaseAReviewerTrustRecordRoleV1 {
    #[serde(rename = "offline_reviewer_trust_prerequisite_only_no_authorization_v1")]
    OfflineReviewerTrustPrerequisiteOnlyNoAuthorizationV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ReviewedPhaseAReviewerTrustAlgorithmV1 {
    #[serde(rename = "ed25519")]
    Ed25519,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum ReviewedPhaseARequiredReviewerRoleV1 {
    #[serde(rename = "phase_a_reviewer_v1")]
    PhaseAReviewerV1,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReviewedPhaseAReviewerApproverV1 {
    pub(crate) approver_id: String,
    pub(crate) role: ReviewedPhaseARequiredReviewerRoleV1,
    pub(crate) key_id: String,
    pub(crate) public_key_base64url_no_pad: String,
}

/// Exact offline policy schema. It contains public verification keys only and
/// carries no signature, request, current-time observation, or authority.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReviewedPhaseAReviewerTrustPolicyV1 {
    pub(crate) schema_version: u32,
    pub(crate) policy_id: String,
    pub(crate) record_role: ReviewedPhaseAReviewerTrustRecordRoleV1,
    pub(crate) signature_algorithm: ReviewedPhaseAReviewerTrustAlgorithmV1,
    pub(crate) maximum_authorization_ttl_seconds: u64,
    pub(crate) required_reviewer_roles: [ReviewedPhaseARequiredReviewerRoleV1; 1],
    pub(crate) required_distinct_approver_quorum: u8,
    pub(crate) approvers: Vec<ReviewedPhaseAReviewerApproverV1>,
}

impl ReviewedPhaseAReviewerTrustPolicyV1 {
    fn validate_intrinsic(&self) -> Result<(), PmReviewedPhaseAReviewerTrustPolicyV1Error> {
        if self.schema_version != REVIEWED_PHASE_A_REVIEWER_TRUST_POLICY_V1_SCHEMA_VERSION
            || self.policy_id != REVIEWED_PHASE_A_REVIEWER_TRUST_POLICY_ID_V1
            || self.record_role
                != ReviewedPhaseAReviewerTrustRecordRoleV1::OfflineReviewerTrustPrerequisiteOnlyNoAuthorizationV1
            || self.signature_algorithm != ReviewedPhaseAReviewerTrustAlgorithmV1::Ed25519
        {
            return Err(invalid(
                "reviewer trust policy identity, role, or algorithm is invalid",
            ));
        }
        if self.maximum_authorization_ttl_seconds == 0
            || self.maximum_authorization_ttl_seconds > MAX_PHASE_A_AUTHORIZATION_TTL_SECONDS_V1
        {
            return Err(invalid(
                "reviewer trust policy maximum authorization TTL is invalid",
            ));
        }
        if self.required_reviewer_roles != [ReviewedPhaseARequiredReviewerRoleV1::PhaseAReviewerV1]
            || self.required_distinct_approver_quorum != REQUIRED_PHASE_A_REVIEWER_QUORUM_V1
        {
            return Err(invalid(
                "reviewer trust policy required roles or quorum are invalid",
            ));
        }
        if !(MIN_PHASE_A_REVIEWER_APPROVERS_V1..=MAX_PHASE_A_REVIEWER_APPROVERS_V1)
            .contains(&self.approvers.len())
        {
            return Err(invalid("reviewer trust policy approver bound is invalid"));
        }
        if self
            .approvers
            .windows(2)
            .any(|pair| pair[0].approver_id >= pair[1].approver_id)
        {
            return Err(invalid(
                "reviewer trust policy approvers are not in canonical unique ID order",
            ));
        }

        let required_roles = self
            .required_reviewer_roles
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let mut approver_ids = BTreeSet::new();
        let mut roles = BTreeSet::new();
        let mut key_ids = BTreeSet::new();
        let mut public_keys = BTreeSet::new();
        for approver in &self.approvers {
            validate_identifier(&approver.approver_id)?;
            validate_identifier(&approver.key_id)?;
            if !required_roles.contains(&approver.role)
                || !approver_ids.insert(approver.approver_id.as_str())
                || !key_ids.insert(approver.key_id.as_str())
                || !public_keys.insert(approver.public_key_base64url_no_pad.as_str())
            {
                return Err(invalid(
                    "reviewer trust policy approver identity or key is duplicated or its role is out of scope",
                ));
            }
            roles.insert(approver.role);
            validate_ed25519_public_key(&approver.public_key_base64url_no_pad)?;
        }
        if roles != required_roles
            || roles.len() != usize::from(self.required_distinct_approver_quorum)
        {
            return Err(invalid(
                "reviewer trust policy does not cover the exact required role quorum",
            ));
        }
        Ok(())
    }
}

impl fmt::Debug for ReviewedPhaseAReviewerTrustPolicyV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "ReviewedPhaseAReviewerTrustPolicyV1(<offline-public-key-trust-prerequisite; no-authorship-currentness-runtime-or-authority; redacted; denied>)",
        )
    }
}

/// Move-only exact canonical holder. Public callers can project only its three
/// byte commitments; the parsed policy remains crate-internal for a future V5.
pub struct CanonicalReviewedPhaseAReviewerTrustPolicyV1 {
    value: ReviewedPhaseAReviewerTrustPolicyV1,
    canonical_bytes: Zeroizing<Vec<u8>>,
    canonical_sha256: String,
    fingerprint: String,
}

impl CanonicalReviewedPhaseAReviewerTrustPolicyV1 {
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

    #[allow(dead_code, reason = "reserved for the reviewed Phase-A V5 join")]
    pub(crate) fn value(&self) -> &ReviewedPhaseAReviewerTrustPolicyV1 {
        &self.value
    }
}

impl fmt::Debug for CanonicalReviewedPhaseAReviewerTrustPolicyV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "CanonicalReviewedPhaseAReviewerTrustPolicyV1(<exact-protected-canonical-bytes; no-value-id-role-key-path-signature-runtime-or-authority-projection; redacted; denied>)",
        )
    }
}

/// Exhaustive structural-only report. Every authorship, currentness, runtime,
/// signature, quorum-realization, permit, network, and authority fact is false.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewedPhaseAReviewerTrustPolicyVerificationV1 {
    pub schema_version: u32,
    pub reviewed_phase_a_reviewer_trust_policy_v1_fingerprint: String,
    pub exact_policy_id_and_record_role_structurally_valid: bool,
    pub ed25519_only_algorithm_structurally_valid: bool,
    pub maximum_authorization_ttl_at_most_900_seconds_structurally_valid: bool,
    pub closed_phase_a_reviewer_role_structurally_valid: bool,
    pub exact_one_reviewer_quorum_structurally_valid: bool,
    pub bounded_unique_approvers_structurally_valid: bool,
    pub unique_approver_ids_and_key_ids_structurally_valid: bool,
    pub canonical_unique_32_byte_public_key_encodings_structurally_valid: bool,
    pub policy_reviewer_authorship_attested: bool,
    pub approver_identity_attested: bool,
    pub approver_role_assignment_attested: bool,
    pub corresponding_private_key_possession_attested: bool,
    pub reviewer_key_custody_attested: bool,
    pub reviewer_keys_current: bool,
    pub reviewer_keys_unrevoked: bool,
    pub ed25519_public_keys_cryptographically_validated: bool,
    pub source_owned_current_time_checked: bool,
    pub authorization_request_current: bool,
    pub authorization_request_bound: bool,
    pub authorization_signature_verified: bool,
    pub authorization_quorum_satisfied: bool,
    pub independent_human_review_attested: bool,
    pub runtime_candidate_bound: bool,
    pub runtime_actor_generation_bound: bool,
    pub runtime_attempt_bound: bool,
    pub permit_minted: bool,
    pub authenticated_request_or_hmac_constructed: bool,
    pub network_dispatch_performed: bool,
    pub credential_mutation_authority_attested: bool,
    #[serde(flatten)]
    pub authorization: OfflineAuthorizationState,
}

pub fn load_canonical_reviewed_phase_a_reviewer_trust_policy_v1(
    path: &Path,
) -> Result<CanonicalReviewedPhaseAReviewerTrustPolicyV1, PmReviewedPhaseAReviewerTrustPolicyV1Error>
{
    let bytes = read_one(
        path,
        ProtectedFileKind::ReviewedPhaseAReviewerTrustPolicyV1,
        MAX_CANONICAL_REVIEWED_PHASE_A_REVIEWER_TRUST_POLICY_BYTES_V1,
    )
    .map_err(|_| invalid("reviewer trust policy protection or stability check failed"))?;
    let value: ReviewedPhaseAReviewerTrustPolicyV1 = parse_exact_canonical(&bytes)?;
    value.validate_intrinsic()?;
    let canonical_bytes = Zeroizing::new(bytes.to_vec());
    Ok(CanonicalReviewedPhaseAReviewerTrustPolicyV1 {
        canonical_sha256: hash_bytes(&[], &canonical_bytes),
        fingerprint: hash_bytes(
            REVIEWED_PHASE_A_REVIEWER_TRUST_POLICY_V1_FINGERPRINT_DOMAIN,
            &canonical_bytes,
        ),
        canonical_bytes,
        value,
    })
}

/// Revalidate only the immutable canonical policy grammar. This function does
/// not accept a clock, request, signature, runtime observation, or authority.
pub fn verify_reviewed_phase_a_reviewer_trust_policy_v1(
    reviewed: &CanonicalReviewedPhaseAReviewerTrustPolicyV1,
) -> Result<
    ReviewedPhaseAReviewerTrustPolicyVerificationV1,
    PmReviewedPhaseAReviewerTrustPolicyV1Error,
> {
    reviewed.value.validate_intrinsic()?;
    Ok(ReviewedPhaseAReviewerTrustPolicyVerificationV1 {
        schema_version: reviewed.value.schema_version,
        reviewed_phase_a_reviewer_trust_policy_v1_fingerprint: reviewed.fingerprint.clone(),
        exact_policy_id_and_record_role_structurally_valid: true,
        ed25519_only_algorithm_structurally_valid: true,
        maximum_authorization_ttl_at_most_900_seconds_structurally_valid: true,
        closed_phase_a_reviewer_role_structurally_valid: true,
        exact_one_reviewer_quorum_structurally_valid: true,
        bounded_unique_approvers_structurally_valid: true,
        unique_approver_ids_and_key_ids_structurally_valid: true,
        canonical_unique_32_byte_public_key_encodings_structurally_valid: true,
        policy_reviewer_authorship_attested: false,
        approver_identity_attested: false,
        approver_role_assignment_attested: false,
        corresponding_private_key_possession_attested: false,
        reviewer_key_custody_attested: false,
        reviewer_keys_current: false,
        reviewer_keys_unrevoked: false,
        ed25519_public_keys_cryptographically_validated: false,
        source_owned_current_time_checked: false,
        authorization_request_current: false,
        authorization_request_bound: false,
        authorization_signature_verified: false,
        authorization_quorum_satisfied: false,
        independent_human_review_attested: false,
        runtime_candidate_bound: false,
        runtime_actor_generation_bound: false,
        runtime_attempt_bound: false,
        permit_minted: false,
        authenticated_request_or_hmac_constructed: false,
        network_dispatch_performed: false,
        credential_mutation_authority_attested: false,
        authorization: OfflineAuthorizationState::DENIED,
    })
}

#[derive(Debug, Error)]
pub enum PmReviewedPhaseAReviewerTrustPolicyV1Error {
    #[error("controlled-trial reviewed Phase-A reviewer trust policy V1 is invalid: {0}")]
    Invalid(&'static str),
}

fn invalid(message: &'static str) -> PmReviewedPhaseAReviewerTrustPolicyV1Error {
    PmReviewedPhaseAReviewerTrustPolicyV1Error::Invalid(message)
}

fn parse_exact_canonical<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
) -> Result<T, PmReviewedPhaseAReviewerTrustPolicyV1Error> {
    let value: T = serde_json::from_slice(bytes).map_err(|_| {
        invalid("reviewer trust policy JSON is malformed, duplicated, unknown, or trailing")
    })?;
    let canonical = serde_json::to_vec(&value)
        .map_err(|_| invalid("reviewer trust policy cannot be serialized canonically"))?;
    if canonical != bytes {
        return Err(invalid(
            "reviewer trust policy bytes are not exact canonical compact JSON",
        ));
    }
    Ok(value)
}

fn validate_identifier(value: &str) -> Result<(), PmReviewedPhaseAReviewerTrustPolicyV1Error> {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return Err(invalid("reviewer trust policy identifier is invalid"));
    };
    if value.len() > 128
        || !(first.is_ascii_lowercase() || first.is_ascii_digit())
        || bytes.any(|byte| {
            !(byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'_' | b'.' | b':'))
        })
    {
        return Err(invalid("reviewer trust policy identifier is invalid"));
    }
    Ok(())
}

fn validate_ed25519_public_key(
    value: &str,
) -> Result<(), PmReviewedPhaseAReviewerTrustPolicyV1Error> {
    let decoded = URL_SAFE_NO_PAD
        .decode(value.as_bytes())
        .map_err(|_| invalid("reviewer trust policy Ed25519 public-key encoding is invalid"))?;
    if decoded.len() != 32
        || decoded.iter().all(|byte| *byte == 0)
        || URL_SAFE_NO_PAD.encode(&decoded) != value
    {
        return Err(invalid(
            "reviewer trust policy Ed25519 public key is not canonical nonzero 32-byte base64url without padding",
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
