use std::{fs, io, path::PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    AuthorizationHostBinding, CanonicalAuthorization, CanonicalTrialConfig,
    OfflineAuthorizationState, TrialPhase,
    protected_file::{
        DurableCreateNewFile, ProtectedFileError, ProtectedFileKind, create_new,
        open_existing_append, read_one,
    },
    verify_authorization,
};

const CONSUMPTION_SCHEMA_VERSION: u32 = 1;
const MAX_CONSUMPTION_EVIDENCE_BYTES: usize = 128 * 1024;
const MAX_CONSUMPTION_RECORDS: usize = 32;
const MAX_RECOVERY_PREPARED_RECORD_BYTES: usize = 64 * 1024;
const ZERO_FINGERPRINT: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const BINDING_FINGERPRINT_DOMAIN: &[u8] = b"reap.pm-t2.authorization-consumption.binding.v1\0";
const RECORD_FINGERPRINT_DOMAIN: &[u8] = b"reap.pm-t2.authorization-consumption.record.v1\0";
const CLAIM_FINGERPRINT_DOMAIN: &[u8] = b"reap.pm-t2.authorization-consumption.claim.v1\0";
const RECOVERY_CONTINUATION_ROOT_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.authorization-consumption.recovery-continuation-root.v1\0";
const RECOVERY_CANCEL_PREPARED_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.authorization-consumption.recovery-cancel-prepared.v1\0";
const RECOVERY_TERMINAL_PLAN_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.authorization-consumption.recovery-terminal-plan.v1\0";

/// Non-secret values the future runner must re-observe immediately before a
/// Prepared or Consumed transition. No value in this type grants authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationRuntimeBinding {
    pub release_binary_sha256: String,
    pub release_binary_length: u64,
    pub host: AuthorizationHostBinding,
    pub observed_at_utc: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationConsumptionBindingEvidence {
    pub authorization_id: String,
    pub phase: TrialPhase,
    pub authorization_fingerprint: String,
    pub canonical_config_sha256: String,
    pub canonical_config_length: u64,
    pub canonical_config_fingerprint: String,
    pub trial_plan_fingerprint: String,
    pub release_binary_sha256: String,
    pub release_binary_length: u64,
    pub host: AuthorizationHostBinding,
    pub authorization_not_before_utc: String,
    pub authorization_expires_at_utc: String,
    pub artifact_directory: String,
    pub journal_family: String,
    pub journal_version: u32,
    pub credential_slot_id: String,
    pub credential_slot_nonsecret_fingerprint_sha256: String,
    pub ledger_file: String,
    pub consume_claim_file: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalDisposition {
    Completed,
    Stopped,
    OperatorActionRequired,
}

/// Durable monotonic root for the separate recovery-only cancel continuation.
/// It is evidence in the already-burned consumption ledger, never authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationRecoveryContinuationRootV1 {
    pub continuation_scope_fingerprint: String,
    pub continuation_intent_header_fingerprint: String,
    pub continuation_dispatch_header_fingerprint: String,
    pub previous_anchor_fingerprint: String,
    pub anchor_fingerprint: String,
}

/// One monotonically burned recovery-cancel preparation. The exact
/// continuation record fingerprint covers its request semantics; the
/// duplicated fields make cross-family validation explicit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationRecoveryCancelPreparedAnchorV1 {
    pub continuation_scope_fingerprint: String,
    pub recovery_ordinal: u8,
    pub continuation_intent_sequence: u8,
    pub continuation_intent_record_fingerprint: String,
    pub continuation_prepared_sequence: u8,
    pub continuation_dispatch_previous_record_fingerprint: String,
    pub continuation_prepared_record_fingerprint: String,
    pub continuation_prepared_record_canonical_json: String,
    pub exact_venue_order_id: String,
    pub semantic_request_commitment_sha256: String,
    pub l2_timestamp_seconds: u64,
    pub previous_anchor_fingerprint: String,
    pub anchor_fingerprint: String,
}

/// Sole source-owned plan for closing the recovery-only continuation. The
/// exact canonical records are anchored in the monotonic consumption ledger
/// before either continuation Terminal half may be appended. This is durable
/// evidence only and never grants authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationRecoveryTerminalPlanV1 {
    pub continuation_scope_fingerprint: String,
    pub continuation_intent_predecessor_sequence: u8,
    pub continuation_intent_predecessor_record_fingerprint: String,
    pub continuation_dispatch_predecessor_sequence: u8,
    pub continuation_dispatch_predecessor_record_fingerprint: String,
    pub terminal_at_utc: String,
    pub disposition: TerminalDisposition,
    pub continuation_dispatch_terminal_sequence: u8,
    pub continuation_dispatch_terminal_previous_record_fingerprint: String,
    pub continuation_dispatch_terminal_record_fingerprint: String,
    pub continuation_dispatch_terminal_record_canonical_json: String,
    pub continuation_intent_terminal_sequence: u8,
    pub continuation_intent_terminal_previous_record_fingerprint: String,
    pub continuation_intent_terminal_record_fingerprint: String,
    pub continuation_intent_terminal_record_canonical_json: String,
    pub previous_anchor_fingerprint: String,
    pub anchor_fingerprint: String,
}

