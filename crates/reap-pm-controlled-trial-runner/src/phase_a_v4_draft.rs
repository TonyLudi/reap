//! Strictly offline drafting for the non-authorizing Phase-A V4 envelope.
//!
//! Every exact pin and every closed unavailable requirement is derived from
//! ten protected canonical holders by the controlled-trial crate. The only
//! free text is the record ID, unauthenticated reviewer display label, and
//! reviewed time envelope; callers must not place secrets in those values.
//! This module samples no clock and accepts no pin, proof, fact boolean,
//! currentness claim, authorization DTO, or output pathname.
//!
//! The result is compact canonical JSON on stdout only. This module never
//! creates or writes a file. A caller that retains the result must separately
//! install the exact bytes as a protected 0600 regular file before asking the
//! canonical V4 loader to consume it. Drafting grants no eligibility,
//! authentication, permit, dispatch allowance, or mutation authority.

use std::path::PathBuf;

use reap_pm_controlled_trial::{
    OfflineAuthorizationState, REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4_SCHEMA_VERSION,
    ReviewedPhaseAActorRuntimeAttemptStatusV4, ReviewedPhaseACredentialDeliveryLeaseProofStatusV4,
    ReviewedPhaseACredentialProviderTrustRootStatusV4, ReviewedPhaseADeliveryCustodyJoinStatusV4,
    ReviewedPhaseADurableAttemptLineageStatusV4, ReviewedPhaseAEligibilityEnvelopeContextV4,
    ReviewedPhaseAEligibilityEnvelopeDraftInputsV4, ReviewedPhaseAEligibilityEnvelopeV4,
    ReviewedPhaseARecordRoleV4, ReviewedPhaseARemoteAcceptanceContractStatusV4,
    ReviewedPhaseARemoteCredentialProofStatusV4, ReviewedPhaseAReviewerTrustAnchorStatusV4,
    ReviewedPhaseAScopeV4, ReviewedPhaseASignerProxyContractStatusV4,
    ReviewedPhaseASignerProxyProofStatusV4, ReviewedPhaseASingleDispatchStatusV4,
    draft_non_authorizing_reviewed_phase_a_eligibility_envelope_v4, load_canonical_authorization,
    load_canonical_fresh_credential_delivery_binding_v1, load_canonical_online_authorization_v2,
    load_canonical_online_policy_v2, load_canonical_reviewed_fresh_credential_slot_locator_v1,
    load_canonical_reviewed_production_destination_profile_v1,
    load_canonical_reviewed_remote_credential_proof_policy_v1,
    load_canonical_reviewed_signer_proxy_account_identity_v1,
    load_canonical_reviewed_static_online_authorization_v3, load_canonical_trial_config,
};
use thiserror::Error;

pub(crate) struct DraftNonAuthorizingPhaseAEligibilityEnvelopeV4Paths {
    pub(crate) config: PathBuf,
    pub(crate) authorization: PathBuf,
    pub(crate) online_policy_v2: PathBuf,
    pub(crate) online_authorization_v2: PathBuf,
    pub(crate) reviewed_production_destination_v1: PathBuf,
    pub(crate) reviewed_fresh_credential_slot_locator_v1: PathBuf,
    pub(crate) fresh_credential_delivery_binding_v1: PathBuf,
    pub(crate) reviewed_signer_proxy_account_identity_v1: PathBuf,
    pub(crate) reviewed_remote_credential_proof_policy_v1: PathBuf,
    pub(crate) reviewed_static_online_authorization_v3: PathBuf,
}

/// Exact compact bytes that passed a context-derived structural roundtrip.
/// This move-only wrapper is neither an authorization nor a file capability.
pub(crate) struct VerifiedNonAuthorizingPhaseAEligibilityEnvelopeV4Bytes {
    canonical_bytes: Vec<u8>,
}

impl VerifiedNonAuthorizingPhaseAEligibilityEnvelopeV4Bytes {
    #[must_use]
    pub(crate) fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }
}

