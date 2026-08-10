const MANIFEST: &str = include_str!("../Cargo.toml");
const LIB: &str = include_str!("../src/lib.rs");
const CONFIG: &str = include_str!("../src/config.rs");
const CONSUMPTION: &str = include_str!("../src/consumption.rs");
const CUSTODY: &str = include_str!("../src/custody.rs");
const ONLINE_CONSUMPTION_V2: &str = include_str!("../src/online_consumption_v2.rs");
const ONLINE_POLICY_V2: &str = include_str!("../src/online_policy_v2.rs");
const PREFLIGHT: &str = include_str!("../src/preflight.rs");
const PROTECTED: &str = include_str!("../src/protected_file.rs");
const REVIEWED_DESTINATION_PROFILE_V1: &str =
    include_str!("../src/reviewed_destination_profile_v1.rs");
const REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1: &str =
    include_str!("../src/reviewed_fresh_credential_slot_locator_v1.rs");
const FRESH_CREDENTIAL_DELIVERY_BINDING_V1: &str =
    include_str!("../src/fresh_credential_delivery_binding_v1.rs");
const REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1: &str =
    include_str!("../src/reviewed_signer_proxy_account_identity_v1.rs");
const REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1: &str =
    include_str!("../src/reviewed_remote_credential_proof_policy_v1.rs");
const REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3: &str =
    include_str!("../src/reviewed_static_online_authorization_v3.rs");
const MAIN: &str = include_str!("../src/main.rs");

#[test]
fn crate_has_no_network_live_adapter_or_mutation_dependency() {
    for forbidden in [
        "reqwest",
        "hyper",
        "tokio",
        "reap-pm-live",
        "reap-polymarket-live-adapter",
        "reap-polymarket-adapter",
    ] {
        assert!(
            !MANIFEST.contains(forbidden),
            "forbidden dependency: {forbidden}"
        );
    }
}

#[test]
fn source_exposes_only_offline_dry_run_commands_and_no_mutation_route_or_body() {
    let source = [
        LIB,
        CONFIG,
        CONSUMPTION,
        CUSTODY,
        ONLINE_CONSUMPTION_V2,
        ONLINE_POLICY_V2,
        PREFLIGHT,
        PROTECTED,
        REVIEWED_DESTINATION_PROFILE_V1,
        REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1,
        FRESH_CREDENTIAL_DELIVERY_BINDING_V1,
        REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1,
        REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1,
        REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3,
        MAIN,
    ]
    .join("\n");
    for forbidden in [
        "POST /order",
        "DELETE /order",
        "serialize_gtc_post_only",
        "sign_clob_v2_order",
        "AuthenticatedPlaceRequest",
        "FixedPlaceRequestSink",
        "production_order_entry_authorized: true",
        "real_order_submission_authorized: true",
    ] {
        assert!(
            !source.contains(forbidden),
            "forbidden source surface: {forbidden}"
        );
    }
    assert!(MAIN.contains("VerifyPlan"));
    assert!(MAIN.contains("VerifyAuthorization"));
    assert!(MAIN.contains("InspectCustody"));
    assert!(!MAIN.contains("Place"));
    assert!(!MAIN.contains("Cancel"));
    assert!(!MAIN.contains("ConsumeAuthorization"));
}

