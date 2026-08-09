//! Additive durable V2 online-preflight evidence for one Phase-A attempt.
//!
//! This module does not alter or replace the V1 journal, dispatch barrier, or
//! recovery/cancel lifecycle. Its two-record sidecar is an additional
//! conjunct: `Basis` is durable before either authorization is burned, and
//! `A3Conjunct` is appended only after the unchanged V1 A3 barrier exists.
//! Every public value in this module remains denied evidence. A later
//! runner-private join must still consume current source-owned witnesses before
//! it can mint any HMAC or transport input.

use std::{fmt, path::Path};

use reap_pm_controlled_trial::{
    AuthorizationConsumptionState, AuthorizationRuntimeBinding, CanonicalAuthorization,
    CanonicalTrialConfig, ConsumedOnlineAuthorizationConsumptionV2, OfflineAuthorizationState,
    OnlineAuthorizationConsumptionAttemptV2, OnlineAuthorizationConsumptionStateV2,
    OnlineAuthorizationRuntimeBindingV2, PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_CLAIM_FILE_V2,
    PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_LEDGER_FILE_V2, PM_T2_ONLINE_PREFLIGHT_SIDECAR_FILE_V2,
    PlacePublicRequestIdentity, PmAuthorizationConsumptionError,
    PmOnlineAuthorizationConsumptionV2Error, PreparedAuthorizationConsumption,
    PreparedOnlineAuthorizationConsumptionV2, verify_authorization_consumption,
};
use serde::{Deserialize, Serialize};

use crate::{
    PmControlledTrialLiveJournals, PmDurablePlacePreparedAckV1,
    PmPhaseAPlaceDefinitelyNotDispatchedV1, PmPhaseAPlaceLiveDispatchProfileV1,
    PmPhaseAPlaceMayHaveBeenDispatchedV1, PmPhaseAPlaceNetworkDispatchOwnerV1,
    PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1, PmTrialLiveJournalError,
    hash::{ZERO_FINGERPRINT, canonical_json, hash_domain, validate_fingerprint},
    protected::{ProtectedJournal, read_protected},
    schema::{
        PM_TRIAL_LIVE_DISPATCH_FILE_V1, PM_TRIAL_LIVE_INTENT_FILE_V1, PmPlacePreparationViewV1,
        validate_utc,
    },
};

pub const PM_PHASE_A_ONLINE_PREFLIGHT_SIDECAR_FILE_V2: &str =
    PM_T2_ONLINE_PREFLIGHT_SIDECAR_FILE_V2;

const ONLINE_PREFLIGHT_SIDECAR_SCHEMA_VERSION_V2: u32 = 2;
const ONLINE_PREFLIGHT_SIDECAR_FAMILY_V2: &str = "pm-t2-phase-a-online-preflight-sidecar";
const MAX_ONLINE_PREFLIGHT_SIDECAR_BYTES_V2: usize = 128 * 1024;
const MAX_ONLINE_PREFLIGHT_SIDECAR_LINE_BYTES_V2: usize = 64 * 1024;
const ONLINE_PREFLIGHT_BASIS_FINGERPRINT_DOMAIN_V2: &[u8] =
    b"reap.pm-t2.phase-a.online-preflight.basis.v2\0";
const ONLINE_PREFLIGHT_A3_CONJUNCT_FINGERPRINT_DOMAIN_V2: &[u8] =
    b"reap.pm-t2.phase-a.online-preflight.a3-conjunct.v2\0";
const ONLINE_PREFLIGHT_DURABLE_MANIFEST_FINGERPRINT_DOMAIN_V2: &[u8] =
    b"reap.pm-t2.phase-a.online-preflight.durable-manifest.v2\0";

/// Fingerprints of the exact runner-owned evidence manifest whose source
/// proofs must be retained and rechecked by the later runner-private join.
///
/// This freely constructible descriptor is evidence only. It cannot establish
/// freshness, source provenance, or mutation authority by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PmPhaseAOnlinePreflightEvidenceManifestV2 {
    pub observation_started_at_utc: String,
    pub observation_completed_at_utc: String,
    pub canonical_manifest_sha256: String,
    pub canonical_manifest_length: u64,
    pub reviewed_market_evidence_sha256: String,
    pub reviewed_status_history_sha256: String,
    pub fresh_status_announcements_sha256: String,
    pub clob_ok_liveness_sha256: String,
    pub same_account_closed_only_sha256: String,
    pub public_book_cut_sha256: String,
    pub user_account_cut_sha256: String,
    pub same_authority_rest_cut_sha256: String,
    pub finalized_chain_cut_sha256: String,
    pub data_api_position_cut_sha256: String,
    pub current_runtime_and_egress_sha256: String,
    pub reviewed_repository_state_sha256: String,
}

