use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
};

use reap_pm_controlled_trial::{
    AuthorizationApproval, AuthorizationBuildBinding, AuthorizationHostBinding,
    CanonicalAuthorization, CanonicalReviewedRemoteCredentialProofPolicyV1,
    OfflineAuthorizationState, REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3_SCHEMA_VERSION,
    ReviewedActorCommandSetV3, ReviewedActorGenerationAllocationV3,
    ReviewedActorGenerationSchemeV3, ReviewedActorReadinessV3,
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
    ReviewedStaticV3RuntimeStateV3, TrialAuthorization, load_canonical_authorization,
    load_canonical_reviewed_static_online_authorization_v3,
    verify_reviewed_static_online_authorization_v3,
};
use reap_pm_controlled_trial::{
    ClobLivenessHealthObservationRequirementV2, FreshCredentialDeliveryBindingV1,
    FreshCredentialLinuxDirectoryIdentityV1, FreshCredentialLinuxFileIdentitiesV1,
    FreshCredentialLinuxFileIdentityV1, FreshCredentialLinuxObjectSetV1,
    FreshCredentialSlotLocatorPinsV1, OnlineAttemptScopeApprovalV2, OnlineAuthorizationApprovalV2,
    OnlineAuthorizationBuildBindingV2, OnlineAuthorizationHostBindingV2,
    OnlineAuthorizationPurposeV2, OnlineAuthorizationV2, OnlineCleanupApprovalV2,
    OnlineFillRiskApprovalV2, OnlinePhaseScopeApprovalV2, OnlinePolicyPinsV2, OnlinePolicyV2,
    OnlinePostOnlySemanticsApprovalV2, OnlineProxyConcurrencyApprovalV2,
    OnlineSourceSeparationApprovalV2, OperationalObservationProfileV2,
    PM_T2_FRESH_API_KEY_ENTRY_V1, PM_T2_FRESH_L2_SECRET_ENTRY_V1, PM_T2_FRESH_PASSPHRASE_ENTRY_V1,
    PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1, PM_T2_OFFICIAL_SOURCE_MANIFEST_BYTE_LENGTH_V1,
    PM_T2_OFFICIAL_SOURCE_MANIFEST_RETRIEVED_AT_UTC_V1,
    PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_FAMILY_V1,
    PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_VERSION_V1, PM_T2_OFFICIAL_SOURCE_MANIFEST_SHA256_V1,
    REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_SCHEMA_VERSION,
    REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_SCHEMA_VERSION, ReviewedDestinationIndependentNatV2,
    ReviewedDnsAnswerEvidenceV1, ReviewedDnsAnswerSourceV1, ReviewedFixedTlsDestinationV1,
    ReviewedFixedWebSocketDestinationV1, ReviewedFreshCredentialFilesV1,
    ReviewedFreshCredentialSlotLocatorV1, ReviewedLinuxEgressProfileV2,
    ReviewedMarketClassificationV2, ReviewedMarketEvidenceV2,
    ReviewedMarketObservationRequirementV2, ReviewedOfficialSourceManifestPinsV1,
    ReviewedOnlineAuthorizationPinsV1, ReviewedProductionDestinationProfileV1,
    ReviewedProductionDestinationsV1,
    ReviewedRemoteCredentialAuthenticationAcceptanceContractStatusV1,
    ReviewedRemoteCredentialProofAccountIdentityPinsV1,
    ReviewedRemoteCredentialProofDeliveryPinsV1, ReviewedRemoteCredentialProofDestinationPinsV1,
    ReviewedRemoteCredentialProofDispatchPolicyV1, ReviewedRemoteCredentialProofEndpointPolicyV1,
    ReviewedRemoteCredentialProofFreshnessPolicyV1,
    ReviewedRemoteCredentialProofHmacPreimageGrammarV1,
    ReviewedRemoteCredentialProofHmacPreimageOrderedVariantV1,
    ReviewedRemoteCredentialProofLocatorPinsV1, ReviewedRemoteCredentialProofOfficialSourcesV1,
    ReviewedRemoteCredentialProofPolicyV1, ReviewedRemoteCredentialProofProtocolPolicyV1,
    ReviewedRemoteCredentialProofRequestPolicyV1, ReviewedRemoteCredentialProofResponsePolicyV1,
    ReviewedRemoteCredentialProofSensitiveHeaderNamesV1,
    ReviewedRemoteCredentialProofSourceEntryPinsV1, ReviewedRepositoryStateV2,
    ReviewedSignerProxyAccountEvidenceKindV1, ReviewedSignerProxyAccountIdentityV1,
    ReviewedSignerProxyClaimedAccountV1, ReviewedStatusClobComponentV2,
    ReviewedStatusHistoryObservationRequirementV2, ReviewedStatusNoticeHistoryCutV2,
    ReviewedStatusNoticeHistoryFindingV2, ReviewedStatusNoticeHistorySourceV2,
    SameAccountClosedOnlyObservationRequirementV2, TrialAccount, TrialConfig, TrialCredentialSlot,
    TrialDomain, TrialJournalBinding, TrialMarket, TrialOrder, TrialOrderType, TrialPhase,
    TrialSide, TrialTimeLimits, UnattestedFreshCredentialProviderGenerationV1,
    UnattestedReviewedSignerProxyAccountEvidenceV1, V1ConfigPinsV2,
    load_canonical_fresh_credential_delivery_binding_v1, load_canonical_online_authorization_v2,
    load_canonical_online_policy_v2, load_canonical_reviewed_fresh_credential_slot_locator_v1,
    load_canonical_reviewed_production_destination_profile_v1,
    load_canonical_reviewed_remote_credential_proof_policy_v1,
    load_canonical_reviewed_signer_proxy_account_identity_v1, load_canonical_trial_config,
};
use sha2::{Digest as _, Sha256};
use tempfile::TempDir;

const SIGNER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const FUNDER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const REVIEWED_CLOB_COMPONENT_NAME: &str = "  Trading API (CLOB)";
const REVIEWED_CREDENTIAL_DIRECTORY: &str = "/run/reap/pm-t2/credentials/pm-t2-slot-1";

const GOLDEN_CANONICAL_JSON: &str = r#"{"schema_version":3,"static_authorization_id":"pm-t2-static-online-authorization-v3","reviewer_label":"operator-reviewer","reviewed_at_utc":"2026-08-09T12:00:00Z","not_before_utc":"2026-08-09T12:00:00Z","expires_at_utc":"2026-08-09T12:15:00Z","cleanup_not_after_utc":"2026-08-09T12:20:00Z","v1_config":{"schema_version":1,"canonical_sha256":"23056742d14de855cd263e14a0e4549fb5d172f1a71683f525863c9cb92e465f","canonical_length":2450,"fingerprint":"f2b3c0ec4445c7c02a8cd2ee692edc47f0564c74c398ff6253e0959e31063df7","plan_fingerprint":"c422f334503711dac5c91174e8f6f2274f54a71cebace3ea8297a85ff3afd008"},"v1_authorization":{"schema_version":1,"authorization_id":"pm-t2-a-authorization-1","canonical_sha256":"c86652753a457c8622a12713d105616bb0e1894c75b34fd9223964a666008c25","canonical_length":3830,"fingerprint":"533538400b8b28d50982ac671c82e5346a11322458c4f903d7607d6c7bf79fc5"},"online_policy_v2":{"schema_version":2,"policy_id":"pm-t2-online-policy-v2","canonical_sha256":"8656a900122f2278e187d28e86503168577c3ab660813b907c009f0ca022649d","canonical_length":1278,"fingerprint":"58a8a69165a6690dc2394b43d7e96e12f8016cb47fe39aa65104c91df4800589"},"online_authorization_v2":{"schema_version":2,"authorization_id":"pm-t2-online-authorization-v2","canonical_sha256":"20966b61e04a40b90daccf97085082bbba7657699a32eb6c67fc868d4997938c","canonical_length":2663,"fingerprint":"e880375bda260b557c8c8360fba2678500c37c4e46f6aaaa29a07ec478e81ccb"},"reviewed_production_destination_v1":{"schema_version":1,"profile_id":"pm-t2-reviewed-production-destinations-v1","canonical_sha256":"dc3c0dc61ca669c8e0f6390608e9efa6873139965de567bade87f632685e612f","canonical_length":2370,"fingerprint":"d3323ab9e5b8a839451521d7d380f1461c5ee818dc549889a04f3eed855e50f0"},"reviewed_fresh_credential_slot_locator_v1":{"schema_version":1,"locator_id":"pm-t2-reviewed-fresh-credential-slot-locator-v1","canonical_sha256":"a52ed7c70e681fff9ae747513c29db43aa9cf2084085597f70f0b9648930aacc","canonical_length":1422,"fingerprint":"52db4cb5f58e7e1cd830a7d16e5b0c737271b16e2f07c2968d9f328d5e447e3b"},"fresh_credential_delivery_binding_v1":{"schema_version":1,"binding_id":"pm-t2-fresh-credential-delivery-binding-v1","canonical_sha256":"cf7526d00a1c24543ab9319060a10cf4fbda4652247bf79eb0222991e38e838e","canonical_length":1939,"fingerprint":"74276425237900ab146e7a43b9824c6adadcfb6ca930193b4e3fa0f485fc4726"},"reviewed_signer_proxy_account_identity_v1":{"schema_version":1,"identity_id":"pm-t2-account-v1","canonical_sha256":"c0f4bc76d0cf57218ab66bd79793cdc6608d9f48f415c84040a17e87a37901b3","canonical_length":1887,"fingerprint":"c6a1fdb9b16522568ee6efbf99ad092f3f82d66f745ece1ee1df92eb80357800"},"reviewed_remote_credential_proof_policy_v1":{"schema_version":1,"canonical_sha256":"9566e5867e660c97fda8076527611177605e660f342d14a24ca4af45e5927497","canonical_length":6841,"fingerprint":"c4e060a2960436001d6e3da2701f0bdb01ad5cd1e6da80b2c8ad16d1a62ee462"},"unavailable_positive_contracts":{"credential_provider_trust_root_status":"unavailable_in_frozen_sources_v1","authenticated_credential_delivery_lease_protocol_status":"unavailable_in_frozen_sources_v1","authoritative_remote_credential_acceptance_contract_status":"unavailable_in_frozen_sources_v1","authoritative_signer_proxy_control_contract_status":"unattested_reviewed_labels_only_v1"},"selected_actor_profile":{"profile":"shutdown_only_selected_egress_current_thread_local_set_v1","generation_scheme":"process_id_thread_id_and_rc_pointer_identity_v1","generation_allocation":"before_runtime_local_set_and_all_actor_local_construction_v1","readiness":"task_entry_ack_after_generation_membership_revalidation_v1","command_set":"shutdown_only_v1","terminal_requirement":"shutdown_requested_no_abort_task_joined_clean_credentials_dropped_staged_files_removed_generation_revalidated_v1","runtime_attempt_commitment_location":"future_prepared_only_absent_from_static_v3"},"frozen_v1_v2_consumption_lineage":{"prepared_creation_order":"online_v2_prepared_then_v1_prepared_v1","basis_and_burn_order":"basis_after_existing_place_and_both_consumption_prepared_then_v2_claim_then_v1_claim_then_v1_a3_then_v2_a3_conjunct_v1","crash_recovery":"existing_v1_lifecycle_only_no_placement_resume_v1","static_v3_runtime_state":"no_prepared_claim_burn_recovery_or_dispatch_v1"}}"#;
const GOLDEN_CANONICAL_LENGTH: u64 = 4275;
const GOLDEN_CANONICAL_SHA256: &str =
    "91b285cf063ceb39a93de8065f569f6ccd12dee7ed8fd86634c36dd474bc7471";