#[test]
fn reviewed_static_online_authorization_v3_is_exact_static_noncapable_and_denied_only() {
    for required in [
        "REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3_FINGERPRINT_DOMAIN",
        "reap.pm-t2.controlled-trial.reviewed-static-online-authorization.v3\\0",
        "pm-t2-reviewed-static-online-authorization-v3.json",
        "MAX_CANONICAL_REVIEWED_STATIC_ONLINE_AUTHORIZATION_BYTES_V3: usize = 128 * 1024",
        "pub struct ReviewedStaticOnlineAuthorizationConfigPinsV3",
        "pub struct ReviewedStaticOnlineAuthorizationV1AuthorizationPinsV3",
        "pub struct ReviewedStaticOnlineAuthorizationOnlinePolicyPinsV3",
        "pub struct ReviewedStaticOnlineAuthorizationOnlineAuthorizationPinsV3",
        "pub struct ReviewedStaticOnlineAuthorizationDestinationPinsV3",
        "pub struct ReviewedStaticOnlineAuthorizationLocatorPinsV3",
        "pub struct ReviewedStaticOnlineAuthorizationDeliveryPinsV3",
        "pub struct ReviewedStaticOnlineAuthorizationAccountIdentityPinsV3",
        "pub struct ReviewedStaticOnlineAuthorizationRemoteProofPolicyPinsV3",
        "pub schema_version: u32,",
        "pub canonical_sha256: String,",
        "pub canonical_length: u64,",
        "pub fingerprint: String,",
        "pub plan_fingerprint: String,",
        "pub authorization_id: String,",
        "pub policy_id: String,",
        "pub profile_id: String,",
        "pub locator_id: String,",
        "pub binding_id: String,",
        "pub identity_id: String,",
        "pub struct ReviewedStaticOnlineAuthorizationV3",
        "pub v1_config: ReviewedStaticOnlineAuthorizationConfigPinsV3,",
        "pub v1_authorization: ReviewedStaticOnlineAuthorizationV1AuthorizationPinsV3,",
        "pub online_policy_v2: ReviewedStaticOnlineAuthorizationOnlinePolicyPinsV3,",
        "pub online_authorization_v2: ReviewedStaticOnlineAuthorizationOnlineAuthorizationPinsV3,",
        "pub reviewed_production_destination_v1: ReviewedStaticOnlineAuthorizationDestinationPinsV3,",
        "pub reviewed_fresh_credential_slot_locator_v1: ReviewedStaticOnlineAuthorizationLocatorPinsV3,",
        "pub fresh_credential_delivery_binding_v1: ReviewedStaticOnlineAuthorizationDeliveryPinsV3,",
        "ReviewedStaticOnlineAuthorizationAccountIdentityPinsV3,",
        "ReviewedStaticOnlineAuthorizationRemoteProofPolicyPinsV3,",
        "pub struct ReviewedStaticOnlineAuthorizationContextV3<'a>",
        "pub v1_config: &'a CanonicalTrialConfig,",
        "pub v1_authorization: &'a CanonicalAuthorization,",
        "pub online_policy_v2: &'a CanonicalOnlinePolicyV2,",
        "pub online_authorization_v2: &'a CanonicalOnlineAuthorizationV2,",
        "pub reviewed_production_destination_v1: &'a CanonicalReviewedProductionDestinationProfileV1,",
        "CanonicalReviewedFreshCredentialSlotLocatorV1,",
        "pub fresh_credential_delivery_binding_v1: &'a CanonicalFreshCredentialDeliveryBindingV1,",
        "CanonicalReviewedSignerProxyAccountIdentityV1,",
        "CanonicalReviewedRemoteCredentialProofPolicyV1,",
        "pub fn load_canonical_reviewed_static_online_authorization_v3(",
        "ProtectedFileKind::ReviewedStaticOnlineAuthorizationV3",
        "pub fn verify_reviewed_static_online_authorization_v3(",
        "serde_json::to_vec(v1_value)",
        "verify_authorization(",
        "context.v1_authorization,",
        "v1_not_before,",
        "validate_online_authorization_contract_v2(",
        "common_v1_v2_tuple_matches(context)",
        "v1.build.repository_commit == v2.build.repository_commit",
        "v1.build.cargo_lock_sha256 == v2.build.cargo_lock_sha256",
        "v1.build.release_binary_sha256 == v2.build.release_binary_sha256",
        "v1.build.release_binary_length == v2.build.release_binary_length",
        "v1.host.host_identity == v2.host.uts_nodename",
        "v1.host.boot_identity == v2.host.boot_id",
        "v1.host.runtime_user == v2.host.nss_username",
        "v1.host.egress_identity == v2.host.egress.authorized_geoblock_reported_public_ip",
        "v2_times.not_before < v1_not_before",
        "v2_times.expires_at > v1_expires_at",
        "v2_times.cleanup_not_after > v1_cleanup_not_after",
        "static_times.not_before != v2_times.not_before",
        "static_times.expires_at != v2_times.expires_at",
        "static_times.cleanup_not_after != v2_times.cleanup_not_after",
        "std::cmp::max(v1_reviewed_at, v2_times.reviewed_at)",
        "static_review_order_against_v1_and_v2_structurally_valid: true",
        "Static review ordering compares only the V1 and V2 authorization review",
        "Exact pins do not prove that any of the five prerequisite sidecars",
        "existed when this V3 record was reviewed",
        "prerequisite_artifacts_existed_at_static_review_time_attested: false",
        "verify_reviewed_production_destination_profile_v1(",
        "verify_reviewed_fresh_credential_slot_locator_v1(",
        "verify_fresh_credential_delivery_binding_v1(",
        "verify_reviewed_signer_proxy_account_identity_v1(",
        "verify_reviewed_remote_credential_proof_policy_v1(",
        "component_verifications_are_denied(",
        "component_verifiers_denied_and_negative_facts_checked: true",
        "let crate::ReviewedProductionDestinationProfileVerificationV1 {",
        "let crate::ReviewedFreshCredentialSlotLocatorVerificationV1 {",
        "let crate::FreshCredentialDeliveryBindingVerificationV1 {",
        "let crate::ReviewedSignerProxyAccountIdentityVerificationV1 {",
        "let crate::ReviewedRemoteCredentialProofPolicyVerificationV1 {",
        "OnlineV2PreparedThenV1PreparedV1",
        "BasisAfterExistingPlaceAndBothConsumptionPreparedThenV2ClaimThenV1ClaimThenV1A3ThenV2A3ConjunctV1",
        "ExistingV1LifecycleOnlyNoPlacementResumeV1",
        "NoPreparedClaimBurnRecoveryOrDispatchV1",
        "The V2 attempt value in that design is only the existing Basis",
        "fingerprint, never a fresh runtime commitment",
        "a one-record ledger plus its durable claim is",
        "already burned even when no later `Consumed` line exists",
        "failed claim never proves that an attempt is unburned",
        "V2 has no",
        "placement-reopen path",
        "V1 may reopen only to consume or recovery-cancel",
        "always with zero placement resumption",
        "OfflineAuthorizationState::DENIED",
        "CanonicalReviewedStaticOnlineAuthorizationV3(<exact-protected-canonical-bytes; no-value-id-address-path-actor-or-lineage-projection; redacted; denied>)",
        "separately versioned Prepared lineage must consume the retained delivery",
        "evidence and load token, bind a selected actor generation",
        "The schema defines no designated",
        "secret, credential-value, cryptographic-signature, lease-token",
        "Arbitrary ID",
        "and reviewer-label strings cannot be proven secret-free",
        "operators must never place secrets or secret-derived material in them",
    ] {
        assert!(
            REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3.contains(required),
            "missing reviewed static online-authorization V3 source pin `{required}`"
        );
    }

    for (enum_name, exact_body) in [
        (
            "ReviewedCredentialProviderTrustRootStatusV3",
            "#[serde(rename = \"unavailable_in_frozen_sources_v1\")]\n    UnavailableInFrozenSourcesV1,",
        ),
        (
            "ReviewedCredentialDeliveryLeaseProtocolStatusV3",
            "#[serde(rename = \"unavailable_in_frozen_sources_v1\")]\n    UnavailableInFrozenSourcesV1,",
        ),
        (
            "ReviewedRemoteCredentialAcceptanceContractStatusV3",
            "#[serde(rename = \"unavailable_in_frozen_sources_v1\")]\n    UnavailableInFrozenSourcesV1,",
        ),
        (
            "ReviewedSignerProxyControlContractStatusV3",
            "#[serde(rename = \"unattested_reviewed_labels_only_v1\")]\n    UnattestedReviewedLabelsOnlyV1,",
        ),
        (
            "ReviewedSelectedActorProfileV3",
            "#[serde(rename = \"shutdown_only_selected_egress_current_thread_local_set_v1\")]\n    ShutdownOnlySelectedEgressCurrentThreadLocalSetV1,",
        ),
        (
            "ReviewedActorGenerationSchemeV3",
            "#[serde(rename = \"process_id_thread_id_and_rc_pointer_identity_v1\")]\n    ProcessIdThreadIdAndRcPointerIdentityV1,",
        ),
        (
            "ReviewedActorGenerationAllocationV3",
            "#[serde(rename = \"before_runtime_local_set_and_all_actor_local_construction_v1\")]\n    BeforeRuntimeLocalSetAndAllActorLocalConstructionV1,",
        ),
        (
            "ReviewedActorReadinessV3",
            "#[serde(rename = \"task_entry_ack_after_generation_membership_revalidation_v1\")]\n    TaskEntryAckAfterGenerationMembershipRevalidationV1,",
        ),
        (
            "ReviewedActorCommandSetV3",
            "#[serde(rename = \"shutdown_only_v1\")]\n    ShutdownOnlyV1,",
        ),
        (
            "ReviewedActorTerminalRequirementV3",
            "#[serde(\n        rename = \"shutdown_requested_no_abort_task_joined_clean_credentials_dropped_staged_files_removed_generation_revalidated_v1\"\n    )]\n    ShutdownRequestedNoAbortTaskJoinedCleanCredentialsDroppedStagedFilesRemovedGenerationRevalidatedV1,",
        ),
        (
            "ReviewedActorRuntimeAttemptCommitmentLocationV3",
            "#[serde(rename = \"future_prepared_only_absent_from_static_v3\")]\n    FuturePreparedOnlyAbsentFromStaticV3,",
        ),
        (
            "ReviewedPreparedCreationOrderV3",
            "#[serde(rename = \"online_v2_prepared_then_v1_prepared_v1\")]\n    OnlineV2PreparedThenV1PreparedV1,",
        ),
        (
            "ReviewedBasisAndBurnOrderV3",
            "#[serde(\n        rename = \"basis_after_existing_place_and_both_consumption_prepared_then_v2_claim_then_v1_claim_then_v1_a3_then_v2_a3_conjunct_v1\"\n    )]\n    BasisAfterExistingPlaceAndBothConsumptionPreparedThenV2ClaimThenV1ClaimThenV1A3ThenV2A3ConjunctV1,",
        ),
        (
            "ReviewedCrashRecoveryProfileV3",
            "#[serde(rename = \"existing_v1_lifecycle_only_no_placement_resume_v1\")]\n    ExistingV1LifecycleOnlyNoPlacementResumeV1,",
        ),
        (
            "ReviewedStaticV3RuntimeStateV3",
            "#[serde(rename = \"no_prepared_claim_burn_recovery_or_dispatch_v1\")]\n    NoPreparedClaimBurnRecoveryOrDispatchV1,",
        ),
    ] {
        let declaration = format!("pub enum {enum_name} {{");
        let body = REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3
            .split_once(&declaration)
            .and_then(|(_, tail)| tail.split_once("\n}").map(|(body, _)| body.trim()))
            .expect("closed static V3 enum must remain recognizable");
        assert_eq!(body, exact_body, "closed enum changed: {enum_name}");
    }

    let false_claims = [
        "prerequisite_artifacts_existed_at_static_review_time_attested",
        "reviewer_authorship_attested",
        "credential_provider_trust_root_available",
        "credential_provider_trust_root_authenticated",
        "credential_provider_authorship_attested",
        "authenticated_credential_delivery_lease_protocol_available",
        "credential_provider_signature_verified",
        "credential_delivery_lease_verified",
        "credential_delivery_lease_current_and_unrevoked",
        "delivery_generation_attested",
        "rotation_generation_attested",
        "authoritative_remote_credential_acceptance_contract_available",
        "authoritative_signer_proxy_control_contract_available",
        "official_source_bytes_loaded_and_hash_verified",
        "official_source_publisher_authorship_attested",
        "remote_api_key_owner_attested",
        "live_credential_tuple_accepted_by_provider",
        "signer_controls_proxy_attested",
        "signer_proxy_relationship_current_and_unrevoked_attested",
        "signer_on_chain_eoa_status_verified",
        "proxy_on_chain_contract_status_verified",
        "on_chain_account_state_checked",
        "on_chain_finality_checked",
        "proxy_factory_semantics_verified",
        "credential_tuple_current_and_unrevoked_attested",
        "source_owned_current_time_checked",
        "v1_authorization_current",
        "online_authorization_v2_current",
        "static_online_authorization_v3_current",
        "retained_delivery_evidence_joined",
        "fresh_credential_delivery_load_token_consumed",
        "retained_delivery_evidence_and_load_token_joined",
        "same_loaded_credential_holder_attested",
        "delivery_and_remote_proof_same_source_generation_attested",
        "globally_unique_credential_delivery_attested",
        "loaded_credentials_match_delivery_binding",
        "protected_credential_directory_and_four_files_checked",
        "private_key_derived_signer_matches_config_checked",
        "l2_credentials_match_configured_signer_checked",
        "runner_actor_profile_implementation_checked",
        "actor_started",
        "selected_actor_bound",
        "selected_actor_generation_bound",
        "selected_actor_generation_membership_revalidated",
        "actor_runtime_attempt_commitment_bound",
        "runtime_attempt_commitment_present",
        "runtime_attempt_commitment_source_owned",
        "runtime_attempt_commitment_fresh",
        "product_clock_owner_bound",
        "server_time_proof_authenticated_and_fresh",
        "preflight_collected_and_validated",
        "v1_prepared_consumption_inspected",
        "v2_prepared_consumption_inspected",
        "prospective_v3_prepared_implemented",
        "prospective_v3_prepared_created",
        "v1_durable_prepared_observed",
        "v2_durable_prepared_observed",
        "v1_claim_absence_reserved",
        "v2_claim_absence_reserved",
        "artifact_file_created_or_written",
        "v3_create_new_performed",
        "v3_file_fsynced",
        "v3_parent_directory_fsynced",
        "journal_opened_for_append",
        "parent_directory_snapshot_refreshed",
        "basis_inspected",
        "basis_durable",
        "online_v2_claim_inspected",
        "online_v2_claim_durable",
        "v1_claim_inspected",
        "v1_claim_durable",
        "v1_authorization_consumption_checked",
        "online_authorization_v2_consumption_checked",
        "static_online_authorization_v3_consumption_checked",
        "atomic_consumption_claim_created",
        "v1_authorization_burn_performed",
        "online_authorization_v2_burn_performed",
        "v3_authorization_burn_performed",
        "burn_and_no_resend_established",
        "recovery_state_checked",
        "v1_recovery_reopen_eligibility_established",
        "v2_recovery_reopen_eligibility_established",
        "placement_resumption_allowed",
        "v1_a3_created",
        "v1_a3_checked",
        "v1_a3_durable",
        "online_v2_a3_conjunct_created",
        "online_v2_a3_conjunct_checked",
        "online_v2_a3_conjunct_durable",
        "authenticated_request_or_hmac_constructed",
        "signed_order_body_constructed",
        "place_dispatch_owner_or_grant_minted",
        "network_dispatch_performed",
        "pid_tid_or_rc_pointer_value_recorded",
        "randomness_or_runtime_nonce_sampled",
        "v1_config_reverse_pins_static_authorization_v3",
        "v1_authorization_reverse_pins_static_authorization_v3",
        "online_policy_v2_reverse_pins_static_authorization_v3",
        "online_authorization_v2_reverse_pins_static_authorization_v3",
        "reviewed_destination_reverse_pins_static_authorization_v3",
        "reviewed_locator_reverse_pins_static_authorization_v3",
        "fresh_delivery_reverse_pins_static_authorization_v3",
        "reviewed_identity_reverse_pins_static_authorization_v3",
        "remote_proof_policy_reverse_pins_static_authorization_v3",
        "static_authorization_fingerprint_pinned_by_prospective_v3_prepared",
        "durable_static_authorization_consumption_recorded",
        "credential_mutation_authority_attested",
    ];
    for claim in false_claims {
        assert!(
            REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3.contains(&format!("{claim}: false")),
            "reviewed static V3 claim is not explicitly false: {claim}"
        );
        assert!(
            !REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3.contains(&format!("{claim}: true")),
            "reviewed static V3 claim became true: {claim}"
        );
    }

    assert!(
        !REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3.contains("static_review_order_structurally_valid"),
        "static V3 review-order fact overclaims prerequisite history"
    );
    let component_helper = REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3
        .split_once("fn component_verifications_are_denied(")
        .and_then(|(_, tail)| {
            tail.split_once("struct ExactPinViewV3")
                .map(|(body, _)| body)
        })
        .expect("static V3 exhaustive component helper must remain recognizable");
    assert_eq!(component_helper.matches("let crate::").count(), 5);
    assert!(
        !component_helper.contains(".."),
        "component verification destructuring must remain exhaustive"
    );
    assert!(
        !component_helper.contains("#[allow"),
        "component verification coverage must not suppress unused-field diagnostics"
    );

    for forbidden in [
        "reqwest::",
        "tokio::",
        "TcpStream",
        "reap_polymarket_auth",
        "reap_polymarket_wire",
        "L2Credentials",
        "AuthenticatedL2Headers",
        "EoaPrivateKeyInput",
        "DurableCreateNewFile",
        "OpenOptions",
        "File::create",
        "File::open",
        "fs::write",
        "write_all",
        "open_for_append",
        "prepare_authorization_consumption",
        "prepare_online_authorization_consumption_v2",
        "claim_authorization_consumption",
        "consume_authorization",
        "Utc::now",
        "SystemTime",
        "rand::",
        "getrandom",
        "pub fn bind_",
        "pub fn into_parts",
        "pub fn prepare_",
        "pub fn inspect_",
        "pub fn consume_",
        "pub struct Prospective",
        "pub struct PreparedReviewedStatic",
        "ProtectedFileKind::Prepared",
        "production_order_entry_authorized: true",
        "real_order_submission_authorized: true",
    ] {
        assert!(
            !REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3.contains(forbidden),
            "reviewed static V3 gained forbidden capability or positive surface `{forbidden}`"
        );
    }

    let record = REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3
        .split_once("pub struct ReviewedStaticOnlineAuthorizationV3 {")
        .and_then(|(_, tail)| tail.split_once("\n}").map(|(body, _)| body))
        .expect("static V3 record must remain recognizable");
    for forbidden_field in [
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
        "request_body",
        "signed_order",
    ] {
        assert!(
            !record.contains(forbidden_field),
            "static V3 record gained forbidden field `{forbidden_field}`"
        );
    }

    let context = REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3
        .split_once("pub struct ReviewedStaticOnlineAuthorizationContextV3<'a> {")
        .and_then(|(_, tail)| tail.split_once("\n}").map(|(body, _)| body))
        .expect("static V3 context must remain recognizable");
    for forbidden_context_input in [
        "Verification",
        "String",
        "&str",
        "fingerprint:",
        "Token",
        "Evidence",
        "Prepared",
        "Actor",
        "Clock",
    ] {
        assert!(
            !context.contains(forbidden_context_input),
            "static V3 context gained caller-supplied evidence `{forbidden_context_input}`"
        );
    }

    let canonical_surface = REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3
        .split_once("impl CanonicalReviewedStaticOnlineAuthorizationV3 {")
        .and_then(|(_, tail)| {
            tail.split_once("impl fmt::Debug for CanonicalReviewedStaticOnlineAuthorizationV3")
                .map(|(surface, _)| surface)
        })
        .expect("canonical static V3 surface must remain recognizable");
    for forbidden_projection in [
        "value(",
        "id(",
        "address(",
        "path(",
        "actor(",
        "lineage(",
        "bytes(",
        "config(",
        "authorization(",
    ] {
        assert!(
            !canonical_surface.contains(forbidden_projection),
            "canonical static V3 holder gained projection `{forbidden_projection}`"
        );
    }

    let declaration = "pub struct CanonicalReviewedStaticOnlineAuthorizationV3 {";
    let before = REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3
        .split_once(declaration)
        .map(|(before, _)| before)
        .expect("canonical static V3 declaration must remain recognizable");
    let declaration_attributes = before
        .rsplit_once("\n}\n\n")
        .map_or(before, |(_, attributes)| attributes);
    assert!(!declaration_attributes.contains("#[derive"));
    for implementation in REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3.split("\nimpl") {
        let header = implementation
            .split_once('{')
            .map_or(implementation, |(header, _)| header);
        assert!(
            !(header.contains("CanonicalReviewedStaticOnlineAuthorizationV3")
                && ["Clone", "Copy", "Serialize", "Deserialize"]
                    .iter()
                    .any(|trait_name| header.contains(trait_name))),
            "canonical static V3 holder gained manual Clone, Copy, Serialize, or Deserialize"
        );
    }

    assert!(PROTECTED.contains("ReviewedStaticOnlineAuthorizationV3"));
    assert!(LIB.contains("mod reviewed_static_online_authorization_v3;"));
    for upstream in [
        CONFIG,
        CONSUMPTION,
        CUSTODY,
        ONLINE_CONSUMPTION_V2,
        ONLINE_POLICY_V2,
        PREFLIGHT,
        REVIEWED_DESTINATION_PROFILE_V1,
        REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1,
        FRESH_CREDENTIAL_DELIVERY_BINDING_V1,
        REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1,
        REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1,
        MAIN,
    ] {
        assert!(
            !upstream.contains("ReviewedStaticOnlineAuthorizationV3"),
            "upstream or runtime source reverse-wired static V3"
        );
    }
}

