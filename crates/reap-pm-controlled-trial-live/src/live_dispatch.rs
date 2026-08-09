use std::path::{Path, PathBuf};

use reap_pm_controlled_trial::{
    AuthorizationBuildBinding, AuthorizationHostBinding, AuthorizationRuntimeBinding,
    CanonicalAuthorization, CanonicalTrialConfig, ExpectedOrderId, PlacePublicRequestIdentity,
    PlaceSemanticRequestCommitment, TrialCredentialSlot, TrialPhase,
};
use serde::{Deserialize, Serialize};

use crate::{
    PmTrialLiveJournalError,
    hash::{ZERO_FINGERPRINT, canonical_json, hash_domain, validate_fingerprint},
    protected::{ProtectedJournal, read_protected},
    schema::{
        DispatchLineV1, DispatchRecordV1, PmPlacePreparationViewV1,
        PmTrialLiveConsumedFingerprintsV1, PmTrialLiveJournalScopeV1,
        PmTrialLivePreflightBindingV1, dispatch_fingerprint,
    },
};

/// Separate create-new durability barrier for the sole positive Phase-A place
/// dispatch profile. The evidence-only V1 dispatch journal remains unchanged.
pub const PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1: &str =
    "pm-t2-phase-a-live-place-dispatch-barrier-v1.jsonl";

const PM_PHASE_A_LIVE_DISPATCH_FAMILY_V1: &str = "pm-t2-phase-a-live-place-dispatch";
const PM_PHASE_A_LIVE_DISPATCH_PROFILE_V1: &str = "pm_t2_type1_proxy_phase_a_live_dispatch_v1";
const PM_PHASE_A_LIVE_DISPATCH_RECORD_KIND_V1: &str = "place_dispatch_authorized";
const PM_PHASE_A_LIVE_DISPATCH_SCHEMA_VERSION_V1: u32 = 1;
const MAX_PHASE_A_LIVE_DISPATCH_BARRIER_BYTES: usize = 64 * 1_024;
const PHASE_A_LIVE_DISPATCH_RECORD_FINGERPRINT_DOMAIN: &[u8] =
    b"reap.pm-t2.phase-a.live-place-dispatch.record.v1\0";

/// Exact non-secret creation context retained in memory from initial journal
/// creation. It is not authority and cannot mint a grant by itself.
pub(crate) struct PmPhaseAPlaceLiveDispatchContextV1 {
    canonical_config_sha256: String,
    canonical_config_length: u64,
    canonical_config_fingerprint: String,
    trial_plan_fingerprint: String,
    authorization_id: String,
    authorization_fingerprint: String,
    authorization_not_before_utc: String,
    authorization_expires_at_utc: String,
    authorization_cleanup_not_after_utc: String,
    source_pin_manifest_sha256: String,
    build: AuthorizationBuildBinding,
    host: AuthorizationHostBinding,
    credential_slot: TrialCredentialSlot,
    public_request_identity: PlacePublicRequestIdentity,
}

