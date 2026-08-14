//! Durable Phase-A attempt-lineage V4 and read-only crash classification.
//!
//! This additive scaffold records an exact V4 `Prepared`, create-new claim,
//! `Consumed`, and final-conjunct lineage around the unchanged V2 -> V1 -> A3
//! transition. Every public projection is denied evidence. The external proof
//! commitment bundle, writers, and owners are crate-private; the bundle has no
//! production constructor. Hash commitments attest no proof validity. A future,
//! separately versioned positive successor must wrap and consume complete V4
//! evidence before any network owner can exist.
//!
//! The read-only inspector's directory lease is cooperative and assumes trusted
//! local storage. It does not protect against a same-EUID process bypassing the
//! advisory lock or against coordinated rollback of the whole artifact set.
//! Strong rollback resistance requires a TPM-backed monotonic anchor, WORM
//! storage, or an authenticated remote registry in a future reviewed design.

#![allow(
    dead_code,
    reason = "the V4 writer is intentionally unreachable until a separately versioned successor exists"
)]

use std::{fmt, path::Path};

use reap_pm_controlled_trial::{
    AuthorizationRuntimeBinding, CanonicalAuthorization,
    CanonicalReviewedPhaseAEligibilityEnvelopeV4, CanonicalTrialConfig, OfflineAuthorizationState,
    OnlineAuthorizationRuntimeBindingV2,
};
use serde::{Deserialize, Serialize};

use crate::{
    PmControlledTrialLiveJournals, PmPendingPhaseAOnlinePreflightBasisV2,
    PmPhaseAOnlinePreflightDispatchOwnerV2, PmPhaseAOnlinePreflightV2Error,
    PmTrialLiveJournalError,
    hash::{ZERO_FINGERPRINT, canonical_json, hash_domain, validate_fingerprint},
    online_preflight_v2::PmOnlinePreflightV2BoundCreateRefresh,
    protected::{ProtectedArtifactLease, ProtectedJournal, read_protected},
};

pub const PM_PHASE_A_ATTEMPT_LINEAGE_LEDGER_FILE_V4: &str =
    "pm-t2-phase-a-attempt-lineage-v4.jsonl";
pub const PM_PHASE_A_ATTEMPT_BURN_CLAIM_FILE_V4: &str = "pm-t2-phase-a-attempt-burn-claim-v4.json";

const PHASE_A_ATTEMPT_LINEAGE_SCHEMA_VERSION_V4: u32 = 4;
const PHASE_A_ATTEMPT_LINEAGE_FAMILY_V4: &str = "pm-t2-phase-a-attempt-lineage";
const PHASE_A_ATTEMPT_BURN_CLAIM_FAMILY_V4: &str = "pm-t2-phase-a-attempt-burn-claim";
const MAX_PHASE_A_ATTEMPT_LINEAGE_LEDGER_BYTES_V4: usize = 192 * 1024;
const MAX_PHASE_A_ATTEMPT_LINEAGE_LINE_BYTES_V4: usize = 64 * 1024;
const MAX_PHASE_A_ATTEMPT_BURN_CLAIM_BYTES_V4: usize = 64 * 1024;
const PHASE_A_ATTEMPT_LINEAGE_BINDING_FINGERPRINT_DOMAIN_V4: &[u8] =
    b"reap.pm-t2.phase-a.attempt-lineage.binding.v4\0";
const PHASE_A_ATTEMPT_LINEAGE_PREPARED_FINGERPRINT_DOMAIN_V4: &[u8] =
    b"reap.pm-t2.phase-a.attempt-lineage.prepared.v4\0";
const PHASE_A_ATTEMPT_BURN_CLAIM_FINGERPRINT_DOMAIN_V4: &[u8] =
    b"reap.pm-t2.phase-a.attempt-lineage.burn-claim.v4\0";
const PHASE_A_ATTEMPT_LINEAGE_CONSUMED_FINGERPRINT_DOMAIN_V4: &[u8] =
    b"reap.pm-t2.phase-a.attempt-lineage.consumed.v4\0";
const PHASE_A_ATTEMPT_LINEAGE_FINAL_CONJUNCT_FINGERPRINT_DOMAIN_V4: &[u8] =
    b"reap.pm-t2.phase-a.attempt-lineage.final-conjunct.v4\0";
const PHASE_A_ATTEMPT_LINEAGE_EXTERNAL_PROOF_COMMITMENTS_FINGERPRINT_DOMAIN_V4: &[u8] =
    b"reap.pm-t2.phase-a.attempt-lineage.external-proof-commitments.v4\0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PhaseAAttemptLineagePinsV4 {
    canonical_sha256: String,
    canonical_length: u64,
    fingerprint: String,
}