#[test]
fn reviewed_remote_credential_proof_policy_v1_is_exact_noncapable_and_denied_only() {
    for required in [
        "REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_FINGERPRINT_DOMAIN",
        "reap.pm-t2.controlled-trial.reviewed-remote-credential-proof-policy.v1\\0",
        "pm-t2-reviewed-remote-credential-proof-policy-v1.json",
        "pub struct ReviewedRemoteCredentialProofDestinationPinsV1",
        "pub struct ReviewedRemoteCredentialProofLocatorPinsV1",
        "pub struct ReviewedRemoteCredentialProofDeliveryPinsV1",
        "pub struct ReviewedRemoteCredentialProofAccountIdentityPinsV1",
        "REVIEWED_PRODUCTION_DESTINATION_PROFILE_V1_SCHEMA_VERSION",
        "REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_SCHEMA_VERSION",
        "FRESH_CREDENTIAL_DELIVERY_BINDING_V1_SCHEMA_VERSION",
        "REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_SCHEMA_VERSION",
        "pub profile_id: String,",
        "pub locator_id: String,",
        "pub binding_id: String,",
        "pub identity_id: String,",
        "pub canonical_sha256: String,",
        "pub canonical_length: u64,",
        "pub fingerprint: String,",
        "verify_reviewed_production_destination_profile_v1(",
        "verify_reviewed_fresh_credential_slot_locator_v1(",
        "verify_fresh_credential_delivery_binding_v1(",
        "verify_reviewed_signer_proxy_account_identity_v1(",
        "context.reviewed_fresh_credential_locator",
        "pub struct ReviewedRemoteCredentialProofOfficialSourcesV1",
        "pub manifest: ReviewedOfficialSourceManifestPinsV1,",
        "pub api_authentication: ReviewedRemoteCredentialProofSourceEntryPinsV1,",
        "pub manage_orders: ReviewedRemoteCredentialProofSourceEntryPinsV1,",
        "PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_FAMILY_V1",
        "PM_T2_OFFICIAL_SOURCE_MANIFEST_RETRIEVED_AT_UTC_V1",
        "PM_T2_OFFICIAL_SOURCE_MANIFEST_BYTE_LENGTH_V1",
        "PM_T2_OFFICIAL_SOURCE_MANIFEST_SHA256_V1",
        "api_authentication",
        "https://docs.polymarket.com/getting-started/api.md",
        "API_AUTHENTICATION_SOURCE_BYTE_LENGTH_V1: u64 = 9_391",
        "6c397c66109852220b3f5d8033ea274061b3fc44b426edc9faa60673ecbef8fc",
        "manage_orders",
        "https://docs.polymarket.com/trading/manage-orders.md",
        "MANAGE_ORDERS_SOURCE_BYTE_LENGTH_V1: u64 = 52_401",
        "e4a0238db31d5137b4d0da0d4333b1fb90be8f7c7b47d92968edfd993c8c4482",
        "text/markdown; charset=utf-8",
        "pub enum ReviewedRemoteCredentialAuthenticationAcceptanceContractStatusV1",
        "#[serde(rename = \"unavailable_in_frozen_sources_v1\")]",
        "UnavailableInFrozenSourcesV1,",
        "accepts no caller-supplied substitute digest or signature",
        "pub struct ReviewedRemoteCredentialProofEndpointPolicyV1",
        "pub scheme: String,",
        "pub dns_name: String,",
        "pub tls_server_name: String,",
        "pub http_host: String,",
        "pub tcp_port: u16,",
        "pub selected_peer_ip: String,",
        "pub network_namespace_device: u64,",
        "pub network_namespace_inode: u64,",
        "pub interface_name: String,",
        "pub interface_index: u32,",
        "pub local_egress_ip: String,",
        "pub dedicated_tunnel_or_gateway_profile_reference: String,",
        "pub dedicated_tunnel_or_gateway_profile_sha256: String,",
        "endpoint.network_namespace_device",
        "endpoint.network_namespace_inode",
        "endpoint.interface_name",
        "endpoint.interface_index",
        "endpoint.dedicated_tunnel_or_gateway_profile_reference",
        "endpoint.dedicated_tunnel_or_gateway_profile_sha256",
        "pub struct ReviewedRemoteCredentialProofSensitiveHeaderNamesV1",
        "self.poly_address != \"POLY_ADDRESS\"",
        "self.poly_signature != \"POLY_SIGNATURE\"",
        "self.poly_timestamp != \"POLY_TIMESTAMP\"",
        "self.poly_api_key != \"POLY_API_KEY\"",
        "self.poly_passphrase != \"POLY_PASSPHRASE\"",
        "pub enum ReviewedRemoteCredentialProofHmacPreimageOrderedVariantV1",
        "decimal_timestamp_then_uppercase_get_then_exact_path_no_separators_v1",
        "DecimalTimestampThenUppercaseGetThenExactPathNoSeparatorsV1,",
        "pub hmac_algorithm: String,",
        "hmac_sha256",
        "pub hmac_key_source: String,",
        "same_loaded_l2_credential_holder_decoded_url_safe_base64_secret_bytes",
        "pub l2_secret_input_encoding: String,",
        "rfc4648_url_safe_base64_with_padding_canonical",
        "pub maximum_l2_secret_encoded_bytes: u16,",
        "MAXIMUM_L2_SECRET_ENCODED_BYTES_V1: u16 = 172",
        "pub minimum_hmac_key_bytes: u16,",
        "MINIMUM_HMAC_KEY_BYTES_V1: u16 = 1",
        "pub maximum_hmac_key_bytes: u16,",
        "MAXIMUM_HMAC_KEY_BYTES_V1: u16 = 128",
        "fresh_clob_server_unix_seconds_decimal_ascii",
        "pub timestamp_decimal_digits: u8,",
        "HMAC_TIMESTAMP_DECIMAL_DIGITS_V1: u8 = 10",
        "pub minimum_timestamp_unix_seconds: u64,",
        "HMAC_MINIMUM_TIMESTAMP_UNIX_SECONDS_V1: u64 = 1_000_000_000",
        "pub maximum_timestamp_unix_seconds: u64,",
        "HMAC_MAXIMUM_TIMESTAMP_UNIX_SECONDS_V1: u64 = 9_999_999_999",
        "excluded_from_preimage_header_only",
        "hmac_key_only_not_preimage",
        "pub signature_encoding: String,",
        "rfc4648_url_safe_base64_with_padding",
        "pub signature_decoded_length: u8,",
        "HMAC_SIGNATURE_DECODED_LENGTH_V1: u8 = 32",
        "pub signature_encoded_length: u8,",
        "HMAC_SIGNATURE_ENCODED_LENGTH_V1: u8 = 44",
        "pub signature_terminal_padding: String,",
        "HMAC_SIGNATURE_TERMINAL_PADDING_V1: &str = \"=\"",
        "self.hmac_algorithm != HMAC_ALGORITHM_V1",
        "self.hmac_key_source != HMAC_KEY_SOURCE_V1",
        "self.l2_secret_input_encoding != L2_SECRET_INPUT_ENCODING_V1",
        "self.maximum_l2_secret_encoded_bytes != MAXIMUM_L2_SECRET_ENCODED_BYTES_V1",
        "self.minimum_hmac_key_bytes != MINIMUM_HMAC_KEY_BYTES_V1",
        "self.maximum_hmac_key_bytes != MAXIMUM_HMAC_KEY_BYTES_V1",
        "self.timestamp_decimal_digits != HMAC_TIMESTAMP_DECIMAL_DIGITS_V1",
        "self.minimum_timestamp_unix_seconds",
        "self.maximum_timestamp_unix_seconds",
        "self.signature_encoding != HMAC_SIGNATURE_ENCODING_V1",
        "self.signature_decoded_length != HMAC_SIGNATURE_DECODED_LENGTH_V1",
        "self.signature_encoded_length != HMAC_SIGNATURE_ENCODED_LENGTH_V1",
        "self.signature_terminal_padding != HMAC_SIGNATURE_TERMINAL_PADDING_V1",
        "self.separator != HMAC_SEPARATOR_V1",
        "self.query_component != ABSENT_COMPONENT_V1",
        "self.body_component != ABSENT_COMPONENT_V1",
        "self.poly_address_component != HMAC_EXCLUDED_HEADER_ONLY_V1",
        "self.poly_api_key_component != HMAC_EXCLUDED_HEADER_ONLY_V1",
        "self.poly_passphrase_component != HMAC_EXCLUDED_HEADER_ONLY_V1",
        "pub poly_timestamp_source: String,",
        "same_canonical_l2_timestamp_as_hmac_preimage",
        "pub poly_signature_source: String,",
        "exact_hmac_output",
        "pub poly_api_key_source: String,",
        "pub poly_passphrase_source: String,",
        "same_loaded_l2_credential_holder",
        "self.poly_timestamp_source != POLY_TIMESTAMP_SOURCE_V1",
        "self.poly_signature_source != POLY_SIGNATURE_SOURCE_V1",
        "self.poly_api_key_source != POLY_L2_HOLDER_SOURCE_V1",
        "self.poly_passphrase_source != POLY_L2_HOLDER_SOURCE_V1",
        "pub method: String,",
        "pub path: String,",
        "pub query: String,",
        "pub body: String,",
        "pub content_type: String,",
        "pub accept: String,",
        "pub accept_encoding: String,",
        "self.method != CLOSED_ONLY_METHOD_V1",
        "self.path != CLOSED_ONLY_PATH_V1",
        "self.query != ABSENT_COMPONENT_V1",
        "self.body != ABSENT_COMPONENT_V1",
        "self.content_type != ABSENT_COMPONENT_V1",
        "self.accept != ACCEPT_APPLICATION_JSON_V1",
        "self.accept_encoding != ACCEPT_ENCODING_IDENTITY_V1",
        "pub maximum_server_time_sample_to_dispatch_age_ms: u64,",
        "pub maximum_proof_observation_age_ms: u64,",
        "MAXIMUM_SERVER_TIME_SAMPLE_TO_DISPATCH_AGE_MS_V1: u64 = 5_000",
        "MAXIMUM_PROOF_OBSERVATION_AGE_MS_V1: u64 = 5_000",
        "maximum_preflight_observation_age_ms",
        "maximum_observation_age_ms",
        "pub maximum_authenticated_dispatch_count: u8,",
        "pub connect_timeout_ms: u64,",
        "pub request_timeout_ms: u64,",
        "pub redirects_allowed: bool,",
        "pub retries_allowed: bool,",
        "pub forward_proxy_allowed: bool,",
        "pub destination_fallback_allowed: bool,",
        "pub connected_peer_check_before_status_and_body_required: bool,",
        "pub ambiguous_outcome_requires_durable_burn: bool,",
        "self.maximum_authenticated_dispatch_count != 1",
        "self.connect_timeout_ms != CONNECT_TIMEOUT_MS_V1",
        "self.request_timeout_ms != REQUEST_TIMEOUT_MS_V1",
        "pub required_status_code: u16,",
        "pub content_type_header_required: bool,",
        "!self.content_type_header_required",
        "pub required_content_type_essence: String,",
        "pub allowed_content_type_charset: String,",
        "pub allowed_content_encoding: String,",
        "pub maximum_body_bytes: u64,",
        "pub required_json_object_field_count: u8,",
        "pub required_json_field_name: String,",
        "pub required_json_field_type: String,",
        "pub allowed_json_boolean_values: String,",
        "pub closed_only_false_semantics: String,",
        "pub closed_only_true_semantics: String,",
        "pub authentication_semantics: String,",
        "none_or_single_charset_utf8_ascii_case_insensitive",
        "absent_or_single_identity_ascii_case_insensitive",
        "self.maximum_body_bytes != 64",
        "true_or_false_shape_only",
        "placement_candidate_evaluated_separately",
        "hard_block",
        "neither_value_proves_authentication_acceptance_or_failure",
        "let source_retrieval_times = self.official_sources.validate()?;",
        "retrieved_at > reviewed_at",
        "reviewed_at > valid_not_before",
        "valid_not_before >= valid_not_after",
        "reviewed_times.reviewed_at < authorization_times.reviewed_at",
        "reviewed_times.reviewed_at > authorization_times.not_before",
        "reviewed_times.valid_not_before != authorization_times.not_before",
        "reviewed_times.valid_not_after != authorization_times.cleanup_not_after",
        "pub struct ReviewedRemoteCredentialProofPolicyContextV1<'a>",
        "pre-bind offline policy conjunction only",
        "A future V3 must join the whole retained",
        "must join the whole retained evidence and load token to the selected actor",
        "retained delivery evidence or a post-split load-token join",
        "pub config: &'a CanonicalTrialConfig,",
        "pub online_policy: &'a CanonicalOnlinePolicyV2,",
        "pub online_authorization: &'a CanonicalOnlineAuthorizationV2,",
        "pub reviewed_destination: &'a CanonicalReviewedProductionDestinationProfileV1,",
        "pub reviewed_fresh_credential_locator: &'a CanonicalReviewedFreshCredentialSlotLocatorV1,",
        "pub fresh_credential_delivery: &'a CanonicalFreshCredentialDeliveryBindingV1,",
        "pub reviewed_signer_proxy_identity: &'a CanonicalReviewedSignerProxyAccountIdentityV1,",
        "pub fn load_canonical_reviewed_remote_credential_proof_policy_v1(",
        "pub fn verify_reviewed_remote_credential_proof_policy_v1(",
        "context: &ReviewedRemoteCredentialProofPolicyContextV1<'_>",
        "reviewed_policy: &CanonicalReviewedRemoteCredentialProofPolicyV1",
        "without a clock, network, or load",
        "OfflineAuthorizationState::DENIED",
        "ProtectedFileKind::ReviewedRemoteCredentialProofPolicyV1",
        "CanonicalReviewedRemoteCredentialProofPolicyV1(<exact-protected-canonical-bytes; no-value-route-address-source-or-request-projection; redacted; denied>)",
    ] {
        assert!(
            REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1.contains(required),
            "missing reviewed remote credential-proof policy V1 source pin `{required}`"
        );
    }

    let false_claims = [
        "official_source_manifest_bytes_loaded_and_hash_verified",
        "api_authentication_source_bytes_loaded_and_hash_verified",
        "manage_orders_source_bytes_loaded_and_hash_verified",
        "official_source_publisher_authorship_attested",
        "official_source_manifest_publisher_authorship_attested",
        "api_authentication_source_publisher_authorship_attested",
        "manage_orders_source_publisher_authorship_attested",
        "reviewer_authorship_attested",
        "remote_api_key_owner_attested",
        "authoritative_authentication_acceptance_contract_available",
        "api_key_mismatch_rejected_before_http_200_attested",
        "l2_secret_mismatch_rejected_before_http_200_attested",
        "passphrase_mismatch_rejected_before_http_200_attested",
        "poly_address_mismatch_rejected_before_http_200_attested",
        "timestamp_mismatch_rejected_before_http_200_attested",
        "authentication_precedes_closed_only_handler_attested",
        "response_not_shared_or_cache_derived_attested",
        "strict_http_200_implies_live_credential_tuple_acceptance_attested",
        "credential_provider_authorship_attested",
        "credential_delivery_generation_attested",
        "same_loaded_credential_holder_attested",
        "post_load_same_holder_runtime_conjunction_attested",
        "credential_delivery_and_remote_proof_same_source_generation_attested",
        "globally_unique_credential_delivery_attested",
        "rotation_generation_attested",
        "protected_credential_directory_and_four_objects_checked",
        "loaded_credentials_match_delivery_binding",
        "request_l2_tuple_from_same_loaded_credential_holder_checked",
        "selected_actor_generation_bound",
        "product_clock_owner_bound",
        "retained_delivery_evidence_and_load_token_joined_for_selected_actor",
        "private_key_derived_signer_matches_config_checked",
        "l2_credentials_match_configured_signer_checked",
        "signer_controls_proxy_attested",
        "signer_proxy_relationship_current_and_unrevoked_attested",
        "server_time_sample_received",
        "server_time_proof_authenticated_and_fresh",
        "source_owned_current_time_checked",
        "server_time_sample_to_dispatch_freshness_checked",
        "server_time_and_closed_only_same_peer_pairing_checked",
        "proof_observation_freshness_checked",
        "response_receive_freshness_checked",
        "poly_address_header_from_configured_signer_produced",
        "sensitive_request_headers_produced",
        "request_query_body_and_content_type_absence_enforced",
        "request_accept_application_json_header_produced",
        "accept_encoding_identity_header_produced",
        "hmac_preimage_produced",
        "hmac_signature_produced",
        "fixed_local_egress_selected_and_checked",
        "fixed_reviewed_peer_selected_and_checked",
        "network_namespace_and_interface_selected_and_checked",
        "tunnel_or_gateway_profile_checked",
        "live_dns_answer_checked",
        "dnssec_checked",
        "dns_ttl_freshness_checked",
        "destination_nat_equivalence_checked",
        "authorized_public_ip_checked",
        "connect_and_request_timeouts_enforced",
        "authenticated_dispatch_performed_once",
        "redirect_retry_proxy_and_fallback_absence_enforced",
        "response_received",
        "connected_peer_checked_before_status_and_body",
        "tls_server_identity_verified",
        "http_status_200_checked",
        "response_content_type_checked",
        "response_content_encoding_checked",
        "response_body_length_and_exact_schema_checked",
        "closed_only_boolean_observed",
        "closed_only_false_readiness_checked",
        "closed_only_true_hard_block_checked",
        "ambiguous_outcome_durable_burn_performed",
        "live_credential_tuple_accepted_by_provider",
        "credential_tuple_current_and_unrevoked_attested",
        "online_authorization_v2_reverse_pins_remote_policy",
        "reviewed_destination_reverse_pins_remote_policy",
        "reviewed_locator_reverse_pins_remote_policy",
        "fresh_delivery_reverse_pins_remote_policy",
        "reviewed_identity_reverse_pins_remote_policy",
        "remote_policy_fingerprint_pinned_by_online_authorization_v2",
        "remote_policy_fingerprint_pinned_by_v3",
        "remote_policy_consumption_durably_recorded",
        "authorization_consumption_checked",
        "credential_mutation_authority_attested",
    ];
    for claim in false_claims {
        assert!(
            REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1.contains(&format!("{claim}: false")),
            "reviewed remote credential-proof claim is not explicitly false: {claim}"
        );
        assert!(
            !REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1.contains(&format!("{claim}: true")),
            "reviewed remote credential-proof claim became true: {claim}"
        );
    }

    for forbidden in [
        "reqwest::",
        "tokio::net",
        "TcpStream",
        "RequestBuilder",
        "L2Credentials",
        "AuthenticatedL2Headers",
        "PmFixedTlsPeerSelection",
        "PmLocalEgressSelection",
        "PmReadServerTime",
        "reap_polymarket_auth",
        "reap_polymarket_wire",
        "reap_polymarket_egress_binding",
        "hmac::",
        "HmacSha256",
        "read_four(",
        "include_bytes!",
        "fs::read",
        "File::open",
        "pub fn bind_",
        "pub fn into_parts",
        "ProofTokenV1",
        "acceptance_contract_sha256",
        "expected_closed_only",
        "observed_closed_only",
        "production_order_entry_authorized: true",
        "real_order_submission_authorized: true",
    ] {
        assert!(
            !REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1.contains(forbidden),
            "reviewed remote credential-proof source gained forbidden capability or claim `{forbidden}`"
        );
    }

    let acceptance_status = REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1
        .split_once("pub enum ReviewedRemoteCredentialAuthenticationAcceptanceContractStatusV1 {")
        .and_then(|(_, tail)| tail.split_once("\n}").map(|(body, _)| body.trim()))
        .expect("acceptance-contract status enum must remain recognizable");
    assert_eq!(
        acceptance_status,
        "#[serde(rename = \"unavailable_in_frozen_sources_v1\")]\n    UnavailableInFrozenSourcesV1,"
    );

    let hmac_variant = REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1
        .split_once("pub enum ReviewedRemoteCredentialProofHmacPreimageOrderedVariantV1 {")
        .and_then(|(_, tail)| tail.split_once("\n}").map(|(body, _)| body.trim()))
        .expect("HMAC ordered variant enum must remain recognizable");
    assert_eq!(
        hmac_variant,
        "#[serde(rename = \"decimal_timestamp_then_uppercase_get_then_exact_path_no_separators_v1\")]\n    DecimalTimestampThenUppercaseGetThenExactPathNoSeparatorsV1,"
    );

    let response = REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1
        .split_once("pub struct ReviewedRemoteCredentialProofResponsePolicyV1 {")
        .and_then(|(_, tail)| tail.split_once("\n}").map(|(body, _)| body))
        .expect("reviewed remote credential-proof response schema must remain recognizable");
    for forbidden_response_field in [
        "proxy",
        "funder",
        "signer",
        "address",
        "api_key",
        "passphrase",
        "signature",
        "expected",
        "observed",
        "response_bytes",
    ] {
        assert!(
            !response.contains(forbidden_response_field),
            "response schema gained forbidden field `{forbidden_response_field}`"
        );
    }

    let context = REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1
        .split_once("pub struct ReviewedRemoteCredentialProofPolicyContextV1<'a> {")
        .and_then(|(_, tail)| tail.split_once("\n}").map(|(body, _)| body))
        .expect("reviewed remote credential-proof context must remain recognizable");
    for forbidden_context_input in [
        "VerificationV1",
        "String",
        "&str",
        "fingerprint:",
        "Token",
        "EvidenceV1",
    ] {
        assert!(
            !context.contains(forbidden_context_input),
            "reviewed remote credential-proof context gained caller evidence `{forbidden_context_input}`"
        );
    }

    let canonical_surface = REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1
        .split_once("impl CanonicalReviewedRemoteCredentialProofPolicyV1 {")
        .and_then(|(_, tail)| {
            tail.split_once("impl fmt::Debug for CanonicalReviewedRemoteCredentialProofPolicyV1")
                .map(|(surface, _)| surface)
        })
        .expect("canonical remote credential-proof policy surface must remain recognizable");
    for forbidden_projection in [
        "value(",
        "route(",
        "address(",
        "endpoint(",
        "request(",
        "source(",
        "bytes(",
        "path(",
    ] {
        assert!(
            !canonical_surface.contains(forbidden_projection),
            "canonical remote credential-proof holder gained projection `{forbidden_projection}`"
        );
    }

    let declaration = "pub struct CanonicalReviewedRemoteCredentialProofPolicyV1 {";
    let before = REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1
        .split_once(declaration)
        .map(|(before, _)| before)
        .expect("canonical remote credential-proof declaration must remain recognizable");
    let declaration_attributes = before
        .rsplit_once("\n}\n\n")
        .map_or(before, |(_, attributes)| attributes);
    assert!(!declaration_attributes.contains("#[derive"));
    for implementation in REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1.split("\nimpl") {
        let header = implementation
            .split_once('{')
            .map_or(implementation, |(header, _)| header);
        assert!(
            !(header.contains("CanonicalReviewedRemoteCredentialProofPolicyV1")
                && ["Clone", "Copy", "Serialize", "Deserialize"]
                    .iter()
                    .any(|trait_name| header.contains(trait_name))),
            "canonical remote credential-proof holder gained manual Clone, Copy, Serialize, or Deserialize"
        );
    }

    assert!(PROTECTED.contains("ReviewedRemoteCredentialProofPolicyV1"));
    assert!(LIB.contains("mod reviewed_remote_credential_proof_policy_v1;"));
    assert!(!CONFIG.contains("ReviewedRemoteCredentialProofPolicyV1"));
    assert!(!PREFLIGHT.contains("ReviewedRemoteCredentialProofPolicyV1"));
    assert!(!ONLINE_POLICY_V2.contains("ReviewedRemoteCredentialProofPolicyV1"));
    assert!(!ONLINE_CONSUMPTION_V2.contains("ReviewedRemoteCredentialProofPolicyV1"));
    assert!(!REVIEWED_DESTINATION_PROFILE_V1.contains("ReviewedRemoteCredentialProofPolicyV1"));
    assert!(
        !REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1
            .contains("ReviewedRemoteCredentialProofPolicyV1")
    );
    assert!(
        !FRESH_CREDENTIAL_DELIVERY_BINDING_V1.contains("ReviewedRemoteCredentialProofPolicyV1")
    );
    assert!(
        !REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1
            .contains("ReviewedRemoteCredentialProofPolicyV1")
    );
    assert!(!MAIN.contains("ReviewedRemoteCredentialProofPolicyV1"));
}