const GOLDEN_FINGERPRINT: &str = "a41a380ea9fac013dd3b83bd280ca839780ccbbe2e742f135b81ef2a2eb1bb0c";

#[test]
fn exact_canonical_static_conjunction_is_structural_and_permanently_denied() {
    let fixture = StaticFixture::new();
    let record = fixture.record();
    let canonical = fixture.load_record("static-v3.json", &record);
    let verification =
        verify_reviewed_static_online_authorization_v3(&fixture.context(), &canonical).unwrap();

    assert_eq!(
        verification.authorization,
        OfflineAuthorizationState::DENIED
    );
    assert_eq!(verification.schema_version, 3);
    assert_eq!(
        verification.reviewed_static_online_authorization_v3_fingerprint,
        canonical.fingerprint()
    );
    assert_eq!(
        canonical.canonical_length(),
        serde_json::to_vec(&record).unwrap().len() as u64
    );
    assert_eq!(canonical.canonical_sha256().len(), 64);
    assert_eq!(canonical.fingerprint().len(), 64);

    let true_structural_facts = BTreeSet::from([
        "exact_v1_config_pin_structurally_valid",
        "exact_v1_authorization_pin_structurally_valid",
        "exact_online_policy_v2_pin_structurally_valid",
        "exact_online_authorization_v2_pin_structurally_valid",
        "exact_reviewed_production_destination_v1_pin_structurally_valid",
        "exact_reviewed_fresh_credential_slot_locator_v1_pin_structurally_valid",
        "exact_fresh_credential_delivery_binding_v1_pin_structurally_valid",
        "exact_reviewed_signer_proxy_account_identity_v1_pin_structurally_valid",
        "exact_reviewed_remote_credential_proof_policy_v1_pin_structurally_valid",
        "v1_authorization_exact_canonical_bytes_reconstructed_and_pinned",
        "v1_authorization_structurally_valid_at_own_not_before",
        "online_v2_contract_structurally_valid_without_current_clock",
        "exact_v1_v2_common_phase_build_host_tuple_structurally_valid",
        "online_v2_window_nested_within_v1",
        "static_window_exactly_matches_online_authorization_v2",
        "static_review_order_against_v1_and_v2_structurally_valid",
        "component_verifiers_denied_and_negative_facts_checked",
        "unavailable_positive_contracts_structurally_valid",
        "selected_actor_profile_labels_structurally_valid",
        "frozen_v1_v2_consumption_lineage_labels_structurally_valid",
    ]);
    let serialized = serde_json::to_value(&verification).unwrap();
    let object = serialized.as_object().unwrap();
    for (name, value) in object {
        if let Some(actual) = value.as_bool() {
            assert_eq!(
                actual,
                true_structural_facts.contains(name.as_str()),
                "unexpected verification boolean for {name}"
            );
        }
    }
    for direct_false_fact in [
        "prerequisite_artifacts_existed_at_static_review_time_attested",
        "credential_provider_trust_root_available",
        "delivery_generation_attested",
        "rotation_generation_attested",
        "retained_delivery_evidence_and_load_token_joined",
        "delivery_and_remote_proof_same_source_generation_attested",
        "globally_unique_credential_delivery_attested",
        "signer_on_chain_eoa_status_verified",
        "proxy_on_chain_contract_status_verified",
        "on_chain_account_state_checked",
        "on_chain_finality_checked",
        "proxy_factory_semantics_verified",
        "credential_tuple_current_and_unrevoked_attested",
        "actor_started",
        "runtime_attempt_commitment_present",
        "runtime_attempt_commitment_source_owned",
        "runtime_attempt_commitment_fresh",
        "v3_create_new_performed",
        "v3_file_fsynced",
        "v3_parent_directory_fsynced",
        "basis_inspected",
        "basis_durable",
        "online_v2_claim_inspected",
        "online_v2_claim_durable",
        "v1_claim_inspected",
        "v1_claim_durable",
        "v1_a3_checked",
        "v1_a3_durable",
        "online_v2_a3_conjunct_checked",
        "online_v2_a3_conjunct_durable",
        "burn_and_no_resend_established",
        "recovery_state_checked",
        "placement_resumption_allowed",
    ] {
        assert_eq!(object[direct_false_fact], false, "{direct_false_fact}");
    }
    assert_eq!(object["place_dispatch_allowance"], 0);
    assert_eq!(object["production_order_entry_authorized"], false);
    assert_eq!(object["real_order_submission_authorized"], false);

    let debug = format!("{canonical:?}");
    assert!(debug.contains("no-value-id-address-path-actor-or-lineage-projection"));
    assert!(!debug.contains("pm-t2-static-online-authorization-v3"));
    assert!(!debug.contains(SIGNER));
    assert!(!debug.contains(REVIEWED_CREDENTIAL_DIRECTORY));
}

#[test]
fn loader_rejects_noncanonical_duplicate_unknown_trailing_and_open_contract_inputs() {
    let fixture = StaticFixture::new();
    let record = fixture.record();
    let canonical = serde_json::to_vec(&record).unwrap();

    let duplicate = String::from_utf8(canonical.clone()).unwrap().replacen(
        r#"{"schema_version":3,"#,
        r#"{"schema_version":3,"schema_version":3,"#,
        1,
    );
    fixture.assert_load_rejected("duplicate.json", duplicate.as_bytes());

    let unknown = String::from_utf8(canonical.clone()).unwrap().replacen(
        r#"{"schema_version":3,"#,
        r#"{"schema_version":3,"unknown":false,"#,
        1,
    );
    fixture.assert_load_rejected("unknown.json", unknown.as_bytes());

    let mut trailing = canonical.clone();
    trailing.push(b'\n');
    fixture.assert_load_rejected("trailing.json", &trailing);

    fixture.assert_load_rejected("pretty.json", &serde_json::to_vec_pretty(&record).unwrap());

    let open_contract = String::from_utf8(canonical).unwrap().replacen(
        "unavailable_in_frozen_sources_v1",
        "caller_supplied_digest_v1",
        1,
    );
    fixture.assert_load_rejected("open-contract.json", open_contract.as_bytes());
}