impl PmPhaseAOnlinePreflightEvidenceManifestV2 {
    fn validate(&self) -> Result<(), PmTrialLiveJournalError> {
        let started = validate_utc(&self.observation_started_at_utc)?;
        let completed = validate_utc(&self.observation_completed_at_utc)?;
        if self.canonical_manifest_length == 0 || completed < started {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        for fingerprint in [
            &self.canonical_manifest_sha256,
            &self.reviewed_market_evidence_sha256,
            &self.reviewed_status_history_sha256,
            &self.fresh_status_announcements_sha256,
            &self.clob_ok_liveness_sha256,
            &self.same_account_closed_only_sha256,
            &self.public_book_cut_sha256,
            &self.user_account_cut_sha256,
            &self.same_authority_rest_cut_sha256,
            &self.finalized_chain_cut_sha256,
            &self.data_api_position_cut_sha256,
            &self.current_runtime_and_egress_sha256,
            &self.reviewed_repository_state_sha256,
        ] {
            validate_fingerprint(fingerprint)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OnlinePreflightV2Pins {
    canonical_sha256: String,
    canonical_length: u64,
    fingerprint: String,
}

impl OnlinePreflightV2Pins {
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
struct OnlinePreflightBasisV2 {
    canonical_config_sha256: String,
    canonical_config_length: u64,
    canonical_config_fingerprint: String,
    trial_plan_fingerprint: String,
    preflight_fingerprint: String,
    preflight_canonical_sha256: String,
    preflight_validated_at_utc: String,
    preflight_dispatch_deadline_at_utc: String,
    place_prepared_sequence: u8,
    place_prepared_record_fingerprint: String,
    public_semantic_request_commitment: String,
    prepared_request_commitment: String,
    expected_order_id: String,
    l2_timestamp_seconds: u64,
    v1_consumption_binding_fingerprint: String,
    v1_consumption_prepared_record_fingerprint: String,
    v2_consumption_binding_fingerprint: String,
    v2_consumption_prepared_record_fingerprint: String,
    online_policy: OnlinePreflightV2Pins,
    online_authorization_id: String,
    online_authorization: OnlinePreflightV2Pins,
    online_authorization_not_before_utc: String,
    online_authorization_expires_at_utc: String,
    online_authorization_cleanup_not_after_utc: String,
    evidence: PmPhaseAOnlinePreflightEvidenceManifestV2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OnlinePreflightDurableManifestV2 {
    v1_intent_file: String,
    v1_dispatch_file: String,
    v1_scope_fingerprint: String,
    v1_place_prepared_record_fingerprint: String,
    v1_dispatch_authorized_record_fingerprint: String,
    v1_dispatch_barrier_record_fingerprint: String,
    v1_consumption_binding_fingerprint: String,
    v1_consumption_prepared_record_fingerprint: String,
    v1_consumption_claim_fingerprint: String,
    v1_consumption_consumed_record_fingerprint: String,
    v2_consumption_ledger_file: String,
    v2_consumption_claim_file: String,
    v2_consumption_binding_fingerprint: String,
    v2_consumption_prepared_record_fingerprint: String,
    v2_consumption_claim_fingerprint: String,
    v2_consumption_consumed_record_fingerprint: String,
    online_preflight_sidecar_file: String,
    online_preflight_basis_record_fingerprint: String,
}

impl OnlinePreflightDurableManifestV2 {
    fn validate(&self) -> Result<(), PmTrialLiveJournalError> {
        if self.v1_intent_file != PM_TRIAL_LIVE_INTENT_FILE_V1
            || self.v1_dispatch_file != PM_TRIAL_LIVE_DISPATCH_FILE_V1
            || self.v2_consumption_ledger_file
                != PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_LEDGER_FILE_V2
            || self.v2_consumption_claim_file
                != PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_CLAIM_FILE_V2
            || self.online_preflight_sidecar_file != PM_PHASE_A_ONLINE_PREFLIGHT_SIDECAR_FILE_V2
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        for fingerprint in [
            &self.v1_scope_fingerprint,
            &self.v1_place_prepared_record_fingerprint,
            &self.v1_dispatch_authorized_record_fingerprint,
            &self.v1_dispatch_barrier_record_fingerprint,
            &self.v1_consumption_binding_fingerprint,
            &self.v1_consumption_prepared_record_fingerprint,
            &self.v1_consumption_claim_fingerprint,
            &self.v1_consumption_consumed_record_fingerprint,
            &self.v2_consumption_binding_fingerprint,
            &self.v2_consumption_prepared_record_fingerprint,
            &self.v2_consumption_claim_fingerprint,
            &self.v2_consumption_consumed_record_fingerprint,
            &self.online_preflight_basis_record_fingerprint,
        ] {
            validate_fingerprint(fingerprint)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OnlinePreflightA3ConjunctV2 {
    basis_sequence: u8,
    basis_record_fingerprint: String,
    durable_manifest_fingerprint: String,
    durable_manifest: OnlinePreflightDurableManifestV2,
    online_policy: OnlinePreflightV2Pins,
    online_authorization_id: String,
    online_authorization: OnlinePreflightV2Pins,
    expected_order_id: String,
    public_semantic_request_commitment: String,
    prepared_request_commitment: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case", deny_unknown_fields)]
enum OnlinePreflightSidecarBodyV2 {
    Basis {
        basis: Box<OnlinePreflightBasisV2>,
    },
    A3Conjunct {
        conjunct: Box<OnlinePreflightA3ConjunctV2>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct OnlinePreflightSidecarLineV2 {
    schema_version: u32,
    family: String,
    sequence: u8,
    previous_record_fingerprint: String,
    scope_fingerprint: String,
    #[serde(flatten)]
    body: OnlinePreflightSidecarBodyV2,
    authorization: OfflineAuthorizationState,
    record_fingerprint: String,
}

impl OnlinePreflightSidecarLineV2 {
    fn seal(
        sequence: u8,
        previous_record_fingerprint: String,
        scope_fingerprint: String,
        body: OnlinePreflightSidecarBodyV2,
    ) -> Result<Self, PmTrialLiveJournalError> {
        let mut line = Self {
            schema_version: ONLINE_PREFLIGHT_SIDECAR_SCHEMA_VERSION_V2,
            family: ONLINE_PREFLIGHT_SIDECAR_FAMILY_V2.to_owned(),
            sequence,
            previous_record_fingerprint,
            scope_fingerprint,
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
            OnlinePreflightSidecarBodyV2::Basis { .. } => {
                ONLINE_PREFLIGHT_BASIS_FINGERPRINT_DOMAIN_V2
            }
            OnlinePreflightSidecarBodyV2::A3Conjunct { .. } => {
                ONLINE_PREFLIGHT_A3_CONJUNCT_FINGERPRINT_DOMAIN_V2
            }
        };
        hash_domain(domain, &basis)
    }

    fn validate_structural(&self) -> Result<(), PmTrialLiveJournalError> {
        if self.schema_version != ONLINE_PREFLIGHT_SIDECAR_SCHEMA_VERSION_V2
            || self.family != ONLINE_PREFLIGHT_SIDECAR_FAMILY_V2
            || self.authorization != OfflineAuthorizationState::DENIED
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        validate_fingerprint(&self.previous_record_fingerprint)?;
        validate_fingerprint(&self.scope_fingerprint)?;
        validate_fingerprint(&self.record_fingerprint)?;
        match &self.body {
            OnlinePreflightSidecarBodyV2::Basis { basis } => {
                if self.sequence != 0 || self.previous_record_fingerprint != ZERO_FINGERPRINT {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                validate_basis(basis)?;
            }
            OnlinePreflightSidecarBodyV2::A3Conjunct { conjunct } => {
                if self.sequence != 1 || self.previous_record_fingerprint == ZERO_FINGERPRINT {
                    return Err(PmTrialLiveJournalError::InvalidRecord);
                }
                validate_conjunct(conjunct)?;
            }
        }
        if self.calculate_fingerprint()? != self.record_fingerprint {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
        Ok(())
    }
}

/// Move-only custody of Basis, both still-unconsumed authorization owners, and
/// the exact canonical V2 records held inside the V2 owner.
///
/// ```compile_fail
/// use reap_pm_controlled_trial_live::PmPendingPhaseAOnlinePreflightBasisV2;
/// fn cannot_clone(owner: PmPendingPhaseAOnlinePreflightBasisV2) {
///     let _ = owner.clone();
/// }
/// ```
///
/// ```compile_fail
/// use reap_pm_controlled_trial_live::PmPendingPhaseAOnlinePreflightBasisV2;
/// fn cannot_extract(owner: PmPendingPhaseAOnlinePreflightBasisV2) {
///     let PmPendingPhaseAOnlinePreflightBasisV2 { prepared, .. } = owner;
///     drop(prepared);
/// }
/// ```
pub struct PmPendingPhaseAOnlinePreflightBasisV2 {
    prepared: PmDurablePlacePreparedAckV1,
    v1_consumption: PreparedAuthorizationConsumption,
    v2_consumption: PreparedOnlineAuthorizationConsumptionV2,
    sidecar: ProtectedJournal,
    expected_sidecar_bytes: Vec<u8>,
    basis: OnlinePreflightSidecarLineV2,
}

impl PmPendingPhaseAOnlinePreflightBasisV2 {
    /// Borrow only the public PlacePrepared evidence needed to construct and
    /// sign the exact body. No acknowledgement or consumption owner escapes.
    #[must_use]
    pub const fn preparation(&self) -> &PmPlacePreparationViewV1 {
        self.prepared.preparation()
    }

    #[must_use]
    pub fn basis_record_fingerprint(&self) -> &str {
        &self.basis.record_fingerprint
    }

    #[must_use]
    pub const fn authorization(&self) -> OfflineAuthorizationState {
        OfflineAuthorizationState::DENIED
    }

    /// Burn V2 first, then V1, create the unchanged V1 A3 barrier, and append
    /// the V2 conjunct. No failure path can resume placement.
    pub fn burn_and_record_a3(
        self,
        journals: &mut PmControlledTrialLiveJournals,
        config: &CanonicalTrialConfig,
        v1_authorization: &CanonicalAuthorization,
        v1_runtime: &AuthorizationRuntimeBinding,
        v2_runtime: &OnlineAuthorizationRuntimeBindingV2,
    ) -> Result<PmPhaseAOnlinePreflightDispatchOwnerV2, PmPhaseAOnlinePreflightV2Error> {
        let Self {
            prepared,
            mut v1_consumption,
            v2_consumption,
            mut sidecar,
            mut expected_sidecar_bytes,
            basis,
        } = self;

        sidecar.validate_exact_bytes(&expected_sidecar_bytes)?;
        let attempt = OnlineAuthorizationConsumptionAttemptV2 {
            online_preflight_basis_record_fingerprint: basis.record_fingerprint.clone(),
        };
        let mut v2_consumption = v2_consumption
            .consume(config, v2_runtime, attempt)
            .map_err(|_| PmPhaseAOnlinePreflightV2Error::V2Consumption)?;

        // The V2 claim is a new directory entry. Refresh every older held
        // view before the V1 burn can create its claim.
        sidecar.refresh_parent_after_bound_create()?;
        v1_consumption
            .refresh_after_bound_artifact_create()
            .map_err(|_| PmPhaseAOnlinePreflightV2Error::V1Consumption)?;
        journals.refresh_after_online_preflight_v2_artifact_create()?;

        let v1_consumption = v1_consumption
            .consume(config, v1_authorization, v1_runtime)
            .map_err(|_| PmPhaseAOnlinePreflightV2Error::V1Consumption)?;

        // The V1 claim is another new directory entry. Refresh V2 and the
        // sidecar before either can be used in the final conjunct.
        sidecar.refresh_parent_after_bound_create()?;
        v2_consumption
            .refresh_after_bound_artifact_create()
            .map_err(|_| PmPhaseAOnlinePreflightV2Error::V2Consumption)?;
        journals.refresh_after_online_preflight_v2_artifact_create()?;

        let verification = verify_authorization_consumption(config, v1_authorization)
            .map_err(|_| PmPhaseAOnlinePreflightV2Error::V1Consumption)?;
        let proof =
            journals.bind_consumed_authorization(prepared, v1_consumption, &verification)?;
        let v1 = journals.record_phase_a_place_live_dispatch_authorized(proof)?;
        let v1 = v1.revalidate_for_runner()?;

        // From here onward a V1 A3 exists. Any V2-side failure returns only a
        // DND-capable sealed owner; it never returns positive V1 authority.
        if sidecar.refresh_parent_after_bound_create().is_err()
            || v2_consumption
                .refresh_after_bound_artifact_create()
                .is_err()
        {
            return Err(post_a3_failure(
                PmPhaseAOnlinePreflightPostA3FailureReasonV2::ParentRefresh,
                v1,
                v2_consumption,
                sidecar,
            ));
        }

        let conjunct = match make_a3_conjunct(&basis, &v1, &v2_consumption) {
            Ok(conjunct) => conjunct,
            Err(_) => {
                return Err(post_a3_failure(
                    PmPhaseAOnlinePreflightPostA3FailureReasonV2::Binding,
                    v1,
                    v2_consumption,
                    sidecar,
                ));
            }
        };
        let encoded = match encode_line(&conjunct) {
            Ok(encoded) => encoded,
            Err(_) => {
                return Err(post_a3_failure(
                    PmPhaseAOnlinePreflightPostA3FailureReasonV2::Binding,
                    v1,
                    v2_consumption,
                    sidecar,
                ));
            }
        };
        if sidecar
            .append_durable(&expected_sidecar_bytes, &encoded)
            .is_err()
        {
            return Err(post_a3_failure(
                PmPhaseAOnlinePreflightPostA3FailureReasonV2::ConjunctAppend,
                v1,
                v2_consumption,
                sidecar,
            ));
        }
        expected_sidecar_bytes.extend_from_slice(&encoded);
        let mut owner = PmPhaseAOnlinePreflightDispatchOwnerV2 {
            v1,
            v2_consumption,
            sidecar,
            expected_sidecar_bytes,
            basis,
            conjunct,
        };
        if owner.validate_held_complete_set().is_err() {
            return Err(owner.into_post_a3_failure(
                PmPhaseAOnlinePreflightPostA3FailureReasonV2::FinalRevalidation,
            ));
        }
        Ok(owner)
    }
}

impl fmt::Debug for PmPendingPhaseAOnlinePreflightBasisV2 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmPendingPhaseAOnlinePreflightBasisV2")
            .field("basis_record_fingerprint", &self.basis.record_fingerprint)
            .field("authorization", &OfflineAuthorizationState::DENIED)
            .field("owned_component_extraction", &false)
            .finish()
    }
}

/// Evidence-only conjunction of unchanged V1 A3 custody, the burned V2 owner,
/// and the held exact two-record sidecar. It is not a runner permit.
///
/// ```compile_fail
/// use reap_pm_controlled_trial_live::PmPhaseAOnlinePreflightDispatchOwnerV2;
/// fn cannot_extract(owner: PmPhaseAOnlinePreflightDispatchOwnerV2) {
///     let PmPhaseAOnlinePreflightDispatchOwnerV2 { v1, .. } = owner;
///     drop(v1);
/// }
/// ```
///
/// ```compile_fail
/// use reap_pm_controlled_trial_live::PmPhaseAOnlinePreflightDispatchOwnerV2;
/// fn cannot_serialize(owner: &PmPhaseAOnlinePreflightDispatchOwnerV2) {
///     let _ = serde_json::to_vec(owner).unwrap();
/// }
/// ```
pub struct PmPhaseAOnlinePreflightDispatchOwnerV2 {
    v1: PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1,
    v2_consumption: ConsumedOnlineAuthorizationConsumptionV2,
    sidecar: ProtectedJournal,
    expected_sidecar_bytes: Vec<u8>,
    basis: OnlinePreflightSidecarLineV2,
    conjunct: OnlinePreflightSidecarLineV2,
}

impl PmPhaseAOnlinePreflightDispatchOwnerV2 {
    #[must_use]
    pub fn basis_record_fingerprint(&self) -> &str {
        &self.basis.record_fingerprint
    }

    #[must_use]
    pub fn a3_conjunct_record_fingerprint(&self) -> &str {
        &self.conjunct.record_fingerprint
    }

    #[must_use]
    pub const fn authorization(&self) -> OfflineAuthorizationState {
        OfflineAuthorizationState::DENIED
    }

    /// Exact public request identity only. It contains no authenticated body,
    /// credential, HMAC, transport, or mutation operation.
    #[must_use]
    pub const fn public_request_identity(&self) -> PlacePublicRequestIdentity {
        self.v1.profile().public_request_identity()
    }

    /// Exact already-bound L2 timestamp. This is not a fresh server-time
    /// witness and cannot satisfy the later runner time gate.
    #[must_use]
    pub const fn l2_timestamp_seconds(&self) -> u64 {
        self.v1.profile().l2_timestamp_seconds()
    }

    /// Revalidate the exact V2 ledger/claim, complete sidecar bytes, and their
    /// structural binding to the already-held V1 profile. This deliberately
    /// does not perform the fresh final V1 journal/barrier/epoch recheck. A
    /// later network handoff must consume this entire composite through the
    /// journals for that separate check. This remains evidence-only.
    pub fn revalidate_held_evidence(&mut self) -> Result<(), PmPhaseAOnlinePreflightV2Error> {
        self.validate_held_complete_set()
            .map_err(PmPhaseAOnlinePreflightV2Error::Journal)
    }

    /// Destroy every possible positive place path and retain only the existing
    /// V1 DND transition input.
    #[must_use]
    pub fn into_definitely_not_dispatched(self) -> PmPhaseAPlaceDefinitelyNotDispatchedV1 {
        let Self {
            v1,
            v2_consumption,
            sidecar,
            ..
        } = self;
        drop(v2_consumption);
        drop(sidecar);
        v1.into_definitely_not_dispatched()
    }

    fn validate_held_complete_set(&mut self) -> Result<(), PmTrialLiveJournalError> {
        self.sidecar
            .validate_exact_bytes(&self.expected_sidecar_bytes)?;
        self.v2_consumption
            .revalidate_held_consumption_evidence()
            .map_err(|_| PmTrialLiveJournalError::Protection)?;
        let lines = parse_sidecar(&self.expected_sidecar_bytes)?;
        if lines.as_slice() != [self.basis.clone(), self.conjunct.clone()] {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
        validate_complete_bindings(
            &self.basis,
            &self.conjunct,
            self.v1.profile(),
            &self.v2_consumption,
        )
    }

    fn into_post_a3_failure(
        self,
        reason: PmPhaseAOnlinePreflightPostA3FailureReasonV2,
    ) -> PmPhaseAOnlinePreflightV2Error {
        let Self {
            v1,
            v2_consumption,
            sidecar,
            ..
        } = self;
        post_a3_failure(reason, v1, v2_consumption, sidecar)
    }
}

impl fmt::Debug for PmPhaseAOnlinePreflightDispatchOwnerV2 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmPhaseAOnlinePreflightDispatchOwnerV2")
            .field("basis_record_fingerprint", &self.basis.record_fingerprint)
            .field(
                "a3_conjunct_record_fingerprint",
                &self.conjunct.record_fingerprint,
            )
            .field("authorization", &OfflineAuthorizationState::DENIED)
            .field("network_send_authority", &false)
            .field("owned_component_extraction", &false)
            .finish()
    }
}

/// Final V2 durable-evidence wrapper around the existing journal-borrowing V1
/// network owner. This still performs no HMAC or transport operation. A later
/// runner-private permit must own this whole value.
///
/// ```compile_fail
/// use reap_pm_controlled_trial_live::PmPhaseAOnlinePreflightNetworkDispatchOwnerV2;
/// fn cannot_extract(owner: PmPhaseAOnlinePreflightNetworkDispatchOwnerV2<'_>) {
///     let PmPhaseAOnlinePreflightNetworkDispatchOwnerV2 { v1, .. } = owner;
///     drop(v1);
/// }
/// ```
pub struct PmPhaseAOnlinePreflightNetworkDispatchOwnerV2<'journal> {
    v1: PmPhaseAPlaceNetworkDispatchOwnerV1<'journal>,
    v2_consumption: ConsumedOnlineAuthorizationConsumptionV2,
    sidecar: ProtectedJournal,
    expected_sidecar_bytes: Vec<u8>,
    basis: OnlinePreflightSidecarLineV2,
    conjunct: OnlinePreflightSidecarLineV2,
}

impl PmPhaseAOnlinePreflightNetworkDispatchOwnerV2<'_> {
    #[must_use]
    pub const fn public_request_identity(&self) -> PlacePublicRequestIdentity {
        self.v1.profile().public_request_identity()
    }

    #[must_use]
    pub const fn l2_timestamp_seconds(&self) -> u64 {
        self.v1.profile().l2_timestamp_seconds()
    }

    /// Consume the wrapper only after transport may have sent. V2 is checked
    /// before the existing V1 conversion performs its own final durable-set
    /// recheck.
    pub fn into_may_have_been_dispatched(
        mut self,
    ) -> Result<PmPhaseAPlaceMayHaveBeenDispatchedV1, PmPhaseAOnlinePreflightV2Error> {
        self.validate_held_complete_set()?;
        self.v1.into_may_have_been_dispatched().map_err(Into::into)
    }

    /// Consume the wrapper after a proven no-send outcome. V2 and V1 are both
    /// rechecked before the DND-only input is returned.
    pub fn into_definitely_not_dispatched(
        mut self,
    ) -> Result<PmPhaseAPlaceDefinitelyNotDispatchedV1, PmPhaseAOnlinePreflightV2Error> {
        self.validate_held_complete_set()?;
        self.v1.into_definitely_not_dispatched().map_err(Into::into)
    }

    fn validate_held_complete_set(&mut self) -> Result<(), PmPhaseAOnlinePreflightV2Error> {
        self.sidecar
            .validate_exact_bytes(&self.expected_sidecar_bytes)?;
        self.v2_consumption
            .revalidate_held_consumption_evidence()
            .map_err(|_| PmPhaseAOnlinePreflightV2Error::V2Consumption)?;
        let lines = parse_sidecar(&self.expected_sidecar_bytes)?;
        if lines.as_slice() != [self.basis.clone(), self.conjunct.clone()] {
            return Err(PmTrialLiveJournalError::InvalidRecord.into());
        }
        validate_complete_bindings(
            &self.basis,
            &self.conjunct,
            self.v1.profile(),
            &self.v2_consumption,
        )?;
        Ok(())
    }
}

impl fmt::Debug for PmPhaseAOnlinePreflightNetworkDispatchOwnerV2<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmPhaseAOnlinePreflightNetworkDispatchOwnerV2")
            .field("public_request_identity", &self.public_request_identity())
            .field("l2_timestamp_seconds", &self.l2_timestamp_seconds())
            .field("network_send_authority", &false)
            .field("owned_component_extraction", &false)
            .finish()
    }
}

impl PmControlledTrialLiveJournals {
    /// Consume the complete V2 composite at the final durable boundary. V2 is
    /// revalidated first; the unchanged V1 method then performs the fresh
    /// journal, barrier, consumption, tail, and outstanding-epoch recheck.
    pub fn revalidate_phase_a_online_preflight_v2_for_network_dispatch<'journal>(
        &'journal mut self,
        mut owner: PmPhaseAOnlinePreflightDispatchOwnerV2,
    ) -> Result<
        PmPhaseAOnlinePreflightNetworkDispatchOwnerV2<'journal>,
        PmPhaseAOnlinePreflightV2Error,
    > {
        owner.validate_held_complete_set()?;
        let PmPhaseAOnlinePreflightDispatchOwnerV2 {
            v1,
            v2_consumption,
            sidecar,
            expected_sidecar_bytes,
            basis,
            conjunct,
        } = owner;
        let v1 = self.revalidate_phase_a_place_for_network_dispatch(v1)?;
        Ok(PmPhaseAOnlinePreflightNetworkDispatchOwnerV2 {
            v1,
            v2_consumption,
            sidecar,
            expected_sidecar_bytes,
            basis,
            conjunct,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmPhaseAOnlinePreflightPostA3FailureReasonV2 {
    ParentRefresh,
    Binding,
    ConjunctAppend,
    FinalRevalidation,
}

/// A proven pre-send V2 failure after V1 A3. The sole escape destroys positive
/// authority and yields the existing V1 DND-only input.
///
/// ```compile_fail
/// use reap_pm_controlled_trial_live::PmPhaseAOnlinePreflightPostA3FailureV2;
/// fn cannot_extract(owner: PmPhaseAOnlinePreflightPostA3FailureV2) {
///     let PmPhaseAOnlinePreflightPostA3FailureV2 { v1, .. } = owner;
///     drop(v1);
/// }
/// ```
pub struct PmPhaseAOnlinePreflightPostA3FailureV2 {
    reason: PmPhaseAOnlinePreflightPostA3FailureReasonV2,
    v1: PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1,
    v2_consumption: ConsumedOnlineAuthorizationConsumptionV2,
    sidecar: ProtectedJournal,
}

impl PmPhaseAOnlinePreflightPostA3FailureV2 {
    #[must_use]
    pub const fn reason(&self) -> PmPhaseAOnlinePreflightPostA3FailureReasonV2 {
        self.reason
    }

    #[must_use]
    pub fn into_definitely_not_dispatched(self) -> PmPhaseAPlaceDefinitelyNotDispatchedV1 {
        drop(self.v2_consumption);
        drop(self.sidecar);
        self.v1.into_definitely_not_dispatched()
    }
}

impl fmt::Debug for PmPhaseAOnlinePreflightPostA3FailureV2 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmPhaseAOnlinePreflightPostA3FailureV2")
            .field("reason", &self.reason)
            .field("network_send_authority", &false)
            .field("dnd_only", &true)
            .finish()
    }
}

pub enum PmPhaseAOnlinePreflightV2Error {
    InvalidBinding,
    Journal(PmTrialLiveJournalError),
    V1Consumption,
    V2Consumption,
    PostA3(Box<PmPhaseAOnlinePreflightPostA3FailureV2>),
}

impl fmt::Debug for PmPhaseAOnlinePreflightV2Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBinding => formatter.write_str("InvalidBinding"),
            Self::Journal(error) => formatter.debug_tuple("Journal").field(error).finish(),
            Self::V1Consumption => formatter.write_str("V1Consumption"),
            Self::V2Consumption => formatter.write_str("V2Consumption"),
            Self::PostA3(failure) => formatter.debug_tuple("PostA3").field(failure).finish(),
        }
    }
}

impl fmt::Display for PmPhaseAOnlinePreflightV2Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidBinding => "Phase-A online-preflight V2 binding is invalid",
            Self::Journal(_) => "Phase-A online-preflight V2 protected journal failed",
            Self::V1Consumption => "Phase-A online-preflight V1 consumption failed closed",
            Self::V2Consumption => "Phase-A online-preflight V2 consumption failed closed",
            Self::PostA3(_) => "Phase-A online-preflight V2 failed after A3; DND only",
        })
    }
}

impl std::error::Error for PmPhaseAOnlinePreflightV2Error {}

impl From<PmTrialLiveJournalError> for PmPhaseAOnlinePreflightV2Error {
    fn from(error: PmTrialLiveJournalError) -> Self {
        Self::Journal(error)
    }
}

impl From<PmAuthorizationConsumptionError> for PmPhaseAOnlinePreflightV2Error {
    fn from(_: PmAuthorizationConsumptionError) -> Self {
        Self::V1Consumption
    }
}

impl From<PmOnlineAuthorizationConsumptionV2Error> for PmPhaseAOnlinePreflightV2Error {
    fn from(_: PmOnlineAuthorizationConsumptionV2Error) -> Self {
        Self::V2Consumption
    }
}

/// Read-only evidence summary. No variant can be reopened into a place owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PmPhaseAOnlinePreflightInspectionV2 {
    Absent,
    BasisOnly {
        basis_record_fingerprint: String,
    },
    Complete {
        basis_record_fingerprint: String,
        a3_conjunct_record_fingerprint: String,
    },
    Ambiguous,
}