impl PhaseAAttemptLineagePinsV4 {
    fn validate(&self) -> Result<(), PmTrialLiveJournalError> {
        validate_fingerprint(&self.canonical_sha256)?;
        validate_fingerprint(&self.fingerprint)?;
        if self.canonical_length == 0 {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PhaseAExternalProofCommitmentsV4 {
    reviewer_trust_anchor_commitment: String,
    credential_provider_trust_root_commitment: String,
    credential_delivery_lease_commitment: String,
    authoritative_remote_acceptance_commitment: String,
    signer_proxy_control_commitment: String,
    same_attempt_audience_commitment: String,
    selected_actor_generation_commitment: String,
    runtime_attempt_commitment: String,
}

impl PhaseAExternalProofCommitmentsV4 {
    fn validate(&self) -> Result<(), PmTrialLiveJournalError> {
        for fingerprint in [
            &self.reviewer_trust_anchor_commitment,
            &self.credential_provider_trust_root_commitment,
            &self.credential_delivery_lease_commitment,
            &self.authoritative_remote_acceptance_commitment,
            &self.signer_proxy_control_commitment,
            &self.same_attempt_audience_commitment,
            &self.selected_actor_generation_commitment,
            &self.runtime_attempt_commitment,
        ] {
            validate_fingerprint(fingerprint)?;
            if fingerprint == ZERO_FINGERPRINT {
                return Err(PmTrialLiveJournalError::InvalidBinding);
            }
        }
        Ok(())
    }
}

/// Opaque external-proof commitment custody. It is deliberately neither public nor
/// serializable, has no production constructor, cannot be cloned, and attests
/// no proof validity.
pub(crate) struct PmPhaseAExternalProofCommitmentBundleV4 {
    pins: PhaseAExternalProofCommitmentsV4,
}

impl fmt::Debug for PmPhaseAExternalProofCommitmentBundleV4 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "PmPhaseAExternalProofCommitmentBundleV4(<opaque; no-production-constructor>)",
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PhaseAAttemptPreparedBindingV4 {
    canonical_config: PhaseAAttemptLineagePinsV4,
    trial_plan_fingerprint: String,
    reviewed_phase_a_eligibility_envelope_v4: PhaseAAttemptLineagePinsV4,
    online_policy_v2: PhaseAAttemptLineagePinsV4,
    online_authorization_v2: PhaseAAttemptLineagePinsV4,
    online_preflight_basis_record_fingerprint: String,
    place_prepared_record_fingerprint: String,
    public_semantic_request_commitment: String,
    prepared_request_commitment: String,
    expected_order_id: String,
    l2_timestamp_seconds: u64,
    external_proof_commitments_fingerprint: String,
    external_proof_commitments: PhaseAExternalProofCommitmentsV4,
}

impl PhaseAAttemptPreparedBindingV4 {
    fn validate(&self) -> Result<(), PmTrialLiveJournalError> {
        self.canonical_config.validate()?;
        self.reviewed_phase_a_eligibility_envelope_v4.validate()?;
        self.online_policy_v2.validate()?;
        self.online_authorization_v2.validate()?;
        self.external_proof_commitments.validate()?;
        for fingerprint in [
            &self.trial_plan_fingerprint,
            &self.online_preflight_basis_record_fingerprint,
            &self.place_prepared_record_fingerprint,
            &self.public_semantic_request_commitment,
            &self.prepared_request_commitment,
            &self.expected_order_id,
            &self.external_proof_commitments_fingerprint,
        ] {
            validate_fingerprint(fingerprint)?;
        }
        if self.l2_timestamp_seconds == 0
            || hash_domain(
                PHASE_A_ATTEMPT_LINEAGE_EXTERNAL_PROOF_COMMITMENTS_FINGERPRINT_DOMAIN_V4,
                &self.external_proof_commitments,
            )? != self.external_proof_commitments_fingerprint
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case", deny_unknown_fields)]
enum PhaseAAttemptLineageLedgerBodyV4 {
    Prepared {
        binding_fingerprint: String,
        binding: Box<PhaseAAttemptPreparedBindingV4>,
    },
    Consumed {
        binding_fingerprint: String,
        prepared_record_fingerprint: String,
        claim_fingerprint: String,
    },
    FinalConjunct {
        binding_fingerprint: String,
        prepared_record_fingerprint: String,
        claim_fingerprint: String,
        consumed_record_fingerprint: String,
        online_preflight_basis_record_fingerprint: String,
        online_preflight_a3_conjunct_record_fingerprint: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PhaseAAttemptLineageLedgerLineV4 {
    schema_version: u32,
    family: String,
    sequence: u8,
    previous_record_fingerprint: String,
    #[serde(flatten)]
    body: PhaseAAttemptLineageLedgerBodyV4,
    authorization: OfflineAuthorizationState,
    record_fingerprint: String,
}

impl PhaseAAttemptLineageLedgerLineV4 {
    fn seal(
        sequence: u8,
        previous_record_fingerprint: String,
        body: PhaseAAttemptLineageLedgerBodyV4,
    ) -> Result<Self, PmTrialLiveJournalError> {
        let mut line = Self {
            schema_version: PHASE_A_ATTEMPT_LINEAGE_SCHEMA_VERSION_V4,
            family: PHASE_A_ATTEMPT_LINEAGE_FAMILY_V4.to_owned(),
            sequence,
            previous_record_fingerprint,
            body,
            authorization: OfflineAuthorizationState::DENIED,
            record_fingerprint: ZERO_FINGERPRINT.to_owned(),
        };
        line.record_fingerprint = line.calculate_fingerprint()?;
        line.validate_structural()?;
        Ok(line)
    }

    fn calculate_fingerprint(&self) -> Result<String, PmTrialLiveJournalError> {
        let mut basis = self.clone();
        basis.record_fingerprint = ZERO_FINGERPRINT.to_owned();
        let domain = match basis.body {
            PhaseAAttemptLineageLedgerBodyV4::Prepared { .. } => {
                PHASE_A_ATTEMPT_LINEAGE_PREPARED_FINGERPRINT_DOMAIN_V4
            }
            PhaseAAttemptLineageLedgerBodyV4::Consumed { .. } => {
                PHASE_A_ATTEMPT_LINEAGE_CONSUMED_FINGERPRINT_DOMAIN_V4
            }
            PhaseAAttemptLineageLedgerBodyV4::FinalConjunct { .. } => {
                PHASE_A_ATTEMPT_LINEAGE_FINAL_CONJUNCT_FINGERPRINT_DOMAIN_V4
            }
        };
        hash_domain(domain, &basis)
    }

    fn validate_structural(&self) -> Result<(), PmTrialLiveJournalError> {
        if self.schema_version != PHASE_A_ATTEMPT_LINEAGE_SCHEMA_VERSION_V4
            || self.family != PHASE_A_ATTEMPT_LINEAGE_FAMILY_V4
            || self.authorization != OfflineAuthorizationState::DENIED
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        validate_fingerprint(&self.previous_record_fingerprint)?;
        validate_fingerprint(&self.record_fingerprint)?;
        match &self.body {
            PhaseAAttemptLineageLedgerBodyV4::Prepared {
                binding_fingerprint,
                binding,
            } => {
                if self.sequence != 0 || self.previous_record_fingerprint != ZERO_FINGERPRINT {
                    return Err(PmTrialLiveJournalError::InvalidTransition);
                }
                binding.validate()?;
                validate_fingerprint(binding_fingerprint)?;
                if hash_domain(
                    PHASE_A_ATTEMPT_LINEAGE_BINDING_FINGERPRINT_DOMAIN_V4,
                    binding.as_ref(),
                )? != *binding_fingerprint
                {
                    return Err(PmTrialLiveJournalError::InvalidBinding);
                }
            }
            PhaseAAttemptLineageLedgerBodyV4::Consumed {
                binding_fingerprint,
                prepared_record_fingerprint,
                claim_fingerprint,
            } => {
                if self.sequence != 1 || self.previous_record_fingerprint == ZERO_FINGERPRINT {
                    return Err(PmTrialLiveJournalError::InvalidTransition);
                }
                for fingerprint in [
                    binding_fingerprint,
                    prepared_record_fingerprint,
                    claim_fingerprint,
                ] {
                    validate_fingerprint(fingerprint)?;
                }
            }
            PhaseAAttemptLineageLedgerBodyV4::FinalConjunct {
                binding_fingerprint,
                prepared_record_fingerprint,
                claim_fingerprint,
                consumed_record_fingerprint,
                online_preflight_basis_record_fingerprint,
                online_preflight_a3_conjunct_record_fingerprint,
            } => {
                if self.sequence != 2 || self.previous_record_fingerprint == ZERO_FINGERPRINT {
                    return Err(PmTrialLiveJournalError::InvalidTransition);
                }
                for fingerprint in [
                    binding_fingerprint,
                    prepared_record_fingerprint,
                    claim_fingerprint,
                    consumed_record_fingerprint,
                    online_preflight_basis_record_fingerprint,
                    online_preflight_a3_conjunct_record_fingerprint,
                ] {
                    validate_fingerprint(fingerprint)?;
                }
            }
        }
        if self.calculate_fingerprint()? != self.record_fingerprint {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PhaseAAttemptBurnClaimV4 {
    schema_version: u32,
    family: String,
    ledger_file: String,
    binding_fingerprint: String,
    prepared_record_fingerprint: String,
    authorization: OfflineAuthorizationState,
    claim_fingerprint: String,
}

impl PhaseAAttemptBurnClaimV4 {
    fn seal(
        binding_fingerprint: String,
        prepared_record_fingerprint: String,
    ) -> Result<Self, PmTrialLiveJournalError> {
        let mut claim = Self {
            schema_version: PHASE_A_ATTEMPT_LINEAGE_SCHEMA_VERSION_V4,
            family: PHASE_A_ATTEMPT_BURN_CLAIM_FAMILY_V4.to_owned(),
            ledger_file: PM_PHASE_A_ATTEMPT_LINEAGE_LEDGER_FILE_V4.to_owned(),
            binding_fingerprint,
            prepared_record_fingerprint,
            authorization: OfflineAuthorizationState::DENIED,
            claim_fingerprint: ZERO_FINGERPRINT.to_owned(),
        };
        claim.claim_fingerprint = claim.calculate_fingerprint()?;
        claim.validate_structural()?;
        Ok(claim)
    }

    fn calculate_fingerprint(&self) -> Result<String, PmTrialLiveJournalError> {
        let mut basis = self.clone();
        basis.claim_fingerprint = ZERO_FINGERPRINT.to_owned();
        hash_domain(PHASE_A_ATTEMPT_BURN_CLAIM_FINGERPRINT_DOMAIN_V4, &basis)
    }

    fn validate_structural(&self) -> Result<(), PmTrialLiveJournalError> {
        if self.schema_version != PHASE_A_ATTEMPT_LINEAGE_SCHEMA_VERSION_V4
            || self.family != PHASE_A_ATTEMPT_BURN_CLAIM_FAMILY_V4
            || self.ledger_file != PM_PHASE_A_ATTEMPT_LINEAGE_LEDGER_FILE_V4
            || self.authorization != OfflineAuthorizationState::DENIED
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        validate_fingerprint(&self.binding_fingerprint)?;
        validate_fingerprint(&self.prepared_record_fingerprint)?;
        validate_fingerprint(&self.claim_fingerprint)?;
        if self.calculate_fingerprint()? != self.claim_fingerprint {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
        Ok(())
    }
}

/// Read-only crash evidence. `Complete` means only that the fixed V4 burn
/// lineage is physically and canonically complete; it does not make any
/// external proof, authorization, or current-runtime fact true. No state can
/// recreate or resume placement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PmPhaseAAttemptLineageInspectionV4 {
    Absent,
    PreparedUnclaimed {
        prepared_record_fingerprint: String,
    },
    ClaimOnlyBurned {
        prepared_record_fingerprint: String,
        claim_fingerprint: String,
    },
    BurnedAwaitingFinalConjunct {
        consumed_record_fingerprint: String,
        claim_fingerprint: String,
    },
    Complete {
        final_conjunct_record_fingerprint: String,
        claim_fingerprint: String,
    },
    AmbiguousBeforeClaim,
    AmbiguousBurned,
}

impl PmPhaseAAttemptLineageInspectionV4 {
    #[must_use]
    pub const fn authorization(&self) -> OfflineAuthorizationState {
        OfflineAuthorizationState::DENIED
    }

    #[must_use]
    pub const fn placement_resumption_allowed(&self) -> bool {
        false
    }
}

/// Inspect only the two fixed protected V4 artifacts while holding one
/// cooperative directory lease across both reads and final validation. Lease
/// acquisition or validation failure, claim presence with any inconsistency,
/// and a Consumed/full ledger without its claim are burned/ambiguous.
///
/// This assumes trusted local storage. Advisory locking cannot stop same-EUID
/// lock bypass or coordinated directory rollback; a future stronger design
/// needs a TPM-backed monotonic anchor, WORM storage, or authenticated remote
/// registry.
pub fn inspect_phase_a_attempt_lineage_v4(
    config: &CanonicalTrialConfig,
) -> PmPhaseAAttemptLineageInspectionV4 {
    let directory = Path::new(&config.value().journal.artifact_directory);
    inspect_paths(
        &directory.join(PM_PHASE_A_ATTEMPT_LINEAGE_LEDGER_FILE_V4),
        &directory.join(PM_PHASE_A_ATTEMPT_BURN_CLAIM_FILE_V4),
    )
}

enum InspectedBytes {
    Absent,
    Present(Vec<u8>),
    Unreadable,
}

fn inspect_bytes(path: &Path, maximum_bytes: usize) -> InspectedBytes {
    match read_protected(path, maximum_bytes) {
        Ok(bytes) => InspectedBytes::Present(bytes),
        Err(PmTrialLiveJournalError::Absent) => InspectedBytes::Absent,
        Err(_) => InspectedBytes::Unreadable,
    }
}

fn inspect_paths(ledger_path: &Path, claim_path: &Path) -> PmPhaseAAttemptLineageInspectionV4 {
    let Some(directory) = ledger_path.parent() else {
        return PmPhaseAAttemptLineageInspectionV4::AmbiguousBurned;
    };
    if claim_path.parent() != Some(directory) {
        return PmPhaseAAttemptLineageInspectionV4::AmbiguousBurned;
    }
    let lease = match ProtectedArtifactLease::acquire(directory) {
        Ok(lease) => lease,
        Err(_) => return PmPhaseAAttemptLineageInspectionV4::AmbiguousBurned,
    };
    let inspected = inspect_paths_while_leased(ledger_path, claim_path);
    if lease.validate().is_err() {
        return PmPhaseAAttemptLineageInspectionV4::AmbiguousBurned;
    }
    inspected
}

fn inspect_paths_while_leased(
    ledger_path: &Path,
    claim_path: &Path,
) -> PmPhaseAAttemptLineageInspectionV4 {
    let claim = match inspect_bytes(claim_path, MAX_PHASE_A_ATTEMPT_BURN_CLAIM_BYTES_V4) {
        InspectedBytes::Absent => None,
        InspectedBytes::Present(bytes) => match parse_claim(&bytes) {
            Ok(claim) => Some(claim),
            Err(_) => return PmPhaseAAttemptLineageInspectionV4::AmbiguousBurned,
        },
        InspectedBytes::Unreadable => {
            return PmPhaseAAttemptLineageInspectionV4::AmbiguousBurned;
        }
    };
    let ledger = match inspect_bytes(ledger_path, MAX_PHASE_A_ATTEMPT_LINEAGE_LEDGER_BYTES_V4) {
        InspectedBytes::Absent if claim.is_none() => {
            return PmPhaseAAttemptLineageInspectionV4::Absent;
        }
        InspectedBytes::Absent => {
            return PmPhaseAAttemptLineageInspectionV4::AmbiguousBurned;
        }
        InspectedBytes::Present(bytes) => match parse_ledger(&bytes) {
            Ok(lines) => lines,
            Err(_) if claim.is_some() => {
                return PmPhaseAAttemptLineageInspectionV4::AmbiguousBurned;
            }
            Err(_) => return PmPhaseAAttemptLineageInspectionV4::AmbiguousBeforeClaim,
        },
        InspectedBytes::Unreadable if claim.is_some() => {
            return PmPhaseAAttemptLineageInspectionV4::AmbiguousBurned;
        }
        InspectedBytes::Unreadable => {
            return PmPhaseAAttemptLineageInspectionV4::AmbiguousBeforeClaim;
        }
    };

    match (ledger.as_slice(), claim.as_ref()) {
        ([prepared], None) if prepared_binding(prepared).is_some() => {
            PmPhaseAAttemptLineageInspectionV4::PreparedUnclaimed {
                prepared_record_fingerprint: prepared.record_fingerprint.clone(),
            }
        }
        ([prepared], Some(claim)) if claim_matches_prepared(claim, prepared) => {
            PmPhaseAAttemptLineageInspectionV4::ClaimOnlyBurned {
                prepared_record_fingerprint: prepared.record_fingerprint.clone(),
                claim_fingerprint: claim.claim_fingerprint.clone(),
            }
        }
        ([prepared, consumed], Some(claim))
            if claim_matches_prepared(claim, prepared)
                && consumed_matches(consumed, prepared, claim) =>
        {
            PmPhaseAAttemptLineageInspectionV4::BurnedAwaitingFinalConjunct {
                consumed_record_fingerprint: consumed.record_fingerprint.clone(),
                claim_fingerprint: claim.claim_fingerprint.clone(),
            }
        }
        ([prepared, consumed, final_conjunct], Some(claim))
            if claim_matches_prepared(claim, prepared)
                && consumed_matches(consumed, prepared, claim)
                && final_conjunct_matches(final_conjunct, prepared, consumed, claim) =>
        {
            PmPhaseAAttemptLineageInspectionV4::Complete {
                final_conjunct_record_fingerprint: final_conjunct.record_fingerprint.clone(),
                claim_fingerprint: claim.claim_fingerprint.clone(),
            }
        }
        ([_, _, ..], None) => PmPhaseAAttemptLineageInspectionV4::AmbiguousBurned,
        (_, Some(_)) => PmPhaseAAttemptLineageInspectionV4::AmbiguousBurned,
        _ => PmPhaseAAttemptLineageInspectionV4::AmbiguousBeforeClaim,
    }
}

/// Move-only pending V4 custody. Its type and constructor are crate-private.
pub(crate) struct PmPreparedPhaseAAttemptLineageV4 {
    basis: PmPendingPhaseAOnlinePreflightBasisV2,
    ledger: ProtectedJournal,
    expected_ledger_bytes: Vec<u8>,
    prepared: PhaseAAttemptLineageLedgerLineV4,
}

impl fmt::Debug for PmPreparedPhaseAAttemptLineageV4 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmPreparedPhaseAAttemptLineageV4")
            .field(
                "prepared_record_fingerprint",
                &self.prepared.record_fingerprint,
            )
            .field("authorization", &OfflineAuthorizationState::DENIED)
            .field("production_permit", &false)
            .finish()
    }
}

/// Complete evidence-only V4 lineage. It deliberately has no conversion to a
/// network owner, request, permit, HMAC input, or transport operation.
pub(crate) struct PmBurnedPhaseAAttemptLineageEvidenceV4 {
    v2: PmPhaseAOnlinePreflightDispatchOwnerV2,
    ledger: ProtectedJournal,
    claim_file: ProtectedJournal,
    expected_ledger_bytes: Vec<u8>,
    expected_claim_bytes: Vec<u8>,
    prepared: PhaseAAttemptLineageLedgerLineV4,
    consumed: PhaseAAttemptLineageLedgerLineV4,
    final_conjunct: PhaseAAttemptLineageLedgerLineV4,
    claim: PhaseAAttemptBurnClaimV4,
}

impl PmBurnedPhaseAAttemptLineageEvidenceV4 {
    pub(crate) const fn authorization(&self) -> OfflineAuthorizationState {
        OfflineAuthorizationState::DENIED
    }

    pub(crate) fn revalidate_held_evidence(
        &mut self,
        journals: &mut PmControlledTrialLiveJournals,
    ) -> Result<(), PmPhaseAAttemptLineageV4Error> {
        self.validate_held_v4_and_v2_binding()?;
        journals.revalidate_phase_a_online_preflight_v2_evidence_only(&mut self.v2)?;
        // Close the journal-backed V1/V2 check with the V4 exact set again.
        self.validate_held_v4_and_v2_binding()
    }

    fn validate_held_v4_and_v2_binding(&mut self) -> Result<(), PmPhaseAAttemptLineageV4Error> {
        self.ledger
            .validate_exact_bytes(&self.expected_ledger_bytes)?;
        self.claim_file
            .validate_exact_bytes(&self.expected_claim_bytes)?;
        let lines = parse_ledger(&self.expected_ledger_bytes)?;
        let claim = parse_claim(&self.expected_claim_bytes)?;
        if lines.as_slice()
            != [
                self.prepared.clone(),
                self.consumed.clone(),
                self.final_conjunct.clone(),
            ]
            || claim != self.claim
            || !claim_matches_prepared(&claim, &self.prepared)
            || !consumed_matches(&self.consumed, &self.prepared, &claim)
            || !final_conjunct_matches(&self.final_conjunct, &self.prepared, &self.consumed, &claim)
            || !final_conjunct_matches_v2(&self.final_conjunct, &self.v2)
        {
            return Err(PmTrialLiveJournalError::InvalidBinding.into());
        }
        self.v2.revalidate_held_evidence()?;
        Ok(())
    }
}

impl fmt::Debug for PmBurnedPhaseAAttemptLineageEvidenceV4 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmBurnedPhaseAAttemptLineageEvidenceV4")
            .field(
                "final_conjunct_record_fingerprint",
                &self.final_conjunct.record_fingerprint,
            )
            .field("authorization", &OfflineAuthorizationState::DENIED)
            .field("production_permit", &false)
            .finish()
    }
}

pub(crate) enum PmPhaseAAttemptLineageV4Error {
    Journal(PmTrialLiveJournalError),
    ExistingLineage(PmPhaseAOnlinePreflightV2Error),
}

impl fmt::Debug for PmPhaseAAttemptLineageV4Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Journal(error) => formatter.debug_tuple("Journal").field(error).finish(),
            Self::ExistingLineage(error) => formatter
                .debug_tuple("ExistingLineage")
                .field(error)
                .finish(),
        }
    }
}

impl From<PmTrialLiveJournalError> for PmPhaseAAttemptLineageV4Error {
    fn from(error: PmTrialLiveJournalError) -> Self {
        Self::Journal(error)
    }
}

impl From<PmPhaseAOnlinePreflightV2Error> for PmPhaseAAttemptLineageV4Error {
    fn from(error: PmPhaseAOnlinePreflightV2Error) -> Self {
        Self::ExistingLineage(error)
    }
}

/// Crate-private and unreachable in production because the commitment bundle
/// has no production constructor. This records burn/no-resend evidence only.
pub(crate) fn create_phase_a_attempt_lineage_prepared_v4(
    config: &CanonicalTrialConfig,
    envelope: &CanonicalReviewedPhaseAEligibilityEnvelopeV4,
    journals: &mut PmControlledTrialLiveJournals,
    mut basis: PmPendingPhaseAOnlinePreflightBasisV2,
    commitment_bundle: PmPhaseAExternalProofCommitmentBundleV4,
) -> Result<PmPreparedPhaseAAttemptLineageV4, PmPhaseAAttemptLineageV4Error> {
    let prepared = make_prepared(config, envelope, &basis, commitment_bundle)?;
    let encoded = encode_ledger_line(&prepared)?;
    let path = Path::new(&config.value().journal.artifact_directory)
        .join(PM_PHASE_A_ATTEMPT_LINEAGE_LEDGER_FILE_V4);
    let mut ledger =
        ProtectedJournal::create_new(&path, MAX_PHASE_A_ATTEMPT_LINEAGE_LEDGER_BYTES_V4)?;
    ledger.append_durable(&[], &encoded)?;

    // V4 Prepared creates a directory entry after Basis. Refresh every older
    // held V1/V2/sidecar/journal parent and the new V4 ledger, then validate
    // exact bytes. Attempt every refresh/check before failing closed.
    let basis_refresh = basis.refresh_after_additive_bound_create(journals);
    let ledger_refresh = ledger.refresh_parent_after_bound_create();
    let ledger_validation = ledger.validate_exact_bytes(&encoded);
    if let Err(error) = basis_refresh {
        return Err(error.into());
    }
    ledger_refresh?;
    ledger_validation?;

    Ok(PmPreparedPhaseAAttemptLineageV4 {
        basis,
        ledger,
        expected_ledger_bytes: encoded,
        prepared,
    })
}

impl PmPreparedPhaseAAttemptLineageV4 {
    /// Settled order B: V4 claim+Consumed, existing V2 claim+Consumed, V1
    /// claim+Consumed, V1 DispatchAuthorized+A3, V2 conjunct, V4 conjunct.
    pub(crate) fn burn_and_complete(
        self,
        journals: &mut PmControlledTrialLiveJournals,
        config: &CanonicalTrialConfig,
        v1_authorization: &CanonicalAuthorization,
        v1_runtime: &AuthorizationRuntimeBinding,
        v2_runtime: &OnlineAuthorizationRuntimeBindingV2,
    ) -> Result<PmBurnedPhaseAAttemptLineageEvidenceV4, PmPhaseAAttemptLineageV4Error> {
        let Self {
            mut basis,
            mut ledger,
            mut expected_ledger_bytes,
            prepared,
        } = self;
        ledger.validate_exact_bytes(&expected_ledger_bytes)?;
        let binding_fingerprint = prepared_binding_fingerprint(&prepared)
            .ok_or(PmTrialLiveJournalError::InvalidBinding)?
            .to_owned();
        let claim = PhaseAAttemptBurnClaimV4::seal(
            binding_fingerprint.clone(),
            prepared.record_fingerprint.clone(),
        )?;
        let claim_bytes = encode_claim(&claim)?;
        let claim_path = Path::new(&config.value().journal.artifact_directory)
            .join(PM_PHASE_A_ATTEMPT_BURN_CLAIM_FILE_V4);
        let mut claim_file =
            ProtectedJournal::create_new(&claim_path, MAX_PHASE_A_ATTEMPT_BURN_CLAIM_BYTES_V4)?;
        claim_file.append_durable(&[], &claim_bytes)?;

        // Claim presence is the V4 burn point. Refresh the complete existing
        // custody and immediately validate exact V4 bytes. Every refresh and
        // check is attempted before the burned path fails closed.
        let ledger_refresh = ledger.refresh_parent_after_bound_create();
        let claim_refresh = claim_file.refresh_parent_after_bound_create();
        let basis_refresh = basis.refresh_after_additive_bound_create(journals);
        let ledger_validation = ledger.validate_exact_bytes(&expected_ledger_bytes);
        let claim_validation = claim_file.validate_exact_bytes(&claim_bytes);
        ledger_refresh?;
        claim_refresh?;
        if let Err(error) = basis_refresh {
            return Err(error.into());
        }
        ledger_validation?;
        claim_validation?;
        let consumed = PhaseAAttemptLineageLedgerLineV4::seal(
            1,
            prepared.record_fingerprint.clone(),
            PhaseAAttemptLineageLedgerBodyV4::Consumed {
                binding_fingerprint: binding_fingerprint.clone(),
                prepared_record_fingerprint: prepared.record_fingerprint.clone(),
                claim_fingerprint: claim.claim_fingerprint.clone(),
            },
        )?;
        let consumed_bytes = encode_ledger_line(&consumed)?;
        ledger.append_durable(&expected_ledger_bytes, &consumed_bytes)?;
        expected_ledger_bytes.extend_from_slice(&consumed_bytes);

        let mut additional_refresh = PhaseAAttemptHeldParentRefreshV4 {
            ledger: &mut ledger,
            claim: &mut claim_file,
            expected_ledger_bytes: &expected_ledger_bytes,
            expected_claim_bytes: &claim_bytes,
        };
        let v2 = basis.burn_and_record_a3_with_refresh(
            journals,
            config,
            v1_authorization,
            v1_runtime,
            v2_runtime,
            &mut additional_refresh,
        )?;

        let final_conjunct = PhaseAAttemptLineageLedgerLineV4::seal(
            2,
            consumed.record_fingerprint.clone(),
            PhaseAAttemptLineageLedgerBodyV4::FinalConjunct {
                binding_fingerprint,
                prepared_record_fingerprint: prepared.record_fingerprint.clone(),
                claim_fingerprint: claim.claim_fingerprint.clone(),
                consumed_record_fingerprint: consumed.record_fingerprint.clone(),
                online_preflight_basis_record_fingerprint: v2.basis_record_fingerprint().to_owned(),
                online_preflight_a3_conjunct_record_fingerprint: v2
                    .a3_conjunct_record_fingerprint()
                    .to_owned(),
            },
        )?;
        let final_bytes = encode_ledger_line(&final_conjunct)?;
        ledger.append_durable(&expected_ledger_bytes, &final_bytes)?;
        expected_ledger_bytes.extend_from_slice(&final_bytes);

        let mut owner = PmBurnedPhaseAAttemptLineageEvidenceV4 {
            v2,
            ledger,
            claim_file,
            expected_ledger_bytes,
            expected_claim_bytes: claim_bytes,
            prepared,
            consumed,
            final_conjunct,
            claim,
        };
        owner.revalidate_held_evidence(journals)?;
        Ok(owner)
    }
}

struct PhaseAAttemptHeldParentRefreshV4<'a> {
    ledger: &'a mut ProtectedJournal,
    claim: &'a mut ProtectedJournal,
    expected_ledger_bytes: &'a [u8],
    expected_claim_bytes: &'a [u8],
}

impl PmOnlinePreflightV2BoundCreateRefresh for PhaseAAttemptHeldParentRefreshV4<'_> {
    fn refresh_after_bound_create(
        &mut self,
        sidecar: &mut ProtectedJournal,
        expected_sidecar_bytes: &[u8],
    ) -> Result<(), PmTrialLiveJournalError> {
        // V2 claim, V1 claim, and A3 each create a directory entry. Attempt to
        // refresh both V4 holders and validate every sidecar/V4 byte set before
        // reporting any failure.
        let sidecar_validation = sidecar.validate_exact_bytes(expected_sidecar_bytes);
        let ledger = self.ledger.refresh_parent_after_bound_create();
        let claim = self.claim.refresh_parent_after_bound_create();
        let ledger_validation = self.ledger.validate_exact_bytes(self.expected_ledger_bytes);
        let claim_validation = self.claim.validate_exact_bytes(self.expected_claim_bytes);
        sidecar_validation?;
        ledger?;
        claim?;
        ledger_validation?;
        claim_validation
    }
}

fn make_prepared(
    config: &CanonicalTrialConfig,
    envelope: &CanonicalReviewedPhaseAEligibilityEnvelopeV4,
    basis: &PmPendingPhaseAOnlinePreflightBasisV2,
    commitment_bundle: PmPhaseAExternalProofCommitmentBundleV4,
) -> Result<PhaseAAttemptLineageLedgerLineV4, PmTrialLiveJournalError> {
    commitment_bundle.pins.validate()?;
    let policy = basis.online_policy();
    let authorization = basis.online_authorization();
    let preparation = basis.preparation();
    let external_proof_commitments_fingerprint = hash_domain(
        PHASE_A_ATTEMPT_LINEAGE_EXTERNAL_PROOF_COMMITMENTS_FINGERPRINT_DOMAIN_V4,
        &commitment_bundle.pins,
    )?;
    let binding = PhaseAAttemptPreparedBindingV4 {
        canonical_config: PhaseAAttemptLineagePinsV4 {
            canonical_sha256: config.canonical_sha256().to_owned(),
            canonical_length: config.canonical_length(),
            fingerprint: config.fingerprint().to_owned(),
        },
        trial_plan_fingerprint: config.plan_fingerprint().to_owned(),
        reviewed_phase_a_eligibility_envelope_v4: PhaseAAttemptLineagePinsV4 {
            canonical_sha256: envelope.canonical_sha256().to_owned(),
            canonical_length: envelope.canonical_length(),
            fingerprint: envelope.fingerprint().to_owned(),
        },
        online_policy_v2: PhaseAAttemptLineagePinsV4 {
            canonical_sha256: policy.canonical_sha256().to_owned(),
            canonical_length: policy.canonical_length(),
            fingerprint: policy.fingerprint().to_owned(),
        },
        online_authorization_v2: PhaseAAttemptLineagePinsV4 {
            canonical_sha256: authorization.canonical_sha256().to_owned(),
            canonical_length: authorization.canonical_length(),
            fingerprint: authorization.fingerprint().to_owned(),
        },
        online_preflight_basis_record_fingerprint: basis.basis_record_fingerprint().to_owned(),
        place_prepared_record_fingerprint: basis.place_prepared_record_fingerprint().to_owned(),
        public_semantic_request_commitment: lower_hex32(
            preparation.semantic_request_commitment().bytes(),
        ),
        prepared_request_commitment: preparation.request_commitment().to_owned(),
        expected_order_id: lower_hex32(preparation.expected_order_id().bytes()),
        l2_timestamp_seconds: preparation.l2_timestamp_seconds(),
        external_proof_commitments_fingerprint,
        external_proof_commitments: commitment_bundle.pins,
    };
    binding.validate()?;
    let binding_fingerprint = hash_domain(
        PHASE_A_ATTEMPT_LINEAGE_BINDING_FINGERPRINT_DOMAIN_V4,
        &binding,
    )?;
    PhaseAAttemptLineageLedgerLineV4::seal(
        0,
        ZERO_FINGERPRINT.to_owned(),
        PhaseAAttemptLineageLedgerBodyV4::Prepared {
            binding_fingerprint,
            binding: Box::new(binding),
        },
    )
}

fn prepared_binding(
    prepared: &PhaseAAttemptLineageLedgerLineV4,
) -> Option<&PhaseAAttemptPreparedBindingV4> {
    match &prepared.body {
        PhaseAAttemptLineageLedgerBodyV4::Prepared { binding, .. } if prepared.sequence == 0 => {
            Some(binding)
        }
        _ => None,
    }
}

fn prepared_binding_fingerprint(prepared: &PhaseAAttemptLineageLedgerLineV4) -> Option<&str> {
    match &prepared.body {
        PhaseAAttemptLineageLedgerBodyV4::Prepared {
            binding_fingerprint,
            ..
        } if prepared.sequence == 0 => Some(binding_fingerprint),
        _ => None,
    }
}

fn claim_matches_prepared(
    claim: &PhaseAAttemptBurnClaimV4,
    prepared: &PhaseAAttemptLineageLedgerLineV4,
) -> bool {
    prepared_binding_fingerprint(prepared).is_some_and(|binding| {
        claim.binding_fingerprint == binding
            && claim.prepared_record_fingerprint == prepared.record_fingerprint
    })
}

fn consumed_matches(
    consumed: &PhaseAAttemptLineageLedgerLineV4,
    prepared: &PhaseAAttemptLineageLedgerLineV4,
    claim: &PhaseAAttemptBurnClaimV4,
) -> bool {
    if !consumed_links_prepared(consumed, prepared) {
        return false;
    }
    match &consumed.body {
        PhaseAAttemptLineageLedgerBodyV4::Consumed {
            claim_fingerprint, ..
        } => claim_fingerprint == &claim.claim_fingerprint,
        _ => false,
    }
}

fn consumed_links_prepared(
    consumed: &PhaseAAttemptLineageLedgerLineV4,
    prepared: &PhaseAAttemptLineageLedgerLineV4,
) -> bool {
    match &consumed.body {
        PhaseAAttemptLineageLedgerBodyV4::Consumed {
            binding_fingerprint,
            prepared_record_fingerprint,
            ..
        } => {
            consumed.sequence == 1
                && consumed.previous_record_fingerprint == prepared.record_fingerprint
                && prepared_binding_fingerprint(prepared) == Some(binding_fingerprint.as_str())
                && prepared_record_fingerprint == &prepared.record_fingerprint
        }
        _ => false,
    }
}

fn final_conjunct_matches(
    final_conjunct: &PhaseAAttemptLineageLedgerLineV4,
    prepared: &PhaseAAttemptLineageLedgerLineV4,
    consumed: &PhaseAAttemptLineageLedgerLineV4,
    claim: &PhaseAAttemptBurnClaimV4,
) -> bool {
    if !final_conjunct_links_consumed(final_conjunct, prepared, consumed) {
        return false;
    }
    match &final_conjunct.body {
        PhaseAAttemptLineageLedgerBodyV4::FinalConjunct {
            claim_fingerprint, ..
        } => claim_fingerprint == &claim.claim_fingerprint,
        _ => false,
    }
}

fn final_conjunct_links_consumed(
    final_conjunct: &PhaseAAttemptLineageLedgerLineV4,
    prepared: &PhaseAAttemptLineageLedgerLineV4,
    consumed: &PhaseAAttemptLineageLedgerLineV4,
) -> bool {
    match &final_conjunct.body {
        PhaseAAttemptLineageLedgerBodyV4::FinalConjunct {
            binding_fingerprint,
            prepared_record_fingerprint,
            consumed_record_fingerprint,
            online_preflight_basis_record_fingerprint,
            online_preflight_a3_conjunct_record_fingerprint,
            ..
        } => {
            let Some(binding) = prepared_binding(prepared) else {
                return false;
            };
            final_conjunct.sequence == 2
                && final_conjunct.previous_record_fingerprint == consumed.record_fingerprint
                && prepared_binding_fingerprint(prepared) == Some(binding_fingerprint.as_str())
                && prepared_record_fingerprint == &prepared.record_fingerprint
                && consumed_record_fingerprint == &consumed.record_fingerprint
                && online_preflight_basis_record_fingerprint
                    == &binding.online_preflight_basis_record_fingerprint
                && online_preflight_a3_conjunct_record_fingerprint != ZERO_FINGERPRINT
        }
        _ => false,
    }
}

fn final_conjunct_matches_v2(
    final_conjunct: &PhaseAAttemptLineageLedgerLineV4,
    v2: &PmPhaseAOnlinePreflightDispatchOwnerV2,
) -> bool {
    match &final_conjunct.body {
        PhaseAAttemptLineageLedgerBodyV4::FinalConjunct {
            online_preflight_basis_record_fingerprint,
            online_preflight_a3_conjunct_record_fingerprint,
            ..
        } => {
            online_preflight_basis_record_fingerprint == v2.basis_record_fingerprint()
                && online_preflight_a3_conjunct_record_fingerprint
                    == v2.a3_conjunct_record_fingerprint()
        }
        _ => false,
    }
}

fn encode_ledger_line(
    line: &PhaseAAttemptLineageLedgerLineV4,
) -> Result<Vec<u8>, PmTrialLiveJournalError> {
    line.validate_structural()?;
    let mut encoded = canonical_json(line)?;
    encoded.push(b'\n');
    if encoded.len() > MAX_PHASE_A_ATTEMPT_LINEAGE_LINE_BYTES_V4 {
        return Err(PmTrialLiveJournalError::BoundExceeded);
    }
    Ok(encoded)
}

fn encode_claim(claim: &PhaseAAttemptBurnClaimV4) -> Result<Vec<u8>, PmTrialLiveJournalError> {
    claim.validate_structural()?;
    let mut encoded = canonical_json(claim)?;
    encoded.push(b'\n');
    if encoded.len() > MAX_PHASE_A_ATTEMPT_BURN_CLAIM_BYTES_V4 {
        return Err(PmTrialLiveJournalError::BoundExceeded);
    }
    Ok(encoded)
}

fn parse_ledger(
    bytes: &[u8],
) -> Result<Vec<PhaseAAttemptLineageLedgerLineV4>, PmTrialLiveJournalError> {
    if bytes.is_empty()
        || bytes.len() > MAX_PHASE_A_ATTEMPT_LINEAGE_LEDGER_BYTES_V4
        || !bytes.ends_with(b"\n")
    {
        return Err(PmTrialLiveJournalError::AmbiguousTail);
    }
    let mut lines = Vec::new();
    for raw in bytes[..bytes.len() - 1].split(|byte| *byte == b'\n') {
        if raw.is_empty() || raw.len() + 1 > MAX_PHASE_A_ATTEMPT_LINEAGE_LINE_BYTES_V4 {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
        let line: PhaseAAttemptLineageLedgerLineV4 =
            serde_json::from_slice(raw).map_err(|_| PmTrialLiveJournalError::InvalidRecord)?;
        if canonical_json(&line)? != raw {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
        line.validate_structural()?;
        lines.push(line);
    }
    match lines.as_slice() {
        [prepared] if prepared_binding(prepared).is_some() => {}
        [prepared, consumed]
            if prepared_binding(prepared).is_some()
                && consumed_links_prepared(consumed, prepared)
                && matches!(
                    consumed.body,
                    PhaseAAttemptLineageLedgerBodyV4::Consumed { .. }
                ) => {}
        [prepared, consumed, final_conjunct]
            if prepared_binding(prepared).is_some()
                && consumed_links_prepared(consumed, prepared)
                && final_conjunct_links_consumed(final_conjunct, prepared, consumed)
                && matches!(
                    consumed.body,
                    PhaseAAttemptLineageLedgerBodyV4::Consumed { .. }
                )
                && matches!(
                    final_conjunct.body,
                    PhaseAAttemptLineageLedgerBodyV4::FinalConjunct { .. }
                ) => {}
        _ => return Err(PmTrialLiveJournalError::InvalidTransition),
    }
    Ok(lines)
}

fn parse_claim(bytes: &[u8]) -> Result<PhaseAAttemptBurnClaimV4, PmTrialLiveJournalError> {
    if bytes.is_empty()
        || bytes.len() > MAX_PHASE_A_ATTEMPT_BURN_CLAIM_BYTES_V4
        || !bytes.ends_with(b"\n")
        || bytes[..bytes.len() - 1].contains(&b'\n')
    {
        return Err(PmTrialLiveJournalError::AmbiguousTail);
    }
    let raw = &bytes[..bytes.len() - 1];
    let claim: PhaseAAttemptBurnClaimV4 =
        serde_json::from_slice(raw).map_err(|_| PmTrialLiveJournalError::InvalidRecord)?;
    if canonical_json(&claim)? != raw {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    claim.validate_structural()?;
    Ok(claim)
}

fn lower_hex32(bytes: [u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt as _};

    use super::*;

    impl PmPhaseAExternalProofCommitmentBundleV4 {
        fn synthetic_for_tests() -> Self {
            Self {
                pins: PhaseAExternalProofCommitmentsV4 {
                    reviewer_trust_anchor_commitment: "11".repeat(32),
                    credential_provider_trust_root_commitment: "22".repeat(32),
                    credential_delivery_lease_commitment: "33".repeat(32),
                    authoritative_remote_acceptance_commitment: "44".repeat(32),
                    signer_proxy_control_commitment: "55".repeat(32),
                    same_attempt_audience_commitment: "66".repeat(32),
                    selected_actor_generation_commitment: "77".repeat(32),
                    runtime_attempt_commitment: "88".repeat(32),
                },
            }
        }
    }

    fn fixture_records() -> (
        PhaseAAttemptLineageLedgerLineV4,
        PhaseAAttemptBurnClaimV4,
        PhaseAAttemptLineageLedgerLineV4,
        PhaseAAttemptLineageLedgerLineV4,
    ) {
        let commitment_bundle = PmPhaseAExternalProofCommitmentBundleV4::synthetic_for_tests();
        let external_proof_commitments_fingerprint = hash_domain(
            PHASE_A_ATTEMPT_LINEAGE_EXTERNAL_PROOF_COMMITMENTS_FINGERPRINT_DOMAIN_V4,
            &commitment_bundle.pins,
        )
        .unwrap();
        let binding = PhaseAAttemptPreparedBindingV4 {
            canonical_config: PhaseAAttemptLineagePinsV4 {
                canonical_sha256: "91".repeat(32),
                canonical_length: 101,
                fingerprint: "92".repeat(32),
            },
            trial_plan_fingerprint: "93".repeat(32),
            reviewed_phase_a_eligibility_envelope_v4: PhaseAAttemptLineagePinsV4 {
                canonical_sha256: "94".repeat(32),
                canonical_length: 102,
                fingerprint: "95".repeat(32),
            },
            online_policy_v2: PhaseAAttemptLineagePinsV4 {
                canonical_sha256: "96".repeat(32),
                canonical_length: 103,
                fingerprint: "97".repeat(32),
            },
            online_authorization_v2: PhaseAAttemptLineagePinsV4 {
                canonical_sha256: "98".repeat(32),
                canonical_length: 104,
                fingerprint: "99".repeat(32),
            },
            online_preflight_basis_record_fingerprint: "aa".repeat(32),
            place_prepared_record_fingerprint: "ab".repeat(32),
            public_semantic_request_commitment: "ac".repeat(32),
            prepared_request_commitment: "ad".repeat(32),
            expected_order_id: "ae".repeat(32),
            l2_timestamp_seconds: 1_725_000_000,
            external_proof_commitments_fingerprint,
            external_proof_commitments: commitment_bundle.pins,
        };
        let binding_fingerprint = hash_domain(
            PHASE_A_ATTEMPT_LINEAGE_BINDING_FINGERPRINT_DOMAIN_V4,
            &binding,
        )
        .unwrap();
        let prepared = PhaseAAttemptLineageLedgerLineV4::seal(
            0,
            ZERO_FINGERPRINT.to_owned(),
            PhaseAAttemptLineageLedgerBodyV4::Prepared {
                binding_fingerprint: binding_fingerprint.clone(),
                binding: Box::new(binding),
            },
        )
        .unwrap();
        let claim = PhaseAAttemptBurnClaimV4::seal(
            binding_fingerprint.clone(),
            prepared.record_fingerprint.clone(),
        )
        .unwrap();
        let consumed = PhaseAAttemptLineageLedgerLineV4::seal(
            1,
            prepared.record_fingerprint.clone(),
            PhaseAAttemptLineageLedgerBodyV4::Consumed {
                binding_fingerprint: binding_fingerprint.clone(),
                prepared_record_fingerprint: prepared.record_fingerprint.clone(),
                claim_fingerprint: claim.claim_fingerprint.clone(),
            },
        )
        .unwrap();
        let final_conjunct = PhaseAAttemptLineageLedgerLineV4::seal(
            2,
            consumed.record_fingerprint.clone(),
            PhaseAAttemptLineageLedgerBodyV4::FinalConjunct {
                binding_fingerprint,
                prepared_record_fingerprint: prepared.record_fingerprint.clone(),
                claim_fingerprint: claim.claim_fingerprint.clone(),
                consumed_record_fingerprint: consumed.record_fingerprint.clone(),
                online_preflight_basis_record_fingerprint: "aa".repeat(32),
                online_preflight_a3_conjunct_record_fingerprint: "af".repeat(32),
            },
        )
        .unwrap();
        (prepared, claim, consumed, final_conjunct)
    }

    fn protected_directory() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        directory
    }

    fn write_file(path: &Path, maximum: usize, bytes: &[u8]) {
        let mut file = ProtectedJournal::create_new(path, maximum).unwrap();
        file.append_durable(&[], bytes).unwrap();
    }

    #[test]
    fn canonical_records_round_trip_and_freeze_golden_fingerprints() {
        let (prepared, claim, consumed, final_conjunct) = fixture_records();
        let mut ledger = encode_ledger_line(&prepared).unwrap();
        assert_eq!(
            parse_ledger(&ledger).unwrap(),
            std::slice::from_ref(&prepared)
        );
        ledger.extend_from_slice(&encode_ledger_line(&consumed).unwrap());
        assert_eq!(
            parse_ledger(&ledger).unwrap(),
            [prepared.clone(), consumed.clone()]
        );
        ledger.extend_from_slice(&encode_ledger_line(&final_conjunct).unwrap());
        assert_eq!(
            parse_ledger(&ledger).unwrap(),
            [prepared.clone(), consumed.clone(), final_conjunct.clone()]
        );
        let claim_bytes = encode_claim(&claim).unwrap();
        assert_eq!(parse_claim(&claim_bytes).unwrap(), claim);
        assert_eq!(
            prepared.record_fingerprint,
            "0cec4ce1c569b07c1f43e4b48f7fd676df6431cc2592c50978cd412ddf4ed36c"
        );
        assert_eq!(
            claim.claim_fingerprint,
            "ec84f1a5669a202c4134f326a0fe8aba366244ca5bbe80fab0d44ab8544549df"
        );
        assert_eq!(
            consumed.record_fingerprint,
            "45a9da3652b07b8976001d44e0f6eb3d7df1690133a4092659182bd6c5bcaf6f"
        );
        assert_eq!(
            final_conjunct.record_fingerprint,
            "7abe3a493afe5acdc8d05c99bebb688c43ca9d81380c264066abb844b785534e"
        );
        const GOLDEN_CLAIM_JSON: &str = r#"{"schema_version":4,"family":"pm-t2-phase-a-attempt-burn-claim","ledger_file":"pm-t2-phase-a-attempt-lineage-v4.jsonl","binding_fingerprint":"801152d52d70c09b03f38c18ce59a1e366890afd40239f30cb74d7bddbe4e7b0","prepared_record_fingerprint":"0cec4ce1c569b07c1f43e4b48f7fd676df6431cc2592c50978cd412ddf4ed36c","authorization":{"production_order_entry_authorized":false,"real_order_submission_authorized":false,"place_dispatch_allowance":0},"claim_fingerprint":"ec84f1a5669a202c4134f326a0fe8aba366244ca5bbe80fab0d44ab8544549df"}"#;
        assert_eq!(
            String::from_utf8(claim_bytes).unwrap(),
            format!("{GOLDEN_CLAIM_JSON}\n")
        );
    }

    #[test]
    fn crash_prefixes_never_reopen_placement() {
        let (prepared, claim, consumed, final_conjunct) = fixture_records();
        let prepared_bytes = encode_ledger_line(&prepared).unwrap();
        let claim_bytes = encode_claim(&claim).unwrap();
        let consumed_bytes = encode_ledger_line(&consumed).unwrap();
        let final_bytes = encode_ledger_line(&final_conjunct).unwrap();

        for expected_stage in 0_u8..=4 {
            let directory = protected_directory();
            let ledger_path = directory
                .path()
                .join(PM_PHASE_A_ATTEMPT_LINEAGE_LEDGER_FILE_V4);
            let claim_path = directory.path().join(PM_PHASE_A_ATTEMPT_BURN_CLAIM_FILE_V4);
            if expected_stage >= 1 {
                let mut ledger = ProtectedJournal::create_new(
                    &ledger_path,
                    MAX_PHASE_A_ATTEMPT_LINEAGE_LEDGER_BYTES_V4,
                )
                .unwrap();
                ledger.append_durable(&[], &prepared_bytes).unwrap();
                if expected_stage >= 2 {
                    let mut claim_file = ProtectedJournal::create_new(
                        &claim_path,
                        MAX_PHASE_A_ATTEMPT_BURN_CLAIM_BYTES_V4,
                    )
                    .unwrap();
                    claim_file.append_durable(&[], &claim_bytes).unwrap();
                    ledger.refresh_parent_after_bound_create().unwrap();
                    if expected_stage >= 3 {
                        ledger
                            .append_durable(&prepared_bytes, &consumed_bytes)
                            .unwrap();
                    }
                    if expected_stage >= 4 {
                        let mut prefix = prepared_bytes.clone();
                        prefix.extend_from_slice(&consumed_bytes);
                        ledger.append_durable(&prefix, &final_bytes).unwrap();
                    }
                    drop(claim_file);
                }
                drop(ledger);
            }
            let inspected = inspect_paths(&ledger_path, &claim_path);
            assert!(!inspected.placement_resumption_allowed());
            assert_eq!(inspected.authorization(), OfflineAuthorizationState::DENIED);
            assert!(matches!(
                (expected_stage, inspected),
                (0, PmPhaseAAttemptLineageInspectionV4::Absent)
                    | (
                        1,
                        PmPhaseAAttemptLineageInspectionV4::PreparedUnclaimed { .. }
                    )
                    | (
                        2,
                        PmPhaseAAttemptLineageInspectionV4::ClaimOnlyBurned { .. }
                    )
                    | (
                        3,
                        PmPhaseAAttemptLineageInspectionV4::BurnedAwaitingFinalConjunct { .. }
                    )
                    | (4, PmPhaseAAttemptLineageInspectionV4::Complete { .. })
            ));
        }
    }

    #[test]
    fn claim_presence_and_torn_or_invalid_bytes_are_always_ambiguous_burned() {
        let (prepared, claim, _, _) = fixture_records();
        let directory = protected_directory();
        let ledger_path = directory
            .path()
            .join(PM_PHASE_A_ATTEMPT_LINEAGE_LEDGER_FILE_V4);
        let claim_path = directory.path().join(PM_PHASE_A_ATTEMPT_BURN_CLAIM_FILE_V4);
        write_file(
            &ledger_path,
            MAX_PHASE_A_ATTEMPT_LINEAGE_LEDGER_BYTES_V4,
            &encode_ledger_line(&prepared).unwrap(),
        );
        write_file(
            &claim_path,
            MAX_PHASE_A_ATTEMPT_BURN_CLAIM_BYTES_V4,
            b"{\"torn\":true}",
        );
        assert_eq!(
            inspect_paths(&ledger_path, &claim_path),
            PmPhaseAAttemptLineageInspectionV4::AmbiguousBurned
        );

        fs::write(&claim_path, encode_claim(&claim).unwrap()).unwrap();
        let mut ledger = encode_ledger_line(&prepared).unwrap();
        ledger.extend_from_slice(b"{}\n");
        fs::write(&ledger_path, ledger).unwrap();
        assert_eq!(
            inspect_paths(&ledger_path, &claim_path),
            PmPhaseAAttemptLineageInspectionV4::AmbiguousBurned
        );
    }

    #[test]
    fn consumed_or_complete_ledger_without_claim_is_always_ambiguous_burned() {
        let (prepared, _, consumed, final_conjunct) = fixture_records();
        let prepared_bytes = encode_ledger_line(&prepared).unwrap();
        let consumed_bytes = encode_ledger_line(&consumed).unwrap();
        let final_bytes = encode_ledger_line(&final_conjunct).unwrap();

        for include_final_conjunct in [false, true] {
            let directory = protected_directory();
            let ledger_path = directory
                .path()
                .join(PM_PHASE_A_ATTEMPT_LINEAGE_LEDGER_FILE_V4);
            let claim_path = directory.path().join(PM_PHASE_A_ATTEMPT_BURN_CLAIM_FILE_V4);
            let mut ledger_bytes = prepared_bytes.clone();
            ledger_bytes.extend_from_slice(&consumed_bytes);
            if include_final_conjunct {
                ledger_bytes.extend_from_slice(&final_bytes);
            }
            write_file(
                &ledger_path,
                MAX_PHASE_A_ATTEMPT_LINEAGE_LEDGER_BYTES_V4,
                &ledger_bytes,
            );
            assert_eq!(
                inspect_paths(&ledger_path, &claim_path),
                PmPhaseAAttemptLineageInspectionV4::AmbiguousBurned
            );
        }
    }

    #[test]
    fn inspector_lease_acquisition_failure_is_ambiguous_burned() {
        let (prepared, _, _, _) = fixture_records();
        let directory = protected_directory();
        let ledger_path = directory
            .path()
            .join(PM_PHASE_A_ATTEMPT_LINEAGE_LEDGER_FILE_V4);
        let claim_path = directory.path().join(PM_PHASE_A_ATTEMPT_BURN_CLAIM_FILE_V4);
        write_file(
            &ledger_path,
            MAX_PHASE_A_ATTEMPT_LINEAGE_LEDGER_BYTES_V4,
            &encode_ledger_line(&prepared).unwrap(),
        );
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            inspect_paths(&ledger_path, &claim_path),
            PmPhaseAAttemptLineageInspectionV4::AmbiguousBurned
        );
    }

    #[test]
    fn empty_and_torn_create_new_prefixes_classify_fail_closed() {
        let (prepared, claim, _, _) = fixture_records();

        let before_claim = protected_directory();
        let ledger_path = before_claim
            .path()
            .join(PM_PHASE_A_ATTEMPT_LINEAGE_LEDGER_FILE_V4);
        let claim_path = before_claim
            .path()
            .join(PM_PHASE_A_ATTEMPT_BURN_CLAIM_FILE_V4);
        drop(
            ProtectedJournal::create_new(&ledger_path, MAX_PHASE_A_ATTEMPT_LINEAGE_LEDGER_BYTES_V4)
                .unwrap(),
        );
        assert_eq!(
            inspect_paths(&ledger_path, &claim_path),
            PmPhaseAAttemptLineageInspectionV4::AmbiguousBeforeClaim
        );

        let after_claim = protected_directory();
        let ledger_path = after_claim
            .path()
            .join(PM_PHASE_A_ATTEMPT_LINEAGE_LEDGER_FILE_V4);
        let claim_path = after_claim
            .path()
            .join(PM_PHASE_A_ATTEMPT_BURN_CLAIM_FILE_V4);
        write_file(
            &ledger_path,
            MAX_PHASE_A_ATTEMPT_LINEAGE_LEDGER_BYTES_V4,
            &encode_ledger_line(&prepared).unwrap(),
        );
        drop(
            ProtectedJournal::create_new(&claim_path, MAX_PHASE_A_ATTEMPT_BURN_CLAIM_BYTES_V4)
                .unwrap(),
        );
        assert_eq!(
            inspect_paths(&ledger_path, &claim_path),
            PmPhaseAAttemptLineageInspectionV4::AmbiguousBurned
        );

        fs::write(&claim_path, encode_claim(&claim).unwrap()).unwrap();
        fs::write(&ledger_path, b"{\"record\":\"consumed\"").unwrap();
        assert_eq!(
            inspect_paths(&ledger_path, &claim_path),
            PmPhaseAAttemptLineageInspectionV4::AmbiguousBurned
        );
    }

    #[test]
    fn duplicate_unknown_and_trailing_data_are_rejected() {
        let (prepared, claim, consumed, _) = fixture_records();
        let prepared_bytes = encode_ledger_line(&prepared).unwrap();
        let claim_bytes = encode_claim(&claim).unwrap();

        let duplicate = String::from_utf8(prepared_bytes.clone()).unwrap().replacen(
            '{',
            "{\"schema_version\":4,",
            1,
        );
        assert!(parse_ledger(duplicate.as_bytes()).is_err());
        let unknown =
            String::from_utf8(prepared_bytes)
                .unwrap()
                .replacen('{', "{\"unknown\":false,", 1);
        assert!(parse_ledger(unknown.as_bytes()).is_err());
        let mut trailing = claim_bytes.clone();
        trailing.extend_from_slice(b"{}\n");
        assert!(parse_claim(&trailing).is_err());
        let duplicate_claim =
            String::from_utf8(claim_bytes)
                .unwrap()
                .replacen('{', "{\"schema_version\":4,", 1);
        assert!(parse_claim(duplicate_claim.as_bytes()).is_err());

        let mut foreign_consumed = consumed;
        foreign_consumed.previous_record_fingerprint = "ee".repeat(32);
        foreign_consumed.record_fingerprint = ZERO_FINGERPRINT.to_owned();
        foreign_consumed.record_fingerprint = foreign_consumed.calculate_fingerprint().unwrap();
        foreign_consumed.validate_structural().unwrap();
        let mut foreign_chain = encode_ledger_line(&prepared).unwrap();
        foreign_chain.extend_from_slice(&encode_ledger_line(&foreign_consumed).unwrap());
        assert!(parse_ledger(&foreign_chain).is_err());
    }
}
