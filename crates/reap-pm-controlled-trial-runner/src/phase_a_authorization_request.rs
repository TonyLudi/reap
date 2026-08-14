//! Offline Phase-A authorization request, explicitly not an authorization.
//!
//! This module preserves the full non-authorizing release-candidate result,
//! projects the exact designated non-secret trial config as recipient-scoped
//! review material, and binds the additive
//! proxy, local-custody, and L1-derivation policies by protected canonical
//! commitments. Missing reviewer inputs, external proofs, and runtime gates
//! are closed internal enums; callers cannot supply a sufficiency boolean,
//! proof DTO, clock value, credential, journal state, permit, or output path.
//!
//! The result is compact JSON on stdout only. It is a request for later
//! review, never a live authorization record. It always carries DENIED/0 and
//! cannot mint authentication, signing, placement, cancellation, recovery,
//! transport, or mutation authority.

use std::path::PathBuf;

use reap_pm_controlled_trial::{
    CanonicalAuthorization, CanonicalFreshCredentialDeliveryBindingV1,
    CanonicalOnlineAuthorizationV2, CanonicalOnlinePolicyV2,
    CanonicalReviewedFreshCredentialSlotLocatorV1,
    CanonicalReviewedL1CredentialDerivationProofPolicyV1,
    CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1,
    CanonicalReviewedPhaseAEligibilityEnvelopeV4, CanonicalReviewedPolyProxyControlPolicyV1,
    CanonicalReviewedProductionDestinationProfileV1,
    CanonicalReviewedRemoteCredentialProofPolicyV1, CanonicalReviewedSignerProxyAccountIdentityV1,
    CanonicalReviewedStaticOnlineAuthorizationV3, CanonicalTrialConfig, OfflineAuthorizationState,
    ReviewedL1CredentialDerivationProofPolicyContextV1,
    ReviewedL1CredentialDerivationProofPolicyVerificationV1,
    ReviewedLocalOperatorCooperativeCustodyProfileContextV1,
    ReviewedLocalOperatorCooperativeCustodyProfileVerificationV1,
    ReviewedPhaseAEligibilityEnvelopeContextV4, ReviewedPolyProxyControlPolicyVerificationV1,
    TrialConfig, load_canonical_authorization, load_canonical_fresh_credential_delivery_binding_v1,
    load_canonical_online_authorization_v2, load_canonical_online_policy_v2,
    load_canonical_reviewed_fresh_credential_slot_locator_v1,
    load_canonical_reviewed_l1_credential_derivation_proof_policy_v1,
    load_canonical_reviewed_local_operator_cooperative_custody_profile_v1,
    load_canonical_reviewed_phase_a_eligibility_envelope_v4,
    load_canonical_reviewed_poly_proxy_control_policy_v1,
    load_canonical_reviewed_production_destination_profile_v1,
    load_canonical_reviewed_remote_credential_proof_policy_v1,
    load_canonical_reviewed_signer_proxy_account_identity_v1,
    load_canonical_reviewed_static_online_authorization_v3, load_canonical_trial_config,
    verify_reviewed_l1_credential_derivation_proof_policy_v1,
    verify_reviewed_local_operator_cooperative_custody_profile_v1,
    verify_reviewed_poly_proxy_control_policy_v1,
};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

use crate::phase_a_candidate::{
    FreezePhaseACandidatePaths, PhaseANonAuthorizingCandidateGapReportV1, freeze_phase_a_candidate,
};

const REQUEST_SCHEMA_VERSION: u32 = 1;
const REQUEST_KIND: &str = "phase_a_authorization_request_not_authorization_v1";

pub(crate) struct GeneratePhaseAAuthorizationRequestNotAuthorizationPaths {
    pub(crate) repository_root: PathBuf,
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
    pub(crate) reviewed_phase_a_eligibility_envelope_v4: PathBuf,
    pub(crate) reviewed_poly_proxy_control_policy_v1: PathBuf,
    pub(crate) reviewed_local_operator_cooperative_custody_profile_v1: PathBuf,
    pub(crate) reviewed_l1_credential_derivation_proof_policy_v1: PathBuf,
    pub(crate) source_manifest: PathBuf,
    pub(crate) runbook: PathBuf,
}