pub(crate) fn draft_non_authorizing_phase_a_eligibility_envelope_v4(
    paths: DraftNonAuthorizingPhaseAEligibilityEnvelopeV4Paths,
    inputs: ReviewedPhaseAEligibilityEnvelopeDraftInputsV4,
) -> Result<VerifiedNonAuthorizingPhaseAEligibilityEnvelopeV4Bytes, PhaseAV4DraftError> {
    let config = load_canonical_trial_config(&paths.config)
        .map_err(|_| PhaseAV4DraftError::InvalidReviewedInput("V1 config"))?;
    let authorization = load_canonical_authorization(&paths.authorization)
        .map_err(|_| PhaseAV4DraftError::InvalidReviewedInput("V1 authorization"))?;
    let online_policy = load_canonical_online_policy_v2(&paths.online_policy_v2)
        .map_err(|_| PhaseAV4DraftError::InvalidReviewedInput("online policy V2"))?;
    let online_authorization =
        load_canonical_online_authorization_v2(&paths.online_authorization_v2)
            .map_err(|_| PhaseAV4DraftError::InvalidReviewedInput("online authorization V2"))?;
    let destination = load_canonical_reviewed_production_destination_profile_v1(
        &paths.reviewed_production_destination_v1,
    )
    .map_err(|_| PhaseAV4DraftError::InvalidReviewedInput("reviewed destination V1"))?;
    let locator = load_canonical_reviewed_fresh_credential_slot_locator_v1(
        &paths.reviewed_fresh_credential_slot_locator_v1,
    )
    .map_err(|_| PhaseAV4DraftError::InvalidReviewedInput("reviewed credential locator V1"))?;
    let delivery = load_canonical_fresh_credential_delivery_binding_v1(
        &paths.fresh_credential_delivery_binding_v1,
    )
    .map_err(|_| PhaseAV4DraftError::InvalidReviewedInput("credential delivery binding V1"))?;
    let account_identity = load_canonical_reviewed_signer_proxy_account_identity_v1(
        &paths.reviewed_signer_proxy_account_identity_v1,
    )
    .map_err(|_| PhaseAV4DraftError::InvalidReviewedInput("reviewed signer/proxy identity V1"))?;
    let remote_proof_policy = load_canonical_reviewed_remote_credential_proof_policy_v1(
        &paths.reviewed_remote_credential_proof_policy_v1,
    )
    .map_err(|_| PhaseAV4DraftError::InvalidReviewedInput("reviewed remote proof policy V1"))?;
    let static_authorization = load_canonical_reviewed_static_online_authorization_v3(
        &paths.reviewed_static_online_authorization_v3,
    )
    .map_err(|_| PhaseAV4DraftError::InvalidReviewedInput("reviewed static authorization V3"))?;

    let context = ReviewedPhaseAEligibilityEnvelopeContextV4 {
        v1_config: &config,
        v1_authorization: &authorization,
        online_policy_v2: &online_policy,
        online_authorization_v2: &online_authorization,
        reviewed_production_destination_v1: &destination,
        reviewed_fresh_credential_slot_locator_v1: &locator,
        fresh_credential_delivery_binding_v1: &delivery,
        reviewed_signer_proxy_account_identity_v1: &account_identity,
        reviewed_remote_credential_proof_policy_v1: &remote_proof_policy,
        reviewed_static_online_authorization_v3: &static_authorization,
    };
    let drafted =
        draft_non_authorizing_reviewed_phase_a_eligibility_envelope_v4(&context, inputs.clone())
            .map_err(|_| PhaseAV4DraftError::DraftRejected)?;
    let canonical_bytes =
        serde_json::to_vec(&drafted).map_err(|_| PhaseAV4DraftError::CanonicalRoundtripFailed)?;
    let roundtrip: ReviewedPhaseAEligibilityEnvelopeV4 =
        serde_json::from_slice(&canonical_bytes)
            .map_err(|_| PhaseAV4DraftError::CanonicalRoundtripFailed)?;
    let roundtrip_bytes =
        serde_json::to_vec(&roundtrip).map_err(|_| PhaseAV4DraftError::CanonicalRoundtripFailed)?;
    if roundtrip != drafted || roundtrip_bytes != canonical_bytes {
        return Err(PhaseAV4DraftError::CanonicalRoundtripFailed);
    }
    let independently_redrafted =
        draft_non_authorizing_reviewed_phase_a_eligibility_envelope_v4(&context, inputs)
            .map_err(|_| PhaseAV4DraftError::StructuralReverificationFailed)?;
    if independently_redrafted != roundtrip {
        return Err(PhaseAV4DraftError::StructuralReverificationFailed);
    }

    let authorization = closed_negative_authorization(&roundtrip)?;
    if authorization != OfflineAuthorizationState::DENIED
        || authorization.place_dispatch_allowance != 0
    {
        return Err(PhaseAV4DraftError::NonAuthorizingSemanticsViolated);
    }

    Ok(VerifiedNonAuthorizingPhaseAEligibilityEnvelopeV4Bytes { canonical_bytes })
}