#[test]
fn reviewed_signer_proxy_account_identity_v1_is_exact_unattested_and_denied_only() {
    for required in [
        "REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_FINGERPRINT_DOMAIN",
        "reap.pm-t2.controlled-trial.reviewed-signer-proxy-account-identity.v1\\0",
        "pm-t2-reviewed-signer-proxy-account-identity-v1.json",
        "reap-pm-controlled-trial-official-sources",
        "2026-08-09T10:17:00Z",
        "PM_T2_OFFICIAL_SOURCE_MANIFEST_BYTE_LENGTH_V1: u64 = 9_103",
        "ebd07e0dfbb7ee0dd825b7b435b303826130761d156e2f23b6c3428f1486e910",
        "pub struct ReviewedOfficialSourceManifestPinsV1",
        "pub schema_family: String,",
        "pub schema_version: u32,",
        "pub retrieved_at_utc: String,",
        "pub byte_length: u64,",
        "pub sha256: String,",
        "self.schema_family != PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_FAMILY_V1",
        "self.byte_length != PM_T2_OFFICIAL_SOURCE_MANIFEST_BYTE_LENGTH_V1",
        "self.sha256 != PM_T2_OFFICIAL_SOURCE_MANIFEST_SHA256_V1",
        "raw pretty-JSON",
        "not publisher-signed Polymarket evidence",
        "neither byte set is supplied for comparison",
        "pub enum ReviewedSignerProxyAccountEvidenceKindV1",
        "#[serde(rename = \"unattested_reviewed_account_source_v1\")]",
        "UnattestedReviewedAccountSourceV1,",
        "pub evidence_kind: ReviewedSignerProxyAccountEvidenceKindV1,",
        "pub evidence_id_label: String,",
        "pub issuer_label: String,",
        "pub source_reference_label: String,",
        "pub observed_at_utc: String,",
        "pub payload_media_type_label: String,",
        "pub payload_byte_length: u64,",
        "pub payload_sha256: String,",
        "pub claimed_account: ReviewedSignerProxyClaimedAccountV1,",
        "pub chain_id: u64,",
        "pub wallet_profile: String,",
        "pub signature_type: u8,",
        "pub signer: String,",
        "pub proxy_funder: String,",
        "self.chain_id != PM_T2_ACCOUNT_CHAIN_ID_V1",
        "self.wallet_profile != PM_T2_ACCOUNT_WALLET_PROFILE_V1",
        "self.signature_type != PM_T2_ACCOUNT_SIGNATURE_TYPE_V1",
        "parse_eip55_address_syntax(&self.signer)",
        "parse_eip55_address_syntax(&self.proxy_funder)",
        "signer == proxy_funder",
        "strict nonzero EIP-55 spelling",
        "proxy field is not asserted to be an EOA",
        "pub reviewer_label: String,",
        "pub v1_config: V1ConfigPinsV2,",
        "pub online_policy: OnlinePolicyPinsV2,",
        "pub online_authorization: ReviewedOnlineAuthorizationPinsV1,",
        "pub official_source_manifest: ReviewedOfficialSourceManifestPinsV1,",
        "pub evidence: UnattestedReviewedSignerProxyAccountEvidenceV1,",
        "manifest_retrieved_at > reviewed_at",
        "evidence_observed_at > reviewed_at",
        "reviewed_at > valid_not_before",
        "valid_not_before >= valid_not_after",
        "validate_online_authorization_contract_v2",
        "identity_times.reviewed_at < authorization_times.reviewed_at",
        "identity_times.reviewed_at > authorization_times.not_before",
        "identity_times.valid_not_before != authorization_times.not_before",
        "identity_times.valid_not_after != authorization_times.cleanup_not_after",
        "official_source_manifest.sha256",
        "config.value().source_pin_manifest_sha256",
        "claimed_account.proxy_funder != configured_account.funder",
        "format!(\"reviewed-account-record:{}\", identity.value.identity_id)",
        "source_reference_label != configured_evidence_reference.as_str()",
        "only exact label",
        "loads no evidence and proves no authorship",
        "schema defines no credential, private-key, API-key",
        "cryptographic signature bytes or material",
        "so callers must",
        "never place secrets or secret-derived",
        "material in them",
        "pub struct CanonicalReviewedSignerProxyAccountIdentityV1",
        "pub fn load_canonical_reviewed_signer_proxy_account_identity_v1(",
        "pub fn verify_reviewed_signer_proxy_account_identity_v1(",
        "without consulting a caller clock",
        "official_source_manifest_bytes_loaded_and_hash_verified: false",
        "reviewed_account_evidence_bytes_loaded_and_hash_verified: false",
        "official_source_manifest_publisher_authorship_attested: false",
        "reviewer_authorship_attested: false",
        "source_authorship_attested: false",
        "issuer_signature_verified: false",
        "evidence_source_tls_and_server_identity_verified: false",
        "signer_on_chain_eoa_status_verified: false",
        "proxy_on_chain_contract_status_verified: false",
        "on_chain_account_state_checked: false",
        "on_chain_finality_checked: false",
        "proxy_factory_semantics_verified: false",
        "signer_controls_proxy_attested: false",
        "signer_proxy_relationship_current: false",
        "signer_proxy_relationship_unrevoked: false",
        "account_specific_evidence_reference_resolved_and_authenticated: false",
        "source_owned_current_time_checked: false",
        "remote_api_key_owner_attested: false",
        "private_key_derived_signer_matches_config_checked: false",
        "l2_credentials_match_configured_signer_checked: false",
        "identity_fingerprint_pinned_by_online_authorization_v2: false",
        "identity_fingerprint_pinned_by_v3: false",
        "identity_consumption_durably_recorded: false",
        "authorization_consumption_checked: false",
        "credential_mutation_authority_attested: false",
        "OfflineAuthorizationState::DENIED",
        "The frozen online",
        "authorization V2 points nowhere to, and does not consume, this sidecar",
        "ProtectedFileKind::ReviewedSignerProxyAccountIdentityV1",
        "CanonicalReviewedSignerProxyAccountIdentityV1(<exact-protected-canonical-bytes; no-value-address-reference-or-path-projection; redacted; denied>)",
    ] {
        assert!(
            REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1.contains(required),
            "missing reviewed signer/proxy account identity V1 source pin `{required}`"
        );
    }

    for forbidden in [
        "reqwest",
        "tokio",
        "TcpStream",
        "connect_async",
        "JsonRpc",
        "L2Credentials",
        "EoaPrivateKeyInput",
        "read_four(",
        "include_bytes!",
        "fs::read",
        "File::open",
        "production_order_entry_authorized: true",
        "real_order_submission_authorized: true",
        "official_source_manifest_bytes_loaded_and_hash_verified: true",
        "reviewed_account_evidence_bytes_loaded_and_hash_verified: true",
        "official_source_manifest_publisher_authorship_attested: true",
        "reviewer_authorship_attested: true",
        "source_authorship_attested: true",
        "issuer_signature_verified: true",
        "evidence_source_tls_and_server_identity_verified: true",
        "signer_on_chain_eoa_status_verified: true",
        "proxy_on_chain_contract_status_verified: true",
        "on_chain_account_state_checked: true",
        "on_chain_finality_checked: true",
        "proxy_factory_semantics_verified: true",
        "signer_controls_proxy_attested: true",
        "signer_proxy_relationship_current: true",
        "signer_proxy_relationship_unrevoked: true",
        "account_specific_evidence_reference_resolved_and_authenticated: true",
        "source_owned_current_time_checked: true",
        "remote_api_key_owner_attested: true",
        "private_key_derived_signer_matches_config_checked: true",
        "l2_credentials_match_configured_signer_checked: true",
        "identity_fingerprint_pinned_by_online_authorization_v2: true",
        "identity_fingerprint_pinned_by_v3: true",
        "identity_consumption_durably_recorded: true",
        "authorization_consumption_checked: true",
        "credential_mutation_authority_attested: true",
        "impl Clone for CanonicalReviewedSignerProxyAccountIdentityV1",
        "impl Serialize for CanonicalReviewedSignerProxyAccountIdentityV1",
        "Deserialize<'de> for CanonicalReviewedSignerProxyAccountIdentityV1",
        "pub fn value(&self)",
        "pub const fn value(&self)",
        "pub fn signer(&self)",
        "pub fn proxy_funder(&self)",
        "pub fn source_reference(&self)",
        "pub fn canonical_bytes(&self)",
        "pub fn path(&self)",
        "pub struct ReviewedSignerProxyAccountIdentityTokenV1",
        "pub fn bind_reviewed_signer_proxy_account_identity_v1(",
    ] {
        assert!(
            !REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1.contains(forbidden),
            "forbidden transport, secret, projection, clone, or authority surface: {forbidden}"
        );
    }

    let evidence_kind = REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1
        .split_once("pub enum ReviewedSignerProxyAccountEvidenceKindV1 {")
        .and_then(|(_, tail)| tail.split_once("\n}").map(|(body, _)| body.trim()))
        .expect("reviewed account evidence-kind enum must remain recognizable");
    assert_eq!(
        evidence_kind,
        "#[serde(rename = \"unattested_reviewed_account_source_v1\")]\n    UnattestedReviewedAccountSourceV1,"
    );

    let claimed_account = REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1
        .split_once("pub struct ReviewedSignerProxyClaimedAccountV1 {")
        .and_then(|(_, tail)| tail.split_once("\n}").map(|(body, _)| body))
        .expect("reviewed account claimed tuple must remain recognizable");
    assert_eq!(
        claimed_account
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>(),
        [
            "pub chain_id: u64,",
            "pub wallet_profile: String,",
            "pub signature_type: u8,",
            "pub signer: String,",
            "pub proxy_funder: String,",
        ]
    );

    let evidence = REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1
        .split_once("pub struct UnattestedReviewedSignerProxyAccountEvidenceV1 {")
        .and_then(|(_, tail)| tail.split_once("\n}").map(|(body, _)| body))
        .expect("reviewed account evidence schema must remain recognizable");
    for forbidden_field in [
        "private_key",
        "api_key",
        "l2_secret",
        "passphrase",
        "hmac",
        "header",
        "signed_body",
        "signature:",
        "payload_bytes",
    ] {
        assert!(
            !evidence.contains(forbidden_field),
            "reviewed account evidence gained forbidden field `{forbidden_field}`"
        );
    }

    let canonical_surface = REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1
        .split_once("impl CanonicalReviewedSignerProxyAccountIdentityV1 {")
        .and_then(|(_, tail)| {
            tail.split_once("impl fmt::Debug for CanonicalReviewedSignerProxyAccountIdentityV1")
                .map(|(surface, _)| surface)
        })
        .expect("canonical reviewed account surface must remain recognizable");
    for forbidden_projection in [
        "value(",
        "signer(",
        "proxy_funder(",
        "reference(",
        "bytes(",
        "path(",
    ] {
        assert!(
            !canonical_surface.contains(forbidden_projection),
            "canonical reviewed account holder gained projection `{forbidden_projection}`"
        );
    }

    let declaration = "pub struct CanonicalReviewedSignerProxyAccountIdentityV1 {";
    let before = REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1
        .split_once(declaration)
        .map(|(before, _)| before)
        .expect("reviewed account canonical declaration must remain recognizable");
    let declaration_attributes = before
        .rsplit_once("\n}\n\n")
        .map_or(before, |(_, attributes)| attributes);
    assert!(!declaration_attributes.contains("#[derive"));
    for implementation in REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1.split("\nimpl") {
        let header = implementation
            .split_once('{')
            .map_or(implementation, |(header, _)| header);
        assert!(
            !(header.contains("CanonicalReviewedSignerProxyAccountIdentityV1")
                && ["Clone", "Copy", "Serialize", "Deserialize"]
                    .iter()
                    .any(|trait_name| header.contains(trait_name))),
            "canonical reviewed account holder gained manual Clone, Copy, Serialize, or Deserialize"
        );
    }

    assert!(PROTECTED.contains("ReviewedSignerProxyAccountIdentityV1"));
    assert!(LIB.contains("mod reviewed_signer_proxy_account_identity_v1;"));
    assert!(!ONLINE_POLICY_V2.contains("ReviewedSignerProxyAccountIdentityV1"));
    assert!(!ONLINE_CONSUMPTION_V2.contains("ReviewedSignerProxyAccountIdentityV1"));
    assert!(!CONFIG.contains("ReviewedSignerProxyAccountIdentityV1"));
    assert!(!PREFLIGHT.contains("ReviewedSignerProxyAccountIdentityV1"));
    assert!(!FRESH_CREDENTIAL_DELIVERY_BINDING_V1.contains("ReviewedSignerProxyAccountIdentityV1"));
    assert!(
        !REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1.contains("ReviewedSignerProxyAccountIdentityV1")
    );
    assert!(!REVIEWED_DESTINATION_PROFILE_V1.contains("ReviewedSignerProxyAccountIdentityV1"));
}