impl PmPhaseAOnlinePreflightInspectionV2 {
    #[must_use]
    pub const fn authorization(&self) -> OfflineAuthorizationState {
        OfflineAuthorizationState::DENIED
    }

    #[must_use]
    pub const fn placement_resumption_allowed(&self) -> bool {
        false
    }
}

pub fn inspect_phase_a_online_preflight_v2(
    config: &CanonicalTrialConfig,
) -> Result<PmPhaseAOnlinePreflightInspectionV2, PmPhaseAOnlinePreflightV2Error> {
    let path = Path::new(&config.value().journal.artifact_directory)
        .join(PM_PHASE_A_ONLINE_PREFLIGHT_SIDECAR_FILE_V2);
    let bytes = match read_protected(&path, MAX_ONLINE_PREFLIGHT_SIDECAR_BYTES_V2) {
        Ok(bytes) => bytes,
        Err(PmTrialLiveJournalError::Absent) => {
            return Ok(PmPhaseAOnlinePreflightInspectionV2::Absent);
        }
        Err(error) => return Err(error.into()),
    };
    let lines = match parse_sidecar(&bytes) {
        Ok(lines) => lines,
        Err(_) => return Ok(PmPhaseAOnlinePreflightInspectionV2::Ambiguous),
    };
    Ok(match lines.as_slice() {
        [basis] => PmPhaseAOnlinePreflightInspectionV2::BasisOnly {
            basis_record_fingerprint: basis.record_fingerprint.clone(),
        },
        [basis, conjunct] => PmPhaseAOnlinePreflightInspectionV2::Complete {
            basis_record_fingerprint: basis.record_fingerprint.clone(),
            a3_conjunct_record_fingerprint: conjunct.record_fingerprint.clone(),
        },
        _ => PmPhaseAOnlinePreflightInspectionV2::Ambiguous,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn create_phase_a_online_preflight_basis_v2(
    config: &CanonicalTrialConfig,
    v1_authorization: &CanonicalAuthorization,
    journals: &mut PmControlledTrialLiveJournals,
    prepared: PmDurablePlacePreparedAckV1,
    mut v1_consumption: PreparedAuthorizationConsumption,
    mut v2_consumption: PreparedOnlineAuthorizationConsumptionV2,
    evidence: PmPhaseAOnlinePreflightEvidenceManifestV2,
) -> Result<PmPendingPhaseAOnlinePreflightBasisV2, PmPhaseAOnlinePreflightV2Error> {
    evidence.validate()?;
    journals.validate_online_preflight_v2_place_prepared(&prepared)?;
    validate_prepared_bindings(
        config,
        v1_authorization,
        journals,
        &prepared,
        &v1_consumption,
        &v2_consumption,
    )?;

    // Both ledgers predate the V1 journal files. Refresh their parent views
    // before creating the sole sidecar directory entry.
    v1_consumption.refresh_after_bound_artifact_create()?;
    v2_consumption.refresh_after_bound_artifact_create()?;

    let basis = make_basis(
        config,
        v1_authorization,
        journals,
        &prepared,
        &v1_consumption,
        &v2_consumption,
        evidence,
    )?;
    let encoded = encode_line(&basis)?;
    let path = Path::new(&config.value().journal.artifact_directory)
        .join(PM_PHASE_A_ONLINE_PREFLIGHT_SIDECAR_FILE_V2);
    let mut sidecar = ProtectedJournal::create_new(&path, MAX_ONLINE_PREFLIGHT_SIDECAR_BYTES_V2)?;
    sidecar.append_durable(&[], &encoded)?;

    // Basis creation changes the shared parent. No caller sees the pending
    // owner until every already-held descriptor view has been refreshed.
    v1_consumption.refresh_after_bound_artifact_create()?;
    v2_consumption.refresh_after_bound_artifact_create()?;
    journals.refresh_after_online_preflight_v2_artifact_create()?;
    sidecar.validate_exact_bytes(&encoded)?;

    Ok(PmPendingPhaseAOnlinePreflightBasisV2 {
        prepared,
        v1_consumption,
        v2_consumption,
        sidecar,
        expected_sidecar_bytes: encoded,
        basis,
    })
}

fn validate_prepared_bindings(
    config: &CanonicalTrialConfig,
    v1_authorization: &CanonicalAuthorization,
    journals: &PmControlledTrialLiveJournals,
    prepared: &PmDurablePlacePreparedAckV1,
    v1_consumption: &PreparedAuthorizationConsumption,
    v2_consumption: &PreparedOnlineAuthorizationConsumptionV2,
) -> Result<(), PmPhaseAOnlinePreflightV2Error> {
    let v1_verification = verify_authorization_consumption(config, v1_authorization)
        .map_err(|_| PmPhaseAOnlinePreflightV2Error::V1Consumption)?;
    let v1_evidence = v1_consumption.evidence();
    let v1_binding = &v1_evidence.binding;
    let v2_evidence = v2_consumption.evidence();
    let v2_binding = &v2_evidence.binding;
    let identity = config.exact_place_public_request_identity();
    if !matches!(
        v1_verification.state,
        AuthorizationConsumptionState::Prepared { .. }
    ) || v1_verification.ledger_record_count != 1
        || v1_verification.atomic_consumption_claim_durable
        || v1_verification.consumed_ledger_record_durable
        || v1_verification.latest_record_fingerprint == ZERO_FINGERPRINT
        || v1_evidence.sequence != 0
        || v1_evidence.authorization != OfflineAuthorizationState::DENIED
        || !matches!(
            v2_evidence.consumption,
            OnlineAuthorizationConsumptionStateV2::Prepared { .. }
        )
        || v2_evidence.sequence != 0
        || v2_evidence.authorization != OfflineAuthorizationState::DENIED
        || v1_binding.canonical_config_sha256 != config.canonical_sha256()
        || v1_binding.canonical_config_length != config.canonical_length()
        || v1_binding.canonical_config_fingerprint != config.fingerprint()
        || v1_binding.trial_plan_fingerprint != config.plan_fingerprint()
        || v2_binding.v1_config.canonical_config_sha256 != config.canonical_sha256()
        || v2_binding.v1_config.canonical_config_length != config.canonical_length()
        || v2_binding.v1_config.canonical_config_fingerprint != config.fingerprint()
        || v2_binding.v1_config.trial_plan_fingerprint != config.plan_fingerprint()
        || v2_binding.artifact_directory != config.value().journal.artifact_directory
        || v2_binding.expected_order_id != identity.expected_order_id().to_string()
        || v2_binding.semantic_request_commitment
            != identity.semantic_request_commitment().to_string()
        || v2_binding.online_preflight_sidecar_file != PM_PHASE_A_ONLINE_PREFLIGHT_SIDECAR_FILE_V2
        || prepared.preparation().expected_order_id() != identity.expected_order_id()
        || prepared.preparation().semantic_request_commitment()
            != identity.semantic_request_commitment()
        || journals.online_preflight_v2_scope_fingerprint().is_empty()
    {
        return Err(PmPhaseAOnlinePreflightV2Error::InvalidBinding);
    }
    let policy = v2_consumption.policy();
    let authorization = v2_consumption.authorization();
    if v2_binding.online_policy.canonical_sha256 != policy.canonical_sha256()
        || v2_binding.online_policy.canonical_length != policy.canonical_length()
        || v2_binding.online_policy.fingerprint != policy.fingerprint()
        || v2_binding.online_authorization.canonical_sha256 != authorization.canonical_sha256()
        || v2_binding.online_authorization.canonical_length != authorization.canonical_length()
        || v2_binding.online_authorization.fingerprint != authorization.fingerprint()
    {
        return Err(PmPhaseAOnlinePreflightV2Error::InvalidBinding);
    }
    Ok(())
}

fn make_basis(
    config: &CanonicalTrialConfig,
    v1_authorization: &CanonicalAuthorization,
    journals: &PmControlledTrialLiveJournals,
    prepared: &PmDurablePlacePreparedAckV1,
    v1_consumption: &PreparedAuthorizationConsumption,
    v2_consumption: &PreparedOnlineAuthorizationConsumptionV2,
    evidence: PmPhaseAOnlinePreflightEvidenceManifestV2,
) -> Result<OnlinePreflightSidecarLineV2, PmPhaseAOnlinePreflightV2Error> {
    let v1_verification = verify_authorization_consumption(config, v1_authorization)
        .map_err(|_| PmPhaseAOnlinePreflightV2Error::V1Consumption)?;
    let v1 = v1_consumption.evidence();
    let v2 = v2_consumption.evidence();
    let policy = v2_consumption.policy();
    let authorization = v2_consumption.authorization();
    let preflight = journals.preflight_binding();
    let preparation = prepared.preparation();
    let body = OnlinePreflightBasisV2 {
        canonical_config_sha256: config.canonical_sha256().to_owned(),
        canonical_config_length: config.canonical_length(),
        canonical_config_fingerprint: config.fingerprint().to_owned(),
        trial_plan_fingerprint: config.plan_fingerprint().to_owned(),
        preflight_fingerprint: preflight.fingerprint().to_owned(),
        preflight_canonical_sha256: preflight.canonical_sha256().to_owned(),
        preflight_validated_at_utc: preflight.validated_at_utc().to_owned(),
        preflight_dispatch_deadline_at_utc: preflight.dispatch_deadline_at_utc().to_owned(),
        place_prepared_sequence: prepared.sequence(),
        place_prepared_record_fingerprint: prepared.record_fingerprint().to_owned(),
        public_semantic_request_commitment: lower_hex32(
            preparation.semantic_request_commitment().bytes(),
        ),
        prepared_request_commitment: preparation.request_commitment().to_owned(),
        expected_order_id: lower_hex32(preparation.expected_order_id().bytes()),
        l2_timestamp_seconds: preparation.l2_timestamp_seconds(),
        v1_consumption_binding_fingerprint: v1.binding_fingerprint.clone(),
        v1_consumption_prepared_record_fingerprint: v1_verification.latest_record_fingerprint,
        v2_consumption_binding_fingerprint: v2.binding_fingerprint.clone(),
        v2_consumption_prepared_record_fingerprint: v2_consumption
            .prepared_record_fingerprint()
            .to_owned(),
        online_policy: OnlinePreflightV2Pins {
            canonical_sha256: policy.canonical_sha256().to_owned(),
            canonical_length: policy.canonical_length(),
            fingerprint: policy.fingerprint().to_owned(),
        },
        online_authorization_id: authorization.value().authorization_id.clone(),
        online_authorization: OnlinePreflightV2Pins {
            canonical_sha256: authorization.canonical_sha256().to_owned(),
            canonical_length: authorization.canonical_length(),
            fingerprint: authorization.fingerprint().to_owned(),
        },
        online_authorization_not_before_utc: authorization.value().not_before_utc.clone(),
        online_authorization_expires_at_utc: authorization.value().expires_at_utc.clone(),
        online_authorization_cleanup_not_after_utc: authorization
            .value()
            .cleanup_not_after_utc
            .clone(),
        evidence,
    };
    Ok(OnlinePreflightSidecarLineV2::seal(
        0,
        ZERO_FINGERPRINT.to_owned(),
        journals.online_preflight_v2_scope_fingerprint().to_owned(),
        OnlinePreflightSidecarBodyV2::Basis {
            basis: Box::new(body),
        },
    )?)
}

fn make_a3_conjunct(
    basis: &OnlinePreflightSidecarLineV2,
    v1: &PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1,
    v2: &ConsumedOnlineAuthorizationConsumptionV2,
) -> Result<OnlinePreflightSidecarLineV2, PmTrialLiveJournalError> {
    let profile = v1.profile();
    let v2_policy = v2.policy();
    let v2_authorization = v2.authorization();
    let durable_manifest = OnlinePreflightDurableManifestV2 {
        v1_intent_file: PM_TRIAL_LIVE_INTENT_FILE_V1.to_owned(),
        v1_dispatch_file: PM_TRIAL_LIVE_DISPATCH_FILE_V1.to_owned(),
        v1_scope_fingerprint: profile.scope_fingerprint().to_owned(),
        v1_place_prepared_record_fingerprint: profile
            .place_prepared_record_fingerprint()
            .to_owned(),
        v1_dispatch_authorized_record_fingerprint: profile
            .v1_dispatch_authorized_record_fingerprint()
            .to_owned(),
        v1_dispatch_barrier_record_fingerprint: profile
            .durable_barrier_record_fingerprint()
            .to_owned(),
        v1_consumption_binding_fingerprint: profile
            .authorization_consumption_binding_fingerprint()
            .to_owned(),
        v1_consumption_prepared_record_fingerprint: profile
            .authorization_consumption_prepared_record_fingerprint()
            .to_owned(),
        v1_consumption_claim_fingerprint: profile.atomic_claim_fingerprint().to_owned(),
        v1_consumption_consumed_record_fingerprint: profile
            .consumed_record_fingerprint()
            .to_owned(),
        v2_consumption_ledger_file: PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_LEDGER_FILE_V2
            .to_owned(),
        v2_consumption_claim_file: PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_CLAIM_FILE_V2.to_owned(),
        v2_consumption_binding_fingerprint: v2.binding_fingerprint().to_owned(),
        v2_consumption_prepared_record_fingerprint: v2.prepared_record_fingerprint().to_owned(),
        v2_consumption_claim_fingerprint: v2.atomic_claim_fingerprint().to_owned(),
        v2_consumption_consumed_record_fingerprint: v2.consumed_record_fingerprint().to_owned(),
        online_preflight_sidecar_file: PM_PHASE_A_ONLINE_PREFLIGHT_SIDECAR_FILE_V2.to_owned(),
        online_preflight_basis_record_fingerprint: basis.record_fingerprint.clone(),
    };
    durable_manifest.validate()?;
    let durable_manifest_fingerprint = hash_domain(
        ONLINE_PREFLIGHT_DURABLE_MANIFEST_FINGERPRINT_DOMAIN_V2,
        &durable_manifest,
    )?;
    let body = OnlinePreflightA3ConjunctV2 {
        basis_sequence: basis.sequence,
        basis_record_fingerprint: basis.record_fingerprint.clone(),
        durable_manifest_fingerprint,
        durable_manifest,
        online_policy: OnlinePreflightV2Pins {
            canonical_sha256: v2_policy.canonical_sha256().to_owned(),
            canonical_length: v2_policy.canonical_length(),
            fingerprint: v2_policy.fingerprint().to_owned(),
        },
        online_authorization_id: v2_authorization.value().authorization_id.clone(),
        online_authorization: OnlinePreflightV2Pins {
            canonical_sha256: v2_authorization.canonical_sha256().to_owned(),
            canonical_length: v2_authorization.canonical_length(),
            fingerprint: v2_authorization.fingerprint().to_owned(),
        },
        expected_order_id: lower_hex32(profile.expected_order_id().bytes()),
        public_semantic_request_commitment: lower_hex32(
            profile.semantic_request_commitment().bytes(),
        ),
        prepared_request_commitment: profile.prepared_request_commitment().to_owned(),
    };
    OnlinePreflightSidecarLineV2::seal(
        1,
        basis.record_fingerprint.clone(),
        basis.scope_fingerprint.clone(),
        OnlinePreflightSidecarBodyV2::A3Conjunct {
            conjunct: Box::new(body),
        },
    )
}

fn validate_complete_bindings(
    basis: &OnlinePreflightSidecarLineV2,
    conjunct: &OnlinePreflightSidecarLineV2,
    profile: &PmPhaseAPlaceLiveDispatchProfileV1,
    v2: &ConsumedOnlineAuthorizationConsumptionV2,
) -> Result<(), PmTrialLiveJournalError> {
    basis.validate_structural()?;
    conjunct.validate_structural()?;
    let OnlinePreflightSidecarBodyV2::Basis { basis: basis_body } = &basis.body else {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    };
    let OnlinePreflightSidecarBodyV2::A3Conjunct { conjunct: body } = &conjunct.body else {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    };
    let policy = v2.policy();
    let authorization = v2.authorization();
    if conjunct.previous_record_fingerprint != basis.record_fingerprint
        || conjunct.scope_fingerprint != basis.scope_fingerprint
        || body.basis_sequence != basis.sequence
        || body.basis_record_fingerprint != basis.record_fingerprint
        || body
            .durable_manifest
            .online_preflight_basis_record_fingerprint
            != basis.record_fingerprint
        || body.durable_manifest.v1_scope_fingerprint != profile.scope_fingerprint()
        || body.durable_manifest.v1_place_prepared_record_fingerprint
            != profile.place_prepared_record_fingerprint()
        || body
            .durable_manifest
            .v1_dispatch_authorized_record_fingerprint
            != profile.v1_dispatch_authorized_record_fingerprint()
        || body.durable_manifest.v1_dispatch_barrier_record_fingerprint
            != profile.durable_barrier_record_fingerprint()
        || body.durable_manifest.v1_consumption_binding_fingerprint
            != profile.authorization_consumption_binding_fingerprint()
        || body
            .durable_manifest
            .v1_consumption_prepared_record_fingerprint
            != profile.authorization_consumption_prepared_record_fingerprint()
        || body.durable_manifest.v1_consumption_claim_fingerprint
            != profile.atomic_claim_fingerprint()
        || body
            .durable_manifest
            .v1_consumption_consumed_record_fingerprint
            != profile.consumed_record_fingerprint()
        || body.durable_manifest.v2_consumption_binding_fingerprint != v2.binding_fingerprint()
        || body
            .durable_manifest
            .v2_consumption_prepared_record_fingerprint
            != v2.prepared_record_fingerprint()
        || body.durable_manifest.v2_consumption_claim_fingerprint != v2.atomic_claim_fingerprint()
        || body
            .durable_manifest
            .v2_consumption_consumed_record_fingerprint
            != v2.consumed_record_fingerprint()
        || body.online_policy.canonical_sha256 != policy.canonical_sha256()
        || body.online_policy.canonical_length != policy.canonical_length()
        || body.online_policy.fingerprint != policy.fingerprint()
        || body.online_authorization_id != authorization.value().authorization_id
        || body.online_authorization.canonical_sha256 != authorization.canonical_sha256()
        || body.online_authorization.canonical_length != authorization.canonical_length()
        || body.online_authorization.fingerprint != authorization.fingerprint()
        || body.expected_order_id != lower_hex32(profile.expected_order_id().bytes())
        || body.public_semantic_request_commitment
            != lower_hex32(profile.semantic_request_commitment().bytes())
        || body.prepared_request_commitment != profile.prepared_request_commitment()
        || basis_body.online_policy != body.online_policy
        || basis_body.online_authorization_id != body.online_authorization_id
        || basis_body.online_authorization != body.online_authorization
        || basis_body.expected_order_id != body.expected_order_id
        || basis_body.public_semantic_request_commitment != body.public_semantic_request_commitment
        || basis_body.prepared_request_commitment != body.prepared_request_commitment
        || hash_domain(
            ONLINE_PREFLIGHT_DURABLE_MANIFEST_FINGERPRINT_DOMAIN_V2,
            &body.durable_manifest,
        )? != body.durable_manifest_fingerprint
    {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    Ok(())
}

fn validate_basis(value: &OnlinePreflightBasisV2) -> Result<(), PmTrialLiveJournalError> {
    value.online_policy.validate()?;
    value.online_authorization.validate()?;
    value.evidence.validate()?;
    if value.canonical_config_length == 0 || value.l2_timestamp_seconds == 0 {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    for fingerprint in [
        &value.canonical_config_sha256,
        &value.canonical_config_fingerprint,
        &value.trial_plan_fingerprint,
        &value.preflight_fingerprint,
        &value.preflight_canonical_sha256,
        &value.place_prepared_record_fingerprint,
        &value.public_semantic_request_commitment,
        &value.prepared_request_commitment,
        &value.expected_order_id,
        &value.v1_consumption_binding_fingerprint,
        &value.v1_consumption_prepared_record_fingerprint,
        &value.v2_consumption_binding_fingerprint,
        &value.v2_consumption_prepared_record_fingerprint,
    ] {
        validate_fingerprint(fingerprint)?;
    }
    let validated = validate_utc(&value.preflight_validated_at_utc)?;
    let deadline = validate_utc(&value.preflight_dispatch_deadline_at_utc)?;
    let not_before = validate_utc(&value.online_authorization_not_before_utc)?;
    let expires = validate_utc(&value.online_authorization_expires_at_utc)?;
    let cleanup = validate_utc(&value.online_authorization_cleanup_not_after_utc)?;
    if deadline < validated || expires <= not_before || cleanup < expires {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    Ok(())
}

fn validate_conjunct(value: &OnlinePreflightA3ConjunctV2) -> Result<(), PmTrialLiveJournalError> {
    value.online_policy.validate()?;
    value.online_authorization.validate()?;
    value.durable_manifest.validate()?;
    if value.basis_sequence != 0 {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    for fingerprint in [
        &value.basis_record_fingerprint,
        &value.durable_manifest_fingerprint,
        &value.expected_order_id,
        &value.public_semantic_request_commitment,
        &value.prepared_request_commitment,
    ] {
        validate_fingerprint(fingerprint)?;
    }
    if hash_domain(
        ONLINE_PREFLIGHT_DURABLE_MANIFEST_FINGERPRINT_DOMAIN_V2,
        &value.durable_manifest,
    )? != value.durable_manifest_fingerprint
    {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    Ok(())
}

fn encode_line(line: &OnlinePreflightSidecarLineV2) -> Result<Vec<u8>, PmTrialLiveJournalError> {
    line.validate_structural()?;
    let mut encoded = canonical_json(line)?;
    encoded.push(b'\n');
    if encoded.len() > MAX_ONLINE_PREFLIGHT_SIDECAR_LINE_BYTES_V2 {
        return Err(PmTrialLiveJournalError::BoundExceeded);
    }
    Ok(encoded)
}

fn parse_sidecar(
    bytes: &[u8],
) -> Result<Vec<OnlinePreflightSidecarLineV2>, PmTrialLiveJournalError> {
    if bytes.is_empty()
        || bytes.len() > MAX_ONLINE_PREFLIGHT_SIDECAR_BYTES_V2
        || !bytes.ends_with(b"\n")
    {
        return Err(PmTrialLiveJournalError::AmbiguousTail);
    }
    let mut lines = Vec::new();
    for raw in bytes[..bytes.len() - 1].split(|byte| *byte == b'\n') {
        if raw.is_empty() || raw.len() + 1 > MAX_ONLINE_PREFLIGHT_SIDECAR_LINE_BYTES_V2 {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
        let line: OnlinePreflightSidecarLineV2 =
            serde_json::from_slice(raw).map_err(|_| PmTrialLiveJournalError::InvalidRecord)?;
        if canonical_json(&line)? != raw {
            return Err(PmTrialLiveJournalError::InvalidRecord);
        }
        line.validate_structural()?;
        lines.push(line);
    }
    match lines.as_slice() {
        [basis] if matches!(basis.body, OnlinePreflightSidecarBodyV2::Basis { .. }) => {}
        [basis, conjunct]
            if matches!(basis.body, OnlinePreflightSidecarBodyV2::Basis { .. })
                && matches!(
                    conjunct.body,
                    OnlinePreflightSidecarBodyV2::A3Conjunct { .. }
                )
                && conjunct.previous_record_fingerprint == basis.record_fingerprint
                && conjunct.scope_fingerprint == basis.scope_fingerprint => {}
        _ => return Err(PmTrialLiveJournalError::InvalidRecord),
    }
    Ok(lines)
}

fn post_a3_failure(
    reason: PmPhaseAOnlinePreflightPostA3FailureReasonV2,
    v1: PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1,
    v2_consumption: ConsumedOnlineAuthorizationConsumptionV2,
    sidecar: ProtectedJournal,
) -> PmPhaseAOnlinePreflightV2Error {
    PmPhaseAOnlinePreflightV2Error::PostA3(Box::new(PmPhaseAOnlinePreflightPostA3FailureV2 {
        reason,
        v1,
        v2_consumption,
        sidecar,
    }))
}

fn lower_hex32(bytes: [u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