fn closed_negative_authorization(
    record: &ReviewedPhaseAEligibilityEnvelopeV4,
) -> Result<OfflineAuthorizationState, PhaseAV4DraftError> {
    let external = &record.required_unavailable_external_evidence;
    let runtime = &record.required_unavailable_runtime_lineage;
    if record.schema_version != REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4_SCHEMA_VERSION
        || record.record_role
            != ReviewedPhaseARecordRoleV4::OfflineEligibilityConjunctionOnlyNoAuthorizationV1
        || record.phase_scope
            != ReviewedPhaseAScopeV4::PhaseAExactlyOnePlaceThenExactCancelOnlyV1
        || external.reviewer_trust_anchor_status
            != ReviewedPhaseAReviewerTrustAnchorStatusV4::RequiredExternalReviewerTrustAnchorUnavailableV1
        || external.credential_provider_trust_root_status
            != ReviewedPhaseACredentialProviderTrustRootStatusV4::RequiredAuthenticatedProviderTrustRootUnavailableV1
        || external.provider_signed_credential_delivery_lease_proof_status
            != ReviewedPhaseACredentialDeliveryLeaseProofStatusV4::RequiredProviderSignedAttemptAudienceLeaseUnavailableV1
        || external.authoritative_remote_credential_acceptance_contract_status
            != ReviewedPhaseARemoteAcceptanceContractStatusV4::RequiredAuthoritativeRemoteAcceptanceContractUnavailableV1
        || external.same_holder_live_remote_credential_acceptance_proof_status
            != ReviewedPhaseARemoteCredentialProofStatusV4::RequiredSameHolderLiveRemoteAcceptanceProofUnavailableV1
        || external.authoritative_signer_proxy_control_contract_status
            != ReviewedPhaseASignerProxyContractStatusV4::RequiredAuthoritativeSignerProxyControlContractUnavailableV1
        || external.account_specific_signer_proxy_control_proof_status
            != ReviewedPhaseASignerProxyProofStatusV4::RequiredAccountSpecificCurrentUnrevokedControlProofUnavailableV1
        || runtime.selected_actor_generation_and_source_owned_attempt_status
            != ReviewedPhaseAActorRuntimeAttemptStatusV4::RequiredFutureSelectedActorPreparedLineageUnavailableV1
        || runtime.retained_delivery_evidence_and_load_token_join_status
            != ReviewedPhaseADeliveryCustodyJoinStatusV4::RequiredFutureRetainedEvidenceLoadTokenJoinUnavailableV1
        || runtime.durable_attempt_claim_and_final_conjunct_status
            != ReviewedPhaseADurableAttemptLineageStatusV4::RequiredFutureCreateNewClaimA3LineageUnavailableV1
        || runtime.fixed_egress_single_dispatch_owner_status
            != ReviewedPhaseASingleDispatchStatusV4::RequiredFutureSelectedEgressSingleDispatchOwnerUnavailableV1
    {
        return Err(PhaseAV4DraftError::NonAuthorizingSemanticsViolated);
    }
    Ok(OfflineAuthorizationState::DENIED)
}

#[derive(Debug, Error)]
pub(crate) enum PhaseAV4DraftError {
    #[error("Phase-A V4 draft reviewed input is invalid: {0}")]
    InvalidReviewedInput(&'static str),
    #[error("Phase-A V4 non-authorizing draft was rejected")]
    DraftRejected,
    #[error("Phase-A V4 draft compact canonical JSON roundtrip failed")]
    CanonicalRoundtripFailed,
    #[error("Phase-A V4 draft context-derived structural reverification failed")]
    StructuralReverificationFailed,
    #[error("Phase-A V4 draft violated closed non-authorizing semantics")]
    NonAuthorizingSemanticsViolated,
}