#[test]
fn fresh_credential_delivery_binding_v1_is_unsigned_exact_move_only_and_denied_only() {
    for required in [
        "FRESH_CREDENTIAL_DELIVERY_BINDING_V1_FINGERPRINT_DOMAIN",
        "reap.pm-t2.controlled-trial.fresh-credential-delivery-binding.v1\\0",
        "pm-t2-fresh-credential-delivery-binding-v1.json",
        "pub struct FreshCredentialSlotLocatorPinsV1",
        "pub locator_id: String,",
        "pub canonical_sha256: String,",
        "pub canonical_length: u64,",
        "pub fingerprint: String,",
        "self.locator_id == pins.locator_id",
        "&& self.canonical_sha256 == pins.canonical_sha256",
        "&& self.canonical_length == pins.canonical_length",
        "&& self.fingerprint == pins.fingerprint",
        "struct FreshCredentialSlotLocatorPinsViewV1<'a>",
        "struct FreshCredentialLinuxTimesViewV1",
        "pub struct UnattestedFreshCredentialProviderGenerationV1",
        "pub provider_id: String,",
        "pub provider_key_id: String,",
        "pub rotation_namespace_id: String,",
        "pub delivery_id: String,",
        "pub rotation_generation: u64,",
        "pub struct FreshCredentialLinuxDirectoryIdentityV1",
        "pub struct FreshCredentialLinuxFileIdentityV1",
        "pub struct FreshCredentialLinuxFileIdentitiesV1",
        "pub private_key: FreshCredentialLinuxFileIdentityV1,",
        "pub api_key: FreshCredentialLinuxFileIdentityV1,",
        "pub l2_secret: FreshCredentialLinuxFileIdentityV1,",
        "pub passphrase: FreshCredentialLinuxFileIdentityV1,",
        "pub filesystem_device: u64,",
        "pub inode: u64,",
        "pub owner_uid: u32,",
        "pub permission_mode: u32,",
        "pub hard_link_count: u64,",
        "pub modified_seconds: i64,",
        "pub modified_nanoseconds: i64,",
        "pub status_changed_seconds: i64,",
        "pub status_changed_nanoseconds: i64,",
        "permission_mode != required_permission_mode",
        "self.hard_link_count != 1",
        "file.filesystem_device != self.directory.filesystem_device",
        "file.owner_uid != self.directory.owner_uid",
        "files[left].inode_key() == files[right].inode_key()",
        "pub unattested_delivery_recorded_at_utc: String,",
        "pub unattested_valid_not_before_utc: String,",
        "pub unattested_valid_not_after_utc: String,",
        "binding_times.recorded_at < authorization_times.reviewed_at",
        "binding_times.valid_not_before < authorization_times.not_before",
        "binding_times.valid_not_after > authorization_times.cleanup_not_after",
        "directory.owner_uid != authorization.value().host.linux_euid",
        "not an active placement-window",
        "current-time, provider-freshness, or unrevoked-lease check",
        "future V3",
        "both locator and delivery records by schema/ID/exact",
        "schema defines no designated secret/value, credential-file-length",
        "credential-content-digest field",
        "must never place secret material or",
        "secret-derived values in any label field",
        "there is no provider signature",
        "including its private path",
        "offers no public path projection",
        "Initial combined construction needs no process-local `Arc` correlation",
        "do not prove same-invocation or same-load",
        "provider-owned descriptor delivery",
        "authenticated descriptor-delivery receipt must commit",
        "this exact binding fingerprint and ultimately the exact V3 Prepared",
        "actor/generation audience",
        "generic provider proof is not this",
        "pub struct CanonicalFreshCredentialDeliveryBindingV1",
        "pub struct CanonicalFreshCredentialDeliveryBindingEvidenceV1",
        "pub struct FreshCredentialDeliveryLoadTokenV1",
        "pub fn bind_fresh_credential_delivery_binding_v1(",
        "locator: CanonicalReviewedFreshCredentialSlotLocatorV1,",
        "binding: CanonicalFreshCredentialDeliveryBindingV1,",
        "bind_reviewed_fresh_credential_slot_locator_v1(config, policy, authorization, locator)",
        "pub fn verify_fresh_credential_delivery_binding_evidence_v1(",
        "pub fn into_parts(",
        "ReviewedFreshCredentialLoadTokenV1,",
        "FreshCredentialLinuxObjectSetV1,",
        "source_owned_current_time_checked: false",
        "protected_credential_directory_and_four_files_checked: false",
        "loaded_linux_objects_match_unattested_binding: false",
        "same_loaded_holder_attested: false",
        "globally_unique_delivery_attested: false",
        "provider_authorship_attested: false",
        "provider_signature_verified: false",
        "provider_lease_fresh_and_unrevoked: false",
        "rotation_generation_attested: false",
        "delivery_freshness_attested: false",
        "loaded_bundle_matches_credential_slot_generation: false",
        "remote_api_key_owner_attested: false",
        "locator_fingerprint_pinned_by_v2: false",
        "delivery_binding_fingerprint_pinned_by_v2: false",
        "delivery_consumption_durably_recorded: false",
        "authorization_consumption_checked: false",
        "credential_mutation_authority_attested: false",
        "OfflineAuthorizationState::DENIED",
        "ProtectedFileKind::FreshCredentialDeliveryBindingV1",
        "CanonicalFreshCredentialDeliveryBindingV1(<unsigned-exact-canonical-bytes; redacted; denied>)",
        "CanonicalFreshCredentialDeliveryBindingEvidenceV1(<retained-exact-evidence; no-path-or-provider-projection; redacted; denied>)",
        "FreshCredentialDeliveryLoadTokenV1(<one-shot-local-load; path-signer-and-metadata-redacted; denied>)",
    ] {
        assert!(
            FRESH_CREDENTIAL_DELIVERY_BINDING_V1.contains(required),
            "missing fresh credential delivery binding V1 source pin `{required}`"
        );
    }

    for forbidden in [
        "reqwest",
        "tokio",
        "TcpStream",
        "connect_async",
        "L2Credentials",
        "EoaPrivateKeyInput",
        "CredentialSlotId",
        "authenticated_journal_credential_slot",
        "read_four(",
        "Arc<",
        "production_order_entry_authorized: true",
        "real_order_submission_authorized: true",
        "source_owned_current_time_checked: true",
        "protected_credential_directory_and_four_files_checked: true",
        "loaded_linux_objects_match_unattested_binding: true",
        "same_loaded_holder_attested: true",
        "globally_unique_delivery_attested: true",
        "provider_authorship_attested: true",
        "provider_signature_verified: true",
        "provider_lease_fresh_and_unrevoked: true",
        "rotation_generation_attested: true",
        "delivery_freshness_attested: true",
        "loaded_bundle_matches_credential_slot_generation: true",
        "remote_api_key_owner_attested: true",
        "locator_fingerprint_pinned_by_v2: true",
        "delivery_binding_fingerprint_pinned_by_v2: true",
        "delivery_consumption_durably_recorded: true",
        "authorization_consumption_checked: true",
        "credential_mutation_authority_attested: true",
        "impl Clone for CanonicalFreshCredentialDeliveryBindingV1",
        "impl Serialize for CanonicalFreshCredentialDeliveryBindingV1",
        "Deserialize<'de> for CanonicalFreshCredentialDeliveryBindingV1",
        "impl Clone for CanonicalFreshCredentialDeliveryBindingEvidenceV1",
        "impl Serialize for CanonicalFreshCredentialDeliveryBindingEvidenceV1",
        "Deserialize<'de> for CanonicalFreshCredentialDeliveryBindingEvidenceV1",
        "impl Clone for FreshCredentialDeliveryLoadTokenV1",
        "impl Serialize for FreshCredentialDeliveryLoadTokenV1",
        "Deserialize<'de> for FreshCredentialDeliveryLoadTokenV1",
        "pub const fn value(&self)",
        "pub fn value(&self)",
        "pub fn path(&self)",
        "pub fn provider_id(&self)",
        "pub fn rotation_generation(&self)",
    ] {
        assert!(
            !FRESH_CREDENTIAL_DELIVERY_BINDING_V1.contains(forbidden),
            "forbidden secret, transport, clone, projection, or authority surface: {forbidden}"
        );
    }

    let file_identity = FRESH_CREDENTIAL_DELIVERY_BINDING_V1
        .split_once("pub struct FreshCredentialLinuxFileIdentityV1 {")
        .and_then(|(_, tail)| tail.split_once("\n}").map(|(body, _)| body))
        .expect("fresh credential file identity declaration must remain recognizable");
    for forbidden_field in ["length", "sha256", "hash", "digest", "content", "value"] {
        assert!(
            !file_identity.contains(forbidden_field),
            "credential file identity gained forbidden field `{forbidden_field}`"
        );
    }

    let binder_signature = FRESH_CREDENTIAL_DELIVERY_BINDING_V1
        .split_once("pub fn bind_fresh_credential_delivery_binding_v1(")
        .and_then(|(_, tail)| {
            tail.split_once(") -> Result<")
                .map(|(signature, _)| signature)
        })
        .expect("fresh credential delivery binder signature must remain recognizable");
    for forbidden_input in [
        "EvidenceV1",
        "LoadTokenV1",
        "signer",
        "provider_signature",
        "Arc",
    ] {
        assert!(
            !binder_signature.contains(forbidden_input),
            "fresh credential delivery binder gained forbidden input `{forbidden_input}`"
        );
    }

    for declaration in [
        "pub struct CanonicalFreshCredentialDeliveryBindingV1 {",
        "pub struct CanonicalFreshCredentialDeliveryBindingEvidenceV1 {",
        "pub struct FreshCredentialDeliveryLoadTokenV1 {",
    ] {
        let before = FRESH_CREDENTIAL_DELIVERY_BINDING_V1
            .split_once(declaration)
            .map(|(before, _)| before)
            .expect("delivery binding move-only declaration must remain recognizable");
        let declaration_attributes = before
            .rsplit_once("\n}\n\n")
            .map_or(before, |(_, attributes)| attributes);
        assert!(
            !declaration_attributes.contains("#[derive"),
            "move-only delivery binding type gained a derive: {declaration}"
        );

        let type_name = declaration
            .strip_prefix("pub struct ")
            .and_then(|value| value.strip_suffix(" {"))
            .expect("delivery binding declaration name must remain recognizable");
        for implementation in FRESH_CREDENTIAL_DELIVERY_BINDING_V1.split("\nimpl") {
            let header = implementation
                .split_once('{')
                .map_or(implementation, |(header, _)| header);
            assert!(
                !(header.contains(type_name) && header.contains("Deserialize")),
                "move-only delivery binding type gained manual Deserialize: {type_name}"
            );
        }
    }

    assert!(PROTECTED.contains("FreshCredentialDeliveryBindingV1"));
    assert!(LIB.contains("mod fresh_credential_delivery_binding_v1;"));
    assert!(!ONLINE_POLICY_V2.contains("FreshCredentialDeliveryBindingV1"));
    assert!(!ONLINE_CONSUMPTION_V2.contains("FreshCredentialDeliveryBindingV1"));
    assert!(
        !REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1.contains("FreshCredentialDeliveryBindingV1")
    );
}