/// Crash-recovery monotonic registry for the recovery-only continuation.
///
/// The fsynced consumption ledger is the non-rollback trust boundary: ordinary
/// crash/power-loss durability and trusted local-filesystem semantics are in
/// scope. Coordinated valid-prefix rollback of this ledger plus every bound
/// artifact by a post-crash same-privilege adversary is not locally detectable
/// without TPM, remote, or WORM state and is outside this profile's model.
/// Every field is deliberately non-secret evidence, so `Debug` may show it;
/// this type never contains credentials, authenticated requests, or authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthorizationRecoveryContinuationRegistryV1 {
    pub root: AuthorizationRecoveryContinuationRootV1,
    pub prepared: Vec<AuthorizationRecoveryCancelPreparedAnchorV1>,
    pub terminal_plan: Option<AuthorizationRecoveryTerminalPlanV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuthorizationConsumptionState {
    Prepared {
        verified_at_utc: String,
    },
    Consumed {
        consumed_at_utc: String,
        burned_before_dispatch_authority: bool,
        crash_allows_recovery_cancel_only: bool,
        placement_can_never_resume: bool,
    },
    RecoveryContinuationRootAnchored {
        root: AuthorizationRecoveryContinuationRootV1,
        recovery_cancel_only: bool,
        placement_can_never_resume: bool,
    },
    RecoveryCancelPreparedAnchored {
        anchor: AuthorizationRecoveryCancelPreparedAnchorV1,
        recovery_cancel_only: bool,
        placement_can_never_resume: bool,
    },
    RecoveryTerminalPlanAnchored {
        plan: AuthorizationRecoveryTerminalPlanV1,
        recovery_cancel_only: bool,
        placement_can_never_resume: bool,
    },
    Terminal {
        terminal_at_utc: String,
        disposition: TerminalDisposition,
        terminal_is_evidence_not_authority: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationConsumptionEvidence {
    pub schema_version: u32,
    pub sequence: u8,
    pub previous_record_fingerprint: String,
    pub binding_fingerprint: String,
    pub binding: AuthorizationConsumptionBindingEvidence,
    pub consumption: AuthorizationConsumptionState,
    pub authorization: OfflineAuthorizationState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AtomicConsumptionClaim {
    schema_version: u32,
    prepared_record_fingerprint: String,
    binding_fingerprint: String,
    binding: AuthorizationConsumptionBindingEvidence,
    consumed_at_utc: String,
    burned_before_dispatch_authority: bool,
    crash_allows_recovery_cancel_only: bool,
    placement_can_never_resume: bool,
    authorization: OfflineAuthorizationState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthorizationConsumptionVerification {
    pub schema_version: u32,
    pub state: AuthorizationConsumptionState,
    pub ledger_record_count: u8,
    pub atomic_consumption_claim_durable: bool,
    pub consumed_ledger_record_durable: bool,
    pub latest_record_fingerprint: String,
    pub claim_fingerprint: Option<String>,
    pub binding_fingerprint: String,
    pub exact_bindings_structurally_valid: bool,
    pub ambiguous_tail: bool,
    pub recovery_continuation_registry: Option<AuthorizationRecoveryContinuationRegistryV1>,
    #[serde(flatten)]
    pub authorization: OfflineAuthorizationState,
}

/// Move-only Prepared evidence owner. It is not a dispatch permit.
pub struct PreparedAuthorizationConsumption {
    journal: DurableCreateNewFile,
    ledger_bytes: Vec<u8>,
    prepared_record: AuthorizationConsumptionEvidence,
    binding: AuthorizationConsumptionBindingEvidence,
    claim_path: PathBuf,
}

impl PreparedAuthorizationConsumption {
    #[must_use]
    pub const fn evidence(&self) -> &AuthorizationConsumptionEvidence {
        &self.prepared_record
    }

    /// Atomically burn the authorization before a future runner may consider
    /// constructing any authenticated dispatch grant or signed-send capability.
    pub fn consume(
        self,
        config: &CanonicalTrialConfig,
        authorization: &CanonicalAuthorization,
        runtime: &AuthorizationRuntimeBinding,
    ) -> Result<ConsumedAuthorizationConsumption, PmAuthorizationConsumptionError> {
        consume_prepared(self, config, authorization, runtime)
    }
}

/// Move-only proof that the one place attempt is burned. This type provides no
/// placement API; after a crash, only later recovery/cancel code may proceed.
pub struct ConsumedAuthorizationConsumption {
    journal: DurableCreateNewFile,
    ledger_bytes: Vec<u8>,
    claim: DurableCreateNewFile,
    claim_bytes: Vec<u8>,
    consumed_record: AuthorizationConsumptionEvidence,
    binding: AuthorizationConsumptionBindingEvidence,
    consumed_at_utc: String,
    recovery_cancel_dispatch_budget: u8,
    recovery_continuation_registry: Option<AuthorizationRecoveryContinuationRegistryV1>,
}

impl ConsumedAuthorizationConsumption {
    #[must_use]
    pub const fn evidence(&self) -> &AuthorizationConsumptionEvidence {
        &self.consumed_record
    }

    #[must_use]
    pub const fn recovery_continuation_registry(
        &self,
    ) -> Option<&AuthorizationRecoveryContinuationRegistryV1> {
        self.recovery_continuation_registry.as_ref()
    }

    /// Revalidate the held fixed ledger and atomic-claim descriptors, their
    /// recoverable path identities, lengths, and exact durable bytes. This is
    /// evidence custody only and does not grant dispatch authority.
    pub fn revalidate_held_consumption_evidence(
        &mut self,
    ) -> Result<(), PmAuthorizationConsumptionError> {
        self.claim
            .validate_exact_bytes(&self.claim_bytes)
            .and_then(|()| self.journal.validate_exact_bytes(&self.ledger_bytes))
            .map_err(|_| {
                PmAuthorizationConsumptionError::Ambiguous(
                    "held authorization-consumption ledger or claim changed",
                )
            })
    }

    /// Refresh both held parent-directory snapshots after the caller durably
    /// creates one separately bound artifact, then immediately revalidate the
    /// fixed ledger and claim identities and bytes.
    pub fn refresh_after_bound_artifact_create(
        &mut self,
    ) -> Result<(), PmAuthorizationConsumptionError> {
        self.claim
            .refresh_parent_after_bound_create()
            .and_then(|()| self.journal.refresh_parent_after_bound_create())
            .map_err(|_| {
                PmAuthorizationConsumptionError::Ambiguous(
                    "authorization-consumption parent changed after bound artifact creation",
                )
            })?;
        self.revalidate_held_consumption_evidence()
    }

    /// Append, or idempotently confirm, the sole recovery-continuation root.
    /// The continuation headers must already be durable; no recovery cancel
    /// owner may be exposed until this append completes.
    pub fn anchor_recovery_continuation_root(
        &mut self,
        continuation_scope_fingerprint: &str,
        continuation_intent_header_fingerprint: &str,
        continuation_dispatch_header_fingerprint: &str,
    ) -> Result<(), PmAuthorizationConsumptionError> {
        self.revalidate_held_consumption_evidence()?;
        let root = make_recovery_continuation_root(
            continuation_scope_fingerprint,
            continuation_intent_header_fingerprint,
            continuation_dispatch_header_fingerprint,
        )?;
        if let Some(existing) = &self.recovery_continuation_registry {
            return (existing.root == root)
                .then_some(())
                .ok_or_else(|| invalid("recovery-continuation root anchor conflicts"));
        }
        self.append_recovery_registry_record(
            AuthorizationConsumptionState::RecoveryContinuationRootAnchored {
                root: root.clone(),
                recovery_cancel_only: true,
                placement_can_never_resume: true,
            },
        )?;
        self.recovery_continuation_registry = Some(AuthorizationRecoveryContinuationRegistryV1 {
            root,
            prepared: Vec::new(),
            terminal_plan: None,
        });
        Ok(())
    }

    /// Burn one exact sequential recovery-cancel preparation in the
    /// independent ledger registry before its typed dispatch owner can exist.
    #[allow(clippy::too_many_arguments)]
    pub fn anchor_recovery_cancel_prepared(
        &mut self,
        continuation_scope_fingerprint: &str,
        recovery_ordinal: u8,
        continuation_intent_sequence: u8,
        continuation_intent_record_fingerprint: &str,
        continuation_prepared_sequence: u8,
        continuation_dispatch_previous_record_fingerprint: &str,
        continuation_prepared_record_fingerprint: &str,
        continuation_prepared_record_canonical_json: &str,
        exact_venue_order_id: &str,
        semantic_request_commitment_sha256: &str,
        l2_timestamp_seconds: u64,
    ) -> Result<(), PmAuthorizationConsumptionError> {
        self.revalidate_held_consumption_evidence()?;
        let registry = self
            .recovery_continuation_registry
            .as_ref()
            .ok_or_else(|| invalid("recovery-continuation root is not anchored"))?;
        if registry.terminal_plan.is_some() {
            return Err(invalid(
                "recovery preparation cannot follow its Terminal plan",
            ));
        }
        if continuation_scope_fingerprint != registry.root.continuation_scope_fingerprint {
            return Err(invalid(
                "recovery preparation has a foreign continuation scope",
            ));
        }
        if recovery_ordinal == 0 || recovery_ordinal > self.recovery_cancel_dispatch_budget {
            return Err(invalid("recovery preparation anchor ordinal is not next"));
        }
        let anchor_index = usize::from(recovery_ordinal - 1);
        if let Some(prior) = anchor_index
            .checked_sub(1)
            .and_then(|index| registry.prepared.get(index))
            && (continuation_intent_sequence <= prior.continuation_intent_sequence
                || continuation_prepared_sequence <= prior.continuation_prepared_sequence
                || l2_timestamp_seconds <= prior.l2_timestamp_seconds
                || exact_venue_order_id != prior.exact_venue_order_id
                || semantic_request_commitment_sha256 != prior.semantic_request_commitment_sha256)
        {
            return Err(invalid(
                "recovery preparation anchor does not advance its exact prior lineage",
            ));
        }
        if let Some(existing) = registry.prepared.get(anchor_index) {
            let previous_anchor_fingerprint = anchor_index
                .checked_sub(1)
                .and_then(|prior| registry.prepared.get(prior))
                .map(|prior| prior.anchor_fingerprint.clone())
                .unwrap_or_else(|| registry.root.anchor_fingerprint.clone());
            let anchor = make_recovery_cancel_prepared_anchor(
                continuation_scope_fingerprint,
                recovery_ordinal,
                continuation_intent_sequence,
                continuation_intent_record_fingerprint,
                continuation_prepared_sequence,
                continuation_dispatch_previous_record_fingerprint,
                continuation_prepared_record_fingerprint,
                continuation_prepared_record_canonical_json,
                exact_venue_order_id,
                semantic_request_commitment_sha256,
                l2_timestamp_seconds,
                previous_anchor_fingerprint,
            )?;
            return (existing == &anchor)
                .then_some(())
                .ok_or_else(|| invalid("recovery preparation anchor conflicts"));
        }
        if anchor_index != registry.prepared.len() {
            return Err(invalid("recovery preparation anchor ordinal is not next"));
        }
        let previous_anchor_fingerprint = registry
            .prepared
            .last()
            .map(|anchor| anchor.anchor_fingerprint.clone())
            .unwrap_or_else(|| registry.root.anchor_fingerprint.clone());
        let anchor = make_recovery_cancel_prepared_anchor(
            continuation_scope_fingerprint,
            recovery_ordinal,
            continuation_intent_sequence,
            continuation_intent_record_fingerprint,
            continuation_prepared_sequence,
            continuation_dispatch_previous_record_fingerprint,
            continuation_prepared_record_fingerprint,
            continuation_prepared_record_canonical_json,
            exact_venue_order_id,
            semantic_request_commitment_sha256,
            l2_timestamp_seconds,
            previous_anchor_fingerprint,
        )?;
        self.append_recovery_registry_record(
            AuthorizationConsumptionState::RecoveryCancelPreparedAnchored {
                anchor: anchor.clone(),
                recovery_cancel_only: true,
                placement_can_never_resume: true,
            },
        )?;
        self.recovery_continuation_registry
            .as_mut()
            .ok_or_else(|| invalid("recovery-continuation root disappeared"))?
            .prepared
            .push(anchor);
        Ok(())
    }

    /// Anchor the sole exact recovery-continuation Terminal plan before
    /// either continuation journal may append a Terminal record. Repeating
    /// the exact plan is idempotent; any competing time, disposition, causal
    /// predecessor, or canonical record fails closed.
    #[allow(clippy::too_many_arguments)]
    pub fn anchor_recovery_terminal_plan(
        &mut self,
        continuation_scope_fingerprint: &str,
        continuation_intent_predecessor_sequence: u8,
        continuation_intent_predecessor_record_fingerprint: &str,
        continuation_dispatch_predecessor_sequence: u8,
        continuation_dispatch_predecessor_record_fingerprint: &str,
        terminal_at_utc: &str,
        disposition: TerminalDisposition,
        continuation_dispatch_terminal_sequence: u8,
        continuation_dispatch_terminal_previous_record_fingerprint: &str,
        continuation_dispatch_terminal_record_fingerprint: &str,
        continuation_dispatch_terminal_record_canonical_json: &str,
        continuation_intent_terminal_sequence: u8,
        continuation_intent_terminal_previous_record_fingerprint: &str,
        continuation_intent_terminal_record_fingerprint: &str,
        continuation_intent_terminal_record_canonical_json: &str,
    ) -> Result<(), PmAuthorizationConsumptionError> {
        self.revalidate_held_consumption_evidence()?;
        let registry = self
            .recovery_continuation_registry
            .as_ref()
            .ok_or_else(|| invalid("recovery-continuation root is not anchored"))?;
        if continuation_scope_fingerprint != registry.root.continuation_scope_fingerprint {
            return Err(invalid("recovery Terminal plan has a foreign scope"));
        }
        if parse_canonical_utc(terminal_at_utc)? < parse_canonical_utc(&self.consumed_at_utc)? {
            return Err(invalid("recovery Terminal plan precedes consumption"));
        }
        let previous_anchor_fingerprint = registry
            .prepared
            .last()
            .map(|anchor| anchor.anchor_fingerprint.clone())
            .unwrap_or_else(|| registry.root.anchor_fingerprint.clone());
        let plan = make_recovery_terminal_plan(
            continuation_scope_fingerprint,
            continuation_intent_predecessor_sequence,
            continuation_intent_predecessor_record_fingerprint,
            continuation_dispatch_predecessor_sequence,
            continuation_dispatch_predecessor_record_fingerprint,
            terminal_at_utc,
            disposition,
            continuation_dispatch_terminal_sequence,
            continuation_dispatch_terminal_previous_record_fingerprint,
            continuation_dispatch_terminal_record_fingerprint,
            continuation_dispatch_terminal_record_canonical_json,
            continuation_intent_terminal_sequence,
            continuation_intent_terminal_previous_record_fingerprint,
            continuation_intent_terminal_record_fingerprint,
            continuation_intent_terminal_record_canonical_json,
            previous_anchor_fingerprint,
        )?;
        if let Some(existing) = &registry.terminal_plan {
            return (existing == &plan)
                .then_some(())
                .ok_or_else(|| invalid("recovery Terminal plan conflicts"));
        }
        self.append_recovery_registry_record(
            AuthorizationConsumptionState::RecoveryTerminalPlanAnchored {
                plan: plan.clone(),
                recovery_cancel_only: true,
                placement_can_never_resume: true,
            },
        )?;
        self.recovery_continuation_registry
            .as_mut()
            .ok_or_else(|| invalid("recovery-continuation root disappeared"))?
            .terminal_plan = Some(plan);
        Ok(())
    }

    fn append_recovery_registry_record(
        &mut self,
        consumption: AuthorizationConsumptionState,
    ) -> Result<(), PmAuthorizationConsumptionError> {
        let records = parse_ledger(&self.ledger_bytes)?;
        let sequence = u8::try_from(records.len())
            .map_err(|_| invalid("recovery registry sequence bound exceeded"))?;
        let previous = records
            .last()
            .map(record_fingerprint)
            .transpose()?
            .ok_or_else(|| invalid("consumed ledger is empty"))?;
        let record = make_record(sequence, previous, self.binding.clone(), consumption)?;
        let encoded = encode_ledger_record(&record)?;
        self.journal
            .append_durable(&self.ledger_bytes, &encoded)
            .map_err(|_| {
                PmAuthorizationConsumptionError::Ambiguous(
                    "recovery registry append or fsync failed; attempt remains burned",
                )
            })?;
        self.ledger_bytes.extend_from_slice(&encoded);
        Ok(())
    }

    pub fn terminal(
        mut self,
        terminal_at_utc: String,
        disposition: TerminalDisposition,
    ) -> Result<TerminalAuthorizationConsumption, PmAuthorizationConsumptionError> {
        self.revalidate_held_consumption_evidence()?;
        if self.recovery_continuation_registry.is_some() {
            return Err(invalid(
                "base Terminal is forbidden after recovery-continuation anchoring",
            ));
        }
        let consumed_at = parse_canonical_utc(&self.consumed_at_utc)?;
        let terminal_at = parse_canonical_utc(&terminal_at_utc)?;
        if terminal_at < consumed_at {
            return Err(invalid("terminal time precedes consumption"));
        }
        let record = make_record(
            2,
            record_fingerprint(&self.consumed_record)?,
            self.binding,
            AuthorizationConsumptionState::Terminal {
                terminal_at_utc,
                disposition,
                terminal_is_evidence_not_authority: true,
            },
        )?;
        let encoded = encode_ledger_record(&record)?;
        self.journal
            .append_durable(&self.ledger_bytes, &encoded)
            .map_err(|_| {
                PmAuthorizationConsumptionError::TerminalEvidenceAmbiguous(
                    "terminal append or fsync failed; consumption remains burned",
                )
            })?;
        Ok(TerminalAuthorizationConsumption { evidence: record })
    }
}

pub struct TerminalAuthorizationConsumption {
    evidence: AuthorizationConsumptionEvidence,
}

impl TerminalAuthorizationConsumption {
    #[must_use]
    pub const fn evidence(&self) -> &AuthorizationConsumptionEvidence {
        &self.evidence
    }
}

pub fn prepare_authorization_consumption(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    runtime: &AuthorizationRuntimeBinding,
) -> Result<PreparedAuthorizationConsumption, PmAuthorizationConsumptionError> {
    let binding = validated_binding(config, authorization, runtime)?;
    let (ledger_path, claim_path) = bound_paths(config);
    require_claim_absent(&claim_path)?;
    let binding_fingerprint = binding_fingerprint(&binding)?;
    let record = AuthorizationConsumptionEvidence {
        schema_version: CONSUMPTION_SCHEMA_VERSION,
        sequence: 0,
        previous_record_fingerprint: ZERO_FINGERPRINT.to_owned(),
        binding_fingerprint,
        binding: binding.clone(),
        consumption: AuthorizationConsumptionState::Prepared {
            verified_at_utc: runtime.observed_at_utc.clone(),
        },
        authorization: OfflineAuthorizationState::DENIED,
    };
    let encoded = encode_ledger_record(&record)?;
    let mut journal = create_new(
        &ledger_path,
        ProtectedFileKind::ConsumptionEvidence,
        MAX_CONSUMPTION_EVIDENCE_BYTES,
    )
    .map_err(map_prepare_create_error)?;
    journal.append_durable(&[], &encoded).map_err(|_| {
        PmAuthorizationConsumptionError::Ambiguous(
            "Prepared ledger creation or fsync is incomplete; reuse is forbidden",
        )
    })?;
    Ok(PreparedAuthorizationConsumption {
        journal,
        ledger_bytes: encoded,
        prepared_record: record,
        binding,
        claim_path,
    })
}

/// Restart entry: atomically claim the one exact Prepared ledger. Concurrent
/// callers race on one bound create-new marker, so at most one can succeed.
pub fn claim_prepared_authorization_consumption(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    runtime: &AuthorizationRuntimeBinding,
) -> Result<ConsumedAuthorizationConsumption, PmAuthorizationConsumptionError> {
    let expected_binding = validated_binding(config, authorization, runtime)?;
    let (ledger_path, claim_path) = bound_paths(config);
    let ledger = read_one(
        &ledger_path,
        ProtectedFileKind::ConsumptionEvidence,
        MAX_CONSUMPTION_EVIDENCE_BYTES,
    )
    .map_err(|_| invalid("bound Prepared ledger is absent, unprotected, or unstable"))?;
    let records = parse_ledger(&ledger)?;
    if records.len() != 1
        || !matches!(
            &records[0].consumption,
            AuthorizationConsumptionState::Prepared { .. }
        )
    {
        return Err(PmAuthorizationConsumptionError::AlreadyConsumed(
            "bound ledger is not one resumable Prepared state",
        ));
    }
    validate_records(config, authorization, &records)?;
    if records[0].binding != expected_binding {
        return Err(invalid(
            "restart runtime does not match the bound Prepared ledger",
        ));
    }
    let journal = open_existing_append(
        &ledger_path,
        ProtectedFileKind::ConsumptionEvidence,
        MAX_CONSUMPTION_EVIDENCE_BYTES,
    )
    .map_err(|_| invalid("bound Prepared ledger cannot be pinned for append"))?;
    let prepared = PreparedAuthorizationConsumption {
        journal,
        ledger_bytes: ledger.to_vec(),
        prepared_record: records[0].clone(),
        binding: expected_binding,
        claim_path,
    };
    consume_prepared(prepared, config, authorization, runtime)
}

/// Recovery/cancel-only entry: reopen and pin the exact durable Consumed
/// ledger and its atomic claim without recreating placement authority.
///
/// This accepts the complete Prepared -> Consumed prefix plus an optional
/// validated recovery-continuation registry. A missing claim, an incomplete
/// consume, Terminal evidence, or any binding or byte drift fails closed.
pub fn reopen_consumed_authorization_consumption(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
) -> Result<ConsumedAuthorizationConsumption, PmAuthorizationConsumptionError> {
    let (ledger_path, claim_path) = bound_paths(config);
    let ledger_bytes = read_one(
        &ledger_path,
        ProtectedFileKind::ConsumptionEvidence,
        MAX_CONSUMPTION_EVIDENCE_BYTES,
    )
    .map_err(|_| invalid("bound Consumed ledger is absent, unprotected, or unstable"))?;
    let records = parse_ledger(&ledger_bytes)?;
    if records.len() < 2
        || !matches!(
            &records[1].consumption,
            AuthorizationConsumptionState::Consumed {
                burned_before_dispatch_authority: true,
                crash_allows_recovery_cancel_only: true,
                placement_can_never_resume: true,
                ..
            }
        )
        || records.iter().any(|record| {
            matches!(
                record.consumption,
                AuthorizationConsumptionState::Terminal { .. }
            )
        })
    {
        return Err(invalid(
            "recovery custody requires one complete non-Terminal Consumed ledger",
        ));
    }
    let validation = validate_records(config, authorization, &records)?;
    let claim = read_optional_claim(&claim_path)?
        .ok_or_else(|| invalid("Consumed recovery custody requires its atomic claim"))?;
    let prepared_record_fingerprint = record_fingerprint(&records[0])?;
    validate_claim(&claim, &records[0].binding, &prepared_record_fingerprint)?;
    let consumed_at_utc = consumed_time(&records[1].consumption)?.to_owned();
    if consumed_at_utc != claim.consumed_at_utc {
        return Err(invalid("consume claim and ledger timestamp disagree"));
    }
    let claim_bytes = canonical_json(&claim)?;
    let journal = open_existing_append(
        &ledger_path,
        ProtectedFileKind::ConsumptionEvidence,
        MAX_CONSUMPTION_EVIDENCE_BYTES,
    )
    .map_err(|_| invalid("bound Consumed ledger cannot be pinned"))?;
    let claim_file = open_existing_append(
        &claim_path,
        ProtectedFileKind::ConsumptionEvidence,
        MAX_CONSUMPTION_EVIDENCE_BYTES,
    )
    .map_err(|_| invalid("bound atomic consume claim cannot be pinned"))?;
    let mut owner = ConsumedAuthorizationConsumption {
        journal,
        ledger_bytes: ledger_bytes.to_vec(),
        claim: claim_file,
        claim_bytes,
        consumed_record: records[1].clone(),
        binding: records[0].binding.clone(),
        consumed_at_utc,
        recovery_cancel_dispatch_budget: config.value().order.recovery_cancel_dispatch_budget,
        recovery_continuation_registry: validation.recovery_continuation_registry,
    };
    owner.revalidate_held_consumption_evidence()?;
    Ok(owner)
}

pub fn verify_authorization_consumption(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
) -> Result<AuthorizationConsumptionVerification, PmAuthorizationConsumptionError> {
    let (ledger_path, claim_path) = bound_paths(config);
    let ledger = read_one(
        &ledger_path,
        ProtectedFileKind::ConsumptionEvidence,
        MAX_CONSUMPTION_EVIDENCE_BYTES,
    )
    .map_err(|_| invalid("bound consumption ledger is absent, unprotected, or unstable"))?;
    let records = parse_ledger(&ledger)?;
    let validation = validate_records(config, authorization, &records)?;
    let claim = read_optional_claim(&claim_path)?;
    let claim_fingerprint = claim.as_ref().map(claim_fingerprint).transpose()?;
    let prepared_fingerprint = record_fingerprint(&records[0])?;
    if let Some(claim) = &claim {
        validate_claim(claim, &records[0].binding, &prepared_fingerprint)?;
    }

    let (state, claim_durable, consumed_record_durable) = if records.len() == 1 {
        match claim {
            None => (records[0].consumption.clone(), false, false),
            Some(claim) => (consumed_state(claim.consumed_at_utc), true, false),
        }
    } else {
        let consumed = &records[1];
        let claim = claim.ok_or_else(|| {
            invalid("Consumed ledger record exists without its atomic consume claim")
        })?;
        if consumed_time(&consumed.consumption)? != claim.consumed_at_utc {
            return Err(invalid("consume claim and ledger timestamp disagree"));
        }
        let state = match records.last().map(|record| &record.consumption) {
            Some(AuthorizationConsumptionState::Terminal { .. }) => records
                .last()
                .ok_or_else(|| invalid("consumption ledger is empty"))?
                .consumption
                .clone(),
            _ => consumed.consumption.clone(),
        };
        (state, true, true)
    };
    Ok(AuthorizationConsumptionVerification {
        schema_version: CONSUMPTION_SCHEMA_VERSION,
        state,
        ledger_record_count: records.len() as u8,
        atomic_consumption_claim_durable: claim_durable,
        consumed_ledger_record_durable: consumed_record_durable,
        latest_record_fingerprint: validation.latest_record_fingerprint,
        claim_fingerprint,
        binding_fingerprint: records[0].binding_fingerprint.clone(),
        exact_bindings_structurally_valid: true,
        ambiguous_tail: false,
        recovery_continuation_registry: validation.recovery_continuation_registry,
        authorization: OfflineAuthorizationState::DENIED,
    })
}

fn consume_prepared(
    mut prepared: PreparedAuthorizationConsumption,
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    runtime: &AuthorizationRuntimeBinding,
) -> Result<ConsumedAuthorizationConsumption, PmAuthorizationConsumptionError> {
    let binding = validated_binding(config, authorization, runtime)?;
    if binding != prepared.binding {
        return Err(invalid(
            "consume recheck does not match the exact Prepared binding",
        ));
    }
    let prepared_at = match &prepared.prepared_record.consumption {
        AuthorizationConsumptionState::Prepared { verified_at_utc } => {
            parse_canonical_utc(verified_at_utc)?
        }
        _ => return Err(invalid("in-memory owner is not in Prepared state")),
    };
    let consumed_at = parse_canonical_utc(&runtime.observed_at_utc)?;
    if consumed_at < prepared_at {
        return Err(invalid("consume recheck time precedes Prepared time"));
    }
    let prepared_record_fingerprint = record_fingerprint(&prepared.prepared_record)?;
    let claim = AtomicConsumptionClaim {
        schema_version: CONSUMPTION_SCHEMA_VERSION,
        prepared_record_fingerprint: prepared_record_fingerprint.clone(),
        binding_fingerprint: binding_fingerprint(&binding)?,
        binding: binding.clone(),
        consumed_at_utc: runtime.observed_at_utc.clone(),
        burned_before_dispatch_authority: true,
        crash_allows_recovery_cancel_only: true,
        placement_can_never_resume: true,
        authorization: OfflineAuthorizationState::DENIED,
    };
    let claim_bytes = canonical_json(&claim)?;
    let mut claim_file = create_new(
        &prepared.claim_path,
        ProtectedFileKind::ConsumptionEvidence,
        MAX_CONSUMPTION_EVIDENCE_BYTES,
    )
    .map_err(map_claim_create_error)?;
    claim_file.append_durable(&[], &claim_bytes).map_err(|_| {
        PmAuthorizationConsumptionError::BurnedEvidenceIncomplete(
            "consume claim creation is ambiguous; placement is burned and cannot resume",
        )
    })?;

    // The fsynced create-new claim above is the take-once linearization point.
    // Anything below is evidence completion only and cannot restore placement.
    prepared
        .journal
        .refresh_parent_after_bound_create()
        .map_err(|_| {
            PmAuthorizationConsumptionError::BurnedEvidenceIncomplete(
                "consume claim is durable but ledger parent changed; recovery/cancel only",
            )
        })?;
    let record = make_record(
        1,
        prepared_record_fingerprint,
        binding.clone(),
        consumed_state(runtime.observed_at_utc.clone()),
    )?;
    let encoded = encode_ledger_record(&record)?;
    prepared
        .journal
        .append_durable(&prepared.ledger_bytes, &encoded)
        .map_err(|_| {
            PmAuthorizationConsumptionError::BurnedEvidenceIncomplete(
                "consume claim is durable but ledger completion failed; recovery/cancel only",
            )
        })?;
    prepared.ledger_bytes.extend_from_slice(&encoded);
    Ok(ConsumedAuthorizationConsumption {
        journal: prepared.journal,
        ledger_bytes: prepared.ledger_bytes,
        claim: claim_file,
        claim_bytes,
        consumed_record: record,
        binding,
        consumed_at_utc: runtime.observed_at_utc.clone(),
        recovery_cancel_dispatch_budget: config.value().order.recovery_cancel_dispatch_budget,
        recovery_continuation_registry: None,
    })
}

fn validated_binding(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    runtime: &AuthorizationRuntimeBinding,
) -> Result<AuthorizationConsumptionBindingEvidence, PmAuthorizationConsumptionError> {
    validate_sha256(&runtime.release_binary_sha256)?;
    if runtime.release_binary_length == 0 {
        return Err(invalid("runtime binary length must be positive"));
    }
    let observed = parse_canonical_utc(&runtime.observed_at_utc)?;
    verify_authorization(config, authorization, observed)
        .map_err(|_| invalid("authorization is not valid for the exact runtime observation"))?;
    let record = authorization.value();
    if runtime.release_binary_sha256 != record.build.release_binary_sha256
        || runtime.release_binary_length != record.build.release_binary_length
        || runtime.host != record.host
    {
        return Err(invalid(
            "binary or host observation does not match authorization",
        ));
    }
    Ok(AuthorizationConsumptionBindingEvidence {
        authorization_id: record.authorization_id.clone(),
        phase: record.phase,
        authorization_fingerprint: authorization.fingerprint().to_owned(),
        canonical_config_sha256: config.canonical_sha256().to_owned(),
        canonical_config_length: config.canonical_length(),
        canonical_config_fingerprint: config.fingerprint().to_owned(),
        trial_plan_fingerprint: config.plan_fingerprint().to_owned(),
        release_binary_sha256: runtime.release_binary_sha256.clone(),
        release_binary_length: runtime.release_binary_length,
        host: runtime.host.clone(),
        authorization_not_before_utc: record.not_before_utc.clone(),
        authorization_expires_at_utc: record.expires_at_utc.clone(),
        artifact_directory: config.value().journal.artifact_directory.clone(),
        journal_family: config.value().journal.journal_family.clone(),
        journal_version: config.value().journal.journal_version,
        credential_slot_id: config.value().credential_slot.slot_id.clone(),
        credential_slot_nonsecret_fingerprint_sha256: config
            .value()
            .credential_slot
            .nonsecret_fingerprint_sha256
            .clone(),
        ledger_file: config
            .value()
            .journal
            .authorization_consumption_ledger_file
            .clone(),
        consume_claim_file: config
            .value()
            .journal
            .authorization_consumption_claim_file
            .clone(),
    })
}

struct ValidatedRecords {
    latest_record_fingerprint: String,
    recovery_continuation_registry: Option<AuthorizationRecoveryContinuationRegistryV1>,
}

fn validate_records(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    records: &[AuthorizationConsumptionEvidence],
) -> Result<ValidatedRecords, PmAuthorizationConsumptionError> {
    let maximum_records = 4_usize.saturating_add(usize::from(
        config.value().order.recovery_cancel_dispatch_budget,
    ));
    if records.is_empty() || records.len() > maximum_records {
        return Err(invalid("consumption ledger record count is invalid"));
    }
    let prepared_time = match &records[0].consumption {
        AuthorizationConsumptionState::Prepared { verified_at_utc } => verified_at_utc.clone(),
        _ => return Err(invalid("first consumption record is not Prepared")),
    };
    let runtime = runtime_from_binding(&records[0].binding, prepared_time.clone());
    let expected_binding = validated_binding(config, authorization, &runtime)?;
    if records[0].binding != expected_binding
        || records[0].binding_fingerprint != binding_fingerprint(&expected_binding)?
    {
        return Err(invalid("Prepared evidence binding is invalid"));
    }
    let mut previous = ZERO_FINGERPRINT.to_owned();
    let mut consumed_at: Option<DateTime<Utc>> = None;
    let mut recovery_continuation_registry = None;
    for (index, record) in records.iter().enumerate() {
        if record.schema_version != CONSUMPTION_SCHEMA_VERSION
            || usize::from(record.sequence) != index
            || record.previous_record_fingerprint != previous
            || record.binding != expected_binding
            || record.binding_fingerprint != records[0].binding_fingerprint
            || record.authorization != OfflineAuthorizationState::DENIED
        {
            return Err(invalid(
                "consumption evidence chain or authority fields are invalid",
            ));
        }
        match (&record.consumption, index) {
            (AuthorizationConsumptionState::Prepared { .. }, 0) => {}
            (
                AuthorizationConsumptionState::Consumed {
                    consumed_at_utc,
                    burned_before_dispatch_authority: true,
                    crash_allows_recovery_cancel_only: true,
                    placement_can_never_resume: true,
                },
                1,
            ) => {
                let parsed = parse_canonical_utc(consumed_at_utc)?;
                let runtime = runtime_from_binding(&record.binding, consumed_at_utc.clone());
                if validated_binding(config, authorization, &runtime)? != expected_binding {
                    return Err(invalid("Consumed evidence runtime recheck is invalid"));
                }
                if parsed < parse_canonical_utc(&prepared_time)? {
                    return Err(invalid("Consumed evidence precedes Prepared evidence"));
                }
                consumed_at = Some(parsed);
            }
            (
                AuthorizationConsumptionState::Terminal {
                    terminal_at_utc,
                    terminal_is_evidence_not_authority: true,
                    ..
                },
                2,
            ) if index + 1 == records.len() && recovery_continuation_registry.is_none() => {
                let terminal = parse_canonical_utc(terminal_at_utc)?;
                if consumed_at.is_none_or(|consumed| terminal < consumed) {
                    return Err(invalid("Terminal evidence precedes consumption"));
                }
            }
            (
                AuthorizationConsumptionState::RecoveryContinuationRootAnchored {
                    root,
                    recovery_cancel_only: true,
                    placement_can_never_resume: true,
                },
                2,
            ) => {
                if recovery_continuation_registry.is_some()
                    || make_recovery_continuation_root(
                        &root.continuation_scope_fingerprint,
                        &root.continuation_intent_header_fingerprint,
                        &root.continuation_dispatch_header_fingerprint,
                    )? != *root
                {
                    return Err(invalid("recovery-continuation root anchor is invalid"));
                }
                recovery_continuation_registry =
                    Some(AuthorizationRecoveryContinuationRegistryV1 {
                        root: root.clone(),
                        prepared: Vec::new(),
                        terminal_plan: None,
                    });
            }
            (
                AuthorizationConsumptionState::RecoveryCancelPreparedAnchored {
                    anchor,
                    recovery_cancel_only: true,
                    placement_can_never_resume: true,
                },
                _,
            ) => {
                let registry = recovery_continuation_registry
                    .as_mut()
                    .ok_or_else(|| invalid("recovery preparation precedes its root anchor"))?;
                let expected_ordinal = u8::try_from(registry.prepared.len())
                    .ok()
                    .and_then(|value| value.checked_add(1))
                    .ok_or_else(|| invalid("recovery preparation ordinal bound exceeded"))?;
                let expected_previous = registry
                    .prepared
                    .last()
                    .map(|prior| prior.anchor_fingerprint.as_str())
                    .unwrap_or(&registry.root.anchor_fingerprint);
                let prior = registry.prepared.last();
                if registry.terminal_plan.is_some()
                    || index != usize::from(expected_ordinal) + 2
                    || anchor.recovery_ordinal != expected_ordinal
                    || anchor.recovery_ordinal
                        > config.value().order.recovery_cancel_dispatch_budget
                    || anchor.continuation_scope_fingerprint
                        != registry.root.continuation_scope_fingerprint
                    || anchor.previous_anchor_fingerprint != expected_previous
                    || prior.is_some_and(|prior| {
                        anchor.continuation_intent_sequence <= prior.continuation_intent_sequence
                            || anchor.continuation_prepared_sequence
                                <= prior.continuation_prepared_sequence
                            || anchor.l2_timestamp_seconds <= prior.l2_timestamp_seconds
                            || anchor.exact_venue_order_id != prior.exact_venue_order_id
                            || anchor.semantic_request_commitment_sha256
                                != prior.semantic_request_commitment_sha256
                    })
                    || make_recovery_cancel_prepared_anchor(
                        &anchor.continuation_scope_fingerprint,
                        anchor.recovery_ordinal,
                        anchor.continuation_intent_sequence,
                        &anchor.continuation_intent_record_fingerprint,
                        anchor.continuation_prepared_sequence,
                        &anchor.continuation_dispatch_previous_record_fingerprint,
                        &anchor.continuation_prepared_record_fingerprint,
                        &anchor.continuation_prepared_record_canonical_json,
                        &anchor.exact_venue_order_id,
                        &anchor.semantic_request_commitment_sha256,
                        anchor.l2_timestamp_seconds,
                        anchor.previous_anchor_fingerprint.clone(),
                    )? != *anchor
                {
                    return Err(invalid("recovery preparation anchor is invalid"));
                }
                registry.prepared.push(anchor.clone());
            }
            (
                AuthorizationConsumptionState::RecoveryTerminalPlanAnchored {
                    plan,
                    recovery_cancel_only: true,
                    placement_can_never_resume: true,
                },
                _,
            ) => {
                let registry = recovery_continuation_registry
                    .as_mut()
                    .ok_or_else(|| invalid("recovery Terminal plan precedes its root anchor"))?;
                let expected_previous = registry
                    .prepared
                    .last()
                    .map(|prior| prior.anchor_fingerprint.as_str())
                    .unwrap_or(&registry.root.anchor_fingerprint);
                let terminal_at = parse_canonical_utc(&plan.terminal_at_utc)?;
                if registry.terminal_plan.is_some()
                    || index != registry.prepared.len() + 3
                    || index + 1 != records.len()
                    || plan.continuation_scope_fingerprint
                        != registry.root.continuation_scope_fingerprint
                    || plan.previous_anchor_fingerprint != expected_previous
                    || consumed_at.is_none_or(|consumed| terminal_at < consumed)
                    || make_recovery_terminal_plan(
                        &plan.continuation_scope_fingerprint,
                        plan.continuation_intent_predecessor_sequence,
                        &plan.continuation_intent_predecessor_record_fingerprint,
                        plan.continuation_dispatch_predecessor_sequence,
                        &plan.continuation_dispatch_predecessor_record_fingerprint,
                        &plan.terminal_at_utc,
                        plan.disposition,
                        plan.continuation_dispatch_terminal_sequence,
                        &plan.continuation_dispatch_terminal_previous_record_fingerprint,
                        &plan.continuation_dispatch_terminal_record_fingerprint,
                        &plan.continuation_dispatch_terminal_record_canonical_json,
                        plan.continuation_intent_terminal_sequence,
                        &plan.continuation_intent_terminal_previous_record_fingerprint,
                        &plan.continuation_intent_terminal_record_fingerprint,
                        &plan.continuation_intent_terminal_record_canonical_json,
                        plan.previous_anchor_fingerprint.clone(),
                    )? != *plan
                {
                    return Err(invalid("recovery Terminal plan anchor is invalid"));
                }
                registry.terminal_plan = Some(plan.clone());
            }
            _ => {
                return Err(invalid(
                    "consumption evidence states are out of order or incomplete",
                ));
            }
        }
        previous = record_fingerprint(record)?;
    }
    Ok(ValidatedRecords {
        latest_record_fingerprint: previous,
        recovery_continuation_registry,
    })
}

fn validate_claim(
    claim: &AtomicConsumptionClaim,
    binding: &AuthorizationConsumptionBindingEvidence,
    prepared_record_fingerprint: &str,
) -> Result<(), PmAuthorizationConsumptionError> {
    if claim.schema_version != CONSUMPTION_SCHEMA_VERSION
        || claim.prepared_record_fingerprint != prepared_record_fingerprint
        || &claim.binding != binding
        || claim.binding_fingerprint != binding_fingerprint(binding)?
        || !claim.burned_before_dispatch_authority
        || !claim.crash_allows_recovery_cancel_only
        || !claim.placement_can_never_resume
        || claim.authorization != OfflineAuthorizationState::DENIED
    {
        return Err(invalid("atomic consume claim is invalid or foreign"));
    }
    let consumed = parse_canonical_utc(&claim.consumed_at_utc)?;
    let not_before = parse_canonical_utc(&binding.authorization_not_before_utc)?;
    let expires = parse_canonical_utc(&binding.authorization_expires_at_utc)?;
    if consumed < not_before || consumed >= expires {
        return Err(invalid(
            "atomic consume claim is outside the authorization window",
        ));
    }
    Ok(())
}

fn make_record(
    sequence: u8,
    previous_record_fingerprint: String,
    binding: AuthorizationConsumptionBindingEvidence,
    consumption: AuthorizationConsumptionState,
) -> Result<AuthorizationConsumptionEvidence, PmAuthorizationConsumptionError> {
    Ok(AuthorizationConsumptionEvidence {
        schema_version: CONSUMPTION_SCHEMA_VERSION,
        sequence,
        previous_record_fingerprint,
        binding_fingerprint: binding_fingerprint(&binding)?,
        binding,
        consumption,
        authorization: OfflineAuthorizationState::DENIED,
    })
}

fn consumed_state(consumed_at_utc: String) -> AuthorizationConsumptionState {
    AuthorizationConsumptionState::Consumed {
        consumed_at_utc,
        burned_before_dispatch_authority: true,
        crash_allows_recovery_cancel_only: true,
        placement_can_never_resume: true,
    }
}

fn consumed_time(
    state: &AuthorizationConsumptionState,
) -> Result<&str, PmAuthorizationConsumptionError> {
    match state {
        AuthorizationConsumptionState::Consumed {
            consumed_at_utc, ..
        } => Ok(consumed_at_utc),
        _ => Err(invalid("expected Consumed evidence state is absent")),
    }
}

fn runtime_from_binding(
    binding: &AuthorizationConsumptionBindingEvidence,
    observed_at_utc: String,
) -> AuthorizationRuntimeBinding {
    AuthorizationRuntimeBinding {
        release_binary_sha256: binding.release_binary_sha256.clone(),
        release_binary_length: binding.release_binary_length,
        host: binding.host.clone(),
        observed_at_utc,
    }
}

fn parse_ledger(
    bytes: &[u8],
) -> Result<Vec<AuthorizationConsumptionEvidence>, PmAuthorizationConsumptionError> {
    if bytes.is_empty() || !bytes.ends_with(b"\n") {
        return Err(invalid("consumption ledger has an empty or ambiguous tail"));
    }
    let mut records = Vec::new();
    let mut lines = bytes.split(|byte| *byte == b'\n').peekable();
    while let Some(line) = lines.next() {
        if line.is_empty() {
            if lines.peek().is_none() {
                break;
            }
            return Err(invalid(
                "consumption ledger contains an empty interior record",
            ));
        }
        if records.len() == MAX_CONSUMPTION_RECORDS {
            return Err(invalid("consumption ledger contains extra records"));
        }
        let record: AuthorizationConsumptionEvidence = serde_json::from_slice(line)
            .map_err(|_| invalid("consumption ledger record is malformed or duplicated"))?;
        if canonical_json(&record)? != line {
            return Err(invalid(
                "consumption ledger record is not exact canonical JSON",
            ));
        }
        records.push(record);
    }
    if records.is_empty() {
        return Err(invalid("consumption ledger contains no complete record"));
    }
    Ok(records)
}

fn read_optional_claim(
    path: &std::path::Path,
) -> Result<Option<AtomicConsumptionClaim>, PmAuthorizationConsumptionError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(invalid("consume claim path cannot be inspected safely")),
        Ok(_) => {
            let bytes = read_one(
                path,
                ProtectedFileKind::ConsumptionEvidence,
                MAX_CONSUMPTION_EVIDENCE_BYTES,
            )
            .map_err(|_| invalid("consume claim is unprotected, unstable, or ambiguous"))?;
            let claim: AtomicConsumptionClaim = serde_json::from_slice(&bytes)
                .map_err(|_| invalid("consume claim is malformed or incomplete"))?;
            if canonical_json(&claim)? != bytes.as_slice() {
                return Err(invalid("consume claim is not exact canonical JSON"));
            }
            Ok(Some(claim))
        }
    }
}

fn require_claim_absent(path: &std::path::Path) -> Result<(), PmAuthorizationConsumptionError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(invalid(
            "bound consume claim path cannot be inspected safely",
        )),
        Ok(_) => Err(PmAuthorizationConsumptionError::AlreadyConsumed(
            "bound consume claim already exists or is ambiguous; reuse is forbidden",
        )),
    }
}