#[test]
fn verifier_rejects_drift_in_every_distinct_role_pin() {
    let fixture = StaticFixture::new();

    let mut record = fixture.record();
    record.v1_config.canonical_sha256 = "a1".repeat(32);
    fixture.assert_verification_rejected("drift-config.json", &record);

    let mut record = fixture.record();
    record.v1_authorization.canonical_length += 1;
    fixture.assert_verification_rejected("drift-v1-auth.json", &record);

    let mut record = fixture.record();
    record.online_policy_v2.fingerprint = "a2".repeat(32);
    fixture.assert_verification_rejected("drift-policy.json", &record);

    let mut record = fixture.record();
    record
        .online_authorization_v2
        .authorization_id
        .push_str("-drift");
    fixture.assert_verification_rejected("drift-v2-auth.json", &record);

    let mut record = fixture.record();
    record.reviewed_production_destination_v1.canonical_sha256 = "a3".repeat(32);
    fixture.assert_verification_rejected("drift-destination.json", &record);

    let mut record = fixture.record();
    record
        .reviewed_fresh_credential_slot_locator_v1
        .locator_id
        .push_str("-drift");
    fixture.assert_verification_rejected("drift-locator.json", &record);

    let mut record = fixture.record();
    record.fresh_credential_delivery_binding_v1.canonical_length += 1;
    fixture.assert_verification_rejected("drift-delivery.json", &record);

    let mut record = fixture.record();
    record.reviewed_signer_proxy_account_identity_v1.fingerprint = "a4".repeat(32);
    fixture.assert_verification_rejected("drift-identity.json", &record);

    let mut record = fixture.record();
    record
        .reviewed_remote_credential_proof_policy_v1
        .canonical_sha256 = "a5".repeat(32);
    fixture.assert_verification_rejected("drift-remote-policy.json", &record);
}

#[test]
fn static_time_envelope_is_exactly_the_nested_v2_window() {
    let fixture = StaticFixture::new();

    let mut record = fixture.record();
    record.reviewed_at_utc = "2026-08-09T11:58:59Z".into();
    fixture.assert_verification_rejected("review-before-upstream.json", &record);

    let mut record = fixture.record();
    record.not_before_utc = "2026-08-09T12:00:01Z".into();
    fixture.assert_verification_rejected("nbf-drift.json", &record);

    let mut record = fixture.record();
    record.expires_at_utc = "2026-08-09T12:14:59Z".into();
    fixture.assert_verification_rejected("expiry-drift.json", &record);

    let mut record = fixture.record();
    record.cleanup_not_after_utc = "2026-08-09T12:19:59Z".into();
    fixture.assert_verification_rejected("cleanup-drift.json", &record);

    let mut record = fixture.record();
    record.reviewed_at_utc = "2026-08-09T12:00:00+00:00".into();
    fixture.assert_load_record_rejected("noncanonical-time.json", &record);

    let mut record = fixture.record();
    record.reviewed_at_utc = "2026-08-09T12:00:01Z".into();
    fixture.assert_load_record_rejected("review-after-not-before.json", &record);

    let mut record = fixture.record();
    record.not_before_utc = record.expires_at_utc.clone();
    fixture.assert_load_record_rejected("empty-active-window.json", &record);

    let mut record = fixture.record();
    record.expires_at_utc = "2026-08-09T12:20:01Z".into();
    fixture.assert_load_record_rejected("expiry-after-cleanup.json", &record);
}

#[test]
fn verifier_revalidates_canonical_v1_authorization_and_common_v1_v2_tuple() {
    let fixture = StaticFixture::new();

    let mut malformed_value = v1_authorization(&fixture.base.config);
    malformed_value.approval.only_named_phase = false;
    let malformed_path = write_json(
        &fixture.base.root,
        "canonical-but-malformed-v1-authorization.json",
        &malformed_value,
    );
    let malformed = load_canonical_authorization(&malformed_path).unwrap();
    let malformed_record = fixture.record_for_v1_authorization(&malformed);
    let malformed_static = fixture.load_record("malformed-v1-static.json", &malformed_record);
    assert!(
        verify_reviewed_static_online_authorization_v3(
            &fixture.context_with_v1_authorization(&malformed),
            &malformed_static,
        )
        .is_err()
    );

    let mut mismatched_value = v1_authorization(&fixture.base.config);
    mismatched_value.host.egress_identity = "9.9.9.9".into();
    let mismatched_path = write_json(
        &fixture.base.root,
        "canonical-v1-with-mismatched-common-tuple.json",
        &mismatched_value,
    );
    let mismatched = load_canonical_authorization(&mismatched_path).unwrap();
    let mismatched_record = fixture.record_for_v1_authorization(&mismatched);
    let mismatched_static = fixture.load_record("mismatched-v1-static.json", &mismatched_record);
    assert!(
        verify_reviewed_static_online_authorization_v3(
            &fixture.context_with_v1_authorization(&mismatched),
            &mismatched_static,
        )
        .is_err()
    );

    let mut shorter_value = v1_authorization(&fixture.base.config);
    shorter_value.expires_at_utc = "2026-08-09T12:14:59Z".into();
    let shorter_path = write_json(
        &fixture.base.root,
        "canonical-v1-with-shorter-window.json",
        &shorter_value,
    );
    let shorter = load_canonical_authorization(&shorter_path).unwrap();
    let shorter_record = fixture.record_for_v1_authorization(&shorter);
    let shorter_static = fixture.load_record("shorter-v1-static.json", &shorter_record);
    assert!(
        verify_reviewed_static_online_authorization_v3(
            &fixture.context_with_v1_authorization(&shorter),
            &shorter_static,
        )
        .is_err()
    );
}

#[test]
fn protected_loader_rejects_world_readable_symlink_and_hardlink() {
    use std::os::unix::fs::symlink;

    let fixture = StaticFixture::new();
    let bytes = serde_json::to_vec(&fixture.record()).unwrap();

    let world_readable = fixture.base.root.join("world-readable.json");
    write_0600(&world_readable, &bytes);
    fs::set_permissions(&world_readable, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(load_canonical_reviewed_static_online_authorization_v3(&world_readable).is_err());

    let target = fixture.base.root.join("link-target.json");
    write_0600(&target, &bytes);
    let symbolic = fixture.base.root.join("symbolic.json");
    symlink(&target, &symbolic).unwrap();
    assert!(load_canonical_reviewed_static_online_authorization_v3(&symbolic).is_err());

    let hard = fixture.base.root.join("hard.json");
    fs::hard_link(&target, &hard).unwrap();
    assert!(load_canonical_reviewed_static_online_authorization_v3(&hard).is_err());
}

#[test]
fn canonical_schema_has_no_designated_secret_fields_and_matches_golden() {
    let fixture = StaticFixture::new();
    let record = fixture.record();
    let bytes = serde_json::to_vec(&record).unwrap();
    let canonical = fixture.load_record("golden.json", &record);

    assert_eq!(bytes, GOLDEN_CANONICAL_JSON.as_bytes());
    assert_eq!(canonical.canonical_length(), GOLDEN_CANONICAL_LENGTH);
    assert_eq!(canonical.canonical_sha256(), GOLDEN_CANONICAL_SHA256);
    assert_eq!(canonical.fingerprint(), GOLDEN_FINGERPRINT);

    let text = String::from_utf8(bytes).unwrap();
    for forbidden in [
        "private_key",
        "l2_secret",
        "api_key",
        "passphrase",
        "signature_bytes",
        "lease_token",
        "provider_public_key",
        "process_id_value",
        "thread_id_value",
        "pointer_value",
        "runtime_nonce",
        "prepared_file",
        "consumption_claim",
    ] {
        assert!(
            !text.contains(forbidden),
            "schema gained designated forbidden field {forbidden}"
        );
    }
}

struct StaticFixture {
    base: BaseFixture,
    v1_authorization: CanonicalAuthorization,
    remote_policy: CanonicalReviewedRemoteCredentialProofPolicyV1,
}

impl StaticFixture {
    fn new() -> Self {
        let base = BaseFixture::new();
        let v1_path = write_json(
            &base.root,
            "authorization-v1.json",
            &v1_authorization(&base.config),
        );
        let v1_authorization = load_canonical_authorization(&v1_path).unwrap();
        let remote_path = write_json(
            &base.root,
            "reviewed-remote-policy-v1.json",
            &reviewed_remote_policy(&base),
        );
        let remote_policy =
            load_canonical_reviewed_remote_credential_proof_policy_v1(&remote_path).unwrap();
        Self {
            base,
            v1_authorization,
            remote_policy,
        }
    }

    fn context(&self) -> ReviewedStaticOnlineAuthorizationContextV3<'_> {
        self.context_with_v1_authorization(&self.v1_authorization)
    }

    fn context_with_v1_authorization<'a>(
        &'a self,
        v1_authorization: &'a CanonicalAuthorization,
    ) -> ReviewedStaticOnlineAuthorizationContextV3<'a> {
        ReviewedStaticOnlineAuthorizationContextV3 {
            v1_config: &self.base.config,
            v1_authorization,
            online_policy_v2: &self.base.online_policy,
            online_authorization_v2: &self.base.online_authorization,
            reviewed_production_destination_v1: &self.base.reviewed_destination,
            reviewed_fresh_credential_slot_locator_v1: &self.base.reviewed_fresh_credential_locator,
            fresh_credential_delivery_binding_v1: &self.base.fresh_credential_delivery,
            reviewed_signer_proxy_account_identity_v1: &self.base.reviewed_signer_proxy_identity,
            reviewed_remote_credential_proof_policy_v1: &self.remote_policy,
        }
    }

    fn record(&self) -> ReviewedStaticOnlineAuthorizationV3 {
        static_record(self)
    }

    fn record_for_v1_authorization(
        &self,
        v1_authorization: &CanonicalAuthorization,
    ) -> ReviewedStaticOnlineAuthorizationV3 {
        let mut record = self.record();
        let bytes = serde_json::to_vec(v1_authorization.value()).unwrap();
        record.v1_authorization.authorization_id =
            v1_authorization.value().authorization_id.clone();
        record.v1_authorization.canonical_sha256 = raw_sha256(&bytes);
        record.v1_authorization.canonical_length = bytes.len() as u64;
        record.v1_authorization.fingerprint = v1_authorization.fingerprint().into();
        record
    }

    fn load_record(
        &self,
        name: &str,
        record: &ReviewedStaticOnlineAuthorizationV3,
    ) -> reap_pm_controlled_trial::CanonicalReviewedStaticOnlineAuthorizationV3 {
        let path = write_json(&self.base.root, name, record);
        load_canonical_reviewed_static_online_authorization_v3(&path).unwrap()
    }

    fn assert_load_rejected(&self, name: &str, bytes: &[u8]) {
        let path = self.base.root.join(name);
        write_0600(&path, bytes);
        assert!(load_canonical_reviewed_static_online_authorization_v3(&path).is_err());
    }

    fn assert_load_record_rejected(
        &self,
        name: &str,
        record: &ReviewedStaticOnlineAuthorizationV3,
    ) {
        self.assert_load_rejected(name, &serde_json::to_vec(record).unwrap());
    }

    fn assert_verification_rejected(
        &self,
        name: &str,
        record: &ReviewedStaticOnlineAuthorizationV3,
    ) {
        let canonical = self.load_record(name, record);
        assert!(
            verify_reviewed_static_online_authorization_v3(&self.context(), &canonical).is_err()
        );
    }
}

