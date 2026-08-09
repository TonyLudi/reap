//! Durable, take-once V2 authorization-consumption evidence.
//!
//! This module is a strictly V2 conjunct beside the unchanged V1 lifecycle.
//! It owns exact canonical V2 policy and authorization bytes, but imports no
//! V1 authorization, V1 preflight, live-source, placement-owner, signer, or
//! transport type. Every public result remains
//! `OfflineAuthorizationState::DENIED` and no restart API can recreate a
//! placement owner.
//!
//! The reviewed tunnel/gateway profile is reviewer evidence. Runtime routing
//! facts establish only `SameLocalEgressSelection`; identical public NAT
//! identity across destinations remains the reviewed destination-independent
//! tunnel/gateway assumption unless a signed gateway flow attestor exists.
//!
//! The fsynced ledger and create-new claim are a monotonic crash-durability
//! boundary only on trusted local storage. Coordinated rollback, removal, or
//! replacement of every bound V2 artifact by a post-crash same-EUID actor is
//! outside this local profile and requires a TPM counter, WORM storage, or a
//! trusted remote registry to detect.

use std::{fs, io, path::PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    CanonicalTrialConfig, OfflineAuthorizationState, TrialPhase,
    online_policy_v2::{
        CanonicalOnlineAuthorizationV2, CanonicalOnlinePolicyV2, OnlineAuthorizationBuildBindingV2,
        OnlineAuthorizationHostBindingV2, OnlinePolicyPinsV2, V1ConfigPinsV2,
        ValidatedOnlineAuthorizationTimesV2, validate_online_authorization_contract_v2,
    },
    protected_file::{
        DurableCreateNewFile, ProtectedFileError, ProtectedFileKind, create_new, read_one,
    },
};

pub const ONLINE_AUTHORIZATION_CONSUMPTION_V2_SCHEMA_VERSION: u32 = 2;
pub const PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_LEDGER_FILE_V2: &str =
    "pm-t2-online-authorization-consumption-v2.jsonl";
pub const PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_CLAIM_FILE_V2: &str =
    "pm-t2-online-authorization-consumed-v2.claim";
pub const PM_T2_ONLINE_PREFLIGHT_SIDECAR_FILE_V2: &str = "pm-t2-phase-a-online-preflight-v2.jsonl";

const MAX_ONLINE_AUTHORIZATION_CONSUMPTION_BYTES_V2: usize = 64 * 1024;
const MAX_ONLINE_AUTHORIZATION_CONSUMPTION_RECORDS_V2: usize = 2;
const ZERO_FINGERPRINT: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const BINDING_FINGERPRINT_DOMAIN_V2: &[u8] =
    b"reap.pm-t2.controlled-trial.online-authorization-consumption.binding.v2\0";
const RECORD_FINGERPRINT_DOMAIN_V2: &[u8] =
    b"reap.pm-t2.controlled-trial.online-authorization-consumption.record.v2\0";
const CLAIM_FINGERPRINT_DOMAIN_V2: &[u8] =
    b"reap.pm-t2.controlled-trial.online-authorization-consumption.claim.v2\0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnlineAuthorizationPinsV2 {
    pub authorization_id: String,
    pub canonical_sha256: String,
    pub canonical_length: u64,
    pub fingerprint: String,
}