#[test]
fn reviewed_fresh_credential_slot_locator_v1_is_exact_additive_and_denied_only() {
    for required in [
        "REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_FINGERPRINT_DOMAIN",
        "reap.pm-t2.controlled-trial.reviewed-fresh-credential-slot-locator.v1\\0",
        "pm-t2-reviewed-fresh-credential-slot-locator-v1.json",
        "pub const PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1: &str = \"private-key\";",
        "pub const PM_T2_FRESH_API_KEY_ENTRY_V1: &str = \"api-key\";",
        "pub const PM_T2_FRESH_L2_SECRET_ENTRY_V1: &str = \"l2-secret\";",
        "pub const PM_T2_FRESH_PASSPHRASE_ENTRY_V1: &str = \"passphrase\";",
        "pub protected_fresh_credential_directory: String,",
        "pub credential_slot_id: String,",
        "pub credential_slot_nonsecret_fingerprint_sha256: String,",
        "validate_online_authorization_contract_v2",
        "locator_times.valid_not_before != authorization_times.not_before",
        "locator_times.valid_not_after != authorization_times.cleanup_not_after",
        "locator.credential_slot_id != config.value().credential_slot.slot_id",
        "nonsecret_fingerprint_sha256",
        "validate_absolute_lexical_directory",
        "normalized.as_os_str() != OsStr::new(value)",
        "without consulting a caller clock",
        "source_owned_current_time_checked: false",
        "protected_credential_directory_and_four_files_checked: false",
        "loaded_bundle_matches_credential_slot_generation: false",
        "remote_api_key_owner_attested: false",
        "locator_fingerprint_pinned_by_v2: false",
        "reviewer_authorship_attested: false",
        "load_token_consumption_durably_recorded: false",
        "authorization_consumption_checked: false",
        "OfflineAuthorizationState::DENIED",
        "does not derive a fingerprint from an API key",
        "protected sidecar and absolute directory remain caller-supplied",
        "unattested by the frozen V2 lineage",
        "they do not attest reviewer",
        "authorship, and `issuing_reviewer` is only a reviewer label",
        "caller cannot select a",
        "non-fixed basename",
        "Reviewer-labeled, non-authenticated",
        "filesystem object currently exists at the locator",
        "future positive gate needs",
        "frozen online-authorization V2 consumption",
        "pub struct CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1",
        "pub struct ReviewedFreshCredentialLoadTokenV1",
        "pub fn bind_reviewed_fresh_credential_slot_locator_v1(",
        "pub fn verify_reviewed_fresh_credential_slot_locator_evidence_v1(",
        "let configured_signer = config.value().account.signer.clone();",
        "pub fn into_parts(self) -> (PathBuf, String)",
        "One-shot local projection capability for one loaded canonical holder",
        "returns ordinary cloneable `PathBuf`",
        "does not prevent their later copying or arbitrary",
        "Actual single-load composition is enforced only when",
        "runner's private staged loader immediately consumes this token",
        "Loading the protected sidecar again can",
        "issue another independently denied",
        "ProtectedFileKind::ReviewedFreshCredentialSlotLocatorV1",
        "CanonicalReviewedFreshCredentialSlotLocatorV1(<reviewed-locator-evidence; exact-canonical-bytes; redacted>)",
    ] {
        assert!(
            REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1.contains(required),
            "missing reviewed fresh credential-slot locator V1 source pin `{required}`"
        );
    }
    for forbidden in [
        "reqwest",
        "tokio",
        "TcpStream",
        "connect_async",
        "L2Credentials",
        "EoaPrivateKeyInput",
        "CredentialSlotId",
        "authenticated_journal_credential_slot",
        ".canonicalize(",
        "symlink_metadata",
        "production_order_entry_authorized: true",
        "real_order_submission_authorized: true",
        "remote_api_key_owner_attested: true",
        "loaded_bundle_matches_credential_slot_generation: true",
        "impl Clone for CanonicalReviewedFreshCredentialSlotLocatorV1",
        "impl Serialize for CanonicalReviewedFreshCredentialSlotLocatorV1",
        "impl Clone for CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1",
        "impl Serialize for CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1",
        "Deserialize<'de> for CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1",
        "impl Clone for ReviewedFreshCredentialLoadTokenV1",
        "impl Serialize for ReviewedFreshCredentialLoadTokenV1",
        "Deserialize<'de> for ReviewedFreshCredentialLoadTokenV1",
        "Deserialize<'de> for CanonicalReviewedFreshCredentialSlotLocatorV1",
        "pub const fn value(&self)",
        "pub fn protected_fresh_credential_directory(&self)",
    ] {
        assert!(
            !REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1.contains(forbidden),
            "forbidden secret, filesystem, transport, or authority surface: {forbidden}"
        );
    }
    assert!(PROTECTED.contains("ReviewedFreshCredentialSlotLocatorV1"));
    assert!(LIB.contains("mod reviewed_fresh_credential_slot_locator_v1;"));
    assert!(!ONLINE_POLICY_V2.contains("ReviewedFreshCredentialSlotLocatorV1"));
    assert!(!ONLINE_CONSUMPTION_V2.contains("ReviewedFreshCredentialSlotLocatorV1"));

    let binder_signature = REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1
        .split_once("pub fn bind_reviewed_fresh_credential_slot_locator_v1(")
        .and_then(|(_, tail)| {
            tail.split_once(") -> Result<")
                .map(|(signature, _)| signature)
        })
        .expect("reviewed locator binder signature must remain recognizable");
    assert!(!binder_signature.contains("signer"));

    let canonical_surface = REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1
        .split_once("impl CanonicalReviewedFreshCredentialSlotLocatorV1 {")
        .and_then(|(_, tail)| {
            tail.split_once("impl fmt::Debug for CanonicalReviewedFreshCredentialSlotLocatorV1")
                .map(|(surface, _)| surface)
        })
        .expect("canonical reviewed locator surface must remain recognizable");
    assert!(!canonical_surface.contains("value("));
    assert!(!canonical_surface.contains("directory("));

    let evidence_surface = REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1
        .split_once("impl CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1 {")
        .and_then(|(_, tail)| {
            tail.split_once(
                "impl fmt::Debug for CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1",
            )
            .map(|(surface, _)| surface)
        })
        .expect("retained reviewed locator evidence surface must remain recognizable");
    assert!(!evidence_surface.contains("value("));
    assert!(!evidence_surface.contains("directory("));

    for declaration in [
        "pub struct CanonicalReviewedFreshCredentialSlotLocatorV1 {",
        "pub struct CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1 {",
        "pub struct ReviewedFreshCredentialLoadTokenV1 {",
    ] {
        let before = REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1
            .split_once(declaration)
            .map(|(before, _)| before)
            .expect("reviewed locator declaration must remain recognizable");
        let declaration_attributes = before
            .rsplit_once("\n}\n\n")
            .map_or(before, |(_, attributes)| attributes);
        assert!(
            !declaration_attributes.contains("#[derive"),
            "move-only reviewed locator type gained a derive: {declaration}"
        );

        let type_name = declaration
            .strip_prefix("pub struct ")
            .and_then(|value| value.strip_suffix(" {"))
            .expect("reviewed locator declaration name must remain recognizable");
        for implementation in REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1.split("\nimpl") {
            let header = implementation
                .split_once('{')
                .map_or(implementation, |(header, _)| header);
            assert!(
                !(header.contains(type_name) && header.contains("Deserialize")),
                "move-only reviewed locator type gained manual Deserialize: {type_name}"
            );
        }
    }
}