fn static_record(fixture: &StaticFixture) -> ReviewedStaticOnlineAuthorizationV3 {
    let base = &fixture.base;
    let v1_bytes = serde_json::to_vec(fixture.v1_authorization.value()).unwrap();
    ReviewedStaticOnlineAuthorizationV3 {
        schema_version: REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3_SCHEMA_VERSION,
        static_authorization_id: "pm-t2-static-online-authorization-v3".into(),
        reviewer_label: "operator-reviewer".into(),
        reviewed_at_utc: "2026-08-09T12:00:00Z".into(),
        not_before_utc: "2026-08-09T12:00:00Z".into(),
        expires_at_utc: "2026-08-09T12:15:00Z".into(),
        cleanup_not_after_utc: "2026-08-09T12:20:00Z".into(),
        v1_config: ReviewedStaticOnlineAuthorizationConfigPinsV3 {
            schema_version: 1,
            canonical_sha256: base.config.canonical_sha256().into(),
            canonical_length: base.config.canonical_length(),
            fingerprint: base.config.fingerprint().into(),
            plan_fingerprint: base.config.plan_fingerprint().into(),
        },
        v1_authorization: ReviewedStaticOnlineAuthorizationV1AuthorizationPinsV3 {
            schema_version: 1,
            authorization_id: fixture.v1_authorization.value().authorization_id.clone(),
            canonical_sha256: raw_sha256(&v1_bytes),
            canonical_length: v1_bytes.len() as u64,
            fingerprint: fixture.v1_authorization.fingerprint().into(),
        },
        online_policy_v2: ReviewedStaticOnlineAuthorizationOnlinePolicyPinsV3 {
            schema_version: 2,
            policy_id: base.online_policy.value().policy_id.clone(),
            canonical_sha256: base.online_policy.canonical_sha256().into(),
            canonical_length: base.online_policy.canonical_length(),
            fingerprint: base.online_policy.fingerprint().into(),
        },
        online_authorization_v2: ReviewedStaticOnlineAuthorizationOnlineAuthorizationPinsV3 {
            schema_version: 2,
            authorization_id: base.online_authorization.value().authorization_id.clone(),
            canonical_sha256: base.online_authorization.canonical_sha256().into(),
            canonical_length: base.online_authorization.canonical_length(),
            fingerprint: base.online_authorization.fingerprint().into(),
        },
        reviewed_production_destination_v1: ReviewedStaticOnlineAuthorizationDestinationPinsV3 {
            schema_version: 1,
            profile_id: base.reviewed_destination.value().profile_id.clone(),
            canonical_sha256: base.reviewed_destination.canonical_sha256().into(),
            canonical_length: base.reviewed_destination.canonical_length(),
            fingerprint: base.reviewed_destination.fingerprint().into(),
        },
        reviewed_fresh_credential_slot_locator_v1: ReviewedStaticOnlineAuthorizationLocatorPinsV3 {
            schema_version: 1,
            locator_id: "pm-t2-reviewed-fresh-credential-slot-locator-v1".into(),
            canonical_sha256: base.reviewed_fresh_credential_locator.canonical_sha256().into(),
            canonical_length: base.reviewed_fresh_credential_locator.canonical_length(),
            fingerprint: base.reviewed_fresh_credential_locator.fingerprint().into(),
        },
        fresh_credential_delivery_binding_v1: ReviewedStaticOnlineAuthorizationDeliveryPinsV3 {
            schema_version: 1,
            binding_id: "pm-t2-fresh-credential-delivery-binding-v1".into(),
            canonical_sha256: base.fresh_credential_delivery.canonical_sha256().into(),
            canonical_length: base.fresh_credential_delivery.canonical_length(),
            fingerprint: base.fresh_credential_delivery.fingerprint().into(),
        },
        reviewed_signer_proxy_account_identity_v1:
            ReviewedStaticOnlineAuthorizationAccountIdentityPinsV3 {
                schema_version: 1,
                identity_id: "pm-t2-account-v1".into(),
                canonical_sha256: base.reviewed_signer_proxy_identity.canonical_sha256().into(),
                canonical_length: base.reviewed_signer_proxy_identity.canonical_length(),
                fingerprint: base.reviewed_signer_proxy_identity.fingerprint().into(),
            },
        reviewed_remote_credential_proof_policy_v1:
            ReviewedStaticOnlineAuthorizationRemoteProofPolicyPinsV3 {
                schema_version: 1,
                canonical_sha256: fixture.remote_policy.canonical_sha256().into(),
                canonical_length: fixture.remote_policy.canonical_length(),
                fingerprint: fixture.remote_policy.fingerprint().into(),
            },
        unavailable_positive_contracts:
            ReviewedStaticOnlineAuthorizationUnavailablePositiveContractsV3 {
                credential_provider_trust_root_status:
                    ReviewedCredentialProviderTrustRootStatusV3::UnavailableInFrozenSourcesV1,
                authenticated_credential_delivery_lease_protocol_status:
                    ReviewedCredentialDeliveryLeaseProtocolStatusV3::UnavailableInFrozenSourcesV1,
                authoritative_remote_credential_acceptance_contract_status:
                    ReviewedRemoteCredentialAcceptanceContractStatusV3::UnavailableInFrozenSourcesV1,
                authoritative_signer_proxy_control_contract_status:
                    ReviewedSignerProxyControlContractStatusV3::UnattestedReviewedLabelsOnlyV1,
            },
        selected_actor_profile: ReviewedStaticOnlineAuthorizationSelectedActorProfileV3 {
            profile:
                ReviewedSelectedActorProfileV3::ShutdownOnlySelectedEgressCurrentThreadLocalSetV1,
            generation_scheme:
                ReviewedActorGenerationSchemeV3::ProcessIdThreadIdAndRcPointerIdentityV1,
            generation_allocation:
                ReviewedActorGenerationAllocationV3::BeforeRuntimeLocalSetAndAllActorLocalConstructionV1,
            readiness:
                ReviewedActorReadinessV3::TaskEntryAckAfterGenerationMembershipRevalidationV1,
            command_set: ReviewedActorCommandSetV3::ShutdownOnlyV1,
            terminal_requirement: ReviewedActorTerminalRequirementV3::ShutdownRequestedNoAbortTaskJoinedCleanCredentialsDroppedStagedFilesRemovedGenerationRevalidatedV1,
            runtime_attempt_commitment_location:
                ReviewedActorRuntimeAttemptCommitmentLocationV3::FuturePreparedOnlyAbsentFromStaticV3,
        },
        frozen_v1_v2_consumption_lineage:
            ReviewedStaticOnlineAuthorizationFrozenConsumptionLineageV3 {
                prepared_creation_order:
                    ReviewedPreparedCreationOrderV3::OnlineV2PreparedThenV1PreparedV1,
                basis_and_burn_order: ReviewedBasisAndBurnOrderV3::BasisAfterExistingPlaceAndBothConsumptionPreparedThenV2ClaimThenV1ClaimThenV1A3ThenV2A3ConjunctV1,
                crash_recovery:
                    ReviewedCrashRecoveryProfileV3::ExistingV1LifecycleOnlyNoPlacementResumeV1,
                static_v3_runtime_state:
                    ReviewedStaticV3RuntimeStateV3::NoPreparedClaimBurnRecoveryOrDispatchV1,
            },
    }
}