impl PmPhaseAPlaceLiveDispatchContextV1 {
    pub(crate) fn for_exact_phase_a(
        config: &CanonicalTrialConfig,
        authorization: &CanonicalAuthorization,
        runtime: &AuthorizationRuntimeBinding,
        scope: &PmTrialLiveJournalScopeV1,
    ) -> Result<Option<Self>, PmTrialLiveJournalError> {
        if config.value().phase != TrialPhase::APlaceCancel {
            return Ok(None);
        }
        let authorization_value = authorization.value();
        if authorization_value.phase != TrialPhase::APlaceCancel
            || authorization_value.trial != *config.value()
            || authorization_value.trial_plan_fingerprint != config.plan_fingerprint()
            || authorization_value.build.canonical_config_sha256 != config.canonical_sha256()
            || authorization_value.build.canonical_config_length != config.canonical_length()
            || authorization_value.build.canonical_config_fingerprint != config.fingerprint()
            || !authorization_value.build.clean_tree_attested
            || runtime.release_binary_sha256 != authorization_value.build.release_binary_sha256
            || runtime.release_binary_length != authorization_value.build.release_binary_length
            || runtime.host != authorization_value.host
            || scope.canonical_config_sha256 != config.canonical_sha256()
            || scope.canonical_config_length != config.canonical_length()
            || scope.canonical_config_fingerprint != config.fingerprint()
            || scope.trial_plan_fingerprint != config.plan_fingerprint()
            || scope.authorization_id != authorization_value.authorization_id
            || scope.authorization_fingerprint != authorization.fingerprint()
            || scope.source_pin_manifest_sha256 != config.value().source_pin_manifest_sha256
            || scope.release_binary_sha256 != authorization_value.build.release_binary_sha256
            || scope.release_binary_length != authorization_value.build.release_binary_length
            || scope.host != authorization_value.host
            || scope.credential_slot_id != config.value().credential_slot.slot_id
            || scope.credential_slot_nonsecret_fingerprint_sha256
                != config.value().credential_slot.nonsecret_fingerprint_sha256
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        Ok(Some(Self {
            canonical_config_sha256: config.canonical_sha256().to_owned(),
            canonical_config_length: config.canonical_length(),
            canonical_config_fingerprint: config.fingerprint().to_owned(),
            trial_plan_fingerprint: config.plan_fingerprint().to_owned(),
            authorization_id: authorization_value.authorization_id.clone(),
            authorization_fingerprint: authorization.fingerprint().to_owned(),
            authorization_not_before_utc: authorization_value.not_before_utc.clone(),
            authorization_expires_at_utc: authorization_value.expires_at_utc.clone(),
            authorization_cleanup_not_after_utc: authorization_value.cleanup_not_after_utc.clone(),
            source_pin_manifest_sha256: config.value().source_pin_manifest_sha256.clone(),
            build: authorization_value.build.clone(),
            host: authorization_value.host.clone(),
            credential_slot: config.value().credential_slot.clone(),
            public_request_identity: config.exact_place_public_request_identity(),
        }))
    }
}

pub(crate) struct PmPhaseAPlaceDispatchMintEvidenceV1 {
    pub(crate) preparation: PmPlacePreparationViewV1,
    pub(crate) prepared_sequence: u8,
    pub(crate) prepared_record_fingerprint: String,
    pub(crate) consumption: PmTrialLiveConsumedFingerprintsV1,
    pub(crate) v1_dispatch_authorized_sequence: u8,
    pub(crate) v1_dispatch_authorized_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PmPhaseAPlaceLiveDispatchRecordV1 {
    schema_version: u32,
    family: String,
    profile: String,
    record: String,
    scope_fingerprint: String,
    canonical_config_sha256: String,
    canonical_config_length: u64,
    canonical_config_fingerprint: String,
    trial_plan_fingerprint: String,
    authorization_id: String,
    authorization_fingerprint: String,
    authorization_not_before_utc: String,
    authorization_expires_at_utc: String,
    authorization_cleanup_not_after_utc: String,
    preflight_fingerprint: String,
    preflight_canonical_sha256: String,
    preflight_validated_at_utc: String,
    preflight_dispatch_deadline_at_utc: String,
    source_pin_manifest_sha256: String,
    public_semantic_request_commitment: String,
    prepared_request_commitment: String,
    expected_order_id: String,
    l2_timestamp_seconds: u64,
    authorization_consumption_binding_fingerprint: String,
    authorization_consumption_prepared_record_fingerprint: String,
    atomic_claim_fingerprint: String,
    consumed_record_fingerprint: String,
    place_prepared_sequence: u8,
    place_prepared_record_fingerprint: String,
    v1_dispatch_authorized_sequence: u8,
    v1_dispatch_authorized_fingerprint: String,
    build: AuthorizationBuildBinding,
    host: AuthorizationHostBinding,
    credential_slot: TrialCredentialSlot,
    production_order_entry_authorized: bool,
    real_order_submission_authorized: bool,
    place_dispatch_allowance: u8,
    placement_resumption_allowed: bool,
    record_fingerprint: String,
}

impl PmPhaseAPlaceLiveDispatchRecordV1 {
    fn seal(mut self) -> Result<Self, PmTrialLiveJournalError> {
        self.record_fingerprint = ZERO_FINGERPRINT.to_owned();
        self.record_fingerprint =
            hash_domain(PHASE_A_LIVE_DISPATCH_RECORD_FINGERPRINT_DOMAIN, &self)?;
        self.validate_structural()?;
        Ok(self)
    }