#[test]
fn reviewed_destination_v1_is_exact_additive_denied_peer_evidence_only() {
    for required in [
        "REVIEWED_PRODUCTION_DESTINATION_PROFILE_V1_FINGERPRINT_DOMAIN",
        "reap.pm-t2.controlled-trial.reviewed-production-destinations.v1\\0",
        "pm-t2-reviewed-production-destinations-v1.json",
        "MAX_REVIEWED_DNS_ANSWER_AGE_SECONDS_V1: i64 = 300",
        "ReviewerCapturedFixedAnswers",
        "pub peer_ip: String,",
        "pub geoblock_https: ReviewedFixedTlsDestinationV1,",
        "pub clob_https: ReviewedFixedTlsDestinationV1,",
        "pub status_https: ReviewedFixedTlsDestinationV1,",
        "pub data_api_https: ReviewedFixedTlsDestinationV1,",
        "pub polygon_rpc_https: ReviewedFixedTlsDestinationV1,",
        "pub clob_websocket_wss: ReviewedFixedWebSocketDestinationV1,",
        "polymarket.com",
        "clob.polymarket.com",
        "status.polymarket.com",
        "data-api.polymarket.com",
        "polygon.drpc.org",
        "ws-subscriptions-clob.polymarket.com",
        "/ws/market",
        "/ws/user",
        "dns_name != expected_host",
        "tls_server_name != expected_host",
        "http_host != expected_host",
        "tcp_port != REVIEWED_TLS_PORT_V1",
        "validate_online_authorization_contract_v2",
        "profile_times.valid_not_before != authorization_times.not_before",
        "profile_times.valid_not_after != authorization_times.cleanup_not_after",
        ".checked_sub(profile_times.dns_resolved_at.timestamp())",
        "reviewed DNS answers are older than 300 seconds at authorization start",
        "same_address_family(local_source_ip, peer_ip)",
        "is_public_global_unicast_v4",
        "is_public_global_unicast_v6",
        "live_dns_observation_checked: false",
        "destination_nat_equivalence_checked: false",
        "authorization_consumption_checked: false",
        "OfflineAuthorizationState::DENIED",
        "Loading or verifying it performs no DNS lookup and proves no",
        "live DNS answer, DNSSEC result, TTL",
        "must not fall back to DNS or another",
        "profile makes no destination-independent NAT or common-public-IP",
        "validity window neither resets nor extends the V2 status-history quiet",
        "reusable sidecar is not consumed by the frozen V2 ledger",
        "without consulting a caller clock",
        "ProtectedFileKind::ReviewedProductionDestinationProfileV1",
        "CanonicalReviewedProductionDestinationProfileV1(<reviewed-evidence; exact-canonical-bytes>)",
    ] {
        assert!(
            REVIEWED_DESTINATION_PROFILE_V1.contains(required),
            "missing reviewed destination V1 source pin `{required}`"
        );
    }
    for forbidden in [
        "reqwest",
        "tokio",
        "lookup_host",
        "ToSocketAddrs",
        "TcpStream",
        "connect_async",
        "pub peer_ips:",
        "pub destination_independent_nat_assumption:",
        "production_order_entry_authorized: true",
        "real_order_submission_authorized: true",
        "impl Clone for CanonicalReviewedProductionDestinationProfileV1",
        "impl Serialize for CanonicalReviewedProductionDestinationProfileV1",
    ] {
        assert!(
            !REVIEWED_DESTINATION_PROFILE_V1.contains(forbidden),
            "forbidden resolver, transport, NAT, or authority surface: {forbidden}"
        );
    }
    assert_eq!(
        REVIEWED_DESTINATION_PROFILE_V1
            .matches("pub peer_ip: String,")
            .count(),
        2,
        "each destination record shape must expose one scalar peer, never a list",
    );
    assert!(PROTECTED.contains("ReviewedProductionDestinationProfileV1"));
    assert!(LIB.contains("mod reviewed_destination_profile_v1;"));
    assert!(!ONLINE_POLICY_V2.contains("ReviewedProductionDestinationProfileV1"));
    assert!(!ONLINE_CONSUMPTION_V2.contains("ReviewedProductionDestinationProfileV1"));
}

#[test]
fn online_v2_is_a_separate_canonical_denied_evidence_lineage() {
    for required in [
        "ONLINE_POLICY_V2_FINGERPRINT_DOMAIN",
        "ONLINE_AUTHORIZATION_V2_FINGERPRINT_DOMAIN",
        "reap.pm-t2.controlled-trial.online-policy.v2\\0",
        "reap.pm-t2.controlled-trial.online-authorization.v2\\0",
        "CanonicalOnlinePolicyV2(<reviewed-evidence; exact-canonical-bytes>)",
        "CanonicalOnlineAuthorizationV2(<reviewed-evidence; exact-canonical-bytes>)",
        "OfflineAuthorizationState::DENIED",
        "The separate V2 consumption ledger consumes and",
        "fingerprints these exact V2 authorization bytes directly",
        "A later V2 A3",
        "falling back to an arbitrary V1",
        "authorization is forbidden",
        "Perform an offline reviewer/CLI structural display check",
        "`now` is caller supplied",
        "This check cannot establish live freshness or",
        "source-owned current-runtime witness",
        "None of these records asserts that a global matching-engine",
        "restart, restricted mode, or order-admission mode is absent",
    ] {
        assert!(
            ONLINE_POLICY_V2.contains(required),
            "missing online V2 source pin `{required}`"
        );
    }
    for forbidden in [
        "CanonicalTrialPreflight",
        "TrialPreflightEvidence",
        "TrialPreflightBinding",
        "response_sha256",
        "preflight_fingerprint",
        "CanonicalAuthorization",
        "TrialAuthorization",
        "load_canonical_authorization",
        "verify_authorization",
        "impl Clone for CanonicalOnlinePolicyV2",
        "impl Clone for CanonicalOnlineAuthorizationV2",
        "impl Serialize for CanonicalOnlinePolicyV2",
        "impl Serialize for CanonicalOnlineAuthorizationV2",
        "production_order_entry_authorized: true",
        "real_order_submission_authorized: true",
    ] {
        assert!(
            !ONLINE_POLICY_V2.contains(forbidden),
            "forbidden online V2 fallback or authority surface: {forbidden}"
        );
    }
}

#[test]
fn online_v2_pins_exact_runtime_and_status_source_domains() {
    for required in [
        "value.len() > 64",
        "value.trim() != value",
        "value.chars().any(char::is_control)",
        "byte.is_ascii_lowercase() || byte.is_ascii_digit()",
        "self.host.linux_euid == 0",
        "self.host.linux_euid == u32::MAX",
        "MIN_STATUS_NOTICE_HISTORY_QUIET_INTERVAL_SECONDS_V2",
        "MAX_STATUS_NOTICE_HISTORY_QUIET_INTERVAL_SECONDS_V2",
        ".checked_sub(times.history_window_start.timestamp())",
        "history_reviewed_through != reviewed_at",
        "policy_reviewed_at > times.reviewed_at || times.reviewed_at > times.not_before",
        "!= policy.value.reviewed_status_clob_component",
        "config.value().phase != TrialPhase::APlaceCancel",
        "!policy.value.v1_config.matches(config)",
        "pub egress: ReviewedLinuxEgressProfileV2",
        "pub network_namespace_device: u64",
        "pub network_namespace_inode: u64",
        "pub interface_name: String",
        "pub interface_index: u32",
        "pub local_source_ip: String",
        "pub dedicated_tunnel_or_gateway_profile_reference: String",
        "pub dedicated_tunnel_or_gateway_profile_sha256: String",
        "pub destination_independent_nat_assumption: ReviewedDestinationIndependentNatV2",
        "pub authorized_geoblock_reported_public_ip: String",
        "egress.interface_name.len() > 15",
        "egress.interface_index > 2_147_483_647",
        "parsed.is_unspecified()",
        "parsed.is_loopback()",
        "parsed.is_multicast()",
        "validate_public_egress_ip(",
        "is_public_global_unicast_v4",
        "is_public_global_unicast_v6",
        "(a == 100 && (64..=127).contains(&b))",
        "(a == 198 && b == 51 && c == 100)",
        "(a == 203 && b == 0 && c == 113)",
        "segments[0] & 0xe000 == 0x2000",
        "segments[0] == 0x2001 && segments[1] == 0x0db8",
        "segments[0] == 0x3fff",
        "Tunnel-local `local_source_ip` uses the distinct",
        "restrictive validator above and may remain private",
        "reviewed destination-independent NAT assumption",
        "SameLocalEgressSelection",
    ] {
        assert!(
            ONLINE_POLICY_V2.contains(required),
            "missing exact online V2 domain pin `{required}`"
        );
    }
    assert!(!ONLINE_POLICY_V2.contains("pub authorized_egress_ip: String"));
    assert!(!ONLINE_POLICY_V2.contains(".is_global()"));
}

