//! Offline-only PM-T2 controlled-trial plan, authorization, and custody gate.
//!
//! This crate deliberately has no network transport, mutation execution,
//! signed-order serialization, journal writer, or production authorization.

#![forbid(unsafe_code)]

mod config;
mod consumption;
mod custody;
mod fresh_credential_delivery_binding_v1;
mod online_consumption_v2;
mod online_policy_v2;
mod preflight;
mod protected_file;
mod reviewed_destination_profile_v1;
mod reviewed_fresh_credential_slot_locator_v1;
mod reviewed_l1_credential_derivation_proof_policy_v1;
mod reviewed_local_operator_cooperative_custody_profile_v1;
mod reviewed_phase_a_eligibility_envelope_v4;
mod reviewed_phase_a_reviewer_trust_policy_v1;
mod reviewed_poly_proxy_control_policy_v1;
mod reviewed_remote_credential_proof_policy_v1;
mod reviewed_signer_proxy_account_identity_v1;
mod reviewed_static_online_authorization_v3;

pub use config::{
    AuthorizationApproval, AuthorizationBuildBinding, AuthorizationHostBinding,
    AuthorizationVerification, CanonicalAuthorization, CanonicalTrialConfig,
    PM_T2_AUTHORIZATION_CONSUMPTION_CLAIM_FILE_V1, PM_T2_AUTHORIZATION_CONSUMPTION_LEDGER_FILE_V1,
    PM_T2_JOURNAL_FAMILY_V1, PM_T2_JOURNAL_VERSION_V1, PM_T2_LIVE_DISPATCH_JOURNAL_FILE_V1,
    PM_T2_LIVE_INTENT_JOURNAL_FILE_V1, PlanVerification, PmTrialConfigError, TrialAccount,
    TrialAuthorization, TrialConfig, TrialCredentialSlot, TrialDomain, TrialJournalBinding,
    TrialMarket, TrialOrder, TrialOrderType, TrialPhase, TrialSide, TrialTimeLimits,
    load_canonical_authorization, load_canonical_trial_config, verify_authorization, verify_plan,
};
pub use consumption::{
    AuthorizationConsumptionBindingEvidence, AuthorizationConsumptionEvidence,
    AuthorizationConsumptionState, AuthorizationConsumptionVerification,
    AuthorizationRecoveryCancelPreparedAnchorV1, AuthorizationRecoveryContinuationRegistryV1,
    AuthorizationRecoveryContinuationRootV1, AuthorizationRecoveryTerminalPlanV1,
    AuthorizationRuntimeBinding, ConsumedAuthorizationConsumption, PmAuthorizationConsumptionError,
    PreparedAuthorizationConsumption, TerminalAuthorizationConsumption, TerminalDisposition,
    claim_prepared_authorization_consumption, prepare_authorization_consumption,
    reopen_consumed_authorization_consumption, verify_authorization_consumption,
};
pub use custody::{
    CustodyInspection, CustodyPaths, CustodySummary, PmTrialCustodyError, inspect_custody,
};
pub use fresh_credential_delivery_binding_v1::{
    CanonicalFreshCredentialDeliveryBindingEvidenceV1, CanonicalFreshCredentialDeliveryBindingV1,
    FRESH_CREDENTIAL_DELIVERY_BINDING_V1_SCHEMA_VERSION, FreshCredentialDeliveryBindingV1,
    FreshCredentialDeliveryBindingVerificationV1, FreshCredentialDeliveryLoadTokenV1,
    FreshCredentialLinuxDirectoryIdentityV1, FreshCredentialLinuxFileIdentitiesV1,
    FreshCredentialLinuxFileIdentityV1, FreshCredentialLinuxObjectSetV1,
    FreshCredentialSlotLocatorPinsV1, PM_T2_FRESH_CREDENTIAL_DELIVERY_BINDING_FILE_V1,
    PmFreshCredentialDeliveryBindingV1Error, UnattestedFreshCredentialProviderGenerationV1,
    bind_fresh_credential_delivery_binding_v1, load_canonical_fresh_credential_delivery_binding_v1,
    verify_fresh_credential_delivery_binding_evidence_v1,
    verify_fresh_credential_delivery_binding_v1,
};
pub use online_consumption_v2::{
    ConsumedOnlineAuthorizationConsumptionV2, ONLINE_AUTHORIZATION_CONSUMPTION_V2_SCHEMA_VERSION,
    OnlineAuthorizationConsumptionAttemptV2, OnlineAuthorizationConsumptionBindingV2,
    OnlineAuthorizationConsumptionEvidenceV2, OnlineAuthorizationConsumptionStateV2,
    OnlineAuthorizationConsumptionVerificationV2, OnlineAuthorizationCrashRecoveryV2,
    OnlineAuthorizationPinsV2, OnlineAuthorizationPlacementReuseV2,
    OnlineAuthorizationRuntimeBindingV2, PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_CLAIM_FILE_V2,
    PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_LEDGER_FILE_V2, PM_T2_ONLINE_PREFLIGHT_SIDECAR_FILE_V2,
    PmOnlineAuthorizationConsumptionV2Error, PreparedOnlineAuthorizationConsumptionV2,
    prepare_online_authorization_consumption_v2, verify_online_authorization_consumption_v2,
};
pub use online_policy_v2::{
    CanonicalOnlineAuthorizationV2, CanonicalOnlinePolicyV2,
    ClobLivenessHealthObservationRequirementV2, FreshStatusAnnouncementObservationRequirementV2,
    MAX_ONLINE_OBSERVATION_AGE_MS_V2, MAX_STATUS_NOTICE_HISTORY_QUIET_INTERVAL_SECONDS_V2,
    MIN_STATUS_NOTICE_HISTORY_QUIET_INTERVAL_SECONDS_V2, ONLINE_AUTHORIZATION_V2_SCHEMA_VERSION,
    ONLINE_POLICY_V2_SCHEMA_VERSION, OnlineAttemptScopeApprovalV2, OnlineAuthorizationApprovalV2,
    OnlineAuthorizationBuildBindingV2, OnlineAuthorizationHostBindingV2,
    OnlineAuthorizationPurposeV2, OnlineAuthorizationV2, OnlineAuthorizationVerificationV2,
    OnlineCleanupApprovalV2, OnlineFillRiskApprovalV2, OnlinePhaseScopeApprovalV2,
    OnlinePolicyPinsV2, OnlinePolicyV2, OnlinePolicyVerificationV2,
    OnlinePostOnlySemanticsApprovalV2, OnlineProxyConcurrencyApprovalV2,
    OnlineSourceSeparationApprovalV2, OperationalObservationProfileV2, PmOnlinePolicyV2Error,
    ReviewedDestinationIndependentNatV2, ReviewedLinuxEgressProfileV2,
    ReviewedMarketClassificationV2, ReviewedMarketEvidenceV2,
    ReviewedMarketObservationRequirementV2, ReviewedRepositoryStateV2,
    ReviewedStatusClobComponentV2, ReviewedStatusHistoryObservationRequirementV2,
    ReviewedStatusNoticeHistoryCutV2, ReviewedStatusNoticeHistoryFindingV2,
    ReviewedStatusNoticeHistorySourceV2, SameAccountClosedOnlyObservationRequirementV2,
    V1ConfigPinsV2, load_canonical_online_authorization_v2, load_canonical_online_policy_v2,
    verify_online_authorization_v2, verify_online_policy_v2,
};
pub use preflight::{
    CanonicalTrialPreflight, PmTrialPreflightError, TRIAL_PREFLIGHT_SCHEMA_VERSION,
    TrialAccountPreflight, TrialAuthorizationConsumptionLeaseState, TrialBookPreflight,
    TrialClosedOnlyEvidence, TrialCompleteCutEvidence, TrialConfiguredPositionState,
    TrialDataApiPositionPreflight, TrialEnvironmentPreflight, TrialExactDetailCutEvidence,
    TrialFinalizedChainPreflight, TrialGeoblockEvidence, TrialJournalLeaseEvidence,
    TrialMarketPreflight, TrialObservationStamp, TrialPhaseGateEvidence, TrialPreflightBinding,
    TrialPreflightEvidence, TrialPreflightWindow, TrialPrivateAccountCut,
    TrialReconciliationPreflight, TrialRiskPreflight, TrialServerTimeEvidence,
    TrialUserStreamPreflight, validate_canonical_trial_preflight,
};
pub use reap_polymarket_auth::{
    ExpectedOrderId, FixedOrderId, OwnedCancelSemanticRequestCommitment,
    PlacePublicRequestIdentity, PlaceSemanticRequestCommitment,
    derive_owned_cancel_semantic_request_commitment,
};
pub use reviewed_destination_profile_v1::{
    CanonicalReviewedProductionDestinationProfileV1, MAX_REVIEWED_DNS_ANSWER_AGE_SECONDS_V1,
    PM_T2_REVIEWED_PRODUCTION_DESTINATION_PROFILE_FILE_V1,
    PmReviewedProductionDestinationProfileV1Error, REVIEWED_CLOB_HTTPS_HOST_V1,
    REVIEWED_CLOB_WEBSOCKET_PUBLIC_PATH_V1, REVIEWED_CLOB_WEBSOCKET_USER_PATH_V1,
    REVIEWED_CLOB_WEBSOCKET_WSS_HOST_V1, REVIEWED_DATA_API_HTTPS_HOST_V1,
    REVIEWED_GEOBLOCK_HTTPS_HOST_V1, REVIEWED_POLYGON_RPC_HTTPS_HOST_V1,
    REVIEWED_PRODUCTION_DESTINATION_PROFILE_V1_SCHEMA_VERSION, REVIEWED_STATUS_HTTPS_HOST_V1,
    ReviewedDnsAnswerEvidenceV1, ReviewedDnsAnswerSourceV1, ReviewedFixedTlsDestinationV1,
    ReviewedFixedWebSocketDestinationV1, ReviewedOnlineAuthorizationPinsV1,
    ReviewedProductionDestinationProfileV1, ReviewedProductionDestinationProfileVerificationV1,
    ReviewedProductionDestinationsV1, load_canonical_reviewed_production_destination_profile_v1,
    verify_reviewed_production_destination_profile_v1,
};
pub use reviewed_fresh_credential_slot_locator_v1::{
    CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1,
    CanonicalReviewedFreshCredentialSlotLocatorV1, PM_T2_FRESH_API_KEY_ENTRY_V1,
    PM_T2_FRESH_L2_SECRET_ENTRY_V1, PM_T2_FRESH_PASSPHRASE_ENTRY_V1,
    PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1, PM_T2_REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_FILE_V1,
    PmReviewedFreshCredentialSlotLocatorV1Error,
    REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_SCHEMA_VERSION, ReviewedFreshCredentialFilesV1,
    ReviewedFreshCredentialLoadTokenV1, ReviewedFreshCredentialSlotLocatorV1,
    ReviewedFreshCredentialSlotLocatorVerificationV1,
    bind_reviewed_fresh_credential_slot_locator_v1,
    load_canonical_reviewed_fresh_credential_slot_locator_v1,
    verify_reviewed_fresh_credential_slot_locator_evidence_v1,
    verify_reviewed_fresh_credential_slot_locator_v1,
};
pub use reviewed_l1_credential_derivation_proof_policy_v1::{
    CanonicalReviewedL1CredentialDerivationProofPolicyV1,
    PM_T2_REVIEWED_L1_CREDENTIAL_DERIVATION_PROOF_POLICY_FILE_V1,
    PmReviewedL1CredentialDerivationProofPolicyV1Error,
    REVIEWED_L1_CREDENTIAL_DERIVATION_PROOF_POLICY_V1_SCHEMA_VERSION,
    ReviewedL1CredentialDerivationAccountIdentityPinsV1,
    ReviewedL1CredentialDerivationConfigPinsV1, ReviewedL1CredentialDerivationDeliveryPinsV1,
    ReviewedL1CredentialDerivationDestinationPinsV1,
    ReviewedL1CredentialDerivationDispatchPolicyV1,
    ReviewedL1CredentialDerivationEip712FieldOrderV1, ReviewedL1CredentialDerivationEip712PolicyV1,
    ReviewedL1CredentialDerivationEligibilityEnvelopePinsV1,
    ReviewedL1CredentialDerivationEndpointCorrelationV1,
    ReviewedL1CredentialDerivationEndpointPolicyV1, ReviewedL1CredentialDerivationHeaderNamesV1,
    ReviewedL1CredentialDerivationLocalCustodyProfilePinsV1,
    ReviewedL1CredentialDerivationMonotonicFreshnessRequirementV1,
    ReviewedL1CredentialDerivationNoncePolicyV1,
    ReviewedL1CredentialDerivationOfficialSourceLimitationsV1,
    ReviewedL1CredentialDerivationOfficialSourcesV1,
    ReviewedL1CredentialDerivationOnlineAuthorizationPinsV1,
    ReviewedL1CredentialDerivationOnlinePolicyPinsV1,
    ReviewedL1CredentialDerivationProofPolicyContextV1,
    ReviewedL1CredentialDerivationProofPolicyV1,
    ReviewedL1CredentialDerivationProofPolicyVerificationV1,
    ReviewedL1CredentialDerivationProtocolPolicyV1, ReviewedL1CredentialDerivationRecordRoleV1,
    ReviewedL1CredentialDerivationRemoteProofPolicyPinsV1,
    ReviewedL1CredentialDerivationRequestPolicyV1,
    ReviewedL1CredentialDerivationResponseObjectGrammarV1,
    ReviewedL1CredentialDerivationResponsePolicyV1,
    ReviewedL1CredentialDerivationSourceEntryPinsV1,
    ReviewedL1CredentialDerivationSourceOwnedTimeRequirementV1,
    ReviewedL1CredentialDerivationStaticAuthorizationPinsV1,
    ReviewedL1CredentialDerivationTimePolicyV1,
    ReviewedL1CredentialDerivationTransportDispositionV1,
    ReviewedL1CredentialDerivationTupleAssociationPolicyV1,
    ReviewedL1CredentialDerivationUnavailableSourceContractV1,
    draft_non_authorizing_reviewed_l1_credential_derivation_proof_policy_v1,
    load_canonical_reviewed_l1_credential_derivation_proof_policy_v1,
    verify_reviewed_l1_credential_derivation_proof_policy_v1,
};
pub use reviewed_local_operator_cooperative_custody_profile_v1::{
    CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1,
    PM_T2_REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_FILE_V1,
    PmReviewedLocalOperatorCooperativeCustodyProfileV1Error,
    REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_V1_SCHEMA_VERSION,
    ReviewedLocalOperatorAtomicUnlinkLimitationV1, ReviewedLocalOperatorCleanupObservationV1,
    ReviewedLocalOperatorCooperativeCustodyProfileContextV1,
    ReviewedLocalOperatorCooperativeCustodyProfileV1,
    ReviewedLocalOperatorCooperativeCustodyProfileVerificationV1,
    ReviewedLocalOperatorCooperativeCustodyTrustV1, ReviewedLocalOperatorCurrentnessLimitationV1,
    ReviewedLocalOperatorCustodyAudienceV1, ReviewedLocalOperatorCustodyConfigPinsV1,
    ReviewedLocalOperatorCustodyDeliveryPinsV1,
    ReviewedLocalOperatorCustodyEligibilityEnvelopePinsV1,
    ReviewedLocalOperatorCustodyLocatorPinsV1,
    ReviewedLocalOperatorCustodyOnlineAuthorizationPinsV1,
    ReviewedLocalOperatorCustodyOnlinePolicyPinsV1, ReviewedLocalOperatorCustodyPhaseScopeV1,
    ReviewedLocalOperatorCustodyRecordRoleV1,
    ReviewedLocalOperatorCustodyStaticAuthorizationPinsV1,
    ReviewedLocalOperatorCustodySuccessorDispositionV1, ReviewedLocalOperatorDirectoryLeaseV1,
    ReviewedLocalOperatorGlobalUniquenessLimitationV1,
    ReviewedLocalOperatorOtherCopiesLimitationV1, ReviewedLocalOperatorProviderOriginLimitationV1,
    ReviewedLocalOperatorRecoveryRetentionV1, ReviewedLocalOperatorRevocationLimitationV1,
    ReviewedLocalOperatorSameEuidTrustV1, ReviewedLocalOperatorSecureErasureLimitationV1,
    ReviewedLocalOperatorSourceDescriptorCustodyV1,
    draft_non_authorizing_reviewed_local_operator_cooperative_custody_profile_v1,
    load_canonical_reviewed_local_operator_cooperative_custody_profile_v1,
    verify_reviewed_local_operator_cooperative_custody_profile_v1,
};
pub use reviewed_phase_a_eligibility_envelope_v4::{
    CanonicalReviewedPhaseAEligibilityEnvelopeV4,
    PM_T2_REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_FILE_V4,
    PmReviewedPhaseAEligibilityEnvelopeV4Error,
    REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4_SCHEMA_VERSION, ReviewedPhaseAAccountIdentityPinsV4,
    ReviewedPhaseAActorRuntimeAttemptStatusV4, ReviewedPhaseAConfigPinsV4,
    ReviewedPhaseACredentialDeliveryLeaseProofStatusV4,
    ReviewedPhaseACredentialProviderTrustRootStatusV4, ReviewedPhaseADeliveryCustodyJoinStatusV4,
    ReviewedPhaseADeliveryPinsV4, ReviewedPhaseADestinationPinsV4,
    ReviewedPhaseADurableAttemptLineageStatusV4, ReviewedPhaseAEligibilityEnvelopeContextV4,
    ReviewedPhaseAEligibilityEnvelopeDraftInputsV4, ReviewedPhaseAEligibilityEnvelopeV4,
    ReviewedPhaseAEligibilityEnvelopeVerificationV4, ReviewedPhaseALocatorPinsV4,
    ReviewedPhaseAOnlineAuthorizationPinsV4, ReviewedPhaseAOnlinePolicyPinsV4,
    ReviewedPhaseARecordRoleV4, ReviewedPhaseARemoteAcceptanceContractStatusV4,
    ReviewedPhaseARemoteCredentialProofStatusV4, ReviewedPhaseARemoteProofPolicyPinsV4,
    ReviewedPhaseAReviewerTrustAnchorStatusV4, ReviewedPhaseAScopeV4,
    ReviewedPhaseASignerProxyContractStatusV4, ReviewedPhaseASignerProxyProofStatusV4,
    ReviewedPhaseASingleDispatchStatusV4, ReviewedPhaseAStaticAuthorizationPinsV4,
    ReviewedPhaseAUnavailableExternalEvidenceV4, ReviewedPhaseAUnavailableRuntimeLineageV4,
    ReviewedPhaseAV1AuthorizationPinsV4,
    draft_non_authorizing_reviewed_phase_a_eligibility_envelope_v4,
    load_canonical_reviewed_phase_a_eligibility_envelope_v4,
    verify_reviewed_phase_a_eligibility_envelope_v4,
};
pub use reviewed_phase_a_reviewer_trust_policy_v1::{
    CanonicalReviewedPhaseAReviewerTrustPolicyV1, MAX_PHASE_A_AUTHORIZATION_TTL_SECONDS_V1,
    MAX_PHASE_A_REVIEWER_APPROVERS_V1, PM_T2_REVIEWED_PHASE_A_REVIEWER_TRUST_POLICY_FILE_V1,
    PmReviewedPhaseAReviewerTrustPolicyV1Error,
    REVIEWED_PHASE_A_REVIEWER_TRUST_POLICY_V1_SCHEMA_VERSION,
    ReviewedPhaseAReviewerTrustPolicyVerificationV1,
    load_canonical_reviewed_phase_a_reviewer_trust_policy_v1,
    verify_reviewed_phase_a_reviewer_trust_policy_v1,
};
pub use reviewed_poly_proxy_control_policy_v1::{
    CanonicalReviewedPolyProxyControlPolicyV1, PM_POLY_PROXY_FACTORY_ADDRESS_V1,
    PM_POLY_PROXY_IMPLEMENTATION_ADDRESS_V1, PM_POLY_PROXY_INIT_CODE_BYTE_LENGTH_V1,
    PM_POLY_PROXY_INIT_CODE_HEX_V1, PM_POLY_PROXY_INIT_CODE_KECCAK256_V1,
    PM_POLY_PROXY_NEGATIVE_RISK_V2_EXCHANGE_ADDRESS_V1,
    PM_POLY_PROXY_OWNER_STORAGE_SLOT_LITERAL_V1, PM_POLY_PROXY_OWNER_UTF8_KECCAK256_V1,
    PM_POLY_PROXY_POLYGON_CHAIN_ID_V1, PM_POLY_PROXY_RUNTIME_BYTE_LENGTH_V1,
    PM_POLY_PROXY_RUNTIME_HEX_V1, PM_POLY_PROXY_RUNTIME_KECCAK256_V1,
    PM_POLY_PROXY_STANDARD_V2_EXCHANGE_ADDRESS_V1,
    PM_T2_REVIEWED_POLY_PROXY_CONTROL_POLICY_FILE_V1, PmReviewedPolyProxyControlPolicyV1Error,
    REVIEWED_POLY_PROXY_CONTROL_POLICY_V1_SCHEMA_VERSION, ReviewedPolyProxyControlPolicyV1,
    ReviewedPolyProxyControlPolicyVerificationV1, ReviewedPolyProxyCreate2AddressRelationV1,
    ReviewedPolyProxyCreate2PolicyV1, ReviewedPolyProxyDeterministicRelationLimitationV1,
    ReviewedPolyProxyExchangeOraclePolicyV1, ReviewedPolyProxyExclusiveControlLimitationV1,
    ReviewedPolyProxyFutureProofPolicyV1, ReviewedPolyProxyFutureProofRequirementV1,
    ReviewedPolyProxyInitCodeV1, ReviewedPolyProxyInitializerEncodingV1,
    ReviewedPolyProxyLimitationsV1, ReviewedPolyProxyOtherSignatureTypesStatusV1,
    ReviewedPolyProxyOwnerStoragePolicyV1, ReviewedPolyProxyOwnerStorageSlotCommentStatusV1,
    ReviewedPolyProxyOwnerStorageSlotSourceV1, ReviewedPolyProxyRecordRoleV1,
    ReviewedPolyProxyRequiredEvidenceStatusV1, ReviewedPolyProxyRuntimeRelationV1,
    ReviewedPolyProxyRuntimeTemplateV1, ReviewedPolyProxySignerSaltRelationV1,
    ReviewedPolyProxyStoredOwnerRelationV1, ReviewedPolyProxyStructuralPolicyV1,
    ReviewedPolyProxyTypeOneRelationV1, ReviewedPolyProxyTypeZeroRelationV1,
    ReviewedPolyProxyUnattestedSourceLabelsV1, ReviewedPolyProxyUnavailableEvidenceV1,
    load_canonical_reviewed_poly_proxy_control_policy_v1,
    verify_reviewed_poly_proxy_control_policy_v1,
};
pub use reviewed_remote_credential_proof_policy_v1::{
    CanonicalReviewedRemoteCredentialProofPolicyV1,
    PM_T2_REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_FILE_V1,
    PmReviewedRemoteCredentialProofPolicyV1Error,
    REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_SCHEMA_VERSION,
    ReviewedRemoteCredentialAuthenticationAcceptanceContractStatusV1,
    ReviewedRemoteCredentialProofAccountIdentityPinsV1,
    ReviewedRemoteCredentialProofDeliveryPinsV1, ReviewedRemoteCredentialProofDestinationPinsV1,
    ReviewedRemoteCredentialProofDispatchPolicyV1, ReviewedRemoteCredentialProofEndpointPolicyV1,
    ReviewedRemoteCredentialProofFreshnessPolicyV1,
    ReviewedRemoteCredentialProofHmacPreimageGrammarV1,
    ReviewedRemoteCredentialProofHmacPreimageOrderedVariantV1,
    ReviewedRemoteCredentialProofLocatorPinsV1, ReviewedRemoteCredentialProofOfficialSourcesV1,
    ReviewedRemoteCredentialProofPolicyContextV1, ReviewedRemoteCredentialProofPolicyV1,
    ReviewedRemoteCredentialProofPolicyVerificationV1,
    ReviewedRemoteCredentialProofProtocolPolicyV1, ReviewedRemoteCredentialProofRequestPolicyV1,
    ReviewedRemoteCredentialProofResponsePolicyV1,
    ReviewedRemoteCredentialProofSensitiveHeaderNamesV1,
    ReviewedRemoteCredentialProofSourceEntryPinsV1,
    load_canonical_reviewed_remote_credential_proof_policy_v1,
    verify_reviewed_remote_credential_proof_policy_v1,
};
pub use reviewed_signer_proxy_account_identity_v1::{
    CanonicalReviewedSignerProxyAccountIdentityV1, PM_T2_OFFICIAL_SOURCE_MANIFEST_BYTE_LENGTH_V1,
    PM_T2_OFFICIAL_SOURCE_MANIFEST_RETRIEVED_AT_UTC_V1,
    PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_FAMILY_V1,
    PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_VERSION_V1, PM_T2_OFFICIAL_SOURCE_MANIFEST_SHA256_V1,
    PM_T2_REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_FILE_V1,
    PmReviewedSignerProxyAccountIdentityV1Error,
    REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_SCHEMA_VERSION, ReviewedOfficialSourceManifestPinsV1,
    ReviewedSignerProxyAccountEvidenceKindV1, ReviewedSignerProxyAccountIdentityV1,
    ReviewedSignerProxyAccountIdentityVerificationV1, ReviewedSignerProxyClaimedAccountV1,
    UnattestedReviewedSignerProxyAccountEvidenceV1,
    load_canonical_reviewed_signer_proxy_account_identity_v1,
    verify_reviewed_signer_proxy_account_identity_v1,
};
pub use reviewed_static_online_authorization_v3::{
    CanonicalReviewedStaticOnlineAuthorizationV3,
    PM_T2_REVIEWED_STATIC_ONLINE_AUTHORIZATION_FILE_V3, PmReviewedStaticOnlineAuthorizationV3Error,
    REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3_SCHEMA_VERSION, ReviewedActorCommandSetV3,
    ReviewedActorGenerationAllocationV3, ReviewedActorGenerationSchemeV3, ReviewedActorReadinessV3,
    ReviewedActorRuntimeAttemptCommitmentLocationV3, ReviewedActorTerminalRequirementV3,
    ReviewedBasisAndBurnOrderV3, ReviewedCrashRecoveryProfileV3,
    ReviewedCredentialDeliveryLeaseProtocolStatusV3, ReviewedCredentialProviderTrustRootStatusV3,
    ReviewedPreparedCreationOrderV3, ReviewedRemoteCredentialAcceptanceContractStatusV3,
    ReviewedSelectedActorProfileV3, ReviewedSignerProxyControlContractStatusV3,
    ReviewedStaticOnlineAuthorizationAccountIdentityPinsV3,
    ReviewedStaticOnlineAuthorizationConfigPinsV3, ReviewedStaticOnlineAuthorizationContextV3,
    ReviewedStaticOnlineAuthorizationDeliveryPinsV3,
    ReviewedStaticOnlineAuthorizationDestinationPinsV3,
    ReviewedStaticOnlineAuthorizationFrozenConsumptionLineageV3,
    ReviewedStaticOnlineAuthorizationLocatorPinsV3,
    ReviewedStaticOnlineAuthorizationOnlineAuthorizationPinsV3,
    ReviewedStaticOnlineAuthorizationOnlinePolicyPinsV3,
    ReviewedStaticOnlineAuthorizationRemoteProofPolicyPinsV3,
    ReviewedStaticOnlineAuthorizationSelectedActorProfileV3,
    ReviewedStaticOnlineAuthorizationUnavailablePositiveContractsV3,
    ReviewedStaticOnlineAuthorizationV1AuthorizationPinsV3, ReviewedStaticOnlineAuthorizationV3,
    ReviewedStaticOnlineAuthorizationVerificationV3, ReviewedStaticV3RuntimeStateV3,
    load_canonical_reviewed_static_online_authorization_v3,
    verify_reviewed_static_online_authorization_v3,
};

/// Hard offline outcome of every command in this A0 executable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfflineAuthorizationState {
    pub production_order_entry_authorized: bool,
    pub real_order_submission_authorized: bool,
    pub place_dispatch_allowance: u8,
}

impl OfflineAuthorizationState {
    pub const DENIED: Self = Self {
        production_order_entry_authorized: false,
        real_order_submission_authorized: false,
        place_dispatch_allowance: 0,
    };
}