    fn validate_structural(&self) -> Result<(), PmTrialLiveJournalError> {
        if self.schema_version != PM_PHASE_A_LIVE_DISPATCH_SCHEMA_VERSION_V1
            || self.family != PM_PHASE_A_LIVE_DISPATCH_FAMILY_V1
            || self.profile != PM_PHASE_A_LIVE_DISPATCH_PROFILE_V1
            || self.record != PM_PHASE_A_LIVE_DISPATCH_RECORD_KIND_V1
            || self.canonical_config_length == 0
            || self.l2_timestamp_seconds == 0
            || !self.production_order_entry_authorized
            || !self.real_order_submission_authorized
            || self.place_dispatch_allowance != 1
            || self.placement_resumption_allowed
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        for fingerprint in [
            &self.scope_fingerprint,
            &self.canonical_config_sha256,
            &self.canonical_config_fingerprint,
            &self.trial_plan_fingerprint,
            &self.authorization_fingerprint,
            &self.preflight_fingerprint,
            &self.preflight_canonical_sha256,
            &self.source_pin_manifest_sha256,
            &self.public_semantic_request_commitment,
            &self.prepared_request_commitment,
            &self.expected_order_id,
            &self.authorization_consumption_binding_fingerprint,
            &self.authorization_consumption_prepared_record_fingerprint,
            &self.atomic_claim_fingerprint,
            &self.consumed_record_fingerprint,
            &self.place_prepared_record_fingerprint,
            &self.v1_dispatch_authorized_fingerprint,
            &self.record_fingerprint,
        ] {
            validate_fingerprint(fingerprint)?;
        }
        let mut basis = self.clone();
        basis.record_fingerprint = ZERO_FINGERPRINT.to_owned();
        if hash_domain(PHASE_A_LIVE_DISPATCH_RECORD_FINGERPRINT_DOMAIN, &basis)?
            != self.record_fingerprint
        {
            return Err(PmTrialLiveJournalError::InvalidBinding);
        }
        Ok(())
    }
}

/// Closed positive profile carried only by a successfully fsynced, create-new
/// Phase-A dispatch grant. It intentionally implements neither `Clone` nor
/// Serde traits.
pub struct PmPhaseAPlaceLiveDispatchProfileV1 {
    record: PmPhaseAPlaceLiveDispatchRecordV1,
    public_request_identity: PlacePublicRequestIdentity,
}

impl PmPhaseAPlaceLiveDispatchProfileV1 {
    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        true
    }

    #[must_use]
    pub const fn real_order_submission_authorized(&self) -> bool {
        true
    }

    #[must_use]
    pub const fn place_dispatch_allowance(&self) -> u8 {
        1
    }

    #[must_use]
    pub const fn placement_resumption_allowed(&self) -> bool {
        false
    }

    #[must_use]
    pub const fn phase(&self) -> TrialPhase {
        TrialPhase::APlaceCancel
    }

    #[must_use]
    pub fn scope_fingerprint(&self) -> &str {
        &self.record.scope_fingerprint
    }

    #[must_use]
    pub fn canonical_config_sha256(&self) -> &str {
        &self.record.canonical_config_sha256
    }

    #[must_use]
    pub const fn canonical_config_length(&self) -> u64 {
        self.record.canonical_config_length
    }

    #[must_use]
    pub fn canonical_config_fingerprint(&self) -> &str {
        &self.record.canonical_config_fingerprint
    }

    #[must_use]
    pub fn trial_plan_fingerprint(&self) -> &str {
        &self.record.trial_plan_fingerprint
    }

    #[must_use]
    pub fn authorization_id(&self) -> &str {
        &self.record.authorization_id
    }

    #[must_use]
    pub fn authorization_fingerprint(&self) -> &str {
        &self.record.authorization_fingerprint
    }

    #[must_use]
    pub fn authorization_not_before_utc(&self) -> &str {
        &self.record.authorization_not_before_utc
    }

    #[must_use]
    pub fn authorization_expires_at_utc(&self) -> &str {
        &self.record.authorization_expires_at_utc
    }

    #[must_use]
    pub fn authorization_cleanup_not_after_utc(&self) -> &str {
        &self.record.authorization_cleanup_not_after_utc
    }

    #[must_use]
    pub fn preflight_fingerprint(&self) -> &str {
        &self.record.preflight_fingerprint
    }

    #[must_use]
    pub fn preflight_canonical_sha256(&self) -> &str {
        &self.record.preflight_canonical_sha256
    }