fn bound_paths(config: &CanonicalTrialConfig) -> (PathBuf, PathBuf) {
    let parent = PathBuf::from(&config.value().journal.artifact_directory);
    (
        parent.join(&config.value().journal.authorization_consumption_ledger_file),
        parent.join(&config.value().journal.authorization_consumption_claim_file),
    )
}

fn encode_ledger_record(
    record: &AuthorizationConsumptionEvidence,
) -> Result<Vec<u8>, PmAuthorizationConsumptionError> {
    let mut bytes = canonical_json(record)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn canonical_json(value: &impl Serialize) -> Result<Vec<u8>, PmAuthorizationConsumptionError> {
    serde_json::to_vec(value).map_err(|_| invalid("consumption evidence cannot be canonicalized"))
}

fn binding_fingerprint(
    binding: &AuthorizationConsumptionBindingEvidence,
) -> Result<String, PmAuthorizationConsumptionError> {
    Ok(hash_domain(
        BINDING_FINGERPRINT_DOMAIN,
        &canonical_json(binding)?,
    ))
}

fn record_fingerprint(
    record: &AuthorizationConsumptionEvidence,
) -> Result<String, PmAuthorizationConsumptionError> {
    Ok(hash_domain(
        RECORD_FINGERPRINT_DOMAIN,
        &canonical_json(record)?,
    ))
}

fn claim_fingerprint(
    claim: &AtomicConsumptionClaim,
) -> Result<String, PmAuthorizationConsumptionError> {
    Ok(hash_domain(
        CLAIM_FINGERPRINT_DOMAIN,
        &canonical_json(claim)?,
    ))
}

fn make_recovery_continuation_root(
    continuation_scope_fingerprint: &str,
    continuation_intent_header_fingerprint: &str,
    continuation_dispatch_header_fingerprint: &str,
) -> Result<AuthorizationRecoveryContinuationRootV1, PmAuthorizationConsumptionError> {
    for fingerprint in [
        continuation_scope_fingerprint,
        continuation_intent_header_fingerprint,
        continuation_dispatch_header_fingerprint,
    ] {
        validate_sha256(fingerprint)?;
    }
    let mut root = AuthorizationRecoveryContinuationRootV1 {
        continuation_scope_fingerprint: continuation_scope_fingerprint.to_owned(),
        continuation_intent_header_fingerprint: continuation_intent_header_fingerprint.to_owned(),
        continuation_dispatch_header_fingerprint: continuation_dispatch_header_fingerprint
            .to_owned(),
        previous_anchor_fingerprint: ZERO_FINGERPRINT.to_owned(),
        anchor_fingerprint: ZERO_FINGERPRINT.to_owned(),
    };
    root.anchor_fingerprint = hash_domain(
        RECOVERY_CONTINUATION_ROOT_FINGERPRINT_DOMAIN,
        &canonical_json(&root)?,
    );
    Ok(root)
}

#[allow(clippy::too_many_arguments)]
fn make_recovery_cancel_prepared_anchor(
    continuation_scope_fingerprint: &str,
    recovery_ordinal: u8,
    continuation_intent_sequence: u8,
    continuation_intent_record_fingerprint: &str,
    continuation_prepared_sequence: u8,
    continuation_dispatch_previous_record_fingerprint: &str,
    continuation_prepared_record_fingerprint: &str,
    continuation_prepared_record_canonical_json: &str,
    exact_venue_order_id: &str,
    semantic_request_commitment_sha256: &str,
    l2_timestamp_seconds: u64,
    previous_anchor_fingerprint: String,
) -> Result<AuthorizationRecoveryCancelPreparedAnchorV1, PmAuthorizationConsumptionError> {
    for fingerprint in [
        continuation_scope_fingerprint,
        continuation_intent_record_fingerprint,
        continuation_dispatch_previous_record_fingerprint,
        continuation_prepared_record_fingerprint,
        semantic_request_commitment_sha256,
        previous_anchor_fingerprint.as_str(),
    ] {
        validate_sha256(fingerprint)?;
    }
    if recovery_ordinal == 0
        || continuation_intent_sequence == 0
        || continuation_prepared_sequence == 0
        || l2_timestamp_seconds == 0
        || exact_venue_order_id.len() != 66
        || !exact_venue_order_id.starts_with("0x")
        || !exact_venue_order_id[2..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid("recovery preparation anchor values are invalid"));
    }
    let _: serde_json::Value = serde_json::from_str(continuation_prepared_record_canonical_json)
        .map_err(|_| invalid("recovery preparation canonical record is malformed"))?;
    if continuation_prepared_record_canonical_json.len() > MAX_RECOVERY_PREPARED_RECORD_BYTES
        || continuation_prepared_record_canonical_json
            .bytes()
            .any(|byte| matches!(byte, b'\n' | b'\r'))
    {
        return Err(invalid(
            "recovery preparation canonical record is not one bounded JSON line",
        ));
    }
    let mut anchor = AuthorizationRecoveryCancelPreparedAnchorV1 {
        continuation_scope_fingerprint: continuation_scope_fingerprint.to_owned(),
        recovery_ordinal,
        continuation_intent_sequence,
        continuation_intent_record_fingerprint: continuation_intent_record_fingerprint.to_owned(),
        continuation_prepared_sequence,
        continuation_dispatch_previous_record_fingerprint:
            continuation_dispatch_previous_record_fingerprint.to_owned(),
        continuation_prepared_record_fingerprint: continuation_prepared_record_fingerprint
            .to_owned(),
        continuation_prepared_record_canonical_json: continuation_prepared_record_canonical_json
            .to_owned(),
        exact_venue_order_id: exact_venue_order_id.to_owned(),
        semantic_request_commitment_sha256: semantic_request_commitment_sha256.to_owned(),
        l2_timestamp_seconds,
        previous_anchor_fingerprint,
        anchor_fingerprint: ZERO_FINGERPRINT.to_owned(),
    };
    anchor.anchor_fingerprint = hash_domain(
        RECOVERY_CANCEL_PREPARED_FINGERPRINT_DOMAIN,
        &canonical_json(&anchor)?,
    );
    Ok(anchor)
}