/// Non-secret facts that a later source-owned current-runtime witness must
/// supply immediately before each transition. This freely constructible
/// evidence type is not a live witness and grants no authority by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnlineAuthorizationRuntimeBindingV2 {
    pub release_binary_sha256: String,
    pub release_binary_length: u64,
    pub uts_nodename: String,
    pub boot_id: String,
    pub nss_username: String,
    pub linux_euid: u32,
    pub network_namespace_device: u64,
    pub network_namespace_inode: u64,
    pub interface_name: String,
    pub interface_index: u32,
    pub local_source_ip: String,
    pub geoblock_reported_public_ip: String,
    pub observed_at_utc: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnlineAuthorizationConsumptionBindingV2 {
    pub phase: TrialPhase,
    pub v1_config: V1ConfigPinsV2,
    pub online_policy: OnlinePolicyPinsV2,
    pub online_authorization: OnlineAuthorizationPinsV2,
    pub build: OnlineAuthorizationBuildBindingV2,
    pub host: OnlineAuthorizationHostBindingV2,
    pub authorization_not_before_utc: String,
    pub authorization_expires_at_utc: String,
    pub authorization_cleanup_not_after_utc: String,
    pub artifact_directory: String,
    pub credential_slot_id: String,
    pub credential_slot_nonsecret_fingerprint_sha256: String,
    pub expected_order_id: String,
    pub semantic_request_commitment: String,
    pub ledger_file: String,
    pub consume_claim_file: String,
    pub online_preflight_sidecar_file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnlineAuthorizationConsumptionAttemptV2 {
    pub online_preflight_basis_record_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnlineAuthorizationPlacementReuseV2 {
    PermanentlyBurned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnlineAuthorizationCrashRecoveryV2 {
    ExistingV1LifecycleOnlyNoPlacementResume,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum OnlineAuthorizationConsumptionStateV2 {
    Prepared {
        verified_at_utc: String,
    },
    Consumed {
        consumed_at_utc: String,
        attempt: OnlineAuthorizationConsumptionAttemptV2,
        placement_reuse: OnlineAuthorizationPlacementReuseV2,
        crash_recovery: OnlineAuthorizationCrashRecoveryV2,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnlineAuthorizationConsumptionEvidenceV2 {
    pub schema_version: u32,
    pub sequence: u8,
    pub previous_record_fingerprint: String,
    pub binding_fingerprint: String,
    pub binding: OnlineAuthorizationConsumptionBindingV2,
    pub consumption: OnlineAuthorizationConsumptionStateV2,
    pub authorization: OfflineAuthorizationState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AtomicOnlineAuthorizationConsumptionClaimV2 {
    schema_version: u32,
    prepared_record_fingerprint: String,
    binding_fingerprint: String,
    binding: OnlineAuthorizationConsumptionBindingV2,
    consumed_at_utc: String,
    attempt: OnlineAuthorizationConsumptionAttemptV2,
    placement_reuse: OnlineAuthorizationPlacementReuseV2,
    crash_recovery: OnlineAuthorizationCrashRecoveryV2,
    authorization: OfflineAuthorizationState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OnlineAuthorizationConsumptionVerificationV2 {
    pub schema_version: u32,
    pub state: OnlineAuthorizationConsumptionStateV2,
    pub ledger_record_count: u8,
    pub atomic_consumption_claim_durable: bool,
    pub consumed_ledger_record_durable: bool,
    pub latest_record_fingerprint: String,
    pub atomic_claim_fingerprint: Option<String>,
    pub binding_fingerprint: String,
    pub exact_bindings_structurally_valid: bool,
    pub ambiguous_tail: bool,
    #[serde(flatten)]
    pub authorization: OfflineAuthorizationState,
}

/// Move-only custody of one durable Prepared record and the exact V2 inputs.
/// It is denied evidence, never a placement or dispatch permit.
pub struct PreparedOnlineAuthorizationConsumptionV2 {
    journal: DurableCreateNewFile,
    ledger_bytes: Vec<u8>,
    prepared_record: OnlineAuthorizationConsumptionEvidenceV2,
    prepared_record_fingerprint: String,
    binding: OnlineAuthorizationConsumptionBindingV2,
    claim_path: PathBuf,
    policy: CanonicalOnlinePolicyV2,
    authorization: CanonicalOnlineAuthorizationV2,
}

impl std::fmt::Debug for PreparedOnlineAuthorizationConsumptionV2 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(
            "PreparedOnlineAuthorizationConsumptionV2(<denied-evidence; held-canonical-v2-records>)",
        )
    }
}

impl PreparedOnlineAuthorizationConsumptionV2 {
    #[must_use]
    pub const fn evidence(&self) -> &OnlineAuthorizationConsumptionEvidenceV2 {
        &self.prepared_record
    }

    #[must_use]
    pub const fn policy(&self) -> &CanonicalOnlinePolicyV2 {
        &self.policy
    }

    #[must_use]
    pub const fn authorization(&self) -> &CanonicalOnlineAuthorizationV2 {
        &self.authorization
    }

    #[must_use]
    pub fn binding_fingerprint(&self) -> &str {
        &self.prepared_record.binding_fingerprint
    }

    #[must_use]
    pub fn prepared_record_fingerprint(&self) -> &str {
        &self.prepared_record_fingerprint
    }

    pub fn revalidate_held_consumption_evidence(
        &mut self,
    ) -> Result<(), PmOnlineAuthorizationConsumptionV2Error> {
        self.journal
            .validate_exact_bytes(&self.ledger_bytes)
            .map_err(|_| {
                PmOnlineAuthorizationConsumptionV2Error::Ambiguous(
                    "held V2 authorization-consumption ledger changed",
                )
            })
    }

    pub fn refresh_after_bound_artifact_create(
        &mut self,
    ) -> Result<(), PmOnlineAuthorizationConsumptionV2Error> {
        self.journal
            .refresh_parent_after_bound_create()
            .map_err(|_| {
                PmOnlineAuthorizationConsumptionV2Error::Ambiguous(
                    "V2 consumption parent changed after bound artifact creation",
                )
            })?;
        self.revalidate_held_consumption_evidence()
    }

    /// Atomically burns this V2 authorization. The returned owner still grants
    /// no dispatch authority; a later final runner-private permit must consume
    /// it together with the existing V1 A3 owner and live source witnesses.
    pub fn consume(
        self,
        config: &CanonicalTrialConfig,
        runtime: &OnlineAuthorizationRuntimeBindingV2,
        attempt: OnlineAuthorizationConsumptionAttemptV2,
    ) -> Result<ConsumedOnlineAuthorizationConsumptionV2, PmOnlineAuthorizationConsumptionV2Error>
    {
        consume_prepared(self, config, runtime, attempt)
    }
}

/// Move-only custody of the burned V2 attempt. There is deliberately no
/// restart/reopen constructor and no method that yields a placement owner.
pub struct ConsumedOnlineAuthorizationConsumptionV2 {
    journal: DurableCreateNewFile,
    ledger_bytes: Vec<u8>,
    claim: DurableCreateNewFile,
    claim_bytes: Vec<u8>,
    consumed_record: OnlineAuthorizationConsumptionEvidenceV2,
    prepared_record_fingerprint: String,
    atomic_claim_fingerprint: String,
    consumed_record_fingerprint: String,
    policy: CanonicalOnlinePolicyV2,
    authorization: CanonicalOnlineAuthorizationV2,
}

impl std::fmt::Debug for ConsumedOnlineAuthorizationConsumptionV2 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(
            "ConsumedOnlineAuthorizationConsumptionV2(<denied-burn-evidence; held-canonical-v2-records>)",
        )
    }
}

impl ConsumedOnlineAuthorizationConsumptionV2 {
    #[must_use]
    pub const fn evidence(&self) -> &OnlineAuthorizationConsumptionEvidenceV2 {
        &self.consumed_record
    }

    #[must_use]
    pub const fn policy(&self) -> &CanonicalOnlinePolicyV2 {
        &self.policy
    }

    #[must_use]
    pub const fn authorization(&self) -> &CanonicalOnlineAuthorizationV2 {
        &self.authorization
    }

    #[must_use]
    pub fn binding_fingerprint(&self) -> &str {
        &self.consumed_record.binding_fingerprint
    }

    #[must_use]
    pub fn prepared_record_fingerprint(&self) -> &str {
        &self.prepared_record_fingerprint
    }

    #[must_use]
    pub fn atomic_claim_fingerprint(&self) -> &str {
        &self.atomic_claim_fingerprint
    }

    #[must_use]
    pub fn consumed_record_fingerprint(&self) -> &str {
        &self.consumed_record_fingerprint
    }

    pub fn revalidate_held_consumption_evidence(
        &mut self,
    ) -> Result<(), PmOnlineAuthorizationConsumptionV2Error> {
        self.claim
            .validate_exact_bytes(&self.claim_bytes)
            .and_then(|()| self.journal.validate_exact_bytes(&self.ledger_bytes))
            .map_err(|_| {
                PmOnlineAuthorizationConsumptionV2Error::Ambiguous(
                    "held V2 authorization-consumption ledger or claim changed",
                )
            })
    }

    pub fn refresh_after_bound_artifact_create(
        &mut self,
    ) -> Result<(), PmOnlineAuthorizationConsumptionV2Error> {
        self.claim
            .refresh_parent_after_bound_create()
            .and_then(|()| self.journal.refresh_parent_after_bound_create())
            .map_err(|_| {
                PmOnlineAuthorizationConsumptionV2Error::Ambiguous(
                    "V2 consumption parent changed after bound artifact creation",
                )
            })?;
        self.revalidate_held_consumption_evidence()
    }
}

pub fn prepare_online_authorization_consumption_v2(
    config: &CanonicalTrialConfig,
    policy: CanonicalOnlinePolicyV2,
    authorization: CanonicalOnlineAuthorizationV2,
    runtime: &OnlineAuthorizationRuntimeBindingV2,
) -> Result<PreparedOnlineAuthorizationConsumptionV2, PmOnlineAuthorizationConsumptionV2Error> {
    let times = validate_contract(config, &policy, &authorization)?;
    let binding = validated_binding(config, &authorization, runtime, &times)?;
    let (ledger_path, claim_path) = bound_paths(config)?;
    require_claim_absent(&claim_path)?;
    let binding_fingerprint = binding_fingerprint(&binding)?;
    let record = OnlineAuthorizationConsumptionEvidenceV2 {
        schema_version: ONLINE_AUTHORIZATION_CONSUMPTION_V2_SCHEMA_VERSION,
        sequence: 0,
        previous_record_fingerprint: ZERO_FINGERPRINT.to_owned(),
        binding_fingerprint,
        binding: binding.clone(),
        consumption: OnlineAuthorizationConsumptionStateV2::Prepared {
            verified_at_utc: runtime.observed_at_utc.clone(),
        },
        authorization: OfflineAuthorizationState::DENIED,
    };
    let prepared_record_fingerprint = record_fingerprint(&record)?;
    let encoded = encode_ledger_record(&record)?;
    let mut journal = create_new(
        &ledger_path,
        ProtectedFileKind::OnlineAuthorizationConsumptionV2,
        MAX_ONLINE_AUTHORIZATION_CONSUMPTION_BYTES_V2,
    )
    .map_err(map_prepare_create_error)?;
    journal.append_durable(&[], &encoded).map_err(|_| {
        PmOnlineAuthorizationConsumptionV2Error::Ambiguous(
            "Prepared V2 ledger creation or fsync is incomplete; reuse is forbidden",
        )
    })?;
    Ok(PreparedOnlineAuthorizationConsumptionV2 {
        journal,
        ledger_bytes: encoded,
        prepared_record: record,
        prepared_record_fingerprint,
        binding,
        claim_path,
        policy,
        authorization,
    })
}

/// Read-only structural display of the fixed V2 ledger and claim.
///
/// This returns denied evidence only. In particular, a valid claim-only crash
/// prefix proves the attempt is burned but cannot reopen placement authority.
pub fn verify_online_authorization_consumption_v2(
    config: &CanonicalTrialConfig,
    policy: &CanonicalOnlinePolicyV2,
    authorization: &CanonicalOnlineAuthorizationV2,
) -> Result<OnlineAuthorizationConsumptionVerificationV2, PmOnlineAuthorizationConsumptionV2Error> {
    let times = validate_contract(config, policy, authorization)?;
    let (ledger_path, claim_path) = bound_paths(config)?;
    let ledger = read_one(
        &ledger_path,
        ProtectedFileKind::OnlineAuthorizationConsumptionV2,
        MAX_ONLINE_AUTHORIZATION_CONSUMPTION_BYTES_V2,
    )
    .map_err(|_| invalid("bound V2 consumption ledger is absent, unprotected, or unstable"))?;
    let records = parse_ledger(&ledger)?;
    validate_records(config, policy, authorization, &times, &records)?;
    let prepared_fingerprint = record_fingerprint(&records[0])?;
    let claim = read_optional_claim(&claim_path)?;
    let claim_fingerprint_value = claim.as_ref().map(claim_fingerprint).transpose()?;
    if let Some(claim) = &claim {
        validate_claim(config, claim, &records[0], &prepared_fingerprint, &times)?;
    }

    let (state, claim_durable, consumed_record_durable) = match records.as_slice() {
        [prepared] => match claim {
            None => (prepared.consumption.clone(), false, false),
            Some(claim) => (consumed_state_from_claim(&claim), true, false),
        },
        [_, consumed] => {
            let claim = claim.ok_or_else(|| {
                invalid("Consumed V2 ledger record exists without its atomic consume claim")
            })?;
            if consumed.consumption != consumed_state_from_claim(&claim) {
                return Err(invalid("V2 consume claim and ledger record disagree"));
            }
            (consumed.consumption.clone(), true, true)
        }
        _ => return Err(invalid("V2 consumption ledger record count is invalid")),
    };
    let latest_record_fingerprint = record_fingerprint(
        records
            .last()
            .ok_or_else(|| invalid("V2 consumption ledger is empty"))?,
    )?;
    Ok(OnlineAuthorizationConsumptionVerificationV2 {
        schema_version: ONLINE_AUTHORIZATION_CONSUMPTION_V2_SCHEMA_VERSION,
        state,
        ledger_record_count: records.len() as u8,
        atomic_consumption_claim_durable: claim_durable,
        consumed_ledger_record_durable: consumed_record_durable,
        latest_record_fingerprint,
        atomic_claim_fingerprint: claim_fingerprint_value,
        binding_fingerprint: records[0].binding_fingerprint.clone(),
        exact_bindings_structurally_valid: true,
        ambiguous_tail: false,
        authorization: OfflineAuthorizationState::DENIED,
    })
}

fn consume_prepared(
    mut prepared: PreparedOnlineAuthorizationConsumptionV2,
    config: &CanonicalTrialConfig,
    runtime: &OnlineAuthorizationRuntimeBindingV2,
    attempt: OnlineAuthorizationConsumptionAttemptV2,
) -> Result<ConsumedOnlineAuthorizationConsumptionV2, PmOnlineAuthorizationConsumptionV2Error> {
    prepared.revalidate_held_consumption_evidence()?;
    validate_attempt(&attempt)?;
    let times = validate_contract(config, &prepared.policy, &prepared.authorization)?;
    let binding = validated_binding(config, &prepared.authorization, runtime, &times)?;
    if binding != prepared.binding {
        return Err(invalid(
            "V2 consume recheck does not match the exact Prepared binding",
        ));
    }
    let prepared_at = match &prepared.prepared_record.consumption {
        OnlineAuthorizationConsumptionStateV2::Prepared { verified_at_utc } => {
            parse_canonical_utc(verified_at_utc)?
        }
        _ => return Err(invalid("in-memory V2 owner is not in Prepared state")),
    };
    let consumed_at = parse_canonical_utc(&runtime.observed_at_utc)?;
    if consumed_at < prepared_at {
        return Err(invalid("V2 consume recheck time precedes Prepared time"));
    }

    let claim = AtomicOnlineAuthorizationConsumptionClaimV2 {
        schema_version: ONLINE_AUTHORIZATION_CONSUMPTION_V2_SCHEMA_VERSION,
        prepared_record_fingerprint: prepared.prepared_record_fingerprint.clone(),
        binding_fingerprint: binding_fingerprint(&binding)?,
        binding: binding.clone(),
        consumed_at_utc: runtime.observed_at_utc.clone(),
        attempt: attempt.clone(),
        placement_reuse: OnlineAuthorizationPlacementReuseV2::PermanentlyBurned,
        crash_recovery:
            OnlineAuthorizationCrashRecoveryV2::ExistingV1LifecycleOnlyNoPlacementResume,
        authorization: OfflineAuthorizationState::DENIED,
    };
    let claim_bytes = canonical_json(&claim)?;
    let atomic_claim_fingerprint = claim_fingerprint(&claim)?;
    let mut claim_file = create_new(
        &prepared.claim_path,
        ProtectedFileKind::OnlineAuthorizationConsumptionV2,
        MAX_ONLINE_AUTHORIZATION_CONSUMPTION_BYTES_V2,
    )
    .map_err(map_claim_create_error)?;
    claim_file.append_durable(&[], &claim_bytes).map_err(|_| {
        PmOnlineAuthorizationConsumptionV2Error::BurnedEvidenceIncomplete(
            "V2 claim creation is ambiguous; placement is burned and cannot resume",
        )
    })?;

    // The create-new, fsynced claim is the take-once linearization point.
    // Everything below only completes denied evidence; it cannot restore a
    // place path or alter the existing V1 recovery/cancel substrate.
    prepared
        .journal
        .refresh_parent_after_bound_create()
        .map_err(|_| {
            PmOnlineAuthorizationConsumptionV2Error::BurnedEvidenceIncomplete(
                "V2 claim is durable but ledger parent changed; no placement resume",
            )
        })?;
    let record = make_record(
        1,
        prepared.prepared_record_fingerprint.clone(),
        binding,
        consumed_state(runtime.observed_at_utc.clone(), attempt),
    )?;
    let consumed_record_fingerprint = record_fingerprint(&record)?;
    let encoded = encode_ledger_record(&record)?;
    prepared
        .journal
        .append_durable(&prepared.ledger_bytes, &encoded)
        .map_err(|_| {
            PmOnlineAuthorizationConsumptionV2Error::BurnedEvidenceIncomplete(
                "V2 claim is durable but Consumed ledger completion failed; no placement resume",
            )
        })?;
    prepared.ledger_bytes.extend_from_slice(&encoded);
    Ok(ConsumedOnlineAuthorizationConsumptionV2 {
        journal: prepared.journal,
        ledger_bytes: prepared.ledger_bytes,
        claim: claim_file,
        claim_bytes,
        consumed_record: record,
        prepared_record_fingerprint: prepared.prepared_record_fingerprint,
        atomic_claim_fingerprint,
        consumed_record_fingerprint,
        policy: prepared.policy,
        authorization: prepared.authorization,
    })
}

fn validate_contract(
    config: &CanonicalTrialConfig,
    policy: &CanonicalOnlinePolicyV2,
    authorization: &CanonicalOnlineAuthorizationV2,
) -> Result<ValidatedOnlineAuthorizationTimesV2, PmOnlineAuthorizationConsumptionV2Error> {
    validate_online_authorization_contract_v2(config, policy, authorization)
        .map_err(|_| invalid("exact V2 policy or authorization contract is invalid"))
}

fn validated_binding(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalOnlineAuthorizationV2,
    runtime: &OnlineAuthorizationRuntimeBindingV2,
    times: &ValidatedOnlineAuthorizationTimesV2,
) -> Result<OnlineAuthorizationConsumptionBindingV2, PmOnlineAuthorizationConsumptionV2Error> {
    let observed = parse_canonical_utc(&runtime.observed_at_utc)?;
    if observed < times.not_before || observed >= times.expires_at {
        return Err(invalid(
            "V2 runtime observation is outside the authorization window",
        ));
    }
    validate_cleanup_runway(config, times, observed)?;
    let record = authorization.value();
    let host = &record.host;
    let egress = &host.egress;
    if runtime.release_binary_sha256 != record.build.release_binary_sha256
        || runtime.release_binary_length != record.build.release_binary_length
        || runtime.uts_nodename != host.uts_nodename
        || runtime.boot_id != host.boot_id
        || runtime.nss_username != host.nss_username
        || runtime.linux_euid != host.linux_euid
        || runtime.network_namespace_device != egress.network_namespace_device
        || runtime.network_namespace_inode != egress.network_namespace_inode
        || runtime.interface_name != egress.interface_name
        || runtime.interface_index != egress.interface_index
        || runtime.local_source_ip != egress.local_source_ip
        || runtime.geoblock_reported_public_ip != egress.authorized_geoblock_reported_public_ip
    {
        return Err(invalid(
            "V2 binary, host, or same-local-egress runtime observation drifted",
        ));
    }
    let identity = config.exact_place_public_request_identity();
    Ok(OnlineAuthorizationConsumptionBindingV2 {
        phase: TrialPhase::APlaceCancel,
        v1_config: record.v1_config.clone(),
        online_policy: record.online_policy.clone(),
        online_authorization: OnlineAuthorizationPinsV2 {
            authorization_id: record.authorization_id.clone(),
            canonical_sha256: authorization.canonical_sha256().to_owned(),
            canonical_length: authorization.canonical_length(),
            fingerprint: authorization.fingerprint().to_owned(),
        },
        build: record.build.clone(),
        host: record.host.clone(),
        authorization_not_before_utc: record.not_before_utc.clone(),
        authorization_expires_at_utc: record.expires_at_utc.clone(),
        authorization_cleanup_not_after_utc: record.cleanup_not_after_utc.clone(),
        artifact_directory: config.value().journal.artifact_directory.clone(),
        credential_slot_id: config.value().credential_slot.slot_id.clone(),
        credential_slot_nonsecret_fingerprint_sha256: config
            .value()
            .credential_slot
            .nonsecret_fingerprint_sha256
            .clone(),
        expected_order_id: identity.expected_order_id().to_string(),
        semantic_request_commitment: identity.semantic_request_commitment().to_string(),
        ledger_file: PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_LEDGER_FILE_V2.to_owned(),
        consume_claim_file: PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_CLAIM_FILE_V2.to_owned(),
        online_preflight_sidecar_file: PM_T2_ONLINE_PREFLIGHT_SIDECAR_FILE_V2.to_owned(),
    })
}

fn validate_records(
    config: &CanonicalTrialConfig,
    policy: &CanonicalOnlinePolicyV2,
    authorization: &CanonicalOnlineAuthorizationV2,
    times: &ValidatedOnlineAuthorizationTimesV2,
    records: &[OnlineAuthorizationConsumptionEvidenceV2],
) -> Result<(), PmOnlineAuthorizationConsumptionV2Error> {
    if records.is_empty() || records.len() > MAX_ONLINE_AUTHORIZATION_CONSUMPTION_RECORDS_V2 {
        return Err(invalid("V2 consumption ledger record count is invalid"));
    }
    validate_static_binding(config, policy, authorization, &records[0].binding)?;
    let prepared_at = match &records[0].consumption {
        OnlineAuthorizationConsumptionStateV2::Prepared { verified_at_utc } => {
            parse_canonical_utc(verified_at_utc)?
        }
        _ => return Err(invalid("first V2 consumption record is not Prepared")),
    };
    if prepared_at < times.not_before || prepared_at >= times.expires_at {
        return Err(invalid(
            "V2 Prepared record is outside the authorization window",
        ));
    }
    validate_cleanup_runway(config, times, prepared_at)?;
    let mut previous = ZERO_FINGERPRINT.to_owned();
    for (index, record) in records.iter().enumerate() {
        if record.schema_version != ONLINE_AUTHORIZATION_CONSUMPTION_V2_SCHEMA_VERSION
            || usize::from(record.sequence) != index
            || record.previous_record_fingerprint != previous
            || record.binding != records[0].binding
            || record.binding_fingerprint != binding_fingerprint(&record.binding)?
            || record.authorization != OfflineAuthorizationState::DENIED
        {
            return Err(invalid("V2 consumption ledger chain or binding is invalid"));
        }
        if index == 1 {
            let (consumed_at_utc, attempt) = match &record.consumption {
                OnlineAuthorizationConsumptionStateV2::Consumed {
                    consumed_at_utc,
                    attempt,
                    placement_reuse: OnlineAuthorizationPlacementReuseV2::PermanentlyBurned,
                    crash_recovery:
                        OnlineAuthorizationCrashRecoveryV2::ExistingV1LifecycleOnlyNoPlacementResume,
                } => (consumed_at_utc, attempt),
                _ => return Err(invalid("second V2 consumption record is not Consumed")),
            };
            validate_attempt(attempt)?;
            let consumed_at = parse_canonical_utc(consumed_at_utc)?;
            if consumed_at < prepared_at
                || consumed_at < times.not_before
                || consumed_at >= times.expires_at
            {
                return Err(invalid("V2 Consumed record time is invalid"));
            }
            validate_cleanup_runway(config, times, consumed_at)?;
        }
        previous = record_fingerprint(record)?;
    }
    Ok(())
}

fn validate_static_binding(
    config: &CanonicalTrialConfig,
    policy: &CanonicalOnlinePolicyV2,
    authorization: &CanonicalOnlineAuthorizationV2,
    binding: &OnlineAuthorizationConsumptionBindingV2,
) -> Result<(), PmOnlineAuthorizationConsumptionV2Error> {
    let record = authorization.value();
    let identity = config.exact_place_public_request_identity();
    let expected_authorization = OnlineAuthorizationPinsV2 {
        authorization_id: record.authorization_id.clone(),
        canonical_sha256: authorization.canonical_sha256().to_owned(),
        canonical_length: authorization.canonical_length(),
        fingerprint: authorization.fingerprint().to_owned(),
    };
    if binding.phase != TrialPhase::APlaceCancel
        || binding.v1_config.canonical_config_sha256 != config.canonical_sha256()
        || binding.v1_config.canonical_config_length != config.canonical_length()
        || binding.v1_config.canonical_config_fingerprint != config.fingerprint()
        || binding.v1_config.trial_plan_fingerprint != config.plan_fingerprint()
        || binding.online_policy.canonical_sha256 != policy.canonical_sha256()
        || binding.online_policy.canonical_length != policy.canonical_length()
        || binding.online_policy.fingerprint != policy.fingerprint()
        || binding.online_authorization != expected_authorization
        || binding.build != record.build
        || binding.host != record.host
        || binding.authorization_not_before_utc != record.not_before_utc
        || binding.authorization_expires_at_utc != record.expires_at_utc
        || binding.authorization_cleanup_not_after_utc != record.cleanup_not_after_utc
        || binding.artifact_directory != config.value().journal.artifact_directory
        || binding.credential_slot_id != config.value().credential_slot.slot_id
        || binding.credential_slot_nonsecret_fingerprint_sha256
            != config.value().credential_slot.nonsecret_fingerprint_sha256
        || binding.expected_order_id != identity.expected_order_id().to_string()
        || binding.semantic_request_commitment != identity.semantic_request_commitment().to_string()
        || binding.ledger_file != PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_LEDGER_FILE_V2
        || binding.consume_claim_file != PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_CLAIM_FILE_V2
        || binding.online_preflight_sidecar_file != PM_T2_ONLINE_PREFLIGHT_SIDECAR_FILE_V2
    {
        return Err(invalid(
            "V2 consumption evidence does not bind the exact config, policy, and authorization",
        ));
    }
    Ok(())
}

fn validate_claim(
    config: &CanonicalTrialConfig,
    claim: &AtomicOnlineAuthorizationConsumptionClaimV2,
    prepared: &OnlineAuthorizationConsumptionEvidenceV2,
    prepared_fingerprint: &str,
    times: &ValidatedOnlineAuthorizationTimesV2,
) -> Result<(), PmOnlineAuthorizationConsumptionV2Error> {
    if claim.schema_version != ONLINE_AUTHORIZATION_CONSUMPTION_V2_SCHEMA_VERSION
        || claim.prepared_record_fingerprint != prepared_fingerprint
        || claim.binding != prepared.binding
        || claim.binding_fingerprint != binding_fingerprint(&prepared.binding)?
        || claim.placement_reuse != OnlineAuthorizationPlacementReuseV2::PermanentlyBurned
        || claim.crash_recovery
            != OnlineAuthorizationCrashRecoveryV2::ExistingV1LifecycleOnlyNoPlacementResume
        || claim.authorization != OfflineAuthorizationState::DENIED
    {
        return Err(invalid("atomic V2 consume claim is invalid or foreign"));
    }
    validate_attempt(&claim.attempt)?;
    let consumed_at = parse_canonical_utc(&claim.consumed_at_utc)?;
    let prepared_at = match &prepared.consumption {
        OnlineAuthorizationConsumptionStateV2::Prepared { verified_at_utc } => {
            parse_canonical_utc(verified_at_utc)?
        }
        _ => return Err(invalid("atomic V2 claim has no Prepared predecessor")),
    };
    if consumed_at < prepared_at
        || consumed_at < times.not_before
        || consumed_at >= times.expires_at
    {
        return Err(invalid("atomic V2 consume claim time is invalid"));
    }
    validate_cleanup_runway(config, times, consumed_at)?;
    Ok(())
}

fn validate_cleanup_runway(
    config: &CanonicalTrialConfig,
    times: &ValidatedOnlineAuthorizationTimesV2,
    observed: DateTime<Utc>,
) -> Result<(), PmOnlineAuthorizationConsumptionV2Error> {
    let cleanup_runway_ms = i64::try_from(config.value().time_limits.cleanup_not_after_ms)
        .map_err(|_| invalid("V2 cleanup-runway duration does not fit checked time arithmetic"))?;
    let required_cleanup_through_ms = observed
        .timestamp_millis()
        .checked_add(cleanup_runway_ms)
        .ok_or_else(|| invalid("V2 cleanup-runway time arithmetic overflowed"))?;
    if required_cleanup_through_ms > times.cleanup_not_after.timestamp_millis() {
        return Err(invalid(
            "V2 authorization does not preserve the full configured cleanup runway",
        ));
    }
    Ok(())
}

fn validate_attempt(
    attempt: &OnlineAuthorizationConsumptionAttemptV2,
) -> Result<(), PmOnlineAuthorizationConsumptionV2Error> {
    validate_sha256(&attempt.online_preflight_basis_record_fingerprint)?;
    if attempt.online_preflight_basis_record_fingerprint == ZERO_FINGERPRINT {
        return Err(invalid(
            "V2 online-preflight Basis fingerprint must be nonzero",
        ));
    }
    Ok(())
}

fn make_record(
    sequence: u8,
    previous_record_fingerprint: String,
    binding: OnlineAuthorizationConsumptionBindingV2,
    consumption: OnlineAuthorizationConsumptionStateV2,
) -> Result<OnlineAuthorizationConsumptionEvidenceV2, PmOnlineAuthorizationConsumptionV2Error> {
    Ok(OnlineAuthorizationConsumptionEvidenceV2 {
        schema_version: ONLINE_AUTHORIZATION_CONSUMPTION_V2_SCHEMA_VERSION,
        sequence,
        previous_record_fingerprint,
        binding_fingerprint: binding_fingerprint(&binding)?,
        binding,
        consumption,
        authorization: OfflineAuthorizationState::DENIED,
    })
}

fn consumed_state(
    consumed_at_utc: String,
    attempt: OnlineAuthorizationConsumptionAttemptV2,
) -> OnlineAuthorizationConsumptionStateV2 {
    OnlineAuthorizationConsumptionStateV2::Consumed {
        consumed_at_utc,
        attempt,
        placement_reuse: OnlineAuthorizationPlacementReuseV2::PermanentlyBurned,
        crash_recovery:
            OnlineAuthorizationCrashRecoveryV2::ExistingV1LifecycleOnlyNoPlacementResume,
    }
}

fn consumed_state_from_claim(
    claim: &AtomicOnlineAuthorizationConsumptionClaimV2,
) -> OnlineAuthorizationConsumptionStateV2 {
    OnlineAuthorizationConsumptionStateV2::Consumed {
        consumed_at_utc: claim.consumed_at_utc.clone(),
        attempt: claim.attempt.clone(),
        placement_reuse: claim.placement_reuse,
        crash_recovery: claim.crash_recovery,
    }
}

fn parse_ledger(
    bytes: &[u8],
) -> Result<Vec<OnlineAuthorizationConsumptionEvidenceV2>, PmOnlineAuthorizationConsumptionV2Error>
{
    if bytes.is_empty() || !bytes.ends_with(b"\n") {
        return Err(invalid(
            "V2 consumption ledger has an empty or ambiguous tail",
        ));
    }
    let mut records = Vec::new();
    let mut lines = bytes.split(|byte| *byte == b'\n').peekable();
    while let Some(line) = lines.next() {
        if line.is_empty() {
            if lines.peek().is_none() {
                break;
            }
            return Err(invalid(
                "V2 consumption ledger contains an empty interior record",
            ));
        }
        if records.len() == MAX_ONLINE_AUTHORIZATION_CONSUMPTION_RECORDS_V2 {
            return Err(invalid("V2 consumption ledger contains extra records"));
        }
        let record: OnlineAuthorizationConsumptionEvidenceV2 = serde_json::from_slice(line)
            .map_err(|_| invalid("V2 consumption ledger record is malformed or duplicated"))?;
        if canonical_json(&record)? != line {
            return Err(invalid(
                "V2 consumption ledger record is not exact canonical JSON",
            ));
        }
        records.push(record);
    }
    if records.is_empty() {
        return Err(invalid("V2 consumption ledger contains no complete record"));
    }
    Ok(records)
}

fn read_optional_claim(
    path: &std::path::Path,
) -> Result<
    Option<AtomicOnlineAuthorizationConsumptionClaimV2>,
    PmOnlineAuthorizationConsumptionV2Error,
> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(invalid("V2 consume claim path cannot be inspected safely")),
        Ok(_) => {
            let bytes = read_one(
                path,
                ProtectedFileKind::OnlineAuthorizationConsumptionV2,
                MAX_ONLINE_AUTHORIZATION_CONSUMPTION_BYTES_V2,
            )
            .map_err(|_| invalid("V2 consume claim is unprotected, unstable, or ambiguous"))?;
            let claim: AtomicOnlineAuthorizationConsumptionClaimV2 = serde_json::from_slice(&bytes)
                .map_err(|_| invalid("V2 consume claim is malformed or incomplete"))?;
            if canonical_json(&claim)? != bytes.as_slice() {
                return Err(invalid("V2 consume claim is not exact canonical JSON"));
            }
            Ok(Some(claim))
        }
    }
}

fn require_claim_absent(
    path: &std::path::Path,
) -> Result<(), PmOnlineAuthorizationConsumptionV2Error> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(invalid(
            "bound V2 consume claim path cannot be inspected safely",
        )),
        Ok(_) => Err(PmOnlineAuthorizationConsumptionV2Error::AlreadyConsumed(
            "bound V2 consume claim already exists or is ambiguous; reuse is forbidden",
        )),
    }
}