#[test]
fn online_v2_take_once_consumption_is_separate_denied_evidence_only() {
    for required in [
        "ONLINE_AUTHORIZATION_CONSUMPTION_V2_SCHEMA_VERSION",
        "pm-t2-online-authorization-consumption-v2.jsonl",
        "pm-t2-online-authorization-consumed-v2.claim",
        "pm-t2-phase-a-online-preflight-v2.jsonl",
        "reap.pm-t2.controlled-trial.online-authorization-consumption.binding.v2\\0",
        "reap.pm-t2.controlled-trial.online-authorization-consumption.record.v2\\0",
        "reap.pm-t2.controlled-trial.online-authorization-consumption.claim.v2\\0",
        "PreparedOnlineAuthorizationConsumptionV2",
        "ConsumedOnlineAuthorizationConsumptionV2",
        "create_new(",
        "The create-new, fsynced claim is the take-once linearization point",
        "OnlineAuthorizationPlacementReuseV2::PermanentlyBurned",
        "OnlineAuthorizationCrashRecoveryV2::ExistingV1LifecycleOnlyNoPlacementResume",
        "OfflineAuthorizationState::DENIED",
        "SameLocalEgressSelection",
        "validate_online_authorization_contract_v2",
        ".timestamp_millis()",
        ".checked_add(cleanup_runway_ms)",
        "times.cleanup_not_after.timestamp_millis()",
        "pub fn prepare_online_authorization_consumption_v2(",
        "pub fn verify_online_authorization_consumption_v2(",
        "pub fn consume(",
        "revalidate_held_consumption_evidence",
        "refresh_after_bound_artifact_create",
        "ProtectedFileKind::OnlineAuthorizationConsumptionV2",
        "monotonic crash-durability",
        "trusted local storage",
        "post-crash same-EUID actor",
        "TPM counter, WORM storage, or a",
        "trusted remote registry",
        "atomic V2 claim creation may have created its marker; placement is burned",
        "post_create_claim_errors_are_always_reported_as_burned",
    ] {
        assert!(
            ONLINE_CONSUMPTION_V2.contains(required),
            "missing V2 consumption source pin `{required}`"
        );
    }
    for forbidden in [
        "CanonicalAuthorization,",
        "TrialAuthorization",
        "CanonicalTrialPreflight",
        "TrialPreflightEvidence",
        "response_sha256",
        "preflight_fingerprint",
        "claim_prepared_authorization_consumption",
        "reopen_consumed_authorization_consumption",
        "PmRevalidatedPhaseAPlaceLiveDispatchOwnerV1",
        "SignedClobV2Order",
        "AuthenticatedPlaceRequest",
        "production_order_entry_authorized: true",
        "real_order_submission_authorized: true",
        "impl Clone for PreparedOnlineAuthorizationConsumptionV2",
        "impl Clone for ConsumedOnlineAuthorizationConsumptionV2",
        "impl Serialize for PreparedOnlineAuthorizationConsumptionV2",
        "impl Serialize for ConsumedOnlineAuthorizationConsumptionV2",
    ] {
        assert!(
            !ONLINE_CONSUMPTION_V2.contains(forbidden),
            "forbidden V1 fallback or authority in V2 consumption: {forbidden}"
        );
    }
    assert!(!ONLINE_CONSUMPTION_V2.contains("verify_online_authorization_v2(config"));
    assert!(
        !ONLINE_CONSUMPTION_V2
            .contains("_ => invalid(\"atomic V2 consume claim cannot be created safely\")")
    );
    assert!(PROTECTED.contains("OnlineAuthorizationConsumptionV2"));
    assert!(LIB.contains("mod online_consumption_v2;"));
}

#[test]
fn custody_is_move_only_redacted_and_descriptor_pinned() {
    assert!(CUSTODY.contains(
        "pub struct CustodyInspection {\n    _signer: FixedEoaSigner,\n    _l2: L2Credentials,"
    ));
    assert!(!CUSTODY.contains("impl Clone for CustodyInspection"));
    assert!(!CUSTODY.contains("impl Serialize for CustodyInspection"));
    assert!(CUSTODY.contains("FixedEoaSigner"));
    assert!(CUSTODY.contains("L2Credentials"));
    assert!(PROTECTED.contains("libc::O_NOFOLLOW"));
    assert!(PROTECTED.contains("libc::O_CLOEXEC"));
    assert!(PROTECTED.contains("metadata.nlink() != 1"));
    assert!(PROTECTED.contains("metadata.mode() & 0o7777 != 0o600"));
    assert!(PROTECTED.contains("metadata.mode() & 0o7777 != 0o700"));
    assert!(CUSTODY.contains("Zeroizing"));
}

#[test]
fn schemas_are_closed_canonical_and_domain_separated() {
    assert!(CONFIG.matches("#[serde(deny_unknown_fields)]").count() >= 10);
    assert!(CONFIG.contains("canonical != bytes"));
    assert!(CONFIG.contains("CONFIG_FINGERPRINT_DOMAIN"));
    assert!(CONFIG.contains("PLAN_FINGERPRINT_DOMAIN"));
    assert!(CONFIG.contains("AUTHORIZATION_FINGERPRINT_DOMAIN"));
    assert!(LIB.contains("production_order_entry_authorized: false"));
    assert!(LIB.contains("real_order_submission_authorized: false"));
    assert!(LIB.contains("place_dispatch_allowance: 0"));
    assert!(CONFIG.contains("authorization_consumption_checked: false"));
    assert!(!CONFIG.contains("authorization_consumed: false"));
    assert!(PREFLIGHT.contains("#[serde(deny_unknown_fields)]"));
    assert!(PREFLIGHT.contains("canonical_bytes.is_empty()"));
    assert!(PREFLIGHT.contains("reencoded != canonical_bytes"));
    assert!(PREFLIGHT.contains("PREFLIGHT_FINGERPRINT_DOMAIN"));
}

#[test]
fn authorization_v1_host_schema_is_frozen_and_documents_current_linux_uts_binding() {
    for required in [
        "Exact UTF-8 Linux UTS nodename exposed by the current UTS namespace",
        "Runtime binding is byte-for-byte",
        "performs no DNS lookup",
        "FQDN",
        "expansion, case folding",
        "case folding",
        "trailing-dot",
        "`/etc/hostname`",
        "machine-id",
        "cloud instance alias mapping",
        "`/usr/bin/getent passwd <euid>` lookup",
        "NSS",
        "correctness, integrity, and availability dependencies",
        "not executable-release attestation",
        "validate_reference(&self.host.host_identity, \"host identity is invalid\")?;",
    ] {
        assert!(
            CONFIG.contains(required),
            "missing UTS host pin `{required}`"
        );
    }
    let host_binding = CONFIG
        .split_once("pub struct AuthorizationHostBinding {")
        .and_then(|(_, tail)| tail.split_once("\n}").map(|(body, _)| body))
        .expect("authorization host binding");
    let declared_fields = host_binding
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("///"))
        .collect::<Vec<_>>();
    assert_eq!(
        declared_fields.as_slice(),
        &[
            "pub host_identity: String,",
            "pub boot_identity: String,",
            "pub runtime_user: String,",
            "pub egress_identity: String,",
        ],
        "V1 authorization host binding changed its exact serialized fields",
    );
    for forbidden_v2_field in ["effective_user_id:", "wall_capture", "wall_receive_ns:"] {
        assert!(
            !host_binding.contains(forbidden_v2_field),
            "V2 runtime evidence leaked into V1 authorization bytes: {forbidden_v2_field}",
        );
    }
    assert!(CONFIG.contains("pub const TRIAL_CONFIG_SCHEMA_VERSION: u32 = 1;"));
    assert!(CONFIG.contains("pub const TRIAL_AUTHORIZATION_SCHEMA_VERSION: u32 = 1;"));
}

#[test]
fn preflight_is_move_only_redacted_structural_evidence_and_never_a_permit() {
    assert!(PREFLIGHT.contains("pub struct CanonicalTrialPreflight"));
    assert!(!PREFLIGHT.contains("impl Clone for CanonicalTrialPreflight"));
    assert!(!PREFLIGHT.contains("impl Serialize for CanonicalTrialPreflight"));
    assert!(PREFLIGHT.contains("pub const fn production_order_entry_authorized(&self) -> bool"));
    assert!(PREFLIGHT.contains("pub const fn real_order_submission_authorized(&self) -> bool"));
    assert!(PREFLIGHT.contains("pub const fn place_dispatch_allowance(&self) -> u8"));
    assert!(PREFLIGHT.contains("TrialConfiguredPositionState::Absent"));
    for forbidden_private_numeric in [
        "collateral_balance_base_units",
        "configured_token_balance_base_units",
        "pusd_allowance_base_units",
        "position_size_decimal",
    ] {
        assert!(
            !PREFLIGHT.contains(forbidden_private_numeric),
            "durable preflight exposes private numeric state: {forbidden_private_numeric}"
        );
    }
}

#[test]
fn take_once_consumption_is_fixed_path_atomic_durable_and_never_a_permit() {
    assert!(CONFIG.contains("authorization_consumption_ledger_file"));
    assert!(CONFIG.contains("authorization_consumption_claim_file"));
    assert!(CONSUMPTION.contains("fn bound_paths("));
    assert!(!CONSUMPTION.contains("prepare_authorization_consumption(\n    path:"));
    assert!(CONSUMPTION.contains("create_new("));
    assert!(CONSUMPTION.contains("burned_before_dispatch_authority: true"));
    assert!(CONSUMPTION.contains("placement_can_never_resume: true"));
    assert!(CONSUMPTION.contains("crash_allows_recovery_cancel_only: true"));
    assert!(PROTECTED.contains(".create_new(true)"));
    assert!(PROTECTED.contains(".sync_all()"));
    assert!(CONSUMPTION.contains("claim: DurableCreateNewFile"));
    assert!(CONSUMPTION.contains("revalidate_held_consumption_evidence"));
    assert!(CONSUMPTION.contains("reopen_consumed_authorization_consumption"));
    assert!(CONSUMPTION.contains("records.len() < 2"));
    assert!(CONSUMPTION.contains("AuthorizationConsumptionState::Terminal { .. }"));
    assert!(CONSUMPTION.contains("Consumed recovery custody requires its atomic claim"));
    assert!(CONSUMPTION.contains("owner.revalidate_held_consumption_evidence()?"));
    assert!(
        CONSUMPTION.contains("The fsynced consumption ledger is the non-rollback trust boundary")
    );
    assert!(CONSUMPTION.contains("anchor_recovery_continuation_root"));
    assert!(CONSUMPTION.contains("anchor_recovery_cancel_prepared"));
    assert!(CONSUMPTION.contains("continuation_prepared_record_canonical_json"));
    assert!(CONSUMPTION.contains("anchor_recovery_terminal_plan"));
    assert!(CONSUMPTION.contains("RECOVERY_TERMINAL_PLAN_FINGERPRINT_DOMAIN"));
    assert!(CONSUMPTION.contains("continuation_dispatch_terminal_record_canonical_json"));
    assert!(CONSUMPTION.contains("continuation_intent_terminal_record_canonical_json"));
    assert!(CONSUMPTION.contains("recovery preparation cannot follow its Terminal plan"));
    assert!(CONSUMPTION.contains("recovery_cancel_dispatch_budget: u8"));
    assert!(
        CONSUMPTION.contains("base Terminal is forbidden after recovery-continuation anchoring")
    );
    assert!(PROTECTED.contains("fn validate_exact_bytes("));
    assert!(!CONSUMPTION.contains("DispatchAuthorized"));
    assert!(!CONSUMPTION.contains("SignedClobV2Order"));
    assert!(!CONSUMPTION.contains("AuthenticatedPlaceRequest"));
}