#[allow(clippy::too_many_arguments)]
fn make_recovery_terminal_plan(
    continuation_scope_fingerprint: &str,
    continuation_intent_predecessor_sequence: u8,
    continuation_intent_predecessor_record_fingerprint: &str,
    continuation_dispatch_predecessor_sequence: u8,
    continuation_dispatch_predecessor_record_fingerprint: &str,
    terminal_at_utc: &str,
    disposition: TerminalDisposition,
    continuation_dispatch_terminal_sequence: u8,
    continuation_dispatch_terminal_previous_record_fingerprint: &str,
    continuation_dispatch_terminal_record_fingerprint: &str,
    continuation_dispatch_terminal_record_canonical_json: &str,
    continuation_intent_terminal_sequence: u8,
    continuation_intent_terminal_previous_record_fingerprint: &str,
    continuation_intent_terminal_record_fingerprint: &str,
    continuation_intent_terminal_record_canonical_json: &str,
    previous_anchor_fingerprint: String,
) -> Result<AuthorizationRecoveryTerminalPlanV1, PmAuthorizationConsumptionError> {
    for fingerprint in [
        continuation_scope_fingerprint,
        continuation_intent_predecessor_record_fingerprint,
        continuation_dispatch_predecessor_record_fingerprint,
        continuation_dispatch_terminal_previous_record_fingerprint,
        continuation_dispatch_terminal_record_fingerprint,
        continuation_intent_terminal_previous_record_fingerprint,
        continuation_intent_terminal_record_fingerprint,
        previous_anchor_fingerprint.as_str(),
    ] {
        validate_sha256(fingerprint)?;
    }
    parse_canonical_utc(terminal_at_utc)?;
    if continuation_dispatch_terminal_sequence
        != continuation_dispatch_predecessor_sequence
            .checked_add(1)
            .ok_or_else(|| invalid("recovery Terminal dispatch sequence overflows"))?
        || continuation_intent_terminal_sequence
            != continuation_intent_predecessor_sequence
                .checked_add(1)
                .ok_or_else(|| invalid("recovery Terminal intent sequence overflows"))?
        || continuation_dispatch_terminal_previous_record_fingerprint
            != continuation_dispatch_predecessor_record_fingerprint
        || continuation_intent_terminal_previous_record_fingerprint
            != continuation_intent_predecessor_record_fingerprint
    {
        return Err(invalid(
            "recovery Terminal plan does not immediately follow its predecessors",
        ));
    }
    for canonical in [
        continuation_dispatch_terminal_record_canonical_json,
        continuation_intent_terminal_record_canonical_json,
    ] {
        let _: serde_json::Value = serde_json::from_str(canonical)
            .map_err(|_| invalid("recovery Terminal canonical record is malformed"))?;
        if canonical.len() > MAX_RECOVERY_PREPARED_RECORD_BYTES
            || canonical.bytes().any(|byte| matches!(byte, b'\n' | b'\r'))
        {
            return Err(invalid(
                "recovery Terminal canonical record is not one bounded JSON line",
            ));
        }
    }
    let mut plan = AuthorizationRecoveryTerminalPlanV1 {
        continuation_scope_fingerprint: continuation_scope_fingerprint.to_owned(),
        continuation_intent_predecessor_sequence,
        continuation_intent_predecessor_record_fingerprint:
            continuation_intent_predecessor_record_fingerprint.to_owned(),
        continuation_dispatch_predecessor_sequence,
        continuation_dispatch_predecessor_record_fingerprint:
            continuation_dispatch_predecessor_record_fingerprint.to_owned(),
        terminal_at_utc: terminal_at_utc.to_owned(),
        disposition,
        continuation_dispatch_terminal_sequence,
        continuation_dispatch_terminal_previous_record_fingerprint:
            continuation_dispatch_terminal_previous_record_fingerprint.to_owned(),
        continuation_dispatch_terminal_record_fingerprint:
            continuation_dispatch_terminal_record_fingerprint.to_owned(),
        continuation_dispatch_terminal_record_canonical_json:
            continuation_dispatch_terminal_record_canonical_json.to_owned(),
        continuation_intent_terminal_sequence,
        continuation_intent_terminal_previous_record_fingerprint:
            continuation_intent_terminal_previous_record_fingerprint.to_owned(),
        continuation_intent_terminal_record_fingerprint:
            continuation_intent_terminal_record_fingerprint.to_owned(),
        continuation_intent_terminal_record_canonical_json:
            continuation_intent_terminal_record_canonical_json.to_owned(),
        previous_anchor_fingerprint,
        anchor_fingerprint: ZERO_FINGERPRINT.to_owned(),
    };
    plan.anchor_fingerprint = hash_domain(
        RECOVERY_TERMINAL_PLAN_FINGERPRINT_DOMAIN,
        &canonical_json(&plan)?,
    );
    Ok(plan)
}