fn v1_authorization(config: &reap_pm_controlled_trial::CanonicalTrialConfig) -> TrialAuthorization {
    TrialAuthorization {
        schema_version: 1,
        authorization_id: "pm-t2-a-authorization-1".into(),
        phase: TrialPhase::APlaceCancel,
        issuing_reviewer: "operator-reviewer".into(),
        reviewed_at_utc: "2026-08-09T11:59:00Z".into(),
        purpose: "one_exact_pm_t2_phase_a_passive_place_cancel_attempt".into(),
        not_before_utc: "2026-08-09T12:00:00Z".into(),
        expires_at_utc: "2026-08-09T12:15:00Z".into(),
        cleanup_not_after_utc: "2026-08-09T12:20:00Z".into(),
        build: AuthorizationBuildBinding {
            repository_commit: "66".repeat(20),
            clean_tree_attested: true,
            cargo_lock_sha256: "77".repeat(32),
            release_binary_sha256: "88".repeat(32),
            release_binary_length: 1_000_000,
            canonical_config_sha256: config.canonical_sha256().into(),
            canonical_config_length: config.canonical_length(),
            canonical_config_fingerprint: config.fingerprint().into(),
        },
        host: AuthorizationHostBinding {
            host_identity: "trial-host-1".into(),
            boot_identity: "01234567-89ab-cdef-0123-456789abcdef".into(),
            runtime_user: "reap-trial".into(),
            egress_identity: "8.8.8.8".into(),
        },
        trial: config.value().clone(),
        trial_plan_fingerprint: config.plan_fingerprint().into(),
        approval: AuthorizationApproval {
            only_named_phase: true,
            exactly_one_attempt: true,
            one_possible_fill_is_within_loss_cap: true,
            post_only_does_not_mean_no_fill: true,
            no_concurrent_proxy_trading_attested: true,
            independent_cleanup_method_reviewed: true,
        },
    }
}

fn raw_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn reviewed_remote_policy(fixture: &BaseFixture) -> ReviewedRemoteCredentialProofPolicyV1 {
    ReviewedRemoteCredentialProofPolicyV1 {
        schema_version: REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_SCHEMA_VERSION,
        policy_id: "pm-t2-reviewed-remote-credential-proof-policy-v1".into(),
        reviewer_label: "operator-reviewer".into(),
        reviewed_at_utc: "2026-08-09T12:00:00Z".into(),
        valid_not_before_utc: "2026-08-09T12:00:00Z".into(),
        valid_not_after_utc: "2026-08-09T12:20:00Z".into(),
        v1_config: config_pins(&fixture.config),
        online_policy: OnlinePolicyPinsV2 {
            canonical_sha256: fixture.online_policy.canonical_sha256().into(),
            canonical_length: fixture.online_policy.canonical_length(),
            fingerprint: fixture.online_policy.fingerprint().into(),
        },
        online_authorization: ReviewedOnlineAuthorizationPinsV1 {
            authorization_id: fixture
                .online_authorization
                .value()
                .authorization_id
                .clone(),
            canonical_sha256: fixture.online_authorization.canonical_sha256().into(),
            canonical_length: fixture.online_authorization.canonical_length(),
            fingerprint: fixture.online_authorization.fingerprint().into(),
        },
        reviewed_destination: ReviewedRemoteCredentialProofDestinationPinsV1 {
            schema_version: 1,
            profile_id: fixture.reviewed_destination.value().profile_id.clone(),
            canonical_sha256: fixture.reviewed_destination.canonical_sha256().into(),
            canonical_length: fixture.reviewed_destination.canonical_length(),
            fingerprint: fixture.reviewed_destination.fingerprint().into(),
        },
        reviewed_fresh_credential_locator: ReviewedRemoteCredentialProofLocatorPinsV1 {
            schema_version: REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_SCHEMA_VERSION,
            locator_id: "pm-t2-reviewed-fresh-credential-slot-locator-v1".into(),
            canonical_sha256: fixture
                .reviewed_fresh_credential_locator
                .canonical_sha256()
                .into(),
            canonical_length: fixture
                .reviewed_fresh_credential_locator
                .canonical_length(),
            fingerprint: fixture
                .reviewed_fresh_credential_locator
                .fingerprint()
                .into(),
        },
        fresh_credential_delivery: ReviewedRemoteCredentialProofDeliveryPinsV1 {
            schema_version: 1,
            binding_id: "pm-t2-fresh-credential-delivery-binding-v1".into(),
            canonical_sha256: fixture.fresh_credential_delivery.canonical_sha256().into(),
            canonical_length: fixture.fresh_credential_delivery.canonical_length(),
            fingerprint: fixture.fresh_credential_delivery.fingerprint().into(),
        },
        reviewed_signer_proxy_identity: ReviewedRemoteCredentialProofAccountIdentityPinsV1 {
            schema_version: 1,
            identity_id: "pm-t2-account-v1".into(),
            canonical_sha256: fixture
                .reviewed_signer_proxy_identity
                .canonical_sha256()
                .into(),
            canonical_length: fixture
                .reviewed_signer_proxy_identity
                .canonical_length(),
            fingerprint: fixture
                .reviewed_signer_proxy_identity
                .fingerprint()
                .into(),
        },
        official_sources: ReviewedRemoteCredentialProofOfficialSourcesV1 {
            manifest: ReviewedOfficialSourceManifestPinsV1 {
                schema_family: PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_FAMILY_V1.into(),
                schema_version: PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_VERSION_V1,
                retrieved_at_utc: PM_T2_OFFICIAL_SOURCE_MANIFEST_RETRIEVED_AT_UTC_V1.into(),
                byte_length: PM_T2_OFFICIAL_SOURCE_MANIFEST_BYTE_LENGTH_V1,
                sha256: PM_T2_OFFICIAL_SOURCE_MANIFEST_SHA256_V1.into(),
            },
            api_authentication: ReviewedRemoteCredentialProofSourceEntryPinsV1 {
                id: "api_authentication".into(),
                requested_url: "https://docs.polymarket.com/getting-started/api.md".into(),
                final_url: "https://docs.polymarket.com/getting-started/api.md".into(),
                retrieved_at_utc: PM_T2_OFFICIAL_SOURCE_MANIFEST_RETRIEVED_AT_UTC_V1.into(),
                content_type: "text/markdown; charset=utf-8".into(),
                byte_length: 9_391,
                sha256: "6c397c66109852220b3f5d8033ea274061b3fc44b426edc9faa60673ecbef8fc"
                    .into(),
            },
            manage_orders: ReviewedRemoteCredentialProofSourceEntryPinsV1 {
                id: "manage_orders".into(),
                requested_url: "https://docs.polymarket.com/trading/manage-orders.md".into(),
                final_url: "https://docs.polymarket.com/trading/manage-orders.md".into(),
                retrieved_at_utc: PM_T2_OFFICIAL_SOURCE_MANIFEST_RETRIEVED_AT_UTC_V1.into(),
                content_type: "text/markdown; charset=utf-8".into(),
                byte_length: 52_401,
                sha256: "e4a0238db31d5137b4d0da0d4333b1fb90be8f7c7b47d92968edfd993c8c4482"
                    .into(),
            },
        },
        authentication_acceptance_contract_status:
            ReviewedRemoteCredentialAuthenticationAcceptanceContractStatusV1::UnavailableInFrozenSourcesV1,
        protocol: ReviewedRemoteCredentialProofProtocolPolicyV1 {
            endpoint: ReviewedRemoteCredentialProofEndpointPolicyV1 {
                scheme: "https".into(),
                dns_name: "clob.polymarket.com".into(),
                tls_server_name: "clob.polymarket.com".into(),
                http_host: "clob.polymarket.com".into(),
                tcp_port: 443,
                selected_peer_ip: "1.1.1.1".into(),
                network_namespace_device: 4,
                network_namespace_inode: 4_026_531_999,
                interface_name: "wg0".into(),
                interface_index: 7,
                local_egress_ip: "10.0.0.2".into(),
                dedicated_tunnel_or_gateway_profile_reference:
                    "reviewed-egress:dedicated-wg0-v2".into(),
                dedicated_tunnel_or_gateway_profile_sha256: "ab".repeat(32),
            },
            request: ReviewedRemoteCredentialProofRequestPolicyV1 {
                method: "GET".into(),
                path: "/auth/ban-status/closed-only".into(),
                query: "absent".into(),
                body: "absent".into(),
                content_type: "absent".into(),
                accept: "application/json".into(),
                accept_encoding: "identity".into(),
                sensitive_header_names: ReviewedRemoteCredentialProofSensitiveHeaderNamesV1 {
                    poly_address: "POLY_ADDRESS".into(),
                    poly_signature: "POLY_SIGNATURE".into(),
                    poly_timestamp: "POLY_TIMESTAMP".into(),
                    poly_api_key: "POLY_API_KEY".into(),
                    poly_passphrase: "POLY_PASSPHRASE".into(),
                },
                hmac_preimage: ReviewedRemoteCredentialProofHmacPreimageGrammarV1 {
                    hmac_algorithm: "hmac_sha256".into(),
                    hmac_key_source:
                        "same_loaded_l2_credential_holder_decoded_url_safe_base64_secret_bytes"
                            .into(),
                    l2_secret_input_encoding:
                        "rfc4648_url_safe_base64_with_padding_canonical".into(),
                    maximum_l2_secret_encoded_bytes: 172,
                    minimum_hmac_key_bytes: 1,
                    maximum_hmac_key_bytes: 128,
                    ordered_variant: ReviewedRemoteCredentialProofHmacPreimageOrderedVariantV1::DecimalTimestampThenUppercaseGetThenExactPathNoSeparatorsV1,
                    timestamp_component: "fresh_clob_server_unix_seconds_decimal_ascii".into(),
                    timestamp_decimal_digits: 10,
                    minimum_timestamp_unix_seconds: 1_000_000_000,
                    maximum_timestamp_unix_seconds: 9_999_999_999,
                    method_component: "GET".into(),
                    path_component: "/auth/ban-status/closed-only".into(),
                    separator: "none".into(),
                    query_component: "absent".into(),
                    body_component: "absent".into(),
                    poly_address_component: "excluded_from_preimage_header_only".into(),
                    poly_api_key_component: "excluded_from_preimage_header_only".into(),
                    poly_passphrase_component: "excluded_from_preimage_header_only".into(),
                    l2_secret_role: "hmac_key_only_not_preimage".into(),
                    signature_encoding: "rfc4648_url_safe_base64_with_padding".into(),
                    signature_decoded_length: 32,
                    signature_encoded_length: 44,
                    signature_terminal_padding: "=".into(),
                },
                poly_address_source: "canonical_config_signer_eip55".into(),
                poly_timestamp_source: "same_canonical_l2_timestamp_as_hmac_preimage".into(),
                poly_signature_source: "exact_hmac_output".into(),
                poly_api_key_source: "same_loaded_l2_credential_holder".into(),
                poly_passphrase_source: "same_loaded_l2_credential_holder".into(),
            },
            freshness: ReviewedRemoteCredentialProofFreshnessPolicyV1 {
                server_time_path: "/time".into(),
                server_time_sample_required: true,
                server_time_sample_must_precede_authenticated_dispatch: true,
                maximum_server_time_sample_to_dispatch_age_ms: 5_000,
                maximum_proof_observation_age_ms: 5_000,
            },
            dispatch: ReviewedRemoteCredentialProofDispatchPolicyV1 {
                maximum_authenticated_dispatch_count: 1,
                connect_timeout_ms: 3_000,
                request_timeout_ms: 5_000,
                redirects_allowed: false,
                retries_allowed: false,
                forward_proxy_allowed: false,
                destination_fallback_allowed: false,
                connected_peer_check_before_status_and_body_required: true,
                ambiguous_outcome_requires_durable_burn: true,
            },
            response: ReviewedRemoteCredentialProofResponsePolicyV1 {
                required_status_code: 200,
                content_type_header_required: true,
                required_content_type_essence: "application/json".into(),
                allowed_content_type_charset:
                    "none_or_single_charset_utf8_ascii_case_insensitive".into(),
                allowed_content_encoding:
                    "absent_or_single_identity_ascii_case_insensitive".into(),
                maximum_body_bytes: 64,
                required_json_object_field_count: 1,
                required_json_field_name: "closed_only".into(),
                required_json_field_type: "boolean".into(),
                allowed_json_boolean_values: "true_or_false_shape_only".into(),
                closed_only_false_semantics: "placement_candidate_evaluated_separately".into(),
                closed_only_true_semantics: "hard_block".into(),
                authentication_semantics:
                    "neither_value_proves_authentication_acceptance_or_failure".into(),
            },
        },
    }
}