    #[must_use]
    pub fn preflight_validated_at_utc(&self) -> &str {
        &self.record.preflight_validated_at_utc
    }

    #[must_use]
    pub fn preflight_dispatch_deadline_at_utc(&self) -> &str {
        &self.record.preflight_dispatch_deadline_at_utc
    }

    #[must_use]
    pub const fn public_request_identity(&self) -> PlacePublicRequestIdentity {
        self.public_request_identity
    }

    #[must_use]
    pub const fn semantic_request_commitment(&self) -> PlaceSemanticRequestCommitment {
        self.public_request_identity.semantic_request_commitment()
    }

    #[must_use]
    pub const fn expected_order_id(&self) -> ExpectedOrderId {
        self.public_request_identity.expected_order_id()
    }

    #[must_use]
    pub fn prepared_request_commitment(&self) -> &str {
        &self.record.prepared_request_commitment
    }

    #[must_use]
    pub const fn l2_timestamp_seconds(&self) -> u64 {
        self.record.l2_timestamp_seconds
    }

    #[must_use]
    pub fn authorization_consumption_binding_fingerprint(&self) -> &str {
        &self.record.authorization_consumption_binding_fingerprint
    }

    #[must_use]
    pub fn authorization_consumption_prepared_record_fingerprint(&self) -> &str {
        &self
            .record
            .authorization_consumption_prepared_record_fingerprint
    }

    #[must_use]
    pub fn atomic_claim_fingerprint(&self) -> &str {
        &self.record.atomic_claim_fingerprint
    }

    #[must_use]
    pub fn consumed_record_fingerprint(&self) -> &str {
        &self.record.consumed_record_fingerprint
    }

    #[must_use]
    pub fn place_prepared_record_fingerprint(&self) -> &str {
        &self.record.place_prepared_record_fingerprint
    }

    #[must_use]
    pub fn v1_dispatch_authorized_record_fingerprint(&self) -> &str {
        &self.record.v1_dispatch_authorized_fingerprint
    }

    #[must_use]
    pub fn durable_barrier_record_fingerprint(&self) -> &str {
        &self.record.record_fingerprint
    }

    #[must_use]
    pub const fn build(&self) -> &AuthorizationBuildBinding {
        &self.record.build
    }

    #[must_use]
    pub const fn host(&self) -> &AuthorizationHostBinding {
        &self.record.host
    }

    #[must_use]
    pub const fn credential_slot(&self) -> &TrialCredentialSlot {
        &self.record.credential_slot
    }
}

impl std::fmt::Debug for PmPhaseAPlaceLiveDispatchProfileV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmPhaseAPlaceLiveDispatchProfileV1")
            .field("phase", &TrialPhase::APlaceCancel)
            .field("scope_fingerprint", &self.record.scope_fingerprint)
            .field("expected_order_id", &self.expected_order_id())
            .field("production_order_entry_authorized", &true)
            .field("real_order_submission_authorized", &true)
            .field("place_dispatch_allowance", &1_u8)
            .field("placement_resumption_allowed", &false)
            .finish()
    }
}

/// Move-only runner input backed by the held descriptor for the exact durable
/// positive dispatch barrier. This type has no request bytes, credentials,
/// transport, or mutation method and implements neither `Clone` nor Serde.
pub(crate) struct PmPhaseAPlaceDispatchGrantV1 {
    profile: PmPhaseAPlaceLiveDispatchProfileV1,
    barrier_file: ProtectedJournal,
    expected_barrier_bytes: Vec<u8>,
}

impl PmPhaseAPlaceDispatchGrantV1 {
    /// Consume the minted grant and revalidate the held descriptor, fixed
    /// pathname, inode, length, and exact canonical bytes immediately before a
    /// runner authority join. Higher layers must accept only the returned
    /// typestate, never this pre-join value.
    pub(crate) fn revalidate_for_runner(
        mut self,
    ) -> Result<PmRevalidatedPhaseAPlaceDispatchGrantV1, PmTrialLiveJournalError> {
        self.barrier_file
            .validate_exact_bytes(&self.expected_barrier_bytes)?;
        Ok(PmRevalidatedPhaseAPlaceDispatchGrantV1 {
            profile: self.profile,
            barrier_file: self.barrier_file,
            expected_barrier_bytes: self.expected_barrier_bytes,
        })
    }
}