fn bound_paths(
    config: &CanonicalTrialConfig,
) -> Result<(PathBuf, PathBuf), PmOnlineAuthorizationConsumptionV2Error> {
    for v1_name in [
        config
            .value()
            .journal
            .authorization_consumption_ledger_file
            .as_str(),
        config
            .value()
            .journal
            .authorization_consumption_claim_file
            .as_str(),
    ] {
        if matches!(
            v1_name,
            PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_LEDGER_FILE_V2
                | PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_CLAIM_FILE_V2
                | PM_T2_ONLINE_PREFLIGHT_SIDECAR_FILE_V2
        ) {
            return Err(invalid("V1 and V2 bound artifact names collide"));
        }
    }
    let parent = PathBuf::from(&config.value().journal.artifact_directory);
    Ok((
        parent.join(PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_LEDGER_FILE_V2),
        parent.join(PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_CLAIM_FILE_V2),
    ))
}

fn encode_ledger_record(
    record: &OnlineAuthorizationConsumptionEvidenceV2,
) -> Result<Vec<u8>, PmOnlineAuthorizationConsumptionV2Error> {
    let mut bytes = canonical_json(record)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn canonical_json(
    value: &impl Serialize,
) -> Result<Vec<u8>, PmOnlineAuthorizationConsumptionV2Error> {
    serde_json::to_vec(value)
        .map_err(|_| invalid("V2 authorization-consumption evidence cannot be canonicalized"))
}

fn binding_fingerprint(
    binding: &OnlineAuthorizationConsumptionBindingV2,
) -> Result<String, PmOnlineAuthorizationConsumptionV2Error> {
    Ok(hash_domain(
        BINDING_FINGERPRINT_DOMAIN_V2,
        &canonical_json(binding)?,
    ))
}

fn record_fingerprint(
    record: &OnlineAuthorizationConsumptionEvidenceV2,
) -> Result<String, PmOnlineAuthorizationConsumptionV2Error> {
    Ok(hash_domain(
        RECORD_FINGERPRINT_DOMAIN_V2,
        &canonical_json(record)?,
    ))
}

fn claim_fingerprint(
    claim: &AtomicOnlineAuthorizationConsumptionClaimV2,
) -> Result<String, PmOnlineAuthorizationConsumptionV2Error> {
    Ok(hash_domain(
        CLAIM_FINGERPRINT_DOMAIN_V2,
        &canonical_json(claim)?,
    ))
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

fn validate_sha256(value: &str) -> Result<(), PmOnlineAuthorizationConsumptionV2Error> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(invalid("V2 fingerprint is not canonical lowercase hex"));
    }
    Ok(())
}