struct BaseFixture {
    _directory: TempDir,
    root: PathBuf,
    config: reap_pm_controlled_trial::CanonicalTrialConfig,
    online_policy: reap_pm_controlled_trial::CanonicalOnlinePolicyV2,
    online_authorization: reap_pm_controlled_trial::CanonicalOnlineAuthorizationV2,
    reviewed_destination: reap_pm_controlled_trial::CanonicalReviewedProductionDestinationProfileV1,
    reviewed_fresh_credential_locator:
        reap_pm_controlled_trial::CanonicalReviewedFreshCredentialSlotLocatorV1,
    fresh_credential_delivery: reap_pm_controlled_trial::CanonicalFreshCredentialDeliveryBindingV1,
    reviewed_signer_proxy_identity:
        reap_pm_controlled_trial::CanonicalReviewedSignerProxyAccountIdentityV1,
}

impl BaseFixture {
    fn new() -> Self {
        let directory = protected_dir();
        let root = directory.path().to_owned();
        let config_path = root.join("canonical-config.json");
        write_0600(&config_path, &serde_json::to_vec(&trial_config()).unwrap());
        let config = load_canonical_trial_config(&config_path).unwrap();

        let policy_path = write_json(&root, "online-policy-v2.json", &online_policy(&config));
        let online_policy = load_canonical_online_policy_v2(&policy_path).unwrap();
        let authorization_path = write_json(
            &root,
            "online-authorization-v2.json",
            &online_authorization(&config, &online_policy),
        );
        let online_authorization =
            load_canonical_online_authorization_v2(&authorization_path).unwrap();

        let destination_path = write_json(
            &root,
            "reviewed-destination-v1.json",
            &destination_profile(&config, &online_policy, &online_authorization),
        );
        let reviewed_destination =
            load_canonical_reviewed_production_destination_profile_v1(&destination_path).unwrap();

        let locator_path = write_json(
            &root,
            "reviewed-locator-v1.json",
            &credential_locator(&config, &online_policy, &online_authorization),
        );
        let reviewed_fresh_credential_locator =
            load_canonical_reviewed_fresh_credential_slot_locator_v1(&locator_path).unwrap();
        let delivery_path = write_json(
            &root,
            "delivery-binding-v1.json",
            &delivery_binding(&reviewed_fresh_credential_locator),
        );
        let fresh_credential_delivery =
            load_canonical_fresh_credential_delivery_binding_v1(&delivery_path).unwrap();

        let identity_path = write_json(
            &root,
            "reviewed-account-identity-v1.json",
            &reviewed_account_identity(&config, &online_policy, &online_authorization),
        );
        let reviewed_signer_proxy_identity =
            load_canonical_reviewed_signer_proxy_account_identity_v1(&identity_path).unwrap();

        Self {
            _directory: directory,
            root,
            config,
            online_policy,
            online_authorization,
            reviewed_destination,
            reviewed_fresh_credential_locator,
            fresh_credential_delivery,
            reviewed_signer_proxy_identity,
        }
    }
}

fn destination_profile(
    config: &reap_pm_controlled_trial::CanonicalTrialConfig,
    policy: &reap_pm_controlled_trial::CanonicalOnlinePolicyV2,
    authorization: &reap_pm_controlled_trial::CanonicalOnlineAuthorizationV2,
) -> ReviewedProductionDestinationProfileV1 {
    ReviewedProductionDestinationProfileV1 {
        schema_version: 1,
        profile_id: "pm-t2-reviewed-production-destinations-v1".into(),
        issuing_reviewer: "operator-reviewer".into(),
        reviewed_at_utc: "2026-08-09T12:00:00Z".into(),
        valid_not_before_utc: "2026-08-09T12:00:00Z".into(),
        valid_not_after_utc: "2026-08-09T12:20:00Z".into(),
        v1_config: config_pins(config),
        online_policy: policy_pins(policy),
        online_authorization: authorization_pins(authorization),
        dns_review: ReviewedDnsAnswerEvidenceV1 {
            source_kind: ReviewedDnsAnswerSourceV1::ReviewerCapturedFixedAnswers,
            resolved_at_utc: "2026-08-09T11:55:00Z".into(),
            review_reference: "reviewed-dns:capture:pm-t2-v1".into(),
            review_sha256: "de".repeat(32),
        },
        destinations: ReviewedProductionDestinationsV1 {
            geoblock_https: fixed_tls("polymarket.com", "8.8.8.8"),
            clob_https: fixed_tls("clob.polymarket.com", "1.1.1.1"),
            status_https: fixed_tls("status.polymarket.com", "9.9.9.9"),
            data_api_https: fixed_tls("data-api.polymarket.com", "8.8.4.4"),
            polygon_rpc_https: fixed_tls("polygon.drpc.org", "1.0.0.1"),
            clob_websocket_wss: ReviewedFixedWebSocketDestinationV1 {
                dns_name: "ws-subscriptions-clob.polymarket.com".into(),
                tcp_port: 443,
                tls_server_name: "ws-subscriptions-clob.polymarket.com".into(),
                http_host: "ws-subscriptions-clob.polymarket.com".into(),
                peer_ip: "9.9.9.10".into(),
                public_path: "/ws/market".into(),
                user_path: "/ws/user".into(),
            },
        },
    }
}