impl std::fmt::Debug for PmPhaseAPlaceDispatchGrantV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmPhaseAPlaceDispatchGrantV1")
            .field(
                "durable_barrier_record_fingerprint",
                &self.profile.record.record_fingerprint,
            )
            .field("runner_join_revalidated", &false)
            .field("request_body", &"[ABSENT]")
            .field("credentials", &"[ABSENT]")
            .field("transport", &"[ABSENT]")
            .finish()
    }
}

/// Move-only runner-join typestate emitted only after an exact descriptor,
/// pathname, and byte-for-byte barrier recheck. It has no send operation.
pub(crate) struct PmRevalidatedPhaseAPlaceDispatchGrantV1 {
    profile: PmPhaseAPlaceLiveDispatchProfileV1,
    barrier_file: ProtectedJournal,
    expected_barrier_bytes: Vec<u8>,
}

/// Non-authority custody of the exact positive barrier after the one dispatch
/// grant has been consumed at the transport boundary.
pub(crate) struct PmPhaseAPlaceDispatchBarrierWitnessV1 {
    profile: PmPhaseAPlaceLiveDispatchProfileV1,
    barrier_file: ProtectedJournal,
    expected_barrier_bytes: Vec<u8>,
}

impl PmRevalidatedPhaseAPlaceDispatchGrantV1 {
    #[must_use]
    pub(crate) const fn profile(&self) -> &PmPhaseAPlaceLiveDispatchProfileV1 {
        &self.profile
    }

    pub(crate) fn validate_held_barrier(&mut self) -> Result<(), PmTrialLiveJournalError> {
        self.barrier_file
            .validate_exact_bytes(&self.expected_barrier_bytes)
    }

    pub(crate) fn into_barrier_witness(self) -> PmPhaseAPlaceDispatchBarrierWitnessV1 {
        PmPhaseAPlaceDispatchBarrierWitnessV1 {
            profile: self.profile,
            barrier_file: self.barrier_file,
            expected_barrier_bytes: self.expected_barrier_bytes,
        }
    }
}

impl PmPhaseAPlaceDispatchBarrierWitnessV1 {
    pub(crate) const fn profile(&self) -> &PmPhaseAPlaceLiveDispatchProfileV1 {
        &self.profile
    }

    pub(crate) fn validate_held_barrier(&mut self) -> Result<(), PmTrialLiveJournalError> {
        self.barrier_file
            .validate_exact_bytes(&self.expected_barrier_bytes)
    }
}

impl std::fmt::Debug for PmRevalidatedPhaseAPlaceDispatchGrantV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PmRevalidatedPhaseAPlaceDispatchGrantV1")
            .field("profile", &self.profile)
            .field("runner_join_revalidated", &true)
            .field("request_body", &"[ABSENT]")
            .field("credentials", &"[ABSENT]")
            .field("transport", &"[ABSENT]")
            .finish()
    }
}

pub(crate) struct PmPendingPhaseAPlaceDispatchGrantV1 {
    profile: PmPhaseAPlaceLiveDispatchProfileV1,
    barrier_file: ProtectedJournal,
    expected_barrier_bytes: Vec<u8>,
}

impl PmPendingPhaseAPlaceDispatchGrantV1 {
    pub(crate) fn into_grant(self) -> PmPhaseAPlaceDispatchGrantV1 {
        PmPhaseAPlaceDispatchGrantV1 {
            profile: self.profile,
            barrier_file: self.barrier_file,
            expected_barrier_bytes: self.expected_barrier_bytes,
        }
    }
}

pub(crate) fn phase_a_live_dispatch_barrier_path(scope: &PmTrialLiveJournalScopeV1) -> PathBuf {
    Path::new(&scope.trial.journal.artifact_directory)
        .join(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1)
}