fn hash_domain(domain: &[u8], bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn validate_sha256(value: &str) -> Result<(), PmAuthorizationConsumptionError> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(
            "runtime binary SHA-256 is not canonical lowercase hex",
        ));
    }
    Ok(())
}

fn parse_canonical_utc(value: &str) -> Result<DateTime<Utc>, PmAuthorizationConsumptionError> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid("consumption timestamp is invalid"))?
        .with_timezone(&Utc);
    if parsed.to_rfc3339_opts(chrono::SecondsFormat::Secs, true) != value {
        return Err(invalid(
            "consumption timestamp is not canonical UTC seconds",
        ));
    }
    Ok(parsed)
}

fn map_prepare_create_error(error: ProtectedFileError) -> PmAuthorizationConsumptionError {
    match error {
        ProtectedFileError::Create(_) => PmAuthorizationConsumptionError::AlreadyConsumed(
            "bound authorization-consumption ledger already exists; reuse is forbidden",
        ),
        _ => invalid("bound authorization-consumption ledger cannot be created safely"),
    }
}

fn map_claim_create_error(error: ProtectedFileError) -> PmAuthorizationConsumptionError {
    match error {
        ProtectedFileError::Create(_) => PmAuthorizationConsumptionError::AlreadyConsumed(
            "atomic consume claim already exists or is ambiguous; placement is burned",
        ),
        _ => invalid("atomic consume claim cannot be created safely"),
    }
}

#[derive(Debug, Error)]
pub enum PmAuthorizationConsumptionError {
    #[error("authorization consumption rejected: {0}")]
    Invalid(&'static str),
    #[error("authorization cannot be reused: {0}")]
    AlreadyConsumed(&'static str),
    #[error("authorization-consumption evidence is ambiguous: {0}")]
    Ambiguous(&'static str),
    #[error("authorization is burned with incomplete evidence: {0}")]
    BurnedEvidenceIncomplete(&'static str),
    #[error("terminal evidence is ambiguous: {0}")]
    TerminalEvidenceAmbiguous(&'static str),
}

fn invalid(message: &'static str) -> PmAuthorizationConsumptionError {
    PmAuthorizationConsumptionError::Invalid(message)
}