fn fixed_tls(host: &str, peer_ip: &str) -> ReviewedFixedTlsDestinationV1 {
    ReviewedFixedTlsDestinationV1 {
        dns_name: host.into(),
        tcp_port: 443,
        tls_server_name: host.into(),
        http_host: host.into(),
        peer_ip: peer_ip.into(),
    }
}

fn credential_locator(
    config: &reap_pm_controlled_trial::CanonicalTrialConfig,
    policy: &reap_pm_controlled_trial::CanonicalOnlinePolicyV2,
    authorization: &reap_pm_controlled_trial::CanonicalOnlineAuthorizationV2,
) -> ReviewedFreshCredentialSlotLocatorV1 {
    ReviewedFreshCredentialSlotLocatorV1 {
        schema_version: REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_SCHEMA_VERSION,
        locator_id: "pm-t2-reviewed-fresh-credential-slot-locator-v1".into(),
        issuing_reviewer: "operator-reviewer".into(),
        reviewed_at_utc: "2026-08-09T12:00:00Z".into(),
        valid_not_before_utc: "2026-08-09T12:00:00Z".into(),
        valid_not_after_utc: "2026-08-09T12:20:00Z".into(),
        v1_config: config_pins(config),
        online_policy: policy_pins(policy),
        online_authorization: authorization_pins(authorization),
        credential_slot_id: config.value().credential_slot.slot_id.clone(),
        credential_slot_nonsecret_fingerprint_sha256: config
            .value()
            .credential_slot
            .nonsecret_fingerprint_sha256
            .clone(),
        protected_fresh_credential_directory: REVIEWED_CREDENTIAL_DIRECTORY.into(),
        files: ReviewedFreshCredentialFilesV1 {
            private_key_entry: PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1.into(),
            api_key_entry: PM_T2_FRESH_API_KEY_ENTRY_V1.into(),
            l2_secret_entry: PM_T2_FRESH_L2_SECRET_ENTRY_V1.into(),
            passphrase_entry: PM_T2_FRESH_PASSPHRASE_ENTRY_V1.into(),
        },
    }
}

fn delivery_binding(
    locator: &reap_pm_controlled_trial::CanonicalReviewedFreshCredentialSlotLocatorV1,
) -> FreshCredentialDeliveryBindingV1 {
    FreshCredentialDeliveryBindingV1 {
        schema_version: 1,
        binding_id: "pm-t2-fresh-credential-delivery-binding-v1".into(),
        unattested_delivery_recorded_at_utc: "2026-08-09T12:00:00Z".into(),
        unattested_valid_not_before_utc: "2026-08-09T12:00:00Z".into(),
        unattested_valid_not_after_utc: "2026-08-09T12:20:00Z".into(),
        reviewed_fresh_credential_slot_locator: FreshCredentialSlotLocatorPinsV1 {
            locator_id: "pm-t2-reviewed-fresh-credential-slot-locator-v1".into(),
            canonical_sha256: locator.canonical_sha256().into(),
            canonical_length: locator.canonical_length(),
            fingerprint: locator.fingerprint().into(),
        },
        unattested_provider_generation: UnattestedFreshCredentialProviderGenerationV1 {
            provider_id: "provider-a".into(),
            provider_key_id: "provider-key-a".into(),
            rotation_namespace_id: "rotation-a".into(),
            delivery_id: "delivery-a".into(),
            rotation_generation: 7,
        },
        unattested_linux_objects: FreshCredentialLinuxObjectSetV1 {
            directory: FreshCredentialLinuxDirectoryIdentityV1 {
                filesystem_device: 41,
                inode: 424_242,
                owner_uid: 1_000,
                permission_mode: 0o700,
                modified_seconds: 1_754_741_999,
                modified_nanoseconds: 100,
                status_changed_seconds: 1_754_741_999,
                status_changed_nanoseconds: 200,
            },
            files: FreshCredentialLinuxFileIdentitiesV1 {
                private_key: linux_file(424_243),
                api_key: linux_file(424_244),
                l2_secret: linux_file(424_245),
                passphrase: linux_file(424_246),
            },
        },
    }
}

fn linux_file(inode: u64) -> FreshCredentialLinuxFileIdentityV1 {
    FreshCredentialLinuxFileIdentityV1 {
        filesystem_device: 41,
        inode,
        owner_uid: 1_000,
        permission_mode: 0o600,
        hard_link_count: 1,
        modified_seconds: 1_754_741_999,
        modified_nanoseconds: 300,
        status_changed_seconds: 1_754_741_999,
        status_changed_nanoseconds: 400,
    }
}

fn reviewed_account_identity(
    config: &reap_pm_controlled_trial::CanonicalTrialConfig,
    policy: &reap_pm_controlled_trial::CanonicalOnlinePolicyV2,
    authorization: &reap_pm_controlled_trial::CanonicalOnlineAuthorizationV2,
) -> ReviewedSignerProxyAccountIdentityV1 {
    ReviewedSignerProxyAccountIdentityV1 {
        schema_version: 1,
        identity_id: "pm-t2-account-v1".into(),
        reviewer_label: "operator-reviewer".into(),
        reviewed_at_utc: "2026-08-09T12:00:00Z".into(),
        valid_not_before_utc: "2026-08-09T12:00:00Z".into(),
        valid_not_after_utc: "2026-08-09T12:20:00Z".into(),
        v1_config: config_pins(config),
        online_policy: policy_pins(policy),
        online_authorization: authorization_pins(authorization),
        official_source_manifest: ReviewedOfficialSourceManifestPinsV1 {
            schema_family: PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_FAMILY_V1.into(),
            schema_version: PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_VERSION_V1,
            retrieved_at_utc: PM_T2_OFFICIAL_SOURCE_MANIFEST_RETRIEVED_AT_UTC_V1.into(),
            byte_length: PM_T2_OFFICIAL_SOURCE_MANIFEST_BYTE_LENGTH_V1,
            sha256: PM_T2_OFFICIAL_SOURCE_MANIFEST_SHA256_V1.into(),
        },
        evidence: UnattestedReviewedSignerProxyAccountEvidenceV1 {
            evidence_kind:
                ReviewedSignerProxyAccountEvidenceKindV1::UnattestedReviewedAccountSourceV1,
            evidence_id_label: "pm-t2-account-evidence-v1".into(),
            issuer_label: "account-custodian".into(),
            source_reference_label: config
                .value()
                .credential_slot
                .signer_to_proxy_evidence_reference
                .clone(),
            observed_at_utc: "2026-08-09T11:58:00Z".into(),
            payload_media_type_label: "application/json".into(),
            payload_byte_length: 1_024,
            payload_sha256: "cd".repeat(32),
            claimed_account: ReviewedSignerProxyClaimedAccountV1 {
                chain_id: config.value().account.chain_id,
                wallet_profile: config.value().account.wallet_profile.clone(),
                signature_type: config.value().account.signature_type,
                signer: config.value().account.signer.clone(),
                proxy_funder: config.value().account.funder.clone(),
            },
        },
    }
}

fn online_policy(config: &reap_pm_controlled_trial::CanonicalTrialConfig) -> OnlinePolicyV2 {
    OnlinePolicyV2 {
        schema_version: 2,
        policy_id: "pm-t2-online-policy-v2".into(),
        issuing_reviewer: "operator-reviewer".into(),
        reviewed_at_utc: "2026-08-09T11:59:00Z".into(),
        phase: TrialPhase::APlaceCancel,
        v1_config: config_pins(config),
        reviewed_market: ReviewedMarketEvidenceV2 {
            classification: ReviewedMarketClassificationV2::NonSports,
            review_reference: "reviewed-market:pm-t2-non-sports-v2".into(),
            review_sha256: "91".repeat(32),
        },
        operational_observations: OperationalObservationProfileV2 {
            reviewed_market_classification:
                ReviewedMarketObservationRequirementV2::RequireReviewedNonSports,
            reviewed_official_status_notice_history:
                ReviewedStatusHistoryObservationRequirementV2::RequireReviewedExactClobComponentHistory,
            fresh_official_status_announcements:
                reap_pm_controlled_trial::FreshStatusAnnouncementObservationRequirementV2::RequireFreshSummaryAndComponents,
            clob_ok_liveness_health:
                ClobLivenessHealthObservationRequirementV2::RequireFixedClobGetOkLivenessOnly,
            same_account_closed_only:
                SameAccountClosedOnlyObservationRequirementV2::RequireSignerAuthenticatedFalseForExactAccount,
        },
        reviewed_status_clob_component: ReviewedStatusClobComponentV2 {
            component_id: "clobapi1".into(),
            component_name: REVIEWED_CLOB_COMPONENT_NAME.into(),
        },
        maximum_observation_age_ms: 5_000,
        minimum_notice_history_quiet_interval_seconds: 86_400,
    }
}