pub(crate) fn prepare_phase_a_place_dispatch_grant(
    context: PmPhaseAPlaceLiveDispatchContextV1,
    scope: &PmTrialLiveJournalScopeV1,
    preflight: &PmTrialLivePreflightBindingV1,
    evidence: PmPhaseAPlaceDispatchMintEvidenceV1,
) -> Result<PmPendingPhaseAPlaceDispatchGrantV1, PmTrialLiveJournalError> {
    let identity = context.public_request_identity;
    if evidence.preparation.expected_order_id() != identity.expected_order_id()
        || evidence.preparation.semantic_request_commitment()
            != identity.semantic_request_commitment()
    {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    let record = PmPhaseAPlaceLiveDispatchRecordV1 {
        schema_version: PM_PHASE_A_LIVE_DISPATCH_SCHEMA_VERSION_V1,
        family: PM_PHASE_A_LIVE_DISPATCH_FAMILY_V1.to_owned(),
        profile: PM_PHASE_A_LIVE_DISPATCH_PROFILE_V1.to_owned(),
        record: PM_PHASE_A_LIVE_DISPATCH_RECORD_KIND_V1.to_owned(),
        scope_fingerprint: scope.scope_fingerprint.clone(),
        canonical_config_sha256: context.canonical_config_sha256,
        canonical_config_length: context.canonical_config_length,
        canonical_config_fingerprint: context.canonical_config_fingerprint,
        trial_plan_fingerprint: context.trial_plan_fingerprint,
        authorization_id: context.authorization_id,
        authorization_fingerprint: context.authorization_fingerprint,
        authorization_not_before_utc: context.authorization_not_before_utc,
        authorization_expires_at_utc: context.authorization_expires_at_utc,
        authorization_cleanup_not_after_utc: context.authorization_cleanup_not_after_utc,
        preflight_fingerprint: preflight.fingerprint().to_owned(),
        preflight_canonical_sha256: preflight.canonical_sha256().to_owned(),
        preflight_validated_at_utc: preflight.validated_at_utc().to_owned(),
        preflight_dispatch_deadline_at_utc: preflight.dispatch_deadline_at_utc().to_owned(),
        source_pin_manifest_sha256: context.source_pin_manifest_sha256,
        public_semantic_request_commitment: hex32(identity.semantic_request_commitment().bytes()),
        prepared_request_commitment: evidence.preparation.request_commitment().to_owned(),
        expected_order_id: hex32(identity.expected_order_id().bytes()),
        l2_timestamp_seconds: evidence.preparation.l2_timestamp_seconds(),
        authorization_consumption_binding_fingerprint: evidence.consumption.binding_fingerprint,
        authorization_consumption_prepared_record_fingerprint: evidence
            .consumption
            .prepared_record_fingerprint,
        atomic_claim_fingerprint: evidence.consumption.atomic_claim_fingerprint,
        consumed_record_fingerprint: evidence.consumption.consumed_record_fingerprint,
        place_prepared_sequence: evidence.prepared_sequence,
        place_prepared_record_fingerprint: evidence.prepared_record_fingerprint,
        v1_dispatch_authorized_sequence: evidence.v1_dispatch_authorized_sequence,
        v1_dispatch_authorized_fingerprint: evidence.v1_dispatch_authorized_fingerprint,
        build: context.build,
        host: context.host,
        credential_slot: context.credential_slot,
        production_order_entry_authorized: true,
        real_order_submission_authorized: true,
        place_dispatch_allowance: 1,
        placement_resumption_allowed: false,
        record_fingerprint: ZERO_FINGERPRINT.to_owned(),
    }
    .seal()?;
    validate_record_against_scope(&record, scope, preflight)?;
    let mut encoded = canonical_json(&record)?;
    encoded.push(b'\n');
    if encoded.len() > MAX_PHASE_A_LIVE_DISPATCH_BARRIER_BYTES {
        return Err(PmTrialLiveJournalError::BoundExceeded);
    }
    let path = phase_a_live_dispatch_barrier_path(scope);
    let mut barrier_file =
        ProtectedJournal::create_new(&path, MAX_PHASE_A_LIVE_DISPATCH_BARRIER_BYTES)?;
    barrier_file.append_durable(&[], &encoded)?;
    Ok(PmPendingPhaseAPlaceDispatchGrantV1 {
        profile: PmPhaseAPlaceLiveDispatchProfileV1 {
            record,
            public_request_identity: identity,
        },
        barrier_file,
        expected_barrier_bytes: encoded,
    })
}

pub(crate) fn load_phase_a_live_dispatch_barrier(
    config: &CanonicalTrialConfig,
) -> Result<Option<PmPhaseAPlaceLiveDispatchRecordV1>, PmTrialLiveJournalError> {
    let path = Path::new(&config.value().journal.artifact_directory)
        .join(PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1);
    let bytes = match read_protected(&path, MAX_PHASE_A_LIVE_DISPATCH_BARRIER_BYTES) {
        Ok(bytes) => bytes,
        Err(PmTrialLiveJournalError::Absent) => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(Some(parse_phase_a_live_dispatch_barrier(&bytes)?))
}

/// Reopen and pin the already-validated positive place barrier for a
/// recovery-only cancel custody chain. This never recreates a place grant.
pub(crate) fn reopen_phase_a_place_dispatch_barrier_witness(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    scope: &PmTrialLiveJournalScopeV1,
    preflight: &PmTrialLivePreflightBindingV1,
    dispatch: &[DispatchLineV1],
    expected_record_fingerprint: &str,
) -> Result<PmPhaseAPlaceDispatchBarrierWitnessV1, PmTrialLiveJournalError> {
    let path = phase_a_live_dispatch_barrier_path(scope);
    let expected_barrier_bytes = read_protected(&path, MAX_PHASE_A_LIVE_DISPATCH_BARRIER_BYTES)?;
    let record = parse_phase_a_live_dispatch_barrier(&expected_barrier_bytes)?;
    if record.record_fingerprint != expected_record_fingerprint {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    validate_recovered_phase_a_live_dispatch_barrier(
        &record,
        config,
        authorization,
        scope,
        preflight,
        dispatch,
    )?;
    let mut barrier_file =
        ProtectedJournal::open_existing(&path, MAX_PHASE_A_LIVE_DISPATCH_BARRIER_BYTES)?;
    barrier_file.validate_exact_bytes(&expected_barrier_bytes)?;
    Ok(PmPhaseAPlaceDispatchBarrierWitnessV1 {
        profile: PmPhaseAPlaceLiveDispatchProfileV1 {
            record,
            public_request_identity: config.exact_place_public_request_identity(),
        },
        barrier_file,
        expected_barrier_bytes,
    })
}

fn parse_phase_a_live_dispatch_barrier(
    bytes: &[u8],
) -> Result<PmPhaseAPlaceLiveDispatchRecordV1, PmTrialLiveJournalError> {
    if bytes.is_empty() || !bytes.ends_with(b"\n") {
        return Err(PmTrialLiveJournalError::AmbiguousTail);
    }
    let line = &bytes[..bytes.len() - 1];
    if line.is_empty() || line.contains(&b'\n') {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    let record: PmPhaseAPlaceLiveDispatchRecordV1 =
        serde_json::from_slice(line).map_err(|_| PmTrialLiveJournalError::InvalidRecord)?;
    if canonical_json(&record)? != line {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    record.validate_structural()?;
    Ok(record)
}

pub(crate) fn validate_recovered_phase_a_live_dispatch_barrier(
    record: &PmPhaseAPlaceLiveDispatchRecordV1,
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    scope: &PmTrialLiveJournalScopeV1,
    preflight: &PmTrialLivePreflightBindingV1,
    dispatch: &[DispatchLineV1],
) -> Result<(), PmTrialLiveJournalError> {
    let runtime = AuthorizationRuntimeBinding {
        release_binary_sha256: scope.release_binary_sha256.clone(),
        release_binary_length: scope.release_binary_length,
        host: scope.host.clone(),
        observed_at_utc: scope.runtime_observed_at_utc.clone(),
    };
    let context = PmPhaseAPlaceLiveDispatchContextV1::for_exact_phase_a(
        config,
        authorization,
        &runtime,
        scope,
    )?
    .ok_or(PmTrialLiveJournalError::InvalidBinding)?;
    validate_record_against_scope(record, scope, preflight)?;
    if record.canonical_config_sha256 != context.canonical_config_sha256
        || record.canonical_config_length != context.canonical_config_length
        || record.canonical_config_fingerprint != context.canonical_config_fingerprint
        || record.trial_plan_fingerprint != context.trial_plan_fingerprint
        || record.authorization_id != context.authorization_id
        || record.authorization_fingerprint != context.authorization_fingerprint
        || record.authorization_not_before_utc != context.authorization_not_before_utc
        || record.authorization_expires_at_utc != context.authorization_expires_at_utc
        || record.authorization_cleanup_not_after_utc != context.authorization_cleanup_not_after_utc
        || record.source_pin_manifest_sha256 != context.source_pin_manifest_sha256
        || record.build != context.build
        || record.host != context.host
        || record.credential_slot != context.credential_slot
        || record.public_semantic_request_commitment
            != hex32(
                context
                    .public_request_identity
                    .semantic_request_commitment()
                    .bytes(),
            )
        || record.expected_order_id
            != hex32(context.public_request_identity.expected_order_id().bytes())
    {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }

    let prepared = dispatch
        .get(usize::from(record.place_prepared_sequence))
        .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
    if dispatch_fingerprint(prepared)? != record.place_prepared_record_fingerprint {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    let DispatchRecordV1::PlacePrepared { preparation, .. } = &prepared.body else {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    };
    let preparation = preparation.view(context.public_request_identity)?;
    if preparation.request_commitment() != record.prepared_request_commitment
        || preparation.l2_timestamp_seconds() != record.l2_timestamp_seconds
    {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }

    let v1_dispatch = dispatch
        .get(usize::from(record.v1_dispatch_authorized_sequence))
        .ok_or(PmTrialLiveJournalError::InvalidRecord)?;
    if dispatch_fingerprint(v1_dispatch)? != record.v1_dispatch_authorized_fingerprint {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    let DispatchRecordV1::PlaceDispatchAuthorized {
        prepared_sequence,
        prepared_record_fingerprint,
        consumption,
        production_order_entry_authorized: false,
        real_order_submission_authorized: false,
        place_dispatch_allowance: 0,
    } = &v1_dispatch.body
    else {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    };
    if *prepared_sequence != record.place_prepared_sequence
        || prepared_record_fingerprint != &record.place_prepared_record_fingerprint
        || consumption.binding_fingerprint != record.authorization_consumption_binding_fingerprint
        || consumption.prepared_record_fingerprint
            != record.authorization_consumption_prepared_record_fingerprint
        || consumption.atomic_claim_fingerprint != record.atomic_claim_fingerprint
        || consumption.consumed_record_fingerprint != record.consumed_record_fingerprint
    {
        return Err(PmTrialLiveJournalError::InvalidRecord);
    }
    Ok(())
}

fn validate_record_against_scope(
    record: &PmPhaseAPlaceLiveDispatchRecordV1,
    scope: &PmTrialLiveJournalScopeV1,
    preflight: &PmTrialLivePreflightBindingV1,
) -> Result<(), PmTrialLiveJournalError> {
    record.validate_structural()?;
    if scope.trial.phase != TrialPhase::APlaceCancel
        || record.scope_fingerprint != scope.scope_fingerprint
        || record.canonical_config_sha256 != scope.canonical_config_sha256
        || record.canonical_config_length != scope.canonical_config_length
        || record.canonical_config_fingerprint != scope.canonical_config_fingerprint
        || record.trial_plan_fingerprint != scope.trial_plan_fingerprint
        || record.authorization_id != scope.authorization_id
        || record.authorization_fingerprint != scope.authorization_fingerprint
        || record.authorization_cleanup_not_after_utc != scope.authorization_cleanup_not_after_utc
        || record.preflight_fingerprint != preflight.fingerprint()
        || record.preflight_canonical_sha256 != preflight.canonical_sha256()
        || record.preflight_validated_at_utc != preflight.validated_at_utc()
        || record.preflight_dispatch_deadline_at_utc != preflight.dispatch_deadline_at_utc()
        || record.source_pin_manifest_sha256 != scope.source_pin_manifest_sha256
        || record.expected_order_id != scope.expected_order_id
        || record.public_semantic_request_commitment != scope.place_semantic_request_commitment
        || record.build.release_binary_sha256 != scope.release_binary_sha256
        || record.build.release_binary_length != scope.release_binary_length
        || record.host != scope.host
        || record.credential_slot.slot_id != scope.credential_slot_id
        || record.credential_slot.nonsecret_fingerprint_sha256
            != scope.credential_slot_nonsecret_fingerprint_sha256
    {
        return Err(PmTrialLiveJournalError::InvalidBinding);
    }
    Ok(())
}

pub(crate) fn grant_matches_v1_dispatch(
    grant: &PmRevalidatedPhaseAPlaceDispatchGrantV1,
    sequence: u8,
    record_fingerprint: &str,
) -> bool {
    grant.profile.record.v1_dispatch_authorized_sequence == sequence
        && grant.profile.record.v1_dispatch_authorized_fingerprint == record_fingerprint
}

pub(crate) fn live_dispatch_barrier_fingerprint(
    record: &PmPhaseAPlaceLiveDispatchRecordV1,
) -> &str {
    &record.record_fingerprint
}

fn hex32(bytes: [u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