fn parse_canonical_utc(
    value: &str,
) -> Result<DateTime<Utc>, PmOnlineAuthorizationConsumptionV2Error> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid("V2 consumption timestamp is invalid"))?
        .with_timezone(&Utc);
    if parsed.to_rfc3339_opts(chrono::SecondsFormat::Secs, true) != value {
        return Err(invalid(
            "V2 consumption timestamp is not canonical UTC seconds",
        ));
    }
    Ok(parsed)
}

fn map_prepare_create_error(error: ProtectedFileError) -> PmOnlineAuthorizationConsumptionV2Error {
    match error {
        ProtectedFileError::Create(_) => PmOnlineAuthorizationConsumptionV2Error::AlreadyConsumed(
            "bound V2 consumption ledger already exists; reuse is forbidden",
        ),
        _ => invalid("bound V2 consumption ledger cannot be created safely"),
    }
}

fn map_claim_create_error(error: ProtectedFileError) -> PmOnlineAuthorizationConsumptionV2Error {
    match error {
        ProtectedFileError::Create(_) => PmOnlineAuthorizationConsumptionV2Error::AlreadyConsumed(
            "atomic V2 consume claim already exists or is ambiguous; placement is burned",
        ),
        _ => PmOnlineAuthorizationConsumptionV2Error::BurnedEvidenceIncomplete(
            "atomic V2 claim creation may have created its marker; placement is burned",
        ),
    }
}

