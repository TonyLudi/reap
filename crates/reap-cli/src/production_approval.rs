use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::rand::SystemRandom;
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

use crate::production_evidence::{
    PRODUCTION_EVIDENCE_APPROVAL_SUBJECT_FORMAT_VERSION,
    PRODUCTION_EVIDENCE_MANIFEST_SCHEMA_VERSION, PRODUCTION_EVIDENCE_REPORT_FORMAT_VERSION,
    ProductionEvidenceApprovalSubject, verify_production_evidence_manifest_path,
};
use reap_core::PINNED_JAVA_REVISION;

const APPROVAL_POLICY_SCHEMA_VERSION: u16 = 1;
const APPROVAL_KEY_FORMAT_VERSION: u16 = 1;
const APPROVAL_POLICY_VERIFICATION_FORMAT_VERSION: u16 = 1;
const APPROVAL_REQUEST_FORMAT_VERSION: u16 = 1;
const APPROVAL_SIGNATURE_FORMAT_VERSION: u16 = 1;
const APPROVAL_VERIFICATION_FORMAT_VERSION: u16 = 1;
const APPROVAL_ALGORITHM: &str = "ed25519";
const APPROVAL_ACTION: &str = "production_rollout_review";
const APPROVAL_SIGNATURE_DOMAIN: &str = "reap.production-approval.v1";
const MAX_APPROVAL_POLICY_BYTES: u64 = 64 * 1024;
const MAX_APPROVAL_ARTIFACT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_APPROVAL_PRIVATE_KEY_BYTES: u64 = 16 * 1024;
const MAX_APPROVAL_TTL_MS: u64 = 15 * 60 * 1_000;
const MAX_APPROVAL_CLOCK_SKEW_MS: u64 = 5 * 60 * 1_000;
const MAX_APPROVAL_ROLES: usize = 8;
const MAX_APPROVERS: usize = 32;
const MAX_APPROVAL_SIGNATURES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionApprovalFileEvidence {
    pub source_path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionApprovalPolicy {
    pub schema_version: u16,
    pub policy_id: String,
    pub maximum_request_ttl_ms: u64,
    pub required_roles: Vec<String>,
    pub approvers: Vec<ProductionApprover>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionApprover {
    pub id: String,
    pub role: String,
    pub public_key_base64: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionApprovalPrivateKey {
    format_version: u16,
    algorithm: String,
    pkcs8_base64: String,
    public_key_base64: String,
}

impl Drop for ProductionApprovalPrivateKey {
    fn drop(&mut self) {
        self.algorithm.zeroize();
        self.pkcs8_base64.zeroize();
        self.public_key_base64.zeroize();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionApprovalPublicKey {
    pub format_version: u16,
    pub algorithm: String,
    pub public_key_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionApprovalPolicyVerification {
    pub format_version: u16,
    pub policy: ProductionApprovalFileEvidence,
    pub policy_id: String,
    pub maximum_request_ttl_ms: u64,
    pub required_roles: Vec<String>,
    pub approvers: Vec<ProductionApprover>,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionApprovalRequest {
    pub format_version: u16,
    pub action: String,
    pub request_id: String,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub policy: ProductionApprovalFileEvidence,
    pub policy_id: String,
    pub required_roles: Vec<String>,
    pub evidence_subject: ProductionEvidenceApprovalSubject,
    pub evidence_subject_sha256: String,
    pub production_order_entry_authorized: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionApprovalSignature {
    pub format_version: u16,
    pub algorithm: String,
    pub request_sha256: String,
    pub request_id: String,
    pub policy_sha256: String,
    pub approver_id: String,
    pub role: String,
    pub public_key_base64: String,
    pub signed_at_ms: u64,
    pub signature_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub(crate) enum ProductionApprovalFailure {
    EvidenceBundleRejected,
    EvidenceSubjectMismatch {
        requested_sha256: String,
        current_sha256: String,
    },
    RequestNotYetValid {
        created_at_ms: u64,
        verified_at_ms: u64,
    },
    RequestExpired {
        expires_at_ms: u64,
        verified_at_ms: u64,
    },
    UnknownApprover {
        approver_id: String,
    },
    ApprovalBindingMismatch {
        approver_id: String,
        field: String,
    },
    DuplicateApprover {
        approver_id: String,
    },
    DuplicatePublicKey {
        public_key_base64: String,
    },
    ApprovalTimeInvalid {
        approver_id: String,
        signed_at_ms: u64,
    },
    SignatureEncodingInvalid {
        approver_id: String,
    },
    SignatureInvalid {
        approver_id: String,
    },
    MissingRequiredRole {
        role: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionApprovalSignatureSummary {
    pub source: ProductionApprovalFileEvidence,
    pub approver_id: String,
    pub role: String,
    pub public_key_base64: String,
    pub signed_at_ms: u64,
    pub accepted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionApprovalVerificationReport {
    pub format_version: u16,
    pub verified_at_ms: u64,
    pub verifier_reap_version: String,
    pub policy: ProductionApprovalFileEvidence,
    pub request: ProductionApprovalFileEvidence,
    pub policy_id: String,
    pub request_id: String,
    pub request_created_at_ms: u64,
    pub request_expires_at_ms: u64,
    pub requested_evidence_subject_sha256: String,
    pub current_evidence_subject_sha256: String,
    pub required_roles: Vec<String>,
    pub covered_roles: Vec<String>,
    pub approvals: Vec<ProductionApprovalSignatureSummary>,
    pub failures: Vec<ProductionApprovalFailure>,
    pub limitations: Vec<String>,
    pub approval_gate_passed: bool,
    pub production_order_entry_authorized: bool,
}

#[derive(Serialize)]
struct ApprovalSigningPayload<'a> {
    domain: &'static str,
    format_version: u16,
    algorithm: &'a str,
    request_sha256: &'a str,
    request_id: &'a str,
    policy_sha256: &'a str,
    approver_id: &'a str,
    role: &'a str,
    public_key_base64: &'a str,
    signed_at_ms: u64,
}

struct Loaded<T> {
    evidence: ProductionApprovalFileEvidence,
    value: T,
}

struct SignatureEvaluation {
    covered_roles: BTreeSet<String>,
    summaries: Vec<ProductionApprovalSignatureSummary>,
    failures: Vec<ProductionApprovalFailure>,
}

pub(crate) fn generate_production_approval_key_pair()
-> Result<(ProductionApprovalPrivateKey, ProductionApprovalPublicKey)> {
    let rng = SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng)
        .map_err(|_| anyhow::anyhow!("failed to generate Ed25519 approval key"))?;
    let key_pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
        .map_err(|_| anyhow::anyhow!("generated Ed25519 approval key was rejected"))?;
    let public_key_base64 = URL_SAFE_NO_PAD.encode(key_pair.public_key().as_ref());
    Ok((
        ProductionApprovalPrivateKey {
            format_version: APPROVAL_KEY_FORMAT_VERSION,
            algorithm: APPROVAL_ALGORITHM.to_string(),
            pkcs8_base64: URL_SAFE_NO_PAD.encode(pkcs8.as_ref()),
            public_key_base64: public_key_base64.clone(),
        },
        ProductionApprovalPublicKey {
            format_version: APPROVAL_KEY_FORMAT_VERSION,
            algorithm: APPROVAL_ALGORITHM.to_string(),
            public_key_base64,
        },
    ))
}

pub(crate) fn verify_production_approval_policy_path(
    policy_path: &Path,
) -> Result<ProductionApprovalPolicyVerification> {
    let loaded = load_policy(policy_path)?;
    let final_value = load_policy(&loaded.evidence.source_path)?;
    if final_value.evidence != loaded.evidence || final_value.value != loaded.value {
        bail!("production approval policy changed during verification");
    }
    Ok(ProductionApprovalPolicyVerification {
        format_version: APPROVAL_POLICY_VERIFICATION_FORMAT_VERSION,
        policy: loaded.evidence,
        policy_id: loaded.value.policy_id,
        maximum_request_ttl_ms: loaded.value.maximum_request_ttl_ms,
        required_roles: loaded.value.required_roles,
        approvers: loaded.value.approvers,
        passed: true,
    })
}

pub(crate) fn prepare_production_approval_request(
    manifest_path: &Path,
    policy_path: &Path,
    request_id: &str,
    ttl_ms: u64,
) -> Result<ProductionApprovalRequest> {
    validate_identifier("request_id", request_id)?;
    let policy_start = load_policy(policy_path)?;
    if ttl_ms == 0 || ttl_ms > policy_start.value.maximum_request_ttl_ms {
        bail!(
            "approval request TTL must be within 1..={} milliseconds",
            policy_start.value.maximum_request_ttl_ms
        );
    }
    let evidence = verify_production_evidence_manifest_path(manifest_path)
        .context("production evidence reconstruction failed while preparing approval")?;
    let evidence_subject = ProductionEvidenceApprovalSubject::from_report(&evidence)?;
    let evidence_subject_sha256 = evidence_subject.sha256()?;
    let created_at_ms = unix_time_ms()?;
    let expires_at_ms = created_at_ms
        .checked_add(ttl_ms)
        .context("approval request expiration overflowed")?;
    let request = ProductionApprovalRequest {
        format_version: APPROVAL_REQUEST_FORMAT_VERSION,
        action: APPROVAL_ACTION.to_string(),
        request_id: request_id.to_string(),
        created_at_ms,
        expires_at_ms,
        policy: policy_start.evidence.clone(),
        policy_id: policy_start.value.policy_id.clone(),
        required_roles: policy_start.value.required_roles.clone(),
        evidence_subject,
        evidence_subject_sha256,
        production_order_entry_authorized: false,
    };
    validate_request(&request, &policy_start.value, &policy_start.evidence)?;
    let policy_final = load_policy(&policy_start.evidence.source_path)?;
    if policy_final.evidence != policy_start.evidence || policy_final.value != policy_start.value {
        bail!("production approval policy changed while the request was being prepared");
    }
    Ok(request)
}

pub(crate) fn sign_production_approval_request(
    request_path: &Path,
    policy_path: &Path,
    private_key_path: &Path,
    approver_id: &str,
) -> Result<ProductionApprovalSignature> {
    validate_identifier("approver_id", approver_id)?;
    let policy = load_policy(policy_path)?;
    let request = load_json::<ProductionApprovalRequest>(
        request_path,
        "production approval request",
        MAX_APPROVAL_ARTIFACT_BYTES,
    )?;
    validate_request(&request.value, &policy.value, &policy.evidence)?;
    let private = load_private_key(private_key_path)?;
    let key_pair = private_key_pair(&private.value)?;
    let approver = policy
        .value
        .approvers
        .iter()
        .find(|approver| approver.id == approver_id)
        .with_context(|| format!("approver {approver_id} is not in the approval policy"))?;
    if private.value.public_key_base64 != approver.public_key_base64
        || key_pair.public_key().as_ref() != decode_public_key(&approver.public_key_base64)?
    {
        bail!("approval private key does not match policy approver {approver_id}");
    }
    let signed_at_ms = unix_time_ms()?;
    validate_request_time_for_signing(&request.value, signed_at_ms)?;
    let mut approval = ProductionApprovalSignature {
        format_version: APPROVAL_SIGNATURE_FORMAT_VERSION,
        algorithm: APPROVAL_ALGORITHM.to_string(),
        request_sha256: request.evidence.sha256.clone(),
        request_id: request.value.request_id.clone(),
        policy_sha256: policy.evidence.sha256.clone(),
        approver_id: approver.id.clone(),
        role: approver.role.clone(),
        public_key_base64: approver.public_key_base64.clone(),
        signed_at_ms,
        signature_base64: String::new(),
    };
    let payload = signing_payload_bytes(&approval)?;
    approval.signature_base64 = URL_SAFE_NO_PAD.encode(key_pair.sign(&payload).as_ref());

    let policy_final = load_policy(&policy.evidence.source_path)?;
    let request_final = load_json::<ProductionApprovalRequest>(
        &request.evidence.source_path,
        "production approval request",
        MAX_APPROVAL_ARTIFACT_BYTES,
    )?;
    let private_final = load_private_key(&private.evidence.source_path)?;
    if policy_final.evidence != policy.evidence
        || policy_final.value != policy.value
        || request_final.evidence != request.evidence
        || request_final.value != request.value
        || private_final.evidence != private.evidence
        || private_final.value != private.value
    {
        bail!("approval signing input changed while the signature was being created");
    }
    Ok(approval)
}

pub(crate) fn verify_production_approval_paths(
    manifest_path: &Path,
    policy_path: &Path,
    request_path: &Path,
    approval_paths: &[PathBuf],
) -> Result<ProductionApprovalVerificationReport> {
    if approval_paths.is_empty() || approval_paths.len() > MAX_APPROVAL_SIGNATURES {
        bail!(
            "production approval verification requires 1..={MAX_APPROVAL_SIGNATURES} signature artifacts"
        );
    }
    let policy = load_policy(policy_path)?;
    let request = load_json::<ProductionApprovalRequest>(
        request_path,
        "production approval request",
        MAX_APPROVAL_ARTIFACT_BYTES,
    )?;
    validate_request(&request.value, &policy.value, &policy.evidence)?;
    let mut approvals = Vec::with_capacity(approval_paths.len());
    let mut approval_paths_seen = HashSet::new();
    for path in approval_paths {
        let approval = load_json::<ProductionApprovalSignature>(
            path,
            "production approval signature",
            MAX_APPROVAL_ARTIFACT_BYTES,
        )?;
        if !approval_paths_seen.insert(approval.evidence.source_path.clone()) {
            bail!(
                "duplicate production approval signature path {}",
                approval.evidence.source_path.display()
            );
        }
        approvals.push(approval);
    }

    let current_evidence = verify_production_evidence_manifest_path(manifest_path)
        .context("production evidence reconstruction failed during approval verification")?;
    let verified_at_ms = unix_time_ms()?;
    let mut failures = Vec::new();
    let current_subject = ProductionEvidenceApprovalSubject::from_report(&current_evidence).ok();
    if current_subject.is_none() {
        failures.push(ProductionApprovalFailure::EvidenceBundleRejected);
    }
    let current_evidence_subject_sha256 = current_subject
        .as_ref()
        .map(ProductionEvidenceApprovalSubject::sha256)
        .transpose()?
        .unwrap_or_default();
    if current_subject.as_ref() != Some(&request.value.evidence_subject)
        || current_evidence_subject_sha256 != request.value.evidence_subject_sha256
    {
        failures.push(ProductionApprovalFailure::EvidenceSubjectMismatch {
            requested_sha256: request.value.evidence_subject_sha256.clone(),
            current_sha256: current_evidence_subject_sha256.clone(),
        });
    }
    if request.value.created_at_ms > verified_at_ms {
        failures.push(ProductionApprovalFailure::RequestNotYetValid {
            created_at_ms: request.value.created_at_ms,
            verified_at_ms,
        });
    }
    if verified_at_ms > request.value.expires_at_ms {
        failures.push(ProductionApprovalFailure::RequestExpired {
            expires_at_ms: request.value.expires_at_ms,
            verified_at_ms,
        });
    }

    let signature_evaluation =
        evaluate_approval_signatures(&policy, &request, &approvals, verified_at_ms)?;
    failures.extend(signature_evaluation.failures);
    failures.sort_by_key(|failure| serde_json::to_string(failure).unwrap_or_default());
    failures.dedup();

    let policy_final = load_policy(&policy.evidence.source_path)?;
    let request_final = load_json::<ProductionApprovalRequest>(
        &request.evidence.source_path,
        "production approval request",
        MAX_APPROVAL_ARTIFACT_BYTES,
    )?;
    if policy_final.evidence != policy.evidence
        || policy_final.value != policy.value
        || request_final.evidence != request.evidence
        || request_final.value != request.value
    {
        bail!("production approval policy or request changed during verification");
    }
    for loaded in &approvals {
        let final_value = load_json::<ProductionApprovalSignature>(
            &loaded.evidence.source_path,
            "production approval signature",
            MAX_APPROVAL_ARTIFACT_BYTES,
        )?;
        if final_value.evidence != loaded.evidence || final_value.value != loaded.value {
            bail!(
                "production approval signature {} changed during verification",
                loaded.evidence.source_path.display()
            );
        }
    }

    let approval_gate_passed = failures.is_empty();
    Ok(ProductionApprovalVerificationReport {
        format_version: APPROVAL_VERIFICATION_FORMAT_VERSION,
        verified_at_ms,
        verifier_reap_version: env!("CARGO_PKG_VERSION").to_string(),
        policy: policy.evidence,
        request: request.evidence,
        policy_id: policy.value.policy_id,
        request_id: request.value.request_id,
        request_created_at_ms: request.value.created_at_ms,
        request_expires_at_ms: request.value.expires_at_ms,
        requested_evidence_subject_sha256: request.value.evidence_subject_sha256,
        current_evidence_subject_sha256,
        required_roles: policy.value.required_roles,
        covered_roles: signature_evaluation.covered_roles.into_iter().collect(),
        approvals: signature_evaluation.summaries,
        failures,
        limitations: vec![
            "Ed25519 proves possession of policy-listed private keys; approver identity, role assignment, key custody, and independent human review remain governance controls"
                .to_string(),
            "verification reruns the source-rebuilding production bundle on the declared candidate host, but does not remotely attest clocks, host identity, exchange identity, or supervisor state"
                .to_string(),
            "the verifier does not maintain a rollout replay ledger; deployment control must enforce a unique request ID and one-time use inside the short validity window"
                .to_string(),
            "a passing approval gate is short-lived release evidence and never enables or authorizes production order entry"
                .to_string(),
        ],
        approval_gate_passed,
        production_order_entry_authorized: false,
    })
}

fn evaluate_approval_signatures(
    policy: &Loaded<ProductionApprovalPolicy>,
    request: &Loaded<ProductionApprovalRequest>,
    approvals: &[Loaded<ProductionApprovalSignature>],
    verified_at_ms: u64,
) -> Result<SignatureEvaluation> {
    let mut failures = Vec::new();
    let mut approvers_seen = BTreeSet::new();
    let mut public_keys_seen = BTreeSet::new();
    let mut covered_roles = BTreeSet::new();
    let mut summaries = Vec::with_capacity(approvals.len());
    for loaded in approvals {
        let approval = &loaded.value;
        let mut accepted = true;
        if validate_identifier("approver_id", &approval.approver_id).is_err()
            || validate_identifier("approver role", &approval.role).is_err()
        {
            binding_failure(&mut failures, &approval.approver_id, "identity_format");
            accepted = false;
        }
        if approval.format_version != APPROVAL_SIGNATURE_FORMAT_VERSION
            || approval.algorithm != APPROVAL_ALGORITHM
        {
            binding_failure(&mut failures, &approval.approver_id, "signature_format");
            accepted = false;
        }
        let policy_approver = policy
            .value
            .approvers
            .iter()
            .find(|candidate| candidate.id == approval.approver_id);
        if policy_approver.is_none() {
            failures.push(ProductionApprovalFailure::UnknownApprover {
                approver_id: approval.approver_id.clone(),
            });
            accepted = false;
        }
        for (field, matches) in [
            (
                "request_sha256",
                approval.request_sha256 == request.evidence.sha256,
            ),
            (
                "request_id",
                approval.request_id == request.value.request_id,
            ),
            (
                "policy_sha256",
                approval.policy_sha256 == policy.evidence.sha256,
            ),
        ] {
            if !matches {
                binding_failure(&mut failures, &approval.approver_id, field);
                accepted = false;
            }
        }
        if let Some(expected) = policy_approver {
            for (field, matches) in [
                ("role", approval.role == expected.role),
                (
                    "public_key_base64",
                    approval.public_key_base64 == expected.public_key_base64,
                ),
            ] {
                if !matches {
                    binding_failure(&mut failures, &approval.approver_id, field);
                    accepted = false;
                }
            }
        }
        if !approvers_seen.insert(approval.approver_id.clone()) {
            failures.push(ProductionApprovalFailure::DuplicateApprover {
                approver_id: approval.approver_id.clone(),
            });
            accepted = false;
        }
        if !public_keys_seen.insert(approval.public_key_base64.clone()) {
            failures.push(ProductionApprovalFailure::DuplicatePublicKey {
                public_key_base64: approval.public_key_base64.clone(),
            });
            accepted = false;
        }
        if approval.signed_at_ms
            < request
                .value
                .created_at_ms
                .saturating_sub(MAX_APPROVAL_CLOCK_SKEW_MS)
            || approval.signed_at_ms > request.value.expires_at_ms
            || approval.signed_at_ms > verified_at_ms.saturating_add(MAX_APPROVAL_CLOCK_SKEW_MS)
        {
            failures.push(ProductionApprovalFailure::ApprovalTimeInvalid {
                approver_id: approval.approver_id.clone(),
                signed_at_ms: approval.signed_at_ms,
            });
            accepted = false;
        }
        match verify_approval_signature(approval) {
            Ok(true) => {}
            Ok(false) => {
                failures.push(ProductionApprovalFailure::SignatureInvalid {
                    approver_id: approval.approver_id.clone(),
                });
                accepted = false;
            }
            Err(_) => {
                failures.push(ProductionApprovalFailure::SignatureEncodingInvalid {
                    approver_id: approval.approver_id.clone(),
                });
                accepted = false;
            }
        }
        if accepted {
            covered_roles.insert(approval.role.clone());
        }
        summaries.push(ProductionApprovalSignatureSummary {
            source: loaded.evidence.clone(),
            approver_id: approval.approver_id.clone(),
            role: approval.role.clone(),
            public_key_base64: approval.public_key_base64.clone(),
            signed_at_ms: approval.signed_at_ms,
            accepted,
        });
    }
    for role in &policy.value.required_roles {
        if !covered_roles.contains(role) {
            failures.push(ProductionApprovalFailure::MissingRequiredRole { role: role.clone() });
        }
    }
    failures.sort_by_key(|failure| serde_json::to_string(failure).unwrap_or_default());
    failures.dedup();
    summaries.sort_by(|left, right| left.approver_id.cmp(&right.approver_id));
    Ok(SignatureEvaluation {
        covered_roles,
        summaries,
        failures,
    })
}

fn binding_failure(failures: &mut Vec<ProductionApprovalFailure>, approver_id: &str, field: &str) {
    failures.push(ProductionApprovalFailure::ApprovalBindingMismatch {
        approver_id: approver_id.to_string(),
        field: field.to_string(),
    });
}

fn validate_policy(policy: &ProductionApprovalPolicy) -> Result<()> {
    if policy.schema_version != APPROVAL_POLICY_SCHEMA_VERSION {
        bail!(
            "unsupported production approval policy schema {}; expected {}",
            policy.schema_version,
            APPROVAL_POLICY_SCHEMA_VERSION
        );
    }
    validate_identifier("policy_id", &policy.policy_id)?;
    if policy.maximum_request_ttl_ms == 0 || policy.maximum_request_ttl_ms > MAX_APPROVAL_TTL_MS {
        bail!("maximum_request_ttl_ms must be within 1..={MAX_APPROVAL_TTL_MS}");
    }
    if policy.required_roles.len() < 2 || policy.required_roles.len() > MAX_APPROVAL_ROLES {
        bail!("approval policy must require 2..={MAX_APPROVAL_ROLES} distinct roles");
    }
    validate_sorted_unique_identifiers("required_roles", &policy.required_roles)?;
    if policy.approvers.len() < 2 || policy.approvers.len() > MAX_APPROVERS {
        bail!("approval policy must declare 2..={MAX_APPROVERS} approvers");
    }
    let approver_ids = policy
        .approvers
        .iter()
        .map(|approver| approver.id.clone())
        .collect::<Vec<_>>();
    validate_sorted_unique_identifiers("approvers", &approver_ids)?;
    let required_roles = policy.required_roles.iter().collect::<BTreeSet<_>>();
    let mut covered_roles = BTreeSet::new();
    let mut public_keys = BTreeSet::new();
    for approver in &policy.approvers {
        validate_identifier("approver id", &approver.id)?;
        validate_identifier("approver role", &approver.role)?;
        if !required_roles.contains(&approver.role) {
            bail!(
                "approver {} has role {} which is not required by the policy",
                approver.id,
                approver.role
            );
        }
        decode_public_key(&approver.public_key_base64).with_context(|| {
            format!("approver {} has an invalid Ed25519 public key", approver.id)
        })?;
        if !public_keys.insert(approver.public_key_base64.clone()) {
            bail!("approval policy reuses an Ed25519 public key");
        }
        covered_roles.insert(&approver.role);
    }
    if covered_roles != required_roles {
        bail!("approval policy has no approver for every required role");
    }
    Ok(())
}

fn validate_request(
    request: &ProductionApprovalRequest,
    policy: &ProductionApprovalPolicy,
    policy_evidence: &ProductionApprovalFileEvidence,
) -> Result<()> {
    if request.format_version != APPROVAL_REQUEST_FORMAT_VERSION
        || request.action != APPROVAL_ACTION
    {
        bail!("unsupported production approval request format or action");
    }
    validate_identifier("request_id", &request.request_id)?;
    if request.created_at_ms == 0 || request.expires_at_ms <= request.created_at_ms {
        bail!("production approval request has an invalid time interval");
    }
    let ttl_ms = request.expires_at_ms - request.created_at_ms;
    if ttl_ms > policy.maximum_request_ttl_ms || ttl_ms > MAX_APPROVAL_TTL_MS {
        bail!("production approval request exceeds the policy TTL");
    }
    if request.policy_id != policy.policy_id
        || request.required_roles != policy.required_roles
        || request.policy.bytes != policy_evidence.bytes
        || request.policy.sha256 != policy_evidence.sha256
        || request.evidence_subject.expected.approval_policy_sha256 != policy_evidence.sha256
    {
        bail!("production approval request does not bind the exact approval policy");
    }
    if request.evidence_subject.format_version
        != PRODUCTION_EVIDENCE_APPROVAL_SUBJECT_FORMAT_VERSION
        || request
            .evidence_subject
            .production_evidence_report_format_version
            != PRODUCTION_EVIDENCE_REPORT_FORMAT_VERSION
        || request.evidence_subject.manifest_schema_version
            != PRODUCTION_EVIDENCE_MANIFEST_SCHEMA_VERSION
        || request.evidence_subject.java_reference_revision != PINNED_JAVA_REVISION
        || !request.evidence_subject.evidence_bundle_passed
        || request.evidence_subject.production_order_entry_authorized
        || request.production_order_entry_authorized
        || request.evidence_subject.freshness_observations.is_empty()
        || request
            .evidence_subject
            .freshness_observations
            .iter()
            .any(|observation| !observation.passed)
        || request.evidence_subject.gates.is_empty()
        || request
            .evidence_subject
            .gates
            .iter()
            .any(|gate| !gate.acceptance_passed)
    {
        bail!("production approval request does not contain a passing current evidence subject");
    }
    let subject_sha256 = request.evidence_subject.sha256()?;
    if !is_lower_sha256(&request.evidence_subject_sha256)
        || subject_sha256 != request.evidence_subject_sha256
    {
        bail!("production approval request evidence subject hash is invalid");
    }
    let subject = &request.evidence_subject;
    if subject.expected.reap_version != subject.verifier.reap_version
        || subject.expected.live_executable_sha256 != subject.verifier.executable_sha256
        || subject.expected.host_identity_sha256 != subject.verifier.host_identity_sha256
        || subject.expected.reap_version != subject.observed_demo_identity.reap_version
        || subject.expected.live_executable_sha256
            != subject.observed_demo_identity.executable_sha256
        || subject.expected.host_identity_sha256
            != subject.observed_demo_identity.host_identity_sha256
        || Some(subject.expected.deployment_candidate_id.as_str())
            != subject.observed_deployment_candidate_id.as_deref()
        || subject.expected.demo_account_identity_sha256s
            != subject.observed_demo_identity.account_identity_sha256s
        || subject.expected.production_account_identity_sha256s
            != subject.observed_production_account_identity_sha256s
    {
        bail!("production approval request contains inconsistent release identities");
    }
    if subject.demo_config.environment != reap_live::TradingEnvironment::Demo
        || subject.production_config.environment != reap_live::TradingEnvironment::Production
        || subject.fault_demo_config.environment != reap_live::TradingEnvironment::Demo
        || subject.demo_config.account_ids != subject.fault_demo_config.account_ids
        || subject.demo_config.account_ids != subject.production_config.account_ids
        || subject.demo_config.account_ids.is_empty()
    {
        bail!("production approval request contains inconsistent config roles or accounts");
    }
    let expected_scenarios = reap_live::LiveFaultScenario::REQUIRED
        .into_iter()
        .collect::<BTreeSet<_>>();
    let observed_scenarios = subject
        .fault_proxy_runs
        .iter()
        .map(|run| run.scenario)
        .collect::<BTreeSet<_>>();
    if subject.fault_proxy_runs.len() != expected_scenarios.len()
        || observed_scenarios != expected_scenarios
        || subject
            .fault_proxy_runs
            .iter()
            .any(|run| !run.acceptance_passed)
    {
        bail!("production approval request does not cover every clean fault-proxy run");
    }
    let observed_gates = subject
        .gates
        .iter()
        .map(|gate| gate.gate)
        .collect::<BTreeSet<_>>();
    for gate in [
        crate::production_evidence::ProductionEvidenceGate::Freshness,
        crate::production_evidence::ProductionEvidenceGate::FaultProxyRun,
        crate::production_evidence::ProductionEvidenceGate::ProductionTransition,
        crate::production_evidence::ProductionEvidenceGate::ResearchDeployment,
        crate::production_evidence::ProductionEvidenceGate::DemoSoak,
        crate::production_evidence::ProductionEvidenceGate::FaultConfiguration,
        crate::production_evidence::ProductionEvidenceGate::FaultMatrix,
        crate::production_evidence::ProductionEvidenceGate::LatencyCalibration,
        crate::production_evidence::ProductionEvidenceGate::AccountCertification,
        crate::production_evidence::ProductionEvidenceGate::DeadmanCertification,
        crate::production_evidence::ProductionEvidenceGate::EmergencyCancel,
        crate::production_evidence::ProductionEvidenceGate::FillReconciliation,
    ] {
        if !observed_gates.contains(&gate) {
            bail!("production approval request is missing required evidence gate {gate:?}");
        }
    }
    for value in [
        subject.manifest.sha256.as_str(),
        subject.expected.live_executable_sha256.as_str(),
        subject.expected.host_identity_sha256.as_str(),
        subject.expected.approval_policy_sha256.as_str(),
        subject.demo_config.file.sha256.as_str(),
        subject.production_config.file.sha256.as_str(),
        subject.fault_demo_config.file.sha256.as_str(),
        subject.demo_config.config_fingerprint.as_str(),
        subject.production_config.config_fingerprint.as_str(),
        subject.fault_demo_config.config_fingerprint.as_str(),
    ] {
        if !is_lower_sha256(value) {
            bail!("production approval request contains an invalid SHA-256 identity");
        }
    }
    Ok(())
}

fn validate_request_time_for_signing(
    request: &ProductionApprovalRequest,
    signed_at_ms: u64,
) -> Result<()> {
    if request.created_at_ms > signed_at_ms.saturating_add(MAX_APPROVAL_CLOCK_SKEW_MS) {
        bail!("production approval request is too far in the signer clock's future");
    }
    if signed_at_ms > request.expires_at_ms {
        bail!("production approval request has expired");
    }
    Ok(())
}

fn signing_payload_bytes(approval: &ProductionApprovalSignature) -> Result<Vec<u8>> {
    serde_json::to_vec(&ApprovalSigningPayload {
        domain: APPROVAL_SIGNATURE_DOMAIN,
        format_version: approval.format_version,
        algorithm: &approval.algorithm,
        request_sha256: &approval.request_sha256,
        request_id: &approval.request_id,
        policy_sha256: &approval.policy_sha256,
        approver_id: &approval.approver_id,
        role: &approval.role,
        public_key_base64: &approval.public_key_base64,
        signed_at_ms: approval.signed_at_ms,
    })
    .context("failed to serialize production approval signing payload")
}

fn verify_approval_signature(approval: &ProductionApprovalSignature) -> Result<bool> {
    let signature = URL_SAFE_NO_PAD
        .decode(approval.signature_base64.as_bytes())
        .context("approval signature is not valid base64url")?;
    if signature.len() != 64 || URL_SAFE_NO_PAD.encode(&signature) != approval.signature_base64 {
        bail!("approval signature must be canonical 64-byte base64url");
    }
    let public_key = decode_public_key(&approval.public_key_base64)?;
    let payload = signing_payload_bytes(approval)?;
    Ok(UnparsedPublicKey::new(&ED25519, public_key)
        .verify(&payload, &signature)
        .is_ok())
}

fn load_policy(path: &Path) -> Result<Loaded<ProductionApprovalPolicy>> {
    let (source_path, bytes) = read_regular_file(
        path,
        "production approval policy",
        MAX_APPROVAL_POLICY_BYTES,
    )?;
    let text = std::str::from_utf8(&bytes).context("production approval policy is not UTF-8")?;
    let value: ProductionApprovalPolicy =
        toml::from_str(text).context("failed to parse strict production approval policy")?;
    validate_policy(&value)?;
    Ok(Loaded {
        evidence: file_evidence(source_path, &bytes),
        value,
    })
}

fn load_json<T: DeserializeOwned>(
    path: &Path,
    label: &'static str,
    maximum_bytes: u64,
) -> Result<Loaded<T>> {
    let (source_path, bytes) = read_regular_file(path, label, maximum_bytes)?;
    let value = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse strict {label} JSON"))?;
    Ok(Loaded {
        evidence: file_evidence(source_path, &bytes),
        value,
    })
}

fn load_private_key(path: &Path) -> Result<Loaded<ProductionApprovalPrivateKey>> {
    let loaded = load_json::<ProductionApprovalPrivateKey>(
        path,
        "production approval private key",
        MAX_APPROVAL_PRIVATE_KEY_BYTES,
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = std::fs::metadata(&loaded.evidence.source_path)?
            .permissions()
            .mode()
            & 0o777;
        if mode & 0o077 != 0 {
            bail!(
                "production approval private key {} has insecure mode {mode:o}",
                loaded.evidence.source_path.display()
            );
        }
    }
    private_key_pair(&loaded.value)?;
    Ok(loaded)
}

fn private_key_pair(key: &ProductionApprovalPrivateKey) -> Result<Ed25519KeyPair> {
    if key.format_version != APPROVAL_KEY_FORMAT_VERSION || key.algorithm != APPROVAL_ALGORITHM {
        bail!("unsupported production approval private key format");
    }
    let pkcs8 = Zeroizing::new(
        URL_SAFE_NO_PAD
            .decode(key.pkcs8_base64.as_bytes())
            .context("production approval private key is not valid base64url")?,
    );
    if URL_SAFE_NO_PAD.encode(&pkcs8) != key.pkcs8_base64 {
        bail!("production approval private key is not canonical base64url");
    }
    let key_pair = Ed25519KeyPair::from_pkcs8(&pkcs8)
        .map_err(|_| anyhow::anyhow!("production approval private key PKCS#8 is invalid"))?;
    if URL_SAFE_NO_PAD.encode(key_pair.public_key().as_ref()) != key.public_key_base64 {
        bail!("production approval private key public key binding is invalid");
    }
    Ok(key_pair)
}

fn decode_public_key(value: &str) -> Result<Vec<u8>> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value.as_bytes())
        .context("Ed25519 public key is not valid base64url")?;
    if bytes.len() != 32
        || bytes.iter().all(|byte| *byte == 0)
        || URL_SAFE_NO_PAD.encode(&bytes) != value
    {
        bail!("Ed25519 public key must contain 32 bytes and must not be all zero");
    }
    Ok(bytes)
}

fn read_regular_file(path: &Path, label: &'static str, maximum: u64) -> Result<(PathBuf, Vec<u8>)> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("invalid {label} path {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "{label} {} must be a regular file and not a symbolic link",
            path.display()
        );
    }
    if metadata.len() > maximum {
        bail!(
            "{label} {} exceeds the {maximum}-byte limit",
            path.display()
        );
    }
    let source_path = std::fs::canonicalize(path)
        .with_context(|| format!("failed to canonicalize {label} {}", path.display()))?;
    let bytes = std::fs::read(&source_path)
        .with_context(|| format!("failed to read {label} {}", source_path.display()))?;
    if bytes.len() as u64 > maximum {
        bail!(
            "{label} {} exceeds the {maximum}-byte limit after reading",
            source_path.display()
        );
    }
    Ok((source_path, bytes))
}

fn file_evidence(path: PathBuf, bytes: &[u8]) -> ProductionApprovalFileEvidence {
    ProductionApprovalFileEvidence {
        source_path: path,
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
    }
}

fn validate_sorted_unique_identifiers(label: &str, values: &[String]) -> Result<()> {
    for value in values {
        validate_identifier(label, value)?;
    }
    if values.windows(2).any(|window| window[0] >= window[1]) {
        bail!("{label} must be strictly sorted and unique");
    }
    Ok(())
}

fn validate_identifier(label: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        bail!("{label} must contain 1-128 safe ASCII characters");
    }
    Ok(())
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn unix_time_ms() -> Result<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_millis();
    millis
        .try_into()
        .context("current Unix time does not fit in milliseconds")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs::OpenOptions;
    use std::io::Write;

    use reap_fault::FaultProxyRunFileEvidence;
    use reap_live::{LiveConfigFileEvidence, LiveFaultScenario, TradingEnvironment};

    use super::*;
    use crate::production_evidence::{
        ProductionEvidenceApprovalFreshnessObservation, ProductionEvidenceApprovalGate,
        ProductionEvidenceConfigEvidence, ProductionEvidenceExpectedIdentity,
        ProductionEvidenceFaultProxyRunSummary, ProductionEvidenceFreshnessPolicy,
        ProductionEvidenceGate, ProductionEvidenceLiveIdentity, ProductionEvidenceVerifierIdentity,
    };

    fn sha256(byte: char) -> String {
        byte.to_string().repeat(64)
    }

    fn config_evidence(
        name: &str,
        environment: TradingEnvironment,
        byte: char,
    ) -> ProductionEvidenceConfigEvidence {
        ProductionEvidenceConfigEvidence {
            file: LiveConfigFileEvidence {
                source_path: PathBuf::from(format!("/{name}.toml")),
                bytes: 100,
                sha256: sha256(byte),
            },
            config_fingerprint: sha256(byte),
            evidence_config_fingerprint: sha256(byte),
            environment,
            account_ids: vec!["main".to_string()],
        }
    }

    fn evidence_subject() -> ProductionEvidenceApprovalSubject {
        let demo_accounts = BTreeMap::from([("main".to_string(), sha256('4'))]);
        let production_accounts = BTreeMap::from([("main".to_string(), sha256('5'))]);
        let fault_proxy_runs = LiveFaultScenario::REQUIRED
            .into_iter()
            .enumerate()
            .map(|(index, scenario)| ProductionEvidenceFaultProxyRunSummary {
                scenario,
                run_report: FaultProxyRunFileEvidence {
                    source_path: PathBuf::from(format!("/proxy-{index}.json")),
                    bytes: 100,
                    sha256: sha256('6'),
                },
                proxy_session_id: format!("proxy-{index}"),
                started_at_ms: 1_000 + index as u64 * 100,
                stopped_at_ms: 1_050 + index as u64 * 100,
                completed_faults: u64::from(!matches!(
                    scenario,
                    LiveFaultScenario::CleanObserve
                        | LiveFaultScenario::CleanDemo
                        | LiveFaultScenario::PartialFill
                        | LiveFaultScenario::RestoredSafetyLatch
                )),
                acceptance_passed: true,
            })
            .collect();
        let gates = [
            ProductionEvidenceGate::Freshness,
            ProductionEvidenceGate::FaultProxyRun,
            ProductionEvidenceGate::ProductionTransition,
            ProductionEvidenceGate::ResearchDeployment,
            ProductionEvidenceGate::DemoSoak,
            ProductionEvidenceGate::FaultConfiguration,
            ProductionEvidenceGate::FaultMatrix,
            ProductionEvidenceGate::LatencyCalibration,
            ProductionEvidenceGate::AccountCertification,
            ProductionEvidenceGate::DeadmanCertification,
            ProductionEvidenceGate::EmergencyCancel,
            ProductionEvidenceGate::FillReconciliation,
        ]
        .into_iter()
        .map(|gate| ProductionEvidenceApprovalGate {
            gate,
            subject: None,
            source_paths: vec![PathBuf::from("/evidence.json")],
            reconstructed_sha256: sha256('7'),
            acceptance_passed: true,
        })
        .collect();
        ProductionEvidenceApprovalSubject {
            format_version: PRODUCTION_EVIDENCE_APPROVAL_SUBJECT_FORMAT_VERSION,
            production_evidence_report_format_version: PRODUCTION_EVIDENCE_REPORT_FORMAT_VERSION,
            manifest_schema_version: PRODUCTION_EVIDENCE_MANIFEST_SCHEMA_VERSION,
            java_reference_revision: PINNED_JAVA_REVISION.to_string(),
            manifest: crate::production_evidence::ProductionEvidenceFileEvidence {
                source_path: PathBuf::from("/production-evidence.toml"),
                bytes: 100,
                sha256: sha256('8'),
            },
            expected: ProductionEvidenceExpectedIdentity {
                reap_version: "0.1.0".to_string(),
                live_executable_sha256: sha256('1'),
                host_identity_sha256: sha256('2'),
                approval_policy_sha256: sha256('9'),
                deployment_candidate_id: "candidate-a".to_string(),
                demo_account_identity_sha256s: demo_accounts.clone(),
                production_account_identity_sha256s: production_accounts.clone(),
            },
            freshness_policy: ProductionEvidenceFreshnessPolicy {
                future_tolerance_ms: 1,
                demo_soak_max_age_ms: 1,
                fault_run_max_age_ms: 1,
                latency_source_max_age_ms: 1,
                production_account_certification_max_age_ms: 1,
                deadman_certification_max_age_ms: 1,
                emergency_cancel_max_age_ms: 1,
                fill_collection_max_age_ms: 1,
            },
            freshness_observations: vec![ProductionEvidenceApprovalFreshnessObservation {
                gate: ProductionEvidenceGate::DemoSoak,
                subject: None,
                source_path: PathBuf::from("/soak.json"),
                started_at_ms: 1,
                completed_at_ms: 2,
                maximum_age_ms: 1,
                passed: true,
            }],
            fault_proxy_runs,
            verifier: ProductionEvidenceVerifierIdentity {
                reap_version: "0.1.0".to_string(),
                executable_sha256: sha256('1'),
                host_identity_sha256: sha256('2'),
            },
            demo_config: config_evidence("demo", TradingEnvironment::Demo, 'a'),
            production_config: config_evidence("production", TradingEnvironment::Production, 'b'),
            fault_demo_config: config_evidence("fault", TradingEnvironment::Demo, 'c'),
            observed_demo_identity: ProductionEvidenceLiveIdentity {
                reap_version: "0.1.0".to_string(),
                executable_sha256: sha256('1'),
                host_identity_sha256: sha256('2'),
                account_identity_sha256s: demo_accounts,
            },
            observed_production_account_identity_sha256s: production_accounts,
            observed_deployment_candidate_id: Some("candidate-a".to_string()),
            gates,
            limitations: vec!["test limitation".to_string()],
            evidence_bundle_passed: true,
            production_order_entry_authorized: false,
        }
    }

    fn policy(
        operations: &ProductionApprovalPublicKey,
        risk: &ProductionApprovalPublicKey,
    ) -> ProductionApprovalPolicy {
        ProductionApprovalPolicy {
            schema_version: APPROVAL_POLICY_SCHEMA_VERSION,
            policy_id: "production-v1".to_string(),
            maximum_request_ttl_ms: 600_000,
            required_roles: vec!["operations".to_string(), "risk".to_string()],
            approvers: vec![
                ProductionApprover {
                    id: "operations-approver".to_string(),
                    role: "operations".to_string(),
                    public_key_base64: operations.public_key_base64.clone(),
                },
                ProductionApprover {
                    id: "risk-approver".to_string(),
                    role: "risk".to_string(),
                    public_key_base64: risk.public_key_base64.clone(),
                },
            ],
        }
    }

    fn write_private(path: &Path, key: &ProductionApprovalPrivateKey) {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path).unwrap();
        serde_json::to_writer(&mut file, key).unwrap();
        file.write_all(b"\n").unwrap();
    }

    fn loaded_approval(
        name: &str,
        value: ProductionApprovalSignature,
    ) -> Loaded<ProductionApprovalSignature> {
        let bytes = serde_json::to_vec(&value).unwrap();
        Loaded {
            evidence: file_evidence(PathBuf::from(format!("/{name}.json")), &bytes),
            value,
        }
    }

    #[test]
    fn policy_requires_sorted_distinct_roles_approvers_and_keys() {
        let (_, operations) = generate_production_approval_key_pair().unwrap();
        let (_, risk) = generate_production_approval_key_pair().unwrap();
        let valid = policy(&operations, &risk);
        validate_policy(&valid).unwrap();

        let mut one_role = valid.clone();
        one_role.required_roles = vec!["operations".to_string()];
        one_role.approvers.truncate(1);
        assert!(validate_policy(&one_role).is_err());

        let mut duplicate_key = valid.clone();
        duplicate_key.approvers[1].public_key_base64 =
            duplicate_key.approvers[0].public_key_base64.clone();
        assert!(validate_policy(&duplicate_key).is_err());

        let mut unsorted = valid;
        unsorted.required_roles.reverse();
        assert!(validate_policy(&unsorted).is_err());

        let template: ProductionApprovalPolicy = toml::from_str(include_str!(
            "../../../examples/production-approval-policy.toml"
        ))
        .unwrap();
        assert!(validate_policy(&template).is_err());
    }

    #[test]
    fn approval_subject_hash_binds_release_evidence() {
        let subject = evidence_subject();
        let original = subject.sha256().unwrap();
        let mut changed = subject;
        changed.production_config.file.sha256 = sha256('d');
        assert_ne!(original, changed.sha256().unwrap());
    }

    #[test]
    fn offline_signature_binds_request_policy_role_and_time() {
        let directory = tempfile::tempdir().unwrap();
        let (operations_private, operations_public) =
            generate_production_approval_key_pair().unwrap();
        let (risk_private, risk_public) = generate_production_approval_key_pair().unwrap();
        let policy_value = policy(&operations_public, &risk_public);
        let policy_path = directory.path().join("policy.toml");
        std::fs::write(&policy_path, toml::to_string(&policy_value).unwrap()).unwrap();
        let loaded_policy = load_policy(&policy_path).unwrap();
        let policy_verification = verify_production_approval_policy_path(&policy_path).unwrap();
        assert!(policy_verification.passed);
        assert_eq!(
            policy_verification.policy.sha256,
            loaded_policy.evidence.sha256
        );
        let now_ms = unix_time_ms().unwrap();
        let mut subject = evidence_subject();
        subject.expected.approval_policy_sha256 = loaded_policy.evidence.sha256.clone();
        let request_value = ProductionApprovalRequest {
            format_version: APPROVAL_REQUEST_FORMAT_VERSION,
            action: APPROVAL_ACTION.to_string(),
            request_id: "release-1".to_string(),
            created_at_ms: now_ms,
            expires_at_ms: now_ms + 60_000,
            policy: loaded_policy.evidence.clone(),
            policy_id: policy_value.policy_id.clone(),
            required_roles: policy_value.required_roles.clone(),
            evidence_subject_sha256: subject.sha256().unwrap(),
            evidence_subject: subject,
            production_order_entry_authorized: false,
        };
        validate_request(&request_value, &policy_value, &loaded_policy.evidence).unwrap();
        let mut substituted_policy = loaded_policy.evidence.clone();
        substituted_policy.sha256 = sha256('e');
        assert!(validate_request(&request_value, &policy_value, &substituted_policy).is_err());
        let request_path = directory.path().join("request.json");
        std::fs::write(&request_path, serde_json::to_vec(&request_value).unwrap()).unwrap();
        let loaded_request = load_json::<ProductionApprovalRequest>(
            &request_path,
            "production approval request",
            MAX_APPROVAL_ARTIFACT_BYTES,
        )
        .unwrap();
        let private_path = directory.path().join("operations-private.json");
        write_private(&private_path, &operations_private);
        let risk_private_path = directory.path().join("risk-private.json");
        write_private(&risk_private_path, &risk_private);

        let approval = sign_production_approval_request(
            &request_path,
            &policy_path,
            &private_path,
            "operations-approver",
        )
        .unwrap();
        assert!(verify_approval_signature(&approval).unwrap());
        let risk_approval = sign_production_approval_request(
            &request_path,
            &policy_path,
            &risk_private_path,
            "risk-approver",
        )
        .unwrap();
        let only_operations = evaluate_approval_signatures(
            &loaded_policy,
            &loaded_request,
            &[loaded_approval("operations", approval.clone())],
            now_ms + 1,
        )
        .unwrap();
        assert!(only_operations.failures.iter().any(|failure| matches!(
            failure,
            ProductionApprovalFailure::MissingRequiredRole { role } if role == "risk"
        )));
        let quorum = evaluate_approval_signatures(
            &loaded_policy,
            &loaded_request,
            &[
                loaded_approval("operations", approval.clone()),
                loaded_approval("risk", risk_approval),
            ],
            now_ms + 1,
        )
        .unwrap();
        assert!(quorum.failures.is_empty(), "{:#?}", quorum.failures);
        assert_eq!(
            quorum.covered_roles,
            BTreeSet::from(["operations".to_string(), "risk".to_string()])
        );

        let duplicate = evaluate_approval_signatures(
            &loaded_policy,
            &loaded_request,
            &[
                loaded_approval("operations-one", approval.clone()),
                loaded_approval("operations-two", approval.clone()),
            ],
            now_ms + 1,
        )
        .unwrap();
        assert!(duplicate.failures.iter().any(|failure| matches!(
            failure,
            ProductionApprovalFailure::DuplicateApprover { approver_id }
                if approver_id == "operations-approver"
        )));
        assert!(duplicate.failures.iter().any(|failure| matches!(
            failure,
            ProductionApprovalFailure::DuplicatePublicKey { .. }
        )));

        let mut malformed = approval.clone();
        malformed.signature_base64 = URL_SAFE_NO_PAD.encode([0_u8; 63]);
        assert!(verify_approval_signature(&malformed).is_err());

        let mut tampered = approval;
        tampered.role = "risk".to_string();
        assert!(!verify_approval_signature(&tampered).unwrap());
        assert!(
            sign_production_approval_request(
                &request_path,
                &policy_path,
                &private_path,
                "risk-approver"
            )
            .is_err()
        );

        let mut expired_request = request_value.clone();
        expired_request.created_at_ms = now_ms.saturating_sub(2_000);
        expired_request.expires_at_ms = now_ms.saturating_sub(1);
        let expired_path = directory.path().join("expired.json");
        std::fs::write(&expired_path, serde_json::to_vec(&expired_request).unwrap()).unwrap();
        assert!(
            sign_production_approval_request(
                &expired_path,
                &policy_path,
                &private_path,
                "operations-approver"
            )
            .is_err()
        );

        let mut unknown = serde_json::to_value(request_value).unwrap();
        unknown["unexpected"] = serde_json::json!(true);
        let unknown_path = directory.path().join("unknown.json");
        std::fs::write(&unknown_path, serde_json::to_vec(&unknown).unwrap()).unwrap();
        assert!(
            load_json::<ProductionApprovalRequest>(
                &unknown_path,
                "production approval request",
                MAX_APPROVAL_ARTIFACT_BYTES
            )
            .is_err()
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};

            let insecure_path = directory.path().join("insecure-private.json");
            std::fs::write(
                &insecure_path,
                serde_json::to_vec(&operations_private).unwrap(),
            )
            .unwrap();
            std::fs::set_permissions(&insecure_path, std::fs::Permissions::from_mode(0o644))
                .unwrap();
            assert!(load_private_key(&insecure_path).is_err());

            let linked_policy = directory.path().join("linked-policy.toml");
            symlink(&policy_path, &linked_policy).unwrap();
            assert!(load_policy(&linked_policy).is_err());
        }
    }
}