/// Exact offline request result. It deliberately has no `Deserialize` or
/// `Debug`: no caller may feed statuses back, and arbitrary labels in the
/// protected nested config are not projected through debug output. Protected
/// non-secret config does not mean previously or generally publicly disclosed.
#[derive(Serialize)]
pub(crate) struct PhaseAAuthorizationRequestNotAuthorizationV1 {
    schema_version: u32,
    artifact_kind: &'static str,
    request_only_not_authorization: bool,
    live_authorization_record_generated: bool,
    caller_supplied_authorization_or_proof_dto_accepted: bool,
    exact_public_trial_plan_identity: ExactPublicTrialPlanIdentityV1,
    exact_nonsecret_trial_config: TrialConfig,
    exact_reviewed_artifact_commitments: ReviewedArtifactCommitmentsV1,
    release_candidate_snapshot: PhaseANonAuthorizingCandidateGapReportV1,
    reviewer_inputs_required: Vec<ReviewerInputRequirementV1>,
    external_reviewed_proofs_required: Vec<ExternalProofRequirementV1>,
    runtime_gates_required: Vec<RuntimeGateRequirementV1>,
    additive_structural_verification: AdditiveStructuralVerificationV1,
    #[serde(flatten)]
    authorization: OfflineAuthorizationState,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct ExactPublicTrialPlanIdentityV1 {
    canonical_config_sha256: String,
    canonical_config_length: u64,
    canonical_config_fingerprint: String,
    trial_plan_fingerprint: String,
    expected_order_id: String,
    semantic_request_commitment: String,
}

#[derive(Clone, Serialize)]
#[serde(deny_unknown_fields)]
struct CanonicalArtifactCommitmentV1 {
    canonical_sha256: String,
    canonical_length: u64,
    fingerprint: String,
}

#[derive(Clone, Serialize)]
#[serde(deny_unknown_fields)]
struct FingerprintOnlyCommitmentV1 {
    fingerprint: String,
    projection: FingerprintProjectionV1,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum FingerprintProjectionV1 {
    CanonicalHolderIntentionallyExposesFingerprintOnly,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct ReviewedArtifactCommitmentsV1 {
    canonical_config_v1: CanonicalArtifactCommitmentV1,
    authorization_v1: FingerprintOnlyCommitmentV1,
    online_policy_v2: CanonicalArtifactCommitmentV1,
    online_authorization_v2: CanonicalArtifactCommitmentV1,
    reviewed_production_destination_v1: CanonicalArtifactCommitmentV1,
    reviewed_fresh_credential_slot_locator_v1: CanonicalArtifactCommitmentV1,
    fresh_credential_delivery_binding_v1: CanonicalArtifactCommitmentV1,
    reviewed_signer_proxy_account_identity_v1: CanonicalArtifactCommitmentV1,
    reviewed_remote_credential_proof_policy_v1: CanonicalArtifactCommitmentV1,
    reviewed_static_online_authorization_v3: CanonicalArtifactCommitmentV1,
    reviewed_phase_a_eligibility_envelope_v4: CanonicalArtifactCommitmentV1,
    reviewed_poly_proxy_control_policy_v1: CanonicalArtifactCommitmentV1,
    reviewed_local_operator_cooperative_custody_profile_v1: CanonicalArtifactCommitmentV1,
    reviewed_l1_credential_derivation_proof_policy_v1: CanonicalArtifactCommitmentV1,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum MissingRequirementStatusV1 {
    ReviewerInputIsMissingV1,
    ExternalProofIsUnavailableV1,
    RuntimeGateIsNotObservedV1,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ReviewerInputV1 {
    PositiveSuccessorAuthorizationSchemaAcceptance,
    AuthorizationId,
    IssuingReviewerAuthenticatedIdentity,
    ReviewerTrustAnchorReference,
    ReviewedAtUtc,
    NotBeforeUtc,
    ExpiresAtUtcMaximumFifteenMinuteWindow,
    CleanupNotAfterUtc,
    ExactNonsecretTrialConfigAndPublicPlanFingerprintApproval,
    ExactAccountMarketOrderLossAndTimeTermsApproval,
    ExactReleaseCandidateBuildAndObservedHostSubsetApproval,
    ExactlyOnePhaseAPlaceAttemptApproval,
    OnePlaceNoReplacementAndExactCancelBudgetsApproval,
    OnePossibleFillWithinExactLossCapApproval,
    PostOnlyMayStillFillApproval,
    FiveDistinctOnlineEvidenceClassesApproval,
    NoConcurrentProxyTradingAttestation,
    IndependentManualCleanupMethodApprovalAndReference,
    CooperativeLocalCustodyTrustAndLimitationsApproval,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct ReviewerInputRequirementV1 {
    field: ReviewerInputV1,
    status: MissingRequirementStatusV1,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ExternalReviewedProofV1 {
    PositiveSuccessorAuthorizationContractAndVerifier,
    AuthenticatedReviewerTrustAnchorAndRecordAuthorship,
    SelectedCredentialCustodyTrustAlternative,
    AttemptAudienceBoundDeliveryLeaseOrReviewedLocalCooperationRealization,
    AuthoritativeRemoteCredentialAcceptanceContract,
    SameLoadedHolderLiveL1DeriveAndL2AcceptanceProof,
    AuthoritativeSignerProxyControlContract,
    CurrentAccountSpecificNonexclusiveSignerProxyControlProof,
    AuthenticatedFinalizedPolygonStateProofAndReorgInvalidation,
    IndependentManualCleanupMethodEvidence,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct ExternalProofRequirementV1 {
    proof: ExternalReviewedProofV1,
    status: MissingRequirementStatusV1,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum RuntimeGateV1 {
    ExactNssUsernameAndCompleteV1V2EgressIdentity,
    CurrentPublicEgressGeoblockStatusHealthAndSameAccountClosedOnly,
    CurrentMarketBookFeeTickMinimumAndPassivity,
    CompleteAccountPositionOrderTradeAndUserStreamCuts,
    SelectedActorGenerationAndSourceOwnedAttempt,
    RetainedDeliveryEvidenceLoadTokenAndContinuouslyHeldLease,
    SameHolderL1DeriveResponseAndLoadedL2TupleJoin,
    CurrentSignerChallengeAndProxyStateProof,
    DurableCreateNewAttemptClaimBurnFinalConjunctAndNoResend,
    FixedProductionEgressSinglePlaceDispatchOwner,
    RecoveryOnlyExactOwnedCancelContinuation,
    CredentialCleanupAndTerminalReconciliation,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct RuntimeGateRequirementV1 {
    gate: RuntimeGateV1,
    status: MissingRequirementStatusV1,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct AdditiveStructuralVerificationV1 {
    reviewed_poly_proxy_control_policy_v1: ReviewedPolyProxyControlPolicyVerificationV1,
    reviewed_poly_proxy_control_policy_v1_false_boolean_paths_exhaustive: Vec<String>,
    reviewed_local_operator_cooperative_custody_profile_v1:
        ReviewedLocalOperatorCooperativeCustodyProfileVerificationV1,
    reviewed_local_operator_cooperative_custody_profile_v1_false_boolean_paths_exhaustive:
        Vec<String>,
    reviewed_l1_credential_derivation_proof_policy_v1:
        ReviewedL1CredentialDerivationProofPolicyVerificationV1,
    reviewed_l1_credential_derivation_proof_policy_v1_false_boolean_paths_exhaustive: Vec<String>,
}

#[allow(
    clippy::too_many_lines,
    reason = "the exact holder chain is intentionally explicit"
)]
pub(crate) fn generate_phase_a_authorization_request_not_authorization(
    paths: GeneratePhaseAAuthorizationRequestNotAuthorizationPaths,
) -> Result<PhaseAAuthorizationRequestNotAuthorizationV1, PhaseAAuthorizationRequestError> {
    let config = load_canonical_trial_config(&paths.config)
        .map_err(|_| PhaseAAuthorizationRequestError::InvalidReviewedInput("V1 config"))?;
    let authorization = load_canonical_authorization(&paths.authorization)
        .map_err(|_| PhaseAAuthorizationRequestError::InvalidReviewedInput("V1 authorization"))?;
    let online_policy = load_canonical_online_policy_v2(&paths.online_policy_v2)
        .map_err(|_| PhaseAAuthorizationRequestError::InvalidReviewedInput("online policy V2"))?;
    let online_authorization =
        load_canonical_online_authorization_v2(&paths.online_authorization_v2).map_err(|_| {
            PhaseAAuthorizationRequestError::InvalidReviewedInput("online authorization V2")
        })?;
    let destination = load_canonical_reviewed_production_destination_profile_v1(
        &paths.reviewed_production_destination_v1,
    )
    .map_err(|_| {
        PhaseAAuthorizationRequestError::InvalidReviewedInput("reviewed destination V1")
    })?;
    let locator = load_canonical_reviewed_fresh_credential_slot_locator_v1(
        &paths.reviewed_fresh_credential_slot_locator_v1,
    )
    .map_err(|_| {
        PhaseAAuthorizationRequestError::InvalidReviewedInput("reviewed credential locator V1")
    })?;
    let delivery = load_canonical_fresh_credential_delivery_binding_v1(
        &paths.fresh_credential_delivery_binding_v1,
    )
    .map_err(|_| {
        PhaseAAuthorizationRequestError::InvalidReviewedInput("credential delivery binding V1")
    })?;
    let account_identity = load_canonical_reviewed_signer_proxy_account_identity_v1(
        &paths.reviewed_signer_proxy_account_identity_v1,
    )
    .map_err(|_| {
        PhaseAAuthorizationRequestError::InvalidReviewedInput("reviewed signer/proxy identity V1")
    })?;
    let remote_proof_policy = load_canonical_reviewed_remote_credential_proof_policy_v1(
        &paths.reviewed_remote_credential_proof_policy_v1,
    )
    .map_err(|_| {
        PhaseAAuthorizationRequestError::InvalidReviewedInput("reviewed remote proof policy V1")
    })?;
    let static_authorization = load_canonical_reviewed_static_online_authorization_v3(
        &paths.reviewed_static_online_authorization_v3,
    )
    .map_err(|_| {
        PhaseAAuthorizationRequestError::InvalidReviewedInput("reviewed static authorization V3")
    })?;
    let phase_a_v4 = load_canonical_reviewed_phase_a_eligibility_envelope_v4(
        &paths.reviewed_phase_a_eligibility_envelope_v4,
    )
    .map_err(|_| {
        PhaseAAuthorizationRequestError::InvalidReviewedInput(
            "reviewed Phase-A eligibility envelope V4",
        )
    })?;
    let proxy_policy = load_canonical_reviewed_poly_proxy_control_policy_v1(
        &paths.reviewed_poly_proxy_control_policy_v1,
    )
    .map_err(|_| {
        PhaseAAuthorizationRequestError::InvalidReviewedInput(
            "reviewed Poly proxy control policy V1",
        )
    })?;
    let local_custody = load_canonical_reviewed_local_operator_cooperative_custody_profile_v1(
        &paths.reviewed_local_operator_cooperative_custody_profile_v1,
    )
    .map_err(|_| {
        PhaseAAuthorizationRequestError::InvalidReviewedInput(
            "reviewed local-operator cooperative custody profile V1",
        )
    })?;
    let l1_derivation = load_canonical_reviewed_l1_credential_derivation_proof_policy_v1(
        &paths.reviewed_l1_credential_derivation_proof_policy_v1,
    )
    .map_err(|_| {
        PhaseAAuthorizationRequestError::InvalidReviewedInput(
            "reviewed L1 credential-derivation proof policy V1",
        )
    })?;

    let proxy_verification = verify_reviewed_poly_proxy_control_policy_v1(&proxy_policy)
        .map_err(|_| PhaseAAuthorizationRequestError::AdditivePolicyVerificationFailed)?;
    let local_context = ReviewedLocalOperatorCooperativeCustodyProfileContextV1 {
        phase_a_eligibility_context: phase_a_context(
            &config,
            &authorization,
            &online_policy,
            &online_authorization,
            &destination,
            &locator,
            &delivery,
            &account_identity,
            &remote_proof_policy,
            &static_authorization,
        ),
        reviewed_phase_a_eligibility_envelope_v4: &phase_a_v4,
    };
    let local_verification = verify_reviewed_local_operator_cooperative_custody_profile_v1(
        &local_context,
        &local_custody,
    )
    .map_err(|_| PhaseAAuthorizationRequestError::AdditivePolicyVerificationFailed)?;
    let l1_context = ReviewedL1CredentialDerivationProofPolicyContextV1 {
        phase_a_eligibility_context: phase_a_context(
            &config,
            &authorization,
            &online_policy,
            &online_authorization,
            &destination,
            &locator,
            &delivery,
            &account_identity,
            &remote_proof_policy,
            &static_authorization,
        ),
        reviewed_phase_a_eligibility_envelope_v4: &phase_a_v4,
        reviewed_local_operator_cooperative_custody_profile_v1: &local_custody,
    };
    let l1_verification =
        verify_reviewed_l1_credential_derivation_proof_policy_v1(&l1_context, &l1_derivation)
            .map_err(|_| PhaseAAuthorizationRequestError::AdditivePolicyVerificationFailed)?;
    assert_closed_denial(&proxy_verification.authorization)?;
    assert_closed_denial(&local_verification.authorization)?;
    assert_closed_denial(&l1_verification.authorization)?;

    let proxy_false_boolean_paths = false_boolean_paths(&proxy_verification)?;
    let local_false_boolean_paths = false_boolean_paths(&local_verification)?;
    let l1_false_boolean_paths = false_boolean_paths(&l1_verification)?;
    let additive_structural_verification = AdditiveStructuralVerificationV1 {
        reviewed_poly_proxy_control_policy_v1_false_boolean_paths_exhaustive:
            proxy_false_boolean_paths,
        reviewed_local_operator_cooperative_custody_profile_v1_false_boolean_paths_exhaustive:
            local_false_boolean_paths,
        reviewed_l1_credential_derivation_proof_policy_v1_false_boolean_paths_exhaustive:
            l1_false_boolean_paths,
        reviewed_poly_proxy_control_policy_v1: proxy_verification,
        reviewed_local_operator_cooperative_custody_profile_v1: local_verification,
        reviewed_l1_credential_derivation_proof_policy_v1: l1_verification,
    };

    let candidate = freeze_phase_a_candidate(FreezePhaseACandidatePaths {
        repository_root: paths.repository_root,
        config: paths.config,
        authorization: paths.authorization,
        online_policy_v2: paths.online_policy_v2,
        online_authorization_v2: paths.online_authorization_v2,
        reviewed_production_destination_v1: paths.reviewed_production_destination_v1,
        reviewed_fresh_credential_slot_locator_v1: paths.reviewed_fresh_credential_slot_locator_v1,
        fresh_credential_delivery_binding_v1: paths.fresh_credential_delivery_binding_v1,
        reviewed_signer_proxy_account_identity_v1: paths.reviewed_signer_proxy_account_identity_v1,
        reviewed_remote_credential_proof_policy_v1: paths
            .reviewed_remote_credential_proof_policy_v1,
        reviewed_static_online_authorization_v3: paths.reviewed_static_online_authorization_v3,
        reviewed_phase_a_eligibility_envelope_v4: paths.reviewed_phase_a_eligibility_envelope_v4,
        source_manifest: paths.source_manifest,
        runbook: paths.runbook,
    })
    .map_err(|_| PhaseAAuthorizationRequestError::ReleaseCandidateFreezeFailed)?;
    if !candidate.exact_request_chain_matches(
        config.canonical_sha256(),
        config.canonical_length(),
        phase_a_v4.canonical_sha256(),
        phase_a_v4.canonical_length(),
        phase_a_v4.fingerprint(),
    ) {
        return Err(PhaseAAuthorizationRequestError::ReleaseCandidateBindingChanged);
    }

    let place_identity = config.exact_place_public_request_identity();
    let exact_public_trial_plan_identity = ExactPublicTrialPlanIdentityV1 {
        canonical_config_sha256: config.canonical_sha256().to_owned(),
        canonical_config_length: config.canonical_length(),
        canonical_config_fingerprint: config.fingerprint().to_owned(),
        trial_plan_fingerprint: config.plan_fingerprint().to_owned(),
        expected_order_id: place_identity.expected_order_id().to_string(),
        semantic_request_commitment: place_identity.semantic_request_commitment().to_string(),
    };
    // The protected config is designated non-secret, but it contains
    // recipient-scoped local paths, slot references, and pre-trade terms. Its
    // inclusion here does not classify those values as publicly disclosed.
    let exact_nonsecret_trial_config = config.value().clone();
    let exact_reviewed_artifact_commitments = artifact_commitments(
        &config,
        &authorization,
        &online_policy,
        &online_authorization,
        &destination,
        &locator,
        &delivery,
        &account_identity,
        &remote_proof_policy,
        &static_authorization,
        &phase_a_v4,
        &proxy_policy,
        &local_custody,
        &l1_derivation,
    );
    let request = PhaseAAuthorizationRequestNotAuthorizationV1 {
        schema_version: REQUEST_SCHEMA_VERSION,
        artifact_kind: REQUEST_KIND,
        request_only_not_authorization: true,
        live_authorization_record_generated: false,
        caller_supplied_authorization_or_proof_dto_accepted: false,
        exact_public_trial_plan_identity,
        exact_nonsecret_trial_config,
        exact_reviewed_artifact_commitments,
        release_candidate_snapshot: candidate,
        reviewer_inputs_required: reviewer_inputs_required(),
        external_reviewed_proofs_required: external_proofs_required(),
        runtime_gates_required: runtime_gates_required(),
        additive_structural_verification,
        authorization: OfflineAuthorizationState::DENIED,
    };
    assert_closed_denial(&request.authorization)?;
    Ok(request)
}

#[allow(
    clippy::too_many_arguments,
    reason = "the exact ten-holder V4 chain is explicit"
)]
fn phase_a_context<'a>(
    config: &'a CanonicalTrialConfig,
    authorization: &'a CanonicalAuthorization,
    online_policy: &'a CanonicalOnlinePolicyV2,
    online_authorization: &'a CanonicalOnlineAuthorizationV2,
    destination: &'a CanonicalReviewedProductionDestinationProfileV1,
    locator: &'a CanonicalReviewedFreshCredentialSlotLocatorV1,
    delivery: &'a CanonicalFreshCredentialDeliveryBindingV1,
    account_identity: &'a CanonicalReviewedSignerProxyAccountIdentityV1,
    remote_proof_policy: &'a CanonicalReviewedRemoteCredentialProofPolicyV1,
    static_authorization: &'a CanonicalReviewedStaticOnlineAuthorizationV3,
) -> ReviewedPhaseAEligibilityEnvelopeContextV4<'a> {
    ReviewedPhaseAEligibilityEnvelopeContextV4 {
        v1_config: config,
        v1_authorization: authorization,
        online_policy_v2: online_policy,
        online_authorization_v2: online_authorization,
        reviewed_production_destination_v1: destination,
        reviewed_fresh_credential_slot_locator_v1: locator,
        fresh_credential_delivery_binding_v1: delivery,
        reviewed_signer_proxy_account_identity_v1: account_identity,
        reviewed_remote_credential_proof_policy_v1: remote_proof_policy,
        reviewed_static_online_authorization_v3: static_authorization,
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "all exact reviewed roles are preserved"
)]
fn artifact_commitments(
    config: &CanonicalTrialConfig,
    authorization: &CanonicalAuthorization,
    online_policy: &CanonicalOnlinePolicyV2,
    online_authorization: &CanonicalOnlineAuthorizationV2,
    destination: &CanonicalReviewedProductionDestinationProfileV1,
    locator: &CanonicalReviewedFreshCredentialSlotLocatorV1,
    delivery: &CanonicalFreshCredentialDeliveryBindingV1,
    account_identity: &CanonicalReviewedSignerProxyAccountIdentityV1,
    remote_proof_policy: &CanonicalReviewedRemoteCredentialProofPolicyV1,
    static_authorization: &CanonicalReviewedStaticOnlineAuthorizationV3,
    phase_a_v4: &CanonicalReviewedPhaseAEligibilityEnvelopeV4,
    proxy_policy: &CanonicalReviewedPolyProxyControlPolicyV1,
    local_custody: &CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1,
    l1_derivation: &CanonicalReviewedL1CredentialDerivationProofPolicyV1,
) -> ReviewedArtifactCommitmentsV1 {
    ReviewedArtifactCommitmentsV1 {
        canonical_config_v1: commitment(
            config.canonical_sha256(),
            config.canonical_length(),
            config.fingerprint(),
        ),
        authorization_v1: FingerprintOnlyCommitmentV1 {
            fingerprint: authorization.fingerprint().to_owned(),
            projection: FingerprintProjectionV1::CanonicalHolderIntentionallyExposesFingerprintOnly,
        },
        online_policy_v2: commitment(
            online_policy.canonical_sha256(),
            online_policy.canonical_length(),
            online_policy.fingerprint(),
        ),
        online_authorization_v2: commitment(
            online_authorization.canonical_sha256(),
            online_authorization.canonical_length(),
            online_authorization.fingerprint(),
        ),
        reviewed_production_destination_v1: commitment(
            destination.canonical_sha256(),
            destination.canonical_length(),
            destination.fingerprint(),
        ),
        reviewed_fresh_credential_slot_locator_v1: commitment(
            locator.canonical_sha256(),
            locator.canonical_length(),
            locator.fingerprint(),
        ),
        fresh_credential_delivery_binding_v1: commitment(
            delivery.canonical_sha256(),
            delivery.canonical_length(),
            delivery.fingerprint(),
        ),
        reviewed_signer_proxy_account_identity_v1: commitment(
            account_identity.canonical_sha256(),
            account_identity.canonical_length(),
            account_identity.fingerprint(),
        ),
        reviewed_remote_credential_proof_policy_v1: commitment(
            remote_proof_policy.canonical_sha256(),
            remote_proof_policy.canonical_length(),
            remote_proof_policy.fingerprint(),
        ),
        reviewed_static_online_authorization_v3: commitment(
            static_authorization.canonical_sha256(),
            static_authorization.canonical_length(),
            static_authorization.fingerprint(),
        ),
        reviewed_phase_a_eligibility_envelope_v4: commitment(
            phase_a_v4.canonical_sha256(),
            phase_a_v4.canonical_length(),
            phase_a_v4.fingerprint(),
        ),
        reviewed_poly_proxy_control_policy_v1: commitment(
            proxy_policy.canonical_sha256(),
            proxy_policy.canonical_length(),
            proxy_policy.fingerprint(),
        ),
        reviewed_local_operator_cooperative_custody_profile_v1: commitment(
            local_custody.canonical_sha256(),
            local_custody.canonical_length(),
            local_custody.fingerprint(),
        ),
        reviewed_l1_credential_derivation_proof_policy_v1: commitment(
            l1_derivation.canonical_sha256(),
            l1_derivation.canonical_length(),
            l1_derivation.fingerprint(),
        ),
    }
}

fn commitment(
    canonical_sha256: &str,
    canonical_length: u64,
    fingerprint: &str,
) -> CanonicalArtifactCommitmentV1 {
    CanonicalArtifactCommitmentV1 {
        canonical_sha256: canonical_sha256.to_owned(),
        canonical_length,
        fingerprint: fingerprint.to_owned(),
    }
}

fn reviewer_inputs_required() -> Vec<ReviewerInputRequirementV1> {
    use ReviewerInputV1 as Input;
    [
        Input::PositiveSuccessorAuthorizationSchemaAcceptance,
        Input::AuthorizationId,
        Input::IssuingReviewerAuthenticatedIdentity,
        Input::ReviewerTrustAnchorReference,
        Input::ReviewedAtUtc,
        Input::NotBeforeUtc,
        Input::ExpiresAtUtcMaximumFifteenMinuteWindow,
        Input::CleanupNotAfterUtc,
        Input::ExactNonsecretTrialConfigAndPublicPlanFingerprintApproval,
        Input::ExactAccountMarketOrderLossAndTimeTermsApproval,
        Input::ExactReleaseCandidateBuildAndObservedHostSubsetApproval,
        Input::ExactlyOnePhaseAPlaceAttemptApproval,
        Input::OnePlaceNoReplacementAndExactCancelBudgetsApproval,
        Input::OnePossibleFillWithinExactLossCapApproval,
        Input::PostOnlyMayStillFillApproval,
        Input::FiveDistinctOnlineEvidenceClassesApproval,
        Input::NoConcurrentProxyTradingAttestation,
        Input::IndependentManualCleanupMethodApprovalAndReference,
        Input::CooperativeLocalCustodyTrustAndLimitationsApproval,
    ]
    .into_iter()
    .map(|field| ReviewerInputRequirementV1 {
        field,
        status: MissingRequirementStatusV1::ReviewerInputIsMissingV1,
    })
    .collect()
}

fn external_proofs_required() -> Vec<ExternalProofRequirementV1> {
    use ExternalReviewedProofV1 as Proof;
    [
        Proof::PositiveSuccessorAuthorizationContractAndVerifier,
        Proof::AuthenticatedReviewerTrustAnchorAndRecordAuthorship,
        Proof::SelectedCredentialCustodyTrustAlternative,
        Proof::AttemptAudienceBoundDeliveryLeaseOrReviewedLocalCooperationRealization,
        Proof::AuthoritativeRemoteCredentialAcceptanceContract,
        Proof::SameLoadedHolderLiveL1DeriveAndL2AcceptanceProof,
        Proof::AuthoritativeSignerProxyControlContract,
        Proof::CurrentAccountSpecificNonexclusiveSignerProxyControlProof,
        Proof::AuthenticatedFinalizedPolygonStateProofAndReorgInvalidation,
        Proof::IndependentManualCleanupMethodEvidence,
    ]
    .into_iter()
    .map(|proof| ExternalProofRequirementV1 {
        proof,
        status: MissingRequirementStatusV1::ExternalProofIsUnavailableV1,
    })
    .collect()
}

fn runtime_gates_required() -> Vec<RuntimeGateRequirementV1> {
    use RuntimeGateV1 as Gate;
    [
        Gate::ExactNssUsernameAndCompleteV1V2EgressIdentity,
        Gate::CurrentPublicEgressGeoblockStatusHealthAndSameAccountClosedOnly,
        Gate::CurrentMarketBookFeeTickMinimumAndPassivity,
        Gate::CompleteAccountPositionOrderTradeAndUserStreamCuts,
        Gate::SelectedActorGenerationAndSourceOwnedAttempt,
        Gate::RetainedDeliveryEvidenceLoadTokenAndContinuouslyHeldLease,
        Gate::SameHolderL1DeriveResponseAndLoadedL2TupleJoin,
        Gate::CurrentSignerChallengeAndProxyStateProof,
        Gate::DurableCreateNewAttemptClaimBurnFinalConjunctAndNoResend,
        Gate::FixedProductionEgressSinglePlaceDispatchOwner,
        Gate::RecoveryOnlyExactOwnedCancelContinuation,
        Gate::CredentialCleanupAndTerminalReconciliation,
    ]
    .into_iter()
    .map(|gate| RuntimeGateRequirementV1 {
        gate,
        status: MissingRequirementStatusV1::RuntimeGateIsNotObservedV1,
    })
    .collect()
}

fn false_boolean_paths<T: Serialize>(
    verification: &T,
) -> Result<Vec<String>, PhaseAAuthorizationRequestError> {
    let value = serde_json::to_value(verification)
        .map_err(|_| PhaseAAuthorizationRequestError::ReportSerializationFailed)?;
    let mut paths = Vec::new();
    collect_false_boolean_paths("", &value, &mut paths);
    paths.sort();
    Ok(paths)
}

fn collect_false_boolean_paths(prefix: &str, value: &Value, paths: &mut Vec<String>) {
    match value {
        Value::Bool(false) if !prefix.is_empty() => paths.push(prefix.to_owned()),
        Value::Object(fields) => {
            for (name, value) in fields {
                let path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}.{name}")
                };
                collect_false_boolean_paths(&path, value, paths);
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                collect_false_boolean_paths(&format!("{prefix}[{index}]"), value, paths);
            }
        }
        _ => {}
    }
}

fn assert_closed_denial(
    authorization: &OfflineAuthorizationState,
) -> Result<(), PhaseAAuthorizationRequestError> {
    if *authorization != OfflineAuthorizationState::DENIED
        || authorization.place_dispatch_allowance != 0
    {
        return Err(PhaseAAuthorizationRequestError::NonAuthorizingSemanticsViolated);
    }
    Ok(())
}

#[derive(Debug, Error)]
pub(crate) enum PhaseAAuthorizationRequestError {
    #[error("Phase-A request reviewed input is invalid: {0}")]
    InvalidReviewedInput(&'static str),
    #[error("Phase-A request additive structural policy verification failed")]
    AdditivePolicyVerificationFailed,
    #[error("Phase-A request release-candidate freeze failed")]
    ReleaseCandidateFreezeFailed,
    #[error("Phase-A request release-candidate binding changed")]
    ReleaseCandidateBindingChanged,
    #[error("Phase-A request report serialization failed")]
    ReportSerializationFailed,
    #[error("Phase-A request violated closed non-authorizing semantics")]
    NonAuthorizingSemanticsViolated,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requirement_sets_are_closed_complete_and_missing() {
        let reviewer = serde_json::to_value(reviewer_inputs_required()).unwrap();
        let external = serde_json::to_value(external_proofs_required()).unwrap();
        let runtime = serde_json::to_value(runtime_gates_required()).unwrap();
        assert_eq!(reviewer.as_array().unwrap().len(), 19);
        assert_eq!(external.as_array().unwrap().len(), 10);
        assert_eq!(runtime.as_array().unwrap().len(), 12);
        let encoded = serde_json::to_string(&(reviewer, external, runtime)).unwrap();
        assert!(encoded.contains(
            "\"field\":\"exact_nonsecret_trial_config_and_public_plan_fingerprint_approval\""
        ));
        assert!(!encoded.contains("\"status\":\"available_v1\""));
        assert!(!encoded.contains("\"status\":\"observed_v1\""));
        assert!(!encoded.contains("\"status\":\"authorized_v1\""));
    }

    #[test]
    fn false_fact_collection_is_exhaustive_and_sorted() {
        let value = serde_json::json!({
            "z": false,
            "a": {"present": true, "missing": false},
            "array": [false, true]
        });
        let mut paths = Vec::new();
        collect_false_boolean_paths("", &value, &mut paths);
        paths.sort();
        assert_eq!(paths, ["a.missing", "array[0]", "z"]);
    }
}