#[derive(Debug, Error)]
pub enum PmOnlineAuthorizationConsumptionV2Error {
    #[error("online authorization V2 consumption rejected: {0}")]
    Invalid(&'static str),
    #[error("online authorization V2 cannot be reused: {0}")]
    AlreadyConsumed(&'static str),
    #[error("online authorization V2 consumption evidence is ambiguous: {0}")]
    Ambiguous(&'static str),
    #[error("online authorization V2 is burned with incomplete evidence: {0}")]
    BurnedEvidenceIncomplete(&'static str),
}

fn invalid(message: &'static str) -> PmOnlineAuthorizationConsumptionV2Error {
    PmOnlineAuthorizationConsumptionV2Error::Invalid(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_create_claim_errors_are_always_reported_as_burned() {
        for error in [
            ProtectedFileError::DurableWrite(ProtectedFileKind::OnlineAuthorizationConsumptionV2),
            ProtectedFileError::FileChanged(ProtectedFileKind::OnlineAuthorizationConsumptionV2),
        ] {
            assert!(matches!(
                map_claim_create_error(error),
                PmOnlineAuthorizationConsumptionV2Error::BurnedEvidenceIncomplete(_)
            ));
        }
        assert!(matches!(
            map_claim_create_error(ProtectedFileError::Create(
                ProtectedFileKind::OnlineAuthorizationConsumptionV2,
            )),
            PmOnlineAuthorizationConsumptionV2Error::AlreadyConsumed(_)
        ));
    }
}