fn online_authorization(
    config: &reap_pm_controlled_trial::CanonicalTrialConfig,
    policy: &reap_pm_controlled_trial::CanonicalOnlinePolicyV2,
) -> OnlineAuthorizationV2 {
    OnlineAuthorizationV2 {
        schema_version: 2,
        authorization_id: "pm-t2-online-authorization-v2".into(),
        issuing_reviewer: "operator-reviewer".into(),
        reviewed_at_utc: "2026-08-09T12:00:00Z".into(),
        phase: TrialPhase::APlaceCancel,
        purpose: OnlineAuthorizationPurposeV2::OneExactPhaseAPlaceCancelAttempt,
        not_before_utc: "2026-08-09T12:00:00Z".into(),
        expires_at_utc: "2026-08-09T12:15:00Z".into(),
        cleanup_not_after_utc: "2026-08-09T12:20:00Z".into(),
        v1_config: config_pins(config),
        online_policy: policy_pins(policy),
        build: OnlineAuthorizationBuildBindingV2 {
            repository_commit: "66".repeat(20),
            repository_state: ReviewedRepositoryStateV2::ExactCleanCommit,
            cargo_lock_sha256: "77".repeat(32),
            release_binary_sha256: "88".repeat(32),
            release_binary_length: 1_000_000,
        },
        host: OnlineAuthorizationHostBindingV2 {
            uts_nodename: "trial-host-1".into(),
            boot_id: "01234567-89ab-cdef-0123-456789abcdef".into(),
            nss_username: "reap-trial".into(),
            linux_euid: 1_000,
            egress: ReviewedLinuxEgressProfileV2 {
                network_namespace_device: 4,
                network_namespace_inode: 4_026_531_999,
                interface_name: "wg0".into(),
                interface_index: 7,
                local_source_ip: "10.0.0.2".into(),
                dedicated_tunnel_or_gateway_profile_reference:
                    "reviewed-egress:dedicated-wg0-v2".into(),
                dedicated_tunnel_or_gateway_profile_sha256: "ab".repeat(32),
                destination_independent_nat_assumption:
                    ReviewedDestinationIndependentNatV2::OnePublicIpForPolymarketComAndClobPolymarketCom,
                authorized_geoblock_reported_public_ip: "8.8.8.8".into(),
            },
        },
        status_notice_history: ReviewedStatusNoticeHistoryCutV2 {
            source_kind: ReviewedStatusNoticeHistorySourceV2::OfficialPolymarketStatusHistory,
            review_reference: "official-status-history:clob:2026-08-08/09".into(),
            review_sha256: "99".repeat(32),
            history_window_start_utc: "2026-08-08T12:00:00Z".into(),
            reviewed_through_utc: "2026-08-09T12:00:00Z".into(),
            clob_component: policy.value().reviewed_status_clob_component.clone(),
            finding:
                ReviewedStatusNoticeHistoryFindingV2::NoExactComponentLinkedIncidentOrMaintenanceInWindow,
        },
        approval: OnlineAuthorizationApprovalV2 {
            phase_scope: OnlinePhaseScopeApprovalV2::OnlyExactPhaseA,
            attempt_scope: OnlineAttemptScopeApprovalV2::ExactlyOnePlaceDispatch,
            fill_risk: OnlineFillRiskApprovalV2::OnePossibleFillWithinExactV1LossCap,
            post_only_semantics: OnlinePostOnlySemanticsApprovalV2::MayFill,
            proxy_concurrency: OnlineProxyConcurrencyApprovalV2::NoConcurrentProxyTrading,
            cleanup: OnlineCleanupApprovalV2::IndependentCleanupMethodReviewed,
            source_separation:
                OnlineSourceSeparationApprovalV2::FiveDistinctEvidenceClassesRequired,
        },
    }
}

fn config_pins(config: &reap_pm_controlled_trial::CanonicalTrialConfig) -> V1ConfigPinsV2 {
    V1ConfigPinsV2 {
        canonical_config_sha256: config.canonical_sha256().into(),
        canonical_config_length: config.canonical_length(),
        canonical_config_fingerprint: config.fingerprint().into(),
        trial_plan_fingerprint: config.plan_fingerprint().into(),
    }
}

fn policy_pins(policy: &reap_pm_controlled_trial::CanonicalOnlinePolicyV2) -> OnlinePolicyPinsV2 {
    OnlinePolicyPinsV2 {
        canonical_sha256: policy.canonical_sha256().into(),
        canonical_length: policy.canonical_length(),
        fingerprint: policy.fingerprint().into(),
    }
}

fn authorization_pins(
    authorization: &reap_pm_controlled_trial::CanonicalOnlineAuthorizationV2,
) -> ReviewedOnlineAuthorizationPinsV1 {
    ReviewedOnlineAuthorizationPinsV1 {
        authorization_id: authorization.value().authorization_id.clone(),
        canonical_sha256: authorization.canonical_sha256().into(),
        canonical_length: authorization.canonical_length(),
        fingerprint: authorization.fingerprint().into(),
    }
}

fn trial_config() -> TrialConfig {
    TrialConfig {
        schema_version: 1,
        profile: "pm_t2_type1_proxy_offline_a0".into(),
        phase: TrialPhase::APlaceCancel,
        source_pin_manifest_sha256: PM_T2_OFFICIAL_SOURCE_MANIFEST_SHA256_V1.into(),
        runbook_revision: "pm-t2-runbook-v1".into(),
        runbook_sha256: "22".repeat(32),
        account: TrialAccount {
            chain_id: 137,
            signature_type: 1,
            wallet_profile: "poly_proxy".into(),
            signer: SIGNER.into(),
            funder: FUNDER.into(),
        },
        market: TrialMarket {
            condition_id: format!("0x{}", "33".repeat(32)),
            question_id: format!("0x{}", "44".repeat(32)),
            token_id: "123456789".into(),
            outcome_label: "YES".into(),
            domain: TrialDomain::Standard,
            exchange: "0xE111180000d2663C0091e4f400237545B87B996B".into(),
            pusd_contract: "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB".into(),
            conditional_tokens_contract: "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045".into(),
            maker_base_fee_bps: 0,
            taker_base_fee_bps: 0,
            fee_rate: "0.020".into(),
            fee_exponent: "2.0".into(),
            fee_taker_only: true,
        },
        order: TrialOrder {
            salt: 1,
            timestamp_ms: 1_800_000_000_000,
            side: TrialSide::Buy,
            price: "0.5".into(),
            quantity: "5".into(),
            tick: "0.01".into(),
            minimum_order_size: "5".into(),
            maker_amount: "2500000".into(),
            taker_amount: "5000000".into(),
            maximum_loss_pusd_base_units: "2500000".into(),
            reservation_pusd_base_units: "2500000".into(),
            sell_outcome_share_payout_risk_cap_base_units: None,
            order_type: TrialOrderType::Gtc,
            post_only: true,
            defer_exec: false,
            expiration: "0".into(),
            metadata: format!("0x{}", "00".repeat(32)),
            builder: format!("0x{}", "00".repeat(32)),
            no_fee_or_rebate_credit_in_loss_bound: true,
            place_dispatch_allowance: 1,
            replacement_or_reprice_allowed: false,
            primary_cancel_dispatch_budget: 1,
            recovery_cancel_dispatch_budget: 2,
        },
        time_limits: TrialTimeLimits {
            maximum_preflight_observation_age_ms: 5_000,
            maximum_resting_duration_ms: 30_000,
            primary_cancel_deadline_ms: 35_000,
            cleanup_not_after_ms: 120_000,
            maximum_remediation_duration_ms: 90_000,
        },
        credential_slot: TrialCredentialSlot {
            slot_id: "pm-t2-slot-1".into(),
            nonsecret_fingerprint_sha256: "55".repeat(32),
            signer_to_proxy_evidence_reference: "reviewed-account-record:pm-t2-account-v1".into(),
        },
        journal: TrialJournalBinding {
            artifact_directory: "/tmp/reap-pm-t2-artifacts".into(),
            journal_family: "pm-t2-controlled-trial".into(),
            journal_version: 1,
            authorization_consumption_ledger_file: "authorization-consumption.jsonl".into(),
            authorization_consumption_claim_file: "authorization-consumed.claim".into(),
        },
    }
}

fn write_json<T: serde::Serialize>(root: &Path, name: &str, value: &T) -> PathBuf {
    let path = root.join(name);
    write_0600(&path, &serde_json::to_vec(value).unwrap());
    path
}

fn protected_dir() -> TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    directory
}

fn write_0600(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
