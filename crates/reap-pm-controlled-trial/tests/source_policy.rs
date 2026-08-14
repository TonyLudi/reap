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
const REVIEWED_L1_CREDENTIAL_DERIVATION_PROOF_POLICY_V1: &str =
    include_str!("../src/reviewed_l1_credential_derivation_proof_policy_v1.rs");
const REVIEWED_L1_CREDENTIAL_DERIVATION_PROOF_POLICY_V1_TEST: &str =
    include_str!("reviewed_l1_credential_derivation_proof_policy_v1.rs");
const REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3: &str =
    include_str!("../src/reviewed_static_online_authorization_v3.rs");
const REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4: &str =
    include_str!("../src/reviewed_phase_a_eligibility_envelope_v4.rs");
const REVIEWED_PHASE_A_REVIEWER_TRUST_POLICY_V1: &str =
    include_str!("../src/reviewed_phase_a_reviewer_trust_policy_v1.rs");
const REVIEWED_PHASE_A_REVIEWER_TRUST_POLICY_V1_TEST: &str =
    include_str!("reviewed_phase_a_reviewer_trust_policy_v1.rs");
const REVIEWED_POLY_PROXY_CONTROL_POLICY_V1: &str =
    include_str!("../src/reviewed_poly_proxy_control_policy_v1.rs");
const REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_V1: &str =
    include_str!("../src/reviewed_local_operator_cooperative_custody_profile_v1.rs");
const REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_V1_TEST: &str =
    include_str!("reviewed_local_operator_cooperative_custody_profile_v1.rs");
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
        REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4,
        REVIEWED_PHASE_A_REVIEWER_TRUST_POLICY_V1,
        REVIEWED_POLY_PROXY_CONTROL_POLICY_V1,
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

#[test]
fn phase_a_v4_is_an_exact_offline_eligibility_envelope_and_never_an_approval() {
    for required in [
        "REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4_FINGERPRINT_DOMAIN",
        "reap.pm-t2.controlled-trial.reviewed-phase-a-eligibility-envelope.v4\\0",
        "pm-t2-reviewed-phase-a-eligibility-envelope-v4.json",
        "MAX_CANONICAL_REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_BYTES_V4: usize = 160 * 1024",
        "pub struct ReviewedPhaseAConfigPinsV4",
        "pub struct ReviewedPhaseAV1AuthorizationPinsV4",
        "pub struct ReviewedPhaseAOnlinePolicyPinsV4",
        "pub struct ReviewedPhaseAOnlineAuthorizationPinsV4",
        "pub struct ReviewedPhaseADestinationPinsV4",
        "pub struct ReviewedPhaseALocatorPinsV4",
        "pub struct ReviewedPhaseADeliveryPinsV4",
        "pub struct ReviewedPhaseAAccountIdentityPinsV4",
        "pub struct ReviewedPhaseARemoteProofPolicyPinsV4",
        "pub struct ReviewedPhaseAStaticAuthorizationPinsV4",
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
        "pub struct ReviewedPhaseAEligibilityEnvelopeV4",
        "pub v1_config: ReviewedPhaseAConfigPinsV4,",
        "pub v1_authorization: ReviewedPhaseAV1AuthorizationPinsV4,",
        "pub online_policy_v2: ReviewedPhaseAOnlinePolicyPinsV4,",
        "pub online_authorization_v2: ReviewedPhaseAOnlineAuthorizationPinsV4,",
        "pub reviewed_production_destination_v1: ReviewedPhaseADestinationPinsV4,",
        "pub reviewed_fresh_credential_slot_locator_v1: ReviewedPhaseALocatorPinsV4,",
        "pub fresh_credential_delivery_binding_v1: ReviewedPhaseADeliveryPinsV4,",
        "pub reviewed_signer_proxy_account_identity_v1: ReviewedPhaseAAccountIdentityPinsV4,",
        "pub reviewed_remote_credential_proof_policy_v1: ReviewedPhaseARemoteProofPolicyPinsV4,",
        "pub reviewed_static_online_authorization_v3: ReviewedPhaseAStaticAuthorizationPinsV4,",
        "pub struct ReviewedPhaseAEligibilityEnvelopeContextV4<'a>",
        "pub reviewed_static_online_authorization_v3:",
        "&'a CanonicalReviewedStaticOnlineAuthorizationV3,",
        "pub fn draft_non_authorizing_reviewed_phase_a_eligibility_envelope_v4(",
        "pub fn load_canonical_reviewed_phase_a_eligibility_envelope_v4(",
        "ProtectedFileKind::ReviewedPhaseAEligibilityEnvelopeV4",
        "pub fn verify_reviewed_phase_a_eligibility_envelope_v4(",
        "verify_reviewed_static_online_authorization_v3(",
        "v3.authorization != OfflineAuthorizationState::DENIED",
        "context.v1_authorization.value().phase != TrialPhase::APlaceCancel",
        "online.phase != TrialPhase::APlaceCancel",
        "record.not_before_utc != online.not_before_utc",
        "record.expires_at_utc != online.expires_at_utc",
        "record.cleanup_not_after_utc != online.cleanup_not_after_utc",
        "serde_json::to_vec(context.v1_authorization.value())",
        "OfflineAuthorizationState::DENIED",
        "reviewer_label` is an unauthenticated display label",
        "not a reviewer identity or trust anchor",
        "writes nothing, signs nothing, accepts no external proof",
        "no field for a caller-selected trust digest, public key",
        "does not reinterpret V3",
        "RequiredExternalReviewerTrustAnchorUnavailableV1",
        "required_external_reviewer_trust_anchor_unavailable_v1",
        "RequiredAuthenticatedProviderTrustRootUnavailableV1",
        "required_authenticated_provider_trust_root_unavailable_v1",
        "RequiredProviderSignedAttemptAudienceLeaseUnavailableV1",
        "required_provider_signed_attempt_audience_lease_unavailable_v1",
        "RequiredAuthoritativeRemoteAcceptanceContractUnavailableV1",
        "required_authoritative_remote_acceptance_contract_unavailable_v1",
        "RequiredSameHolderLiveRemoteAcceptanceProofUnavailableV1",
        "required_same_holder_live_remote_acceptance_proof_unavailable_v1",
        "RequiredAuthoritativeSignerProxyControlContractUnavailableV1",
        "required_authoritative_signer_proxy_control_contract_unavailable_v1",
        "RequiredAccountSpecificCurrentUnrevokedControlProofUnavailableV1",
        "required_account_specific_current_unrevoked_control_proof_unavailable_v1",
        "RequiredFutureSelectedActorPreparedLineageUnavailableV1",
        "required_future_selected_actor_prepared_lineage_unavailable_v1",
        "RequiredFutureRetainedEvidenceLoadTokenJoinUnavailableV1",
        "required_future_retained_evidence_load_token_join_unavailable_v1",
        "RequiredFutureCreateNewClaimA3LineageUnavailableV1",
        "required_future_create_new_claim_a3_lineage_unavailable_v1",
        "RequiredFutureSelectedEgressSingleDispatchOwnerUnavailableV1",
        "required_future_selected_egress_single_dispatch_owner_unavailable_v1",
        "CanonicalReviewedPhaseAEligibilityEnvelopeV4(<exact-protected-canonical-bytes; no-value-proof-actor-runtime-or-authority-projection; redacted; denied>)",
    ] {
        assert!(
            REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4.contains(required),
            "missing Phase-A eligibility V4 source pin `{required}`"
        );
    }

    for false_claim in [
        "static_v3_existed_at_v4_review_time_attested",
        "reviewer_trust_anchor_available",
        "reviewer_authorship_attested",
        "credential_provider_trust_root_available",
        "credential_provider_trust_root_authenticated",
        "credential_provider_signature_verified",
        "credential_delivery_lease_verified",
        "credential_delivery_lease_current_and_unrevoked",
        "delivery_generation_attested",
        "authoritative_remote_credential_acceptance_contract_available",
        "same_holder_live_remote_credential_acceptance_proof_verified",
        "live_credential_tuple_accepted_by_provider",
        "authoritative_signer_proxy_control_contract_available",
        "account_specific_signer_proxy_control_proof_verified",
        "signer_controls_proxy_current_and_unrevoked_attested",
        "source_owned_current_time_checked",
        "v1_authorization_current",
        "online_authorization_v2_current",
        "static_online_authorization_v3_current",
        "reviewed_phase_a_eligibility_envelope_v4_current",
        "retained_delivery_evidence_joined",
        "fresh_credential_delivery_load_token_consumed",
        "loaded_credentials_match_delivery_binding",
        "same_loaded_credential_holder_attested",
        "selected_egress_actor_started",
        "selected_actor_generation_bound",
        "source_owned_runtime_attempt_bound",
        "runtime_attempt_fresh",
        "positive_external_proof_bundle_complete",
        "offline_phase_a_eligibility_established",
        "positive_runtime_lineage_implemented",
        "durable_preparation_record_created",
        "atomic_consumption_claim_created",
        "authorization_burn_performed",
        "final_a3_conjunct_durable",
        "burn_and_no_resend_established",
        "authenticated_request_or_hmac_constructed",
        "signed_order_body_constructed",
        "place_dispatch_owner_or_grant_minted",
        "network_dispatch_performed",
        "placement_resumption_allowed",
        "credential_mutation_authority_attested",
    ] {
        assert!(
            REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4.contains(&format!("{false_claim}: false")),
            "Phase-A eligibility V4 claim is not explicitly false: {false_claim}"
        );
        assert!(
            !REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4.contains(&format!("{false_claim}: true")),
            "Phase-A eligibility V4 claim became true: {false_claim}"
        );
    }

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
        "pub fn prepare_",
        "pub fn consume_",
        "pub struct Prepared",
        "pub struct Consumed",
        "pub struct DispatchOwner",
        "pub struct NetworkDispatch",
        "production_order_entry_authorized: true",
        "real_order_submission_authorized: true",
    ] {
        assert!(
            !REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4.contains(forbidden),
            "Phase-A eligibility V4 gained forbidden capability surface `{forbidden}`"
        );
    }

    let record = REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4
        .split_once("pub struct ReviewedPhaseAEligibilityEnvelopeV4 {")
        .and_then(|(_, tail)| tail.split_once("\n}").map(|(body, _)| body))
        .expect("Phase-A eligibility V4 record must remain recognizable");
    for forbidden_field in [
        "reviewer_public_key",
        "reviewer_signature",
        "trust_anchor_sha256",
        "provider_public_key",
        "provider_signature",
        "lease_token",
        "acceptance_contract_sha256",
        "private_key",
        "l2_secret",
        "api_key",
        "passphrase",
        "runtime_nonce",
        "actor_generation",
        "request_body",
        "signed_order",
        "prepared_file",
        "consumption_claim",
    ] {
        assert!(
            !record.contains(forbidden_field),
            "Phase-A eligibility V4 record gained caller trust/capability field `{forbidden_field}`"
        );
    }

    let draft_inputs = REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4
        .split_once("pub struct ReviewedPhaseAEligibilityEnvelopeDraftInputsV4 {")
        .and_then(|(_, tail)| tail.split_once("\n}").map(|(body, _)| body))
        .expect("Phase-A eligibility V4 draft inputs must remain recognizable");
    let draft_fields = draft_inputs
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("pub "))
        .collect::<Vec<_>>();
    assert_eq!(
        draft_fields,
        [
            "pub eligibility_record_id: String,",
            "pub reviewer_label: String,",
            "pub reviewed_at_utc: String,",
            "pub not_before_utc: String,",
            "pub expires_at_utc: String,",
            "pub cleanup_not_after_utc: String,",
        ],
        "draft helper gained trust, proof, runtime, or authorization inputs"
    );

    let canonical_surface = REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4
        .split_once("impl CanonicalReviewedPhaseAEligibilityEnvelopeV4 {")
        .and_then(|(_, tail)| {
            tail.split_once("impl fmt::Debug for CanonicalReviewedPhaseAEligibilityEnvelopeV4")
                .map(|(surface, _)| surface)
        })
        .expect("canonical Phase-A eligibility V4 surface must remain recognizable");
    for forbidden_projection in [
        "value(",
        "bytes(",
        "proof(",
        "actor(",
        "runtime(",
        "authorization(",
        "prepared(",
        "claim(",
        "burn(",
        "dispatch(",
    ] {
        assert!(
            !canonical_surface.contains(forbidden_projection),
            "canonical Phase-A eligibility V4 gained projection `{forbidden_projection}`"
        );
    }

    let declaration = "pub struct CanonicalReviewedPhaseAEligibilityEnvelopeV4 {";
    let before = REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4
        .split_once(declaration)
        .map(|(before, _)| before)
        .expect("canonical Phase-A eligibility V4 declaration must remain recognizable");
    let declaration_attributes = before
        .rsplit_once("\n}\n\n")
        .map_or(before, |(_, attributes)| attributes);
    assert!(!declaration_attributes.contains("#[derive"));
    for implementation in REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4.split("\nimpl") {
        let header = implementation
            .split_once('{')
            .map_or(implementation, |(header, _)| header);
        assert!(
            !(header.contains("CanonicalReviewedPhaseAEligibilityEnvelopeV4")
                && ["Clone", "Copy", "Serialize", "Deserialize"]
                    .iter()
                    .any(|trait_name| header.contains(trait_name))),
            "canonical Phase-A eligibility V4 gained a copy/serialization implementation"
        );
    }

    assert!(PROTECTED.contains("ReviewedPhaseAEligibilityEnvelopeV4"));
    assert!(LIB.contains("mod reviewed_phase_a_eligibility_envelope_v4;"));
    assert!(!MAIN.contains("EligibilityEnvelopeV4"));
    assert!(!MAIN.contains("PhaseAEligibility"));
}

#[test]
fn poly_proxy_control_v1_is_exact_structural_closed_unavailable_and_denied_only() {
    for required in [
        "REVIEWED_POLY_PROXY_CONTROL_POLICY_V1_FINGERPRINT_DOMAIN",
        "reap.pm-t2.controlled-trial.reviewed-poly-proxy-control-policy.v1\\0",
        "pm-t2-reviewed-poly-proxy-control-policy-v1.json",
        "MAX_CANONICAL_REVIEWED_POLY_PROXY_CONTROL_POLICY_BYTES_V1: usize = 128 * 1024",
        "PM_POLY_PROXY_POLYGON_CHAIN_ID_V1: u64 = 137",
        "0xaB45c5A4B0c941a2F231C04C3f49182e1A254052",
        "0x44e999d5c2F66Ef0861317f9A4805AC2e90aEB4f",
        "0xE111180000d2663C0091e4f400237545B87B996B",
        "0xe2222d279d744050d28e00520010520000310F59",
        "PM_POLY_PROXY_INIT_CODE_BYTE_LENGTH_V1: u16 = 167",
        "d21df8dc65880a8606f09fe0ce3df9b8869287ab0b058be05aa9e8af6330a00b",
        "PM_POLY_PROXY_RUNTIME_BYTE_LENGTH_V1: u16 = 45",
        "2fba6fc187f77826faf197b5508d25c98b54581c5123333f1060f2bd87f38b9b",
        "0x734a2a5caf82146a5ddd5263d9af379f9f72724959f0567ddc9df2c40cf2cc20",
        "0x02016836a56b71f0d02689e69e326f4f4c1b9057164ef592671cf0d37c8040c0",
        "arbitrary exact literal observed in",
        "adjacent source comment `keccak256(\"owner\")` is mathematically false",
        "unauthenticated source-comment error",
        "future storage proof at the",
        "exact literal slot must decode to the exact factory, not the signer",
        "Keccak256ExactAbiEncodePackedTwentyByteSignerV1",
        "LowTwentyBytesOfKeccak256FfFactorySaltInitCodeHashV1",
        "abi_padded_thirty_two_byte_signer_input_forbidden",
        "SelectorThenAbiEncodeSingleEmptyBytesValueV1",
        "Eip1167DelegateProxyToExactImplementationV1",
        "ExactLiteralFromProxyWalletLibV1",
        "UnauthenticatedSourceCommentErrorKeccak256OwnerIsFalseV1",
        "ExactFactoryAddressBecauseFactoryCallsInitializeV1",
        "getProxyWalletAddress(address)",
        "getProxyFactory()",
        "getProxyImplementation()",
        "NonemptyEcdsaRecoveredSignerAndExchangeDerivedMakerV1",
        "NotAdmittedWithoutSeparateExactReviewedPinsV1",
        "RequiredButUnavailableV1",
        "RequiredV1",
        "pub exact_source_bytes_and_publisher_authorship: ReviewedPolyProxyFutureProofRequirementV1",
        "pub deployed_code_source_correspondence: ReviewedPolyProxyFutureProofRequirementV1",
        "exact_literal_proxy_owner_slot_storage_proof_equals_factory",
        "StructuralOnlyNoDeploymentStateOrControlClaimV1",
        "LegacyGovernanceAndModulesPreventExclusiveControlSemanticsV1",
        "ccc0596074f4dfd62c944fbca4de252893b82b4b",
        "7137c021e6954d671095f77c94afc3d083d10a84",
        "e19e87da2670a7162ddce1674870fece4521d196",
        "a8cd96a7bfe725c085bc7d4f882519b60e4e6f05",
        "official-source manifest already frozen by PM-T2",
        "does not pin the source bytes or deployed correspondence",
        "deployed-code-to-source correspondence, one trusted finalized Polygon",
        "header/state root, locally verified account/code/storage MPT proofs",
        "selected-exchange code and getter results, an actor-bound fresh",
        "signer challenge, and explicit freshness/reorg handling",
        "permanently prevents this policy from asserting exclusive control",
        "pub struct ReviewedPolyProxyControlPolicyV1",
        "pub struct CanonicalReviewedPolyProxyControlPolicyV1",
        "pub struct ReviewedPolyProxyControlPolicyVerificationV1",
        "pub fn load_canonical_reviewed_poly_proxy_control_policy_v1(",
        "ProtectedFileKind::ReviewedPolyProxyControlPolicyV1",
        "pub fn verify_reviewed_poly_proxy_control_policy_v1(",
        "authorization: OfflineAuthorizationState::DENIED",
        "CanonicalReviewedPolyProxyControlPolicyV1(<exact-protected-canonical-bytes; no-value-signer-maker-proxy-proof-provider-actor-or-authority-projection; redacted; denied>)",
    ] {
        assert!(
            REVIEWED_POLY_PROXY_CONTROL_POLICY_V1.contains(required),
            "missing reviewed Poly proxy control V1 source pin `{required}`"
        );
    }

    for one_to_one_source_evidence_mapping in [
        "exact_source_bytes_and_publisher_authorship",
        "deployed_code_source_correspondence",
    ] {
        assert_eq!(
            REVIEWED_POLY_PROXY_CONTROL_POLICY_V1
                .matches(&format!("pub {one_to_one_source_evidence_mapping}:"))
                .count(),
            2,
            "source evidence must have exactly one unavailable status and one future requirement: {one_to_one_source_evidence_mapping}"
        );
    }

    for false_claim in [
        "official_source_manifest_pins_proxy_sources",
        "exact_source_bytes_loaded_and_hash_verified",
        "source_publisher_authorship_attested",
        "upstream_owner_slot_inline_comment_authenticated",
        "owner_slot_literal_equals_keccak256_utf8_owner",
        "deployed_code_source_correspondence_verified",
        "current_polygon_chain_id_observed",
        "finalized_polygon_header_observed",
        "finalized_polygon_header_authenticated",
        "finalized_polygon_state_root_trusted",
        "finalized_polygon_state_rechecked",
        "same_finalized_state_root_used_for_all_proofs",
        "account_mpt_proofs_locally_verified",
        "code_bytes_loaded_and_keccak_verified",
        "storage_mpt_proofs_locally_verified",
        "factory_account_and_code_verified",
        "implementation_account_and_code_verified",
        "factory_governance_owner_verified",
        "factory_module_state_verified",
        "live_signer_address_bound",
        "live_maker_proxy_address_bound",
        "signer_salt_derived_from_live_signer",
        "proxy_address_derived_for_live_signer",
        "proxy_account_deployed",
        "proxy_runtime_code_verified",
        "proxy_implementation_verified",
        "proxy_owner_storage_proof_verified",
        "proxy_owner_equals_factory_verified",
        "selected_exchange_domain_bound",
        "selected_exchange_account_and_code_verified",
        "selected_exchange_proxy_factory_getter_verified",
        "selected_exchange_proxy_implementation_getter_verified",
        "selected_exchange_get_proxy_wallet_address_call_verified",
        "selected_signature_type_one_checked",
        "type_one_signature_nonempty_checked",
        "type_one_ecdsa_recovered_signer_checked",
        "type_one_derived_proxy_equals_maker_checked",
        "type_two_and_three_exclusion_checked",
        "actor_generation_bound",
        "fresh_signer_challenge_issued",
        "fresh_signer_challenge_signature_verified",
        "signer_private_key_possession_attested",
        "signer_controls_proxy_nonexclusively_attested",
        "signer_exclusively_controls_proxy_attested",
        "factory_governance_exclusivity_absence_attested",
        "signer_proxy_relationship_current_attested",
        "signer_proxy_relationship_unrevoked_attested",
        "factory_and_implementation_current_attested",
        "provider_trust_root_authenticated",
        "provider_authorship_attested",
        "provider_non_equivocation_attested",
        "source_owned_current_time_checked",
        "proof_observation_freshness_checked",
        "finalized_reorg_monitoring_active",
        "stale_or_reorged_proof_invalidation_checked",
        "live_positive_proxy_control_proof_complete",
        "proxy_control_binder_constructed",
        "proxy_control_token_or_permit_minted",
        "credential_mutation_authority_attested",
        "authenticated_request_constructed",
        "signed_order_body_constructed",
        "place_dispatch_owner_or_grant_minted",
        "network_or_rpc_dispatch_performed",
    ] {
        assert!(
            REVIEWED_POLY_PROXY_CONTROL_POLICY_V1.contains(&format!("{false_claim}: false")),
            "Poly proxy control V1 claim is not explicitly false: {false_claim}"
        );
        assert!(
            !REVIEWED_POLY_PROXY_CONTROL_POLICY_V1.contains(&format!("{false_claim}: true")),
            "Poly proxy control V1 claim became true: {false_claim}"
        );
    }

    for forbidden in [
        "reqwest::",
        "tokio::",
        "TcpStream",
        "UdpSocket",
        "JsonRpc",
        "eth_call",
        "Utc::now",
        "SystemTime",
        "Instant::now",
        "OpenOptions",
        "File::create",
        "File::open",
        "fs::write",
        "write_all",
        "DurableCreateNewFile",
        "pub fn bind_",
        "pub fn prepare_",
        "pub fn consume_",
        "pub struct Prepared",
        "pub struct Consumed",
        "pub struct DispatchOwner",
        "pub struct ProxyControlBinder",
        "pub struct ProxyControlToken",
        "pub struct ProxyControlPermit",
        "slot_preimage_utf8",
        "production_order_entry_authorized: true",
        "real_order_submission_authorized: true",
    ] {
        assert!(
            !REVIEWED_POLY_PROXY_CONTROL_POLICY_V1.contains(forbidden),
            "Poly proxy control V1 gained forbidden capability surface `{forbidden}`"
        );
    }

    let canonical_surface = REVIEWED_POLY_PROXY_CONTROL_POLICY_V1
        .split_once("impl CanonicalReviewedPolyProxyControlPolicyV1 {")
        .and_then(|(_, tail)| {
            tail.split_once("impl fmt::Debug for CanonicalReviewedPolyProxyControlPolicyV1")
                .map(|(surface, _)| surface)
        })
        .expect("canonical Poly proxy control V1 surface must remain recognizable");
    for required_projection in [
        "canonical_sha256(&self)",
        "canonical_length(&self)",
        "fingerprint(&self)",
    ] {
        assert!(canonical_surface.contains(required_projection));
    }
    for forbidden_projection in [
        "value(",
        "bytes(",
        "signer(",
        "maker(",
        "proxy(",
        "proof(",
        "provider(",
        "actor(",
        "binder(",
        "token(",
        "permit(",
        "authorization(",
        "dispatch(",
    ] {
        assert!(
            !canonical_surface.contains(forbidden_projection),
            "canonical Poly proxy control V1 gained projection `{forbidden_projection}`"
        );
    }

    let declaration = "pub struct CanonicalReviewedPolyProxyControlPolicyV1 {";
    let before = REVIEWED_POLY_PROXY_CONTROL_POLICY_V1
        .split_once(declaration)
        .map(|(before, _)| before)
        .expect("canonical Poly proxy control V1 declaration must remain recognizable");
    let declaration_attributes = before
        .rsplit_once("\n}\n\n")
        .map_or(before, |(_, attributes)| attributes);
    assert!(!declaration_attributes.contains("#[derive"));
    for implementation in REVIEWED_POLY_PROXY_CONTROL_POLICY_V1.split("\nimpl") {
        let header = implementation
            .split_once('{')
            .map_or(implementation, |(header, _)| header);
        assert!(
            !(header.contains("CanonicalReviewedPolyProxyControlPolicyV1")
                && ["Clone", "Copy", "Serialize", "Deserialize"]
                    .iter()
                    .any(|trait_name| header.contains(trait_name))),
            "canonical Poly proxy control V1 gained copy/serialization implementation"
        );
    }

    let raw_record = REVIEWED_POLY_PROXY_CONTROL_POLICY_V1
        .split_once("pub struct ReviewedPolyProxyControlPolicyV1 {")
        .and_then(|(_, tail)| tail.split_once("\n}").map(|(body, _)| body))
        .expect("raw Poly proxy control V1 record must remain recognizable");
    for forbidden_field in [
        "private_key",
        "api_key",
        "l2_secret",
        "passphrase",
        "rpc_url",
        "block_number",
        "block_hash",
        "state_root_value",
        "proof_nodes",
        "signer_address",
        "maker_address",
        "proxy_address",
        "challenge_bytes",
        "signature_bytes",
        "provider_public_key",
        "provider_signature",
        "runtime_nonce",
        "actor_generation",
        "request_body",
        "dispatch_owner",
    ] {
        assert!(
            !raw_record.contains(forbidden_field),
            "raw Poly proxy policy gained caller/account/proof field `{forbidden_field}`"
        );
    }

    assert!(PROTECTED.contains("ReviewedPolyProxyControlPolicyV1"));
    assert!(LIB.contains("mod reviewed_poly_proxy_control_policy_v1;"));
    assert!(!MAIN.contains("PolyProxyControlPolicyV1"));
    for frozen_prior in [
        CONFIG,
        ONLINE_POLICY_V2,
        REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1,
        REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1,
        REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3,
        REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4,
    ] {
        assert!(!frozen_prior.contains("ReviewedPolyProxyControlPolicyV1"));
    }
}

#[test]
fn phase_a_v4_freezes_exact_upstream_source_paths_for_adversarial_audit() {
    let frozen = [
        (
            "crates/reap-pm-controlled-trial/src/config.rs",
            CONFIG,
            "f3b78a4f18b60c8c4647174b08939e7c239e4b7f58c917777e68e0d43694a533",
        ),
        (
            "crates/reap-pm-controlled-trial/src/consumption.rs",
            CONSUMPTION,
            "f00533077f62b3f27301027c40fe030d917b9405c313ce77218fa7ff26d65c77",
        ),
        (
            "crates/reap-pm-controlled-trial/src/online_policy_v2.rs",
            ONLINE_POLICY_V2,
            "5c3efd9cc1bc6439486628ad65c7e2e85d44dd9746ba6d2bd87ec62bec8647de",
        ),
        (
            "crates/reap-pm-controlled-trial/src/online_consumption_v2.rs",
            ONLINE_CONSUMPTION_V2,
            "842f274357452f9ffd2aec23385afb4b28fbb5ac22603dfbe967d260941d4aeb",
        ),
        (
            "crates/reap-pm-controlled-trial/src/reviewed_destination_profile_v1.rs",
            REVIEWED_DESTINATION_PROFILE_V1,
            "907a2cddae011cef5c92bfe1484942e2d5ca882daaa0799d7a62f6cc3711c967",
        ),
        (
            "crates/reap-pm-controlled-trial/src/reviewed_fresh_credential_slot_locator_v1.rs",
            REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1,
            "cfd3a5089ad7afd11f3e4455187968bd3755456efa8246280005118364dff860",
        ),
        (
            "crates/reap-pm-controlled-trial/src/fresh_credential_delivery_binding_v1.rs",
            FRESH_CREDENTIAL_DELIVERY_BINDING_V1,
            "e84ac2f73a1fc599ee421d5c216a9a1d015a876bff2413770d7f29cd79e370d6",
        ),
        (
            "crates/reap-pm-controlled-trial/src/reviewed_signer_proxy_account_identity_v1.rs",
            REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1,
            "6883ea0ff8a33cc4ebf7fbe79d38bf5519224e026d0ed93a11e09d345a472a44",
        ),
        (
            "crates/reap-pm-controlled-trial/src/reviewed_remote_credential_proof_policy_v1.rs",
            REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1,
            "d5395a6c68c91d8e52091b2a6b965e0b0a7e1b0ad97a8721de958e81955e7f1e",
        ),
        (
            "crates/reap-pm-controlled-trial/src/reviewed_static_online_authorization_v3.rs",
            REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3,
            "e552892d216dba9fc6b596d73ac4d217438091715e6fcf7469e86a68edfd716e",
        ),
        (
            "crates/reap-pm-controlled-trial/src/reviewed_phase_a_eligibility_envelope_v4.rs",
            REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4,
            "34ab2f6ace9023c7f050df9f52bb12f1822b6976e03dc5c6121a2b708c18f53f",
        ),
    ];

    for (path, source, expected_sha256) in frozen {
        use sha2::Digest as _;

        let actual_sha256 = format!("{:x}", sha2::Sha256::digest(source.as_bytes()));
        assert_eq!(
            actual_sha256, expected_sha256,
            "frozen V1/V2/V3/V4 source changed at exact path {path}"
        );
    }
}

#[test]
fn local_operator_cooperative_custody_v1_is_closed_offline_and_denied_only() {
    let source = REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_V1;
    for required in [
        "Offline reviewed local-operator cooperative credential-custody profile V1",
        "future Phase-A V5 may choose",
        "V4 remains byte-for-byte semantically",
        "no atomic unlink-if-name-still-identifies-",
        "no atomic exact-inode",
        "secure erasure",
        "provider origin",
        "global delivery uniqueness",
        "absence of other descriptors",
        "pub struct ReviewedLocalOperatorCooperativeCustodyProfileV1",
        "pub struct ReviewedLocalOperatorCooperativeCustodyProfileContextV1<'a>",
        "pub struct CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1",
        "pub struct ReviewedLocalOperatorCooperativeCustodyProfileVerificationV1",
        "pub fn draft_non_authorizing_reviewed_local_operator_cooperative_custody_profile_v1(",
        "pub fn load_canonical_reviewed_local_operator_cooperative_custody_profile_v1(",
        "pub fn verify_reviewed_local_operator_cooperative_custody_profile_v1(",
        "ProtectedFileKind::ReviewedLocalOperatorCooperativeCustodyProfileV1",
        "OfflineAuthorizationState::DENIED",
        "future_v5_may_select_this_alternative_v4_remains_unchanged_denied_v1",
    ] {
        assert!(
            source.contains(required),
            "local custody profile lost `{required}`"
        );
    }

    for closed_term in [
        "exact_authorized_euid_account_and_all_same_euid_actors_trusted_quiescent_for_complete_window_v1",
        "continuous_advisory_directory_lease_coordination_only_v1",
        "one_held_directory_fd_and_four_held_credential_source_fds_v1",
        "three_named_l2_entries_persist_for_recovery_until_terminal_v1",
        "conditional_expected_basename_removal_directory_fsync_then_absence_observation_under_continuous_lease_v1",
        "no_atomic_unlink_if_name_still_identifies_held_inode_claim_v1",
        "name_removal_and_userspace_drop_are_not_secure_erasure_v1",
        "no_credential_provider_origin_or_authorship_claim_v1",
        "no_global_credential_delivery_uniqueness_claim_v1",
        "no_credential_currentness_claim_v1",
        "no_credential_or_delivery_revocation_state_claim_v1",
        "no_absence_of_other_descriptors_or_credential_copies_claim_v1",
    ] {
        assert!(
            source.contains(closed_term),
            "local custody grammar lost `{closed_term}`"
        );
    }

    for forbidden_runtime_or_mutation in [
        "std::fs::remove_file",
        "OpenOptions",
        "unlink_exact(",
        "unlinkat(",
        "sync_all(",
        "try_lock(",
        "reqwest",
        "hyper",
        "tokio",
        "AuthenticatedPlaceRequest",
        "FixedPlaceRequestSink",
        "place_dispatch_allowance: 1",
    ] {
        assert!(
            !source.contains(forbidden_runtime_or_mutation),
            "offline local custody profile gained runtime/mutation surface `{forbidden_runtime_or_mutation}`"
        );
    }

    for false_fact in [
        "profile_reviewer_authorship_attested: false",
        "source_owned_current_time_checked: false",
        "exact_linux_euid_observed_at_runtime: false",
        "all_same_euid_actors_trusted_and_quiescent_observed: false",
        "advisory_directory_lease_acquired: false",
        "advisory_directory_lease_continuously_held: false",
        "held_directory_descriptor_opened: false",
        "four_held_credential_source_descriptors_opened: false",
        "loaded_linux_objects_match_delivery_binding: false",
        "atomic_unlink_if_inode_attested: false",
        "secure_erasure_attested: false",
        "credential_provider_origin_attested: false",
        "globally_unique_credential_delivery_attested: false",
        "credential_currentness_attested: false",
        "credential_or_delivery_unrevoked_attested: false",
        "no_other_descriptors_or_credential_copies_attested: false",
        "fixed_egress_single_dispatch_owner_minted: false",
        "network_dispatch_performed: false",
        "credential_mutation_authority_attested: false",
    ] {
        assert!(
            source.contains(false_fact),
            "local custody report lost `{false_fact}`"
        );
    }

    let raw_record = source
        .split_once("pub struct ReviewedLocalOperatorCooperativeCustodyProfileV1 {")
        .expect("local custody record declaration")
        .1
        .split_once("impl fmt::Debug for ReviewedLocalOperatorCooperativeCustodyProfileV1")
        .expect("local custody record end")
        .0;
    for forbidden_field in [
        "private_key",
        "api_key",
        "l2_secret",
        "passphrase",
        "protected_fresh_credential_directory",
        "provider_signature",
        "lease_signature",
        "actor_generation",
        "runtime_attempt",
        "request_body",
        "dispatch_owner",
        "permit",
    ] {
        assert!(
            !raw_record.contains(forbidden_field),
            "raw local custody record gained forbidden field `{forbidden_field}`"
        );
    }

    for manual_serde in [
        "impl Serialize for ReviewedLocalOperatorCooperativeCustodyProfileV1",
        "impl<'de> Deserialize<'de> for ReviewedLocalOperatorCooperativeCustodyProfileV1",
        "impl Serialize for ReviewedLocalOperatorCooperativeCustodyTrustV1",
        "impl<'de> Deserialize<'de> for ReviewedLocalOperatorCooperativeCustodyTrustV1",
    ] {
        assert!(
            !source.contains(manual_serde),
            "manual Serde gained `{manual_serde}`"
        );
    }
    assert!(source.contains(
        "#[serde(deny_unknown_fields)]\npub struct ReviewedLocalOperatorCooperativeCustodyProfileV1"
    ));
    assert!(source.contains(
        "#[serde(deny_unknown_fields)]\npub struct ReviewedLocalOperatorCooperativeCustodyTrustV1"
    ));

    assert!(LIB.contains("mod reviewed_local_operator_cooperative_custody_profile_v1;"));
    assert!(LIB.contains("CanonicalReviewedLocalOperatorCooperativeCustodyProfileV1"));
    assert!(PROTECTED.contains("ReviewedLocalOperatorCooperativeCustodyProfileV1"));
    assert!(!MAIN.contains("LocalOperatorCooperativeCustodyProfileV1"));
    assert!(
        !REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4
            .contains("ReviewedLocalOperatorCooperativeCustodyProfileV1")
    );
    assert!(
        !REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3
            .contains("ReviewedLocalOperatorCooperativeCustodyProfileV1")
    );
    assert!(
        !REVIEWED_POLY_PROXY_CONTROL_POLICY_V1
            .contains("ReviewedLocalOperatorCooperativeCustodyProfileV1")
    );
}

#[test]
fn local_operator_cooperative_custody_v1_sources_are_frozen() {
    use sha2::Digest as _;

    for (path, source, expected_sha256) in [
        (
            "crates/reap-pm-controlled-trial/src/reviewed_local_operator_cooperative_custody_profile_v1.rs",
            REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_V1,
            "994630cfb4014630ed6ef6db4082cc244a18d10c2766aacff4c7dcbac3374d99",
        ),
        (
            "crates/reap-pm-controlled-trial/tests/reviewed_local_operator_cooperative_custody_profile_v1.rs",
            REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_V1_TEST,
            "0e63b3a8c87a756c6c36f775411a16c4e060b097cc8ab5f27f09dde49ecc667e",
        ),
    ] {
        let actual_sha256 = format!("{:x}", sha2::Sha256::digest(source.as_bytes()));
        assert_eq!(
            actual_sha256, expected_sha256,
            "frozen local-operator cooperative custody source changed at {path}"
        );
    }
}

#[test]
fn l1_credential_derivation_v1_is_additive_structural_and_permanently_denied() {
    let source = REVIEWED_L1_CREDENTIAL_DERIVATION_PROOF_POLICY_V1;
    for required in [
        "Offline reviewed L1 credential-derivation proof policy V1",
        "GET /auth/derive-api-key",
        "same-holder L2 remote-acceptance policy",
        "does not amend any V1, V2, V3, V4",
        "no-cache response semantics",
        "nonshared response semantics",
        "authentication before handler execution",
        "credential current-and-unrevoked semantics",
        "reap.pm-t2.controlled-trial.reviewed-l1-credential-derivation-proof-policy.v1\\0",
        "pm-t2-reviewed-l1-credential-derivation-proof-policy-v1.json",
        "pub struct ReviewedL1CredentialDerivationProofPolicyV1",
        "pub struct ReviewedL1CredentialDerivationProofPolicyContextV1<'a>",
        "pub struct CanonicalReviewedL1CredentialDerivationProofPolicyV1",
        "pub struct ReviewedL1CredentialDerivationProofPolicyVerificationV1",
        "pub fn draft_non_authorizing_reviewed_l1_credential_derivation_proof_policy_v1(",
        "pub fn load_canonical_reviewed_l1_credential_derivation_proof_policy_v1(",
        "pub fn verify_reviewed_l1_credential_derivation_proof_policy_v1(",
        "ProtectedFileKind::ReviewedL1CredentialDerivationProofPolicyV1",
        "OfflineAuthorizationState::DENIED",
        "credential_derivation_dispatch_allowance: 0",
        "cancel_dispatch_allowance: 0",
        "ClobAuthDomain",
        "ClobAuth(address address,string timestamp,uint256 nonce,string message)",
        "This message attests that I control the given wallet",
        "exact_zero_explicit_reviewed_choice_not_inferred_v1",
        "clob_time_endpoint_or_equivalent_source_owned_proof_required_v1",
        "one_monotonic_origin_timestamp_creation_through_send_and_receive_v1",
        "none_or_exactly_one_charset_utf8_ascii_case_insensitive",
        "absent_or_exactly_one_identity_ascii_case_insensitive",
        "exact_three_strings_no_missing_unknown_duplicate_or_trailing_input_v1",
        "returned_apiKey_exact_local_equality_to_same_loaded_l2_holder_api_key",
        "returned_secret_exact_local_equality_to_same_loaded_l2_holder_secret",
        "returned_passphrase_exact_local_equality_to_same_loaded_l2_holder_passphrase",
    ] {
        assert!(
            source.contains(required),
            "L1 derivation V1 lost `{required}`"
        );
    }

    for pin_type in [
        "ReviewedL1CredentialDerivationConfigPinsV1",
        "ReviewedL1CredentialDerivationOnlinePolicyPinsV1",
        "ReviewedL1CredentialDerivationOnlineAuthorizationPinsV1",
        "ReviewedL1CredentialDerivationDestinationPinsV1",
        "ReviewedL1CredentialDerivationDeliveryPinsV1",
        "ReviewedL1CredentialDerivationAccountIdentityPinsV1",
        "ReviewedL1CredentialDerivationRemoteProofPolicyPinsV1",
        "ReviewedL1CredentialDerivationStaticAuthorizationPinsV1",
        "ReviewedL1CredentialDerivationEligibilityEnvelopePinsV1",
        "ReviewedL1CredentialDerivationLocalCustodyProfilePinsV1",
    ] {
        assert!(
            source.contains(&format!("pub struct {pin_type}")),
            "missing distinct typed role {pin_type}"
        );
    }

    for false_fact in [
        "redirects_allowed",
        "retries_allowed",
        "forward_proxy_allowed",
        "destination_fallback_allowed",
        "official_source_manifest_bytes_loaded",
        "official_source_manifest_hash_verified",
        "api_authentication_source_bytes_loaded",
        "api_authentication_source_hash_verified",
        "official_source_publisher_authorship_attested",
        "policy_reviewer_authorship_attested",
        "response_no_cache_semantics_attested",
        "response_nonshared_semantics_attested",
        "authentication_before_handler_attested",
        "credential_derivation_response_evidence_established",
        "request_constructed",
        "eip712_signature_created",
        "request_signed",
        "request_dispatched",
        "request_received_by_provider",
        "response_received",
        "fixed_socket_created",
        "tls_handshake_observed",
        "tls_server_identity_observed",
        "fixed_reviewed_peer_observed",
        "fixed_local_egress_observed",
        "response_status_observed",
        "response_mime_observed",
        "response_content_type_observed",
        "response_content_encoding_observed",
        "response_body_observed",
        "response_parser_executed",
        "exact_three_field_response_object_observed",
        "returned_tuple_matched_same_loaded_l2_holder",
        "signer_private_key_control_attested",
        "signer_control_current",
        "remote_api_key_owner_attested",
        "later_l2_remote_acceptance_verified",
        "l2_credential_current",
        "l2_credential_unrevoked",
        "same_holder_across_derivation_proof_and_place_attested",
        "credential_provider_origin_attested",
        "credential_provider_delivery_attested",
        "credential_delivery_lease_verified",
        "credential_delivery_lease_current_and_unrevoked",
        "credential_provider_generation_attested",
        "signer_proxy_relationship_attested",
        "signer_proxy_relationship_current",
        "signer_proxy_relationship_unrevoked",
        "selected_actor_bound",
        "source_owned_runtime_attempt_bound",
        "source_clock_owner_bound",
        "source_owned_current_time_observed",
        "source_owned_time_proof_verified",
        "timestamp_creation_observed",
        "monotonic_timestamp_creation_to_send_checked",
        "monotonic_timestamp_creation_to_receive_checked",
        "durable_preparation_record_created",
        "atomic_consumption_claim_created",
        "authorization_burn_performed",
        "durable_no_resend_established",
        "online_authorization_v2_reverse_pin_established",
        "static_online_authorization_v3_reverse_pin_established",
        "phase_a_eligibility_envelope_v4_reverse_pin_established",
        "online_authorization_v2_consumed",
        "static_online_authorization_v3_consumed",
        "phase_a_eligibility_envelope_v4_consumed",
        "online_authorization_v2_current",
        "reviewed_static_online_authorization_v3_current",
        "reviewed_phase_a_eligibility_envelope_v4_current",
        "reviewed_local_operator_cooperative_custody_profile_v1_current",
        "reviewed_l1_credential_derivation_proof_policy_v1_current",
        "place_request_constructed",
        "cancel_request_constructed",
        "hmac_constructed",
        "place_dispatch_owner_or_grant_minted",
        "network_mutation_performed",
        "credential_mutation_authority_attested",
    ] {
        assert!(
            source.contains(&format!("{false_fact}: false")),
            "L1 derivation fact is not explicitly false: {false_fact}"
        );
        assert!(
            !source.contains(&format!("{false_fact}: true")),
            "L1 derivation fact became true: {false_fact}"
        );
    }

    for forbidden_capability_type in [
        "reap_polymarket_auth::",
        "FixedEoaSigner",
        "EoaPrivateKeyInput",
        "AuthenticatedL1CredentialDerivationRequest",
        "L1CredentialDerivationRequestSink",
        "L1CredentialDerivationResponseInput",
        "L1CredentialDerivationMatchedL2Credentials",
        "L2Credentials",
        "AuthenticatedL2Headers",
        "reqwest::",
        "hyper::",
        "tokio::",
        "TcpStream",
        "TlsConnector",
        "RuntimeActor",
        "DurableCreateNewFile",
        "OpenOptions",
        "File::create",
        "File::open",
        "fs::write",
        "write_all",
        "JournalWriter",
        "PreparedAuthorization",
        "ConsumptionClaim",
        "DispatchOwner",
        "MutationCapability",
        "pub fn bind_",
        "pub fn prepare_",
        "pub fn consume_",
        "Utc::now",
        "SystemTime",
        "Instant::now",
        "rand::",
        "getrandom",
    ] {
        assert!(
            !source.contains(forbidden_capability_type),
            "offline L1 derivation policy gained capability type `{forbidden_capability_type}`"
        );
    }

    let expected_positive_structural_spellings = std::collections::BTreeSet::from([
        "connected_peer_check_before_status_and_body_required",
        "content_type_header_required",
        "exact_v1_config_pin_structurally_valid",
        "exact_online_policy_v2_pin_structurally_valid",
        "exact_online_authorization_v2_pin_structurally_valid",
        "exact_reviewed_production_destination_v1_pin_structurally_valid",
        "exact_fresh_credential_delivery_binding_v1_pin_structurally_valid",
        "exact_reviewed_signer_proxy_account_identity_v1_pin_structurally_valid",
        "exact_reviewed_remote_credential_proof_policy_v1_pin_structurally_valid",
        "exact_reviewed_static_online_authorization_v3_pin_structurally_valid",
        "exact_reviewed_phase_a_eligibility_envelope_v4_pin_structurally_valid",
        "exact_reviewed_local_operator_cooperative_custody_profile_v1_pin_structurally_valid",
        "exact_official_manifest_pin_structurally_valid",
        "exact_api_authentication_source_entry_pin_structurally_valid",
        "closed_endpoint_and_route_correlation_grammar_structurally_valid",
        "closed_request_grammar_structurally_valid",
        "closed_eip712_timestamp_and_nonce_grammar_structurally_valid",
        "source_owned_time_requirement_structurally_valid",
        "monotonic_freshness_envelope_structurally_valid",
        "one_request_transport_grammar_structurally_valid",
        "strict_response_grammar_structurally_valid",
        "same_loaded_l2_holder_tuple_association_requirement_structurally_valid",
        "review_time_envelope_nested_within_online_authorization_v2",
    ]);
    let actual_positive_spellings = source
        .lines()
        .filter_map(|line| line.trim().strip_suffix(": true,").map(str::trim))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        actual_positive_spellings, expected_positive_structural_spellings,
        "L1 derivation source gained or lost a positive `true` spelling"
    );

    let raw_record = source
        .split_once("pub struct ReviewedL1CredentialDerivationProofPolicyV1 {")
        .and_then(|(_, tail)| {
            tail.split_once("impl fmt::Debug for ReviewedL1CredentialDerivationProofPolicyV1")
                .map(|(record, _)| record)
        })
        .expect("L1 derivation raw record declaration");
    for forbidden_record_field in [
        "caller_digest",
        "verification_report",
        "verification_boolean",
        "now:",
        "observed_timestamp",
        "signature_bytes",
        "header_bytes",
        "body_bytes",
        "api_uuid",
        "api_key_value",
        "l2_secret_value",
        "passphrase_value",
        "secret_digest",
        "signature_digest",
        "response_hash",
        "response_length",
        "response_value",
        "actor_generation",
        "runtime_attempt",
    ] {
        assert!(
            !raw_record.contains(forbidden_record_field),
            "raw L1 derivation record gained evidence/value field `{forbidden_record_field}`"
        );
    }

    let canonical_surface = source
        .split_once("impl CanonicalReviewedL1CredentialDerivationProofPolicyV1 {")
        .and_then(|(_, tail)| {
            tail.split_once(
                "impl fmt::Debug for CanonicalReviewedL1CredentialDerivationProofPolicyV1",
            )
            .map(|(surface, _)| surface)
        })
        .expect("canonical L1 derivation surface");
    for required_projection in [
        "canonical_sha256(&self)",
        "canonical_length(&self)",
        "fingerprint(&self)",
    ] {
        assert!(canonical_surface.contains(required_projection));
    }
    for forbidden_projection in [
        "value(",
        "bytes(",
        "signature(",
        "header(",
        "body(",
        "credential(",
        "response(",
        "actor(",
        "attempt(",
        "authorization(",
        "dispatch(",
    ] {
        assert!(
            !canonical_surface.contains(forbidden_projection),
            "canonical L1 derivation holder gained projection `{forbidden_projection}`"
        );
    }
    let before_declaration = source
        .split_once("pub struct CanonicalReviewedL1CredentialDerivationProofPolicyV1 {")
        .map(|(before, _)| before)
        .expect("canonical L1 derivation declaration");
    let declaration_attributes = before_declaration
        .rsplit_once("\n}\n\n")
        .map_or(before_declaration, |(_, attributes)| attributes);
    assert!(!declaration_attributes.contains("#[derive"));

    assert!(LIB.contains("mod reviewed_l1_credential_derivation_proof_policy_v1;"));
    assert!(LIB.contains("CanonicalReviewedL1CredentialDerivationProofPolicyV1"));
    assert!(PROTECTED.contains("ReviewedL1CredentialDerivationProofPolicyV1"));
    assert!(!MAIN.contains("L1CredentialDerivationProofPolicyV1"));
}

#[test]
fn l1_derivation_addition_freezes_every_prior_policy_source_at_its_exact_path() {
    use sha2::Digest as _;

    for (path, source, expected_sha256) in [
        (
            "crates/reap-pm-controlled-trial/src/config.rs",
            CONFIG,
            "f3b78a4f18b60c8c4647174b08939e7c239e4b7f58c917777e68e0d43694a533",
        ),
        (
            "crates/reap-pm-controlled-trial/src/online_policy_v2.rs",
            ONLINE_POLICY_V2,
            "5c3efd9cc1bc6439486628ad65c7e2e85d44dd9746ba6d2bd87ec62bec8647de",
        ),
        (
            "crates/reap-pm-controlled-trial/src/reviewed_destination_profile_v1.rs",
            REVIEWED_DESTINATION_PROFILE_V1,
            "907a2cddae011cef5c92bfe1484942e2d5ca882daaa0799d7a62f6cc3711c967",
        ),
        (
            "crates/reap-pm-controlled-trial/src/reviewed_fresh_credential_slot_locator_v1.rs",
            REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1,
            "cfd3a5089ad7afd11f3e4455187968bd3755456efa8246280005118364dff860",
        ),
        (
            "crates/reap-pm-controlled-trial/src/fresh_credential_delivery_binding_v1.rs",
            FRESH_CREDENTIAL_DELIVERY_BINDING_V1,
            "e84ac2f73a1fc599ee421d5c216a9a1d015a876bff2413770d7f29cd79e370d6",
        ),
        (
            "crates/reap-pm-controlled-trial/src/reviewed_signer_proxy_account_identity_v1.rs",
            REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1,
            "6883ea0ff8a33cc4ebf7fbe79d38bf5519224e026d0ed93a11e09d345a472a44",
        ),
        (
            "crates/reap-pm-controlled-trial/src/reviewed_remote_credential_proof_policy_v1.rs",
            REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1,
            "d5395a6c68c91d8e52091b2a6b965e0b0a7e1b0ad97a8721de958e81955e7f1e",
        ),
        (
            "crates/reap-pm-controlled-trial/src/reviewed_static_online_authorization_v3.rs",
            REVIEWED_STATIC_ONLINE_AUTHORIZATION_V3,
            "e552892d216dba9fc6b596d73ac4d217438091715e6fcf7469e86a68edfd716e",
        ),
        (
            "crates/reap-pm-controlled-trial/src/reviewed_phase_a_eligibility_envelope_v4.rs",
            REVIEWED_PHASE_A_ELIGIBILITY_ENVELOPE_V4,
            "34ab2f6ace9023c7f050df9f52bb12f1822b6976e03dc5c6121a2b708c18f53f",
        ),
        (
            "crates/reap-pm-controlled-trial/src/reviewed_local_operator_cooperative_custody_profile_v1.rs",
            REVIEWED_LOCAL_OPERATOR_COOPERATIVE_CUSTODY_PROFILE_V1,
            "994630cfb4014630ed6ef6db4082cc244a18d10c2766aacff4c7dcbac3374d99",
        ),
        (
            "crates/reap-pm-controlled-trial/src/reviewed_poly_proxy_control_policy_v1.rs",
            REVIEWED_POLY_PROXY_CONTROL_POLICY_V1,
            "ed216aff43a07f6eecb4fb296cb4ed703fe9392e47a9a1f6c2cfd12c5d5902bd",
        ),
    ] {
        let actual_sha256 = format!("{:x}", sha2::Sha256::digest(source.as_bytes()));
        assert_eq!(
            actual_sha256, expected_sha256,
            "frozen prerequisite source changed at exact path {path}"
        );
    }
}

#[test]
fn l1_derivation_policy_and_integration_sources_have_exact_audit_hashes() {
    use sha2::Digest as _;

    for (path, source, expected_sha256) in [
        (
            "crates/reap-pm-controlled-trial/src/reviewed_l1_credential_derivation_proof_policy_v1.rs",
            REVIEWED_L1_CREDENTIAL_DERIVATION_PROOF_POLICY_V1,
            "809b4881ddd0945ef513aed1345c5322f66b5755820ffa4c76b2d239c3a30a7f",
        ),
        (
            "crates/reap-pm-controlled-trial/tests/reviewed_l1_credential_derivation_proof_policy_v1.rs",
            REVIEWED_L1_CREDENTIAL_DERIVATION_PROOF_POLICY_V1_TEST,
            "eeb5a1c13e41c2ad64e4a0f92a9d7448d04f8f1727a58dc47865db528bb1927b",
        ),
    ] {
        let actual_sha256 = format!("{:x}", sha2::Sha256::digest(source.as_bytes()));
        assert_eq!(
            actual_sha256, expected_sha256,
            "L1 derivation policy audit source changed at exact path {path}"
        );
    }
}

#[test]
fn phase_a_reviewer_trust_policy_v1_is_protected_structural_and_denied_only() {
    let source = REVIEWED_PHASE_A_REVIEWER_TRUST_POLICY_V1;
    for required in [
        "Protected canonical Phase-A reviewer trust policy prerequisite V1",
        "pm-t2-reviewed-phase-a-reviewer-trust-policy-v1.json",
        "pm-t2-reviewed-phase-a-reviewer-trust-policy-v1",
        "reap.pm-t2.controlled-trial.reviewed-phase-a-reviewer-trust-policy.v1\\0",
        "offline_reviewer_trust_prerequisite_only_no_authorization_v1",
        "phase_a_reviewer_v1",
        "MAX_PHASE_A_AUTHORIZATION_TTL_SECONDS_V1: u64 = 900",
        "MIN_PHASE_A_REVIEWER_APPROVERS_V1: usize = 1",
        "MAX_PHASE_A_REVIEWER_APPROVERS_V1: usize = 8",
        "REQUIRED_PHASE_A_REVIEWER_QUORUM_V1: u8 = 1",
        "signature_algorithm: ReviewedPhaseAReviewerTrustAlgorithmV1",
        "maximum_authorization_ttl_seconds: u64",
        "required_reviewer_roles: [ReviewedPhaseARequiredReviewerRoleV1; 1]",
        "required_distinct_approver_quorum: u8",
        "approver_id: String",
        "role: ReviewedPhaseARequiredReviewerRoleV1",
        "key_id: String",
        "public_key_base64url_no_pad: String",
        "URL_SAFE_NO_PAD",
        "decoded.len() != 32",
        "decoded.iter().all(|byte| *byte == 0)",
        "URL_SAFE_NO_PAD.encode(&decoded) != value",
        ".windows(2)",
        "approver_ids.insert",
        "key_ids.insert",
        "public_keys.insert",
        "ProtectedFileKind::ReviewedPhaseAReviewerTrustPolicyV1",
        "pub fn load_canonical_reviewed_phase_a_reviewer_trust_policy_v1(",
        "pub fn verify_reviewed_phase_a_reviewer_trust_policy_v1(",
        "pub(crate) fn value(&self)",
        "OfflineAuthorizationState::DENIED",
        "CanonicalReviewedPhaseAReviewerTrustPolicyV1(<exact-protected-canonical-bytes; no-value-id-role-key-path-signature-runtime-or-authority-projection; redacted; denied>)",
    ] {
        assert!(
            source.contains(required),
            "reviewer trust policy source lost `{required}`"
        );
    }

    for false_fact in [
        "policy_reviewer_authorship_attested",
        "approver_identity_attested",
        "approver_role_assignment_attested",
        "corresponding_private_key_possession_attested",
        "reviewer_key_custody_attested",
        "reviewer_keys_current",
        "reviewer_keys_unrevoked",
        "ed25519_public_keys_cryptographically_validated",
        "source_owned_current_time_checked",
        "authorization_request_current",
        "authorization_request_bound",
        "authorization_signature_verified",
        "authorization_quorum_satisfied",
        "independent_human_review_attested",
        "runtime_candidate_bound",
        "runtime_actor_generation_bound",
        "runtime_attempt_bound",
        "permit_minted",
        "authenticated_request_or_hmac_constructed",
        "network_dispatch_performed",
        "credential_mutation_authority_attested",
    ] {
        assert!(
            source.contains(&format!("{false_fact}: false")),
            "reviewer trust policy fact is not explicitly false: {false_fact}"
        );
        assert!(
            !source.contains(&format!("{false_fact}: true")),
            "reviewer trust policy fact became true: {false_fact}"
        );
    }

    let expected_positive_structural_spellings = std::collections::BTreeSet::from([
        "bounded_unique_approvers_structurally_valid",
        "canonical_unique_32_byte_public_key_encodings_structurally_valid",
        "closed_phase_a_reviewer_role_structurally_valid",
        "ed25519_only_algorithm_structurally_valid",
        "exact_one_reviewer_quorum_structurally_valid",
        "exact_policy_id_and_record_role_structurally_valid",
        "maximum_authorization_ttl_at_most_900_seconds_structurally_valid",
        "unique_approver_ids_and_key_ids_structurally_valid",
    ]);
    let actual_positive_spellings = source
        .lines()
        .filter_map(|line| line.trim().strip_suffix(": true,").map(str::trim))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        actual_positive_spellings, expected_positive_structural_spellings,
        "reviewer trust policy source gained or lost a positive `true` spelling"
    );

    for forbidden_capability in [
        "pub fn draft_",
        "ring::",
        "ed25519_dalek",
        "SigningKey",
        "FixedEoaSigner",
        "EoaPrivateKeyInput",
        "L2Credentials",
        "AuthenticatedPlaceRequest",
        "FixedPlaceRequestSink",
        "reqwest::",
        "hyper::",
        "tokio::",
        "TcpStream",
        "std::net",
        "Utc::now",
        "SystemTime",
        "Instant::now",
        "rand::",
        "getrandom",
        "OpenOptions",
        "File::create",
        "write_all",
        "PreparedAuthorization",
        "ConsumptionClaim",
        "DispatchOwner",
        "MutationCapability",
        "production_order_entry_authorized: true",
        "real_order_submission_authorized: true",
        "place_dispatch_allowance: 1",
    ] {
        assert!(
            !source.contains(forbidden_capability),
            "reviewer trust policy gained capability `{forbidden_capability}`"
        );
    }

    let canonical_surface = source
        .split_once("impl CanonicalReviewedPhaseAReviewerTrustPolicyV1 {")
        .and_then(|(_, tail)| {
            tail.split_once("impl fmt::Debug for CanonicalReviewedPhaseAReviewerTrustPolicyV1")
                .map(|(surface, _)| surface)
        })
        .expect("canonical reviewer trust policy surface");
    for required_projection in [
        "pub fn canonical_sha256(&self)",
        "pub fn canonical_length(&self)",
        "pub fn fingerprint(&self)",
        "pub(crate) fn value(&self)",
    ] {
        assert!(canonical_surface.contains(required_projection));
    }
    for forbidden_public_projection in [
        "pub fn value(",
        "pub fn bytes(",
        "pub fn policy_id(",
        "pub fn approver(",
        "pub fn role(",
        "pub fn key(",
        "pub fn signature(",
        "pub fn request(",
        "pub fn permit(",
        "pub fn authorization(",
    ] {
        assert!(
            !canonical_surface.contains(forbidden_public_projection),
            "canonical reviewer trust holder gained projection `{forbidden_public_projection}`"
        );
    }
    let before_declaration = source
        .split_once("pub struct CanonicalReviewedPhaseAReviewerTrustPolicyV1 {")
        .map(|(before, _)| before)
        .expect("canonical reviewer trust holder declaration");
    let declaration_attributes = before_declaration
        .rsplit_once("\n}\n\n")
        .map_or(before_declaration, |(_, attributes)| attributes);
    assert!(!declaration_attributes.contains("#[derive"));

    assert!(MANIFEST.contains("base64.workspace = true"));
    assert!(LIB.contains("mod reviewed_phase_a_reviewer_trust_policy_v1;"));
    assert!(LIB.contains("CanonicalReviewedPhaseAReviewerTrustPolicyV1"));
    assert!(LIB.contains("ReviewedPhaseAReviewerTrustPolicyVerificationV1"));
    assert!(!LIB.contains("ReviewedPhaseAReviewerApproverV1"));
    assert!(!LIB.contains("ReviewedPhaseARequiredReviewerRoleV1"));
    assert!(PROTECTED.contains("ReviewedPhaseAReviewerTrustPolicyV1"));
    assert!(!MAIN.contains("PhaseAReviewerTrustPolicyV1"));
}

#[test]
fn phase_a_reviewer_trust_policy_v1_integration_covers_closed_boundary() {
    let test = REVIEWED_PHASE_A_REVIEWER_TRUST_POLICY_V1_TEST;
    for required in [
        "exact_policy_has_a_stable_golden_vector_and_is_structural_denied_only",
        "loader_rejects_malformed_duplicate_unknown_trailing_and_noncanonical_bytes",
        "identity_algorithm_role_ttl_and_quorum_are_closed",
        "approvers_are_bounded_sorted_and_unique_by_identity_key_id_and_public_key",
        "ed25519_public_key_encoding_is_exact_nonzero_32_byte_base64url_without_padding",
        "policy_requires_protected_file_custody_and_projects_no_policy_values",
        "c1a586149c188d0cdd4ba0ebb3a3a08e9b865c4faf1d13f9b9fa5b41fc3d3cfc",
        "d7a133e95f8392ebffd27069a6b880a3b3d251aea48e3dec274292577844f4b9",
        "maximum_authorization_ttl_seconds\":901",
        "required_distinct_approver_quorum\":2",
        "duplicate-public-key.json",
        "duplicate-key-id.json",
        "too-many.json",
        "padded.json",
        "all-zero.json",
        "noncanonical-tail.json",
        "symlink(&target, &symbolic)",
        "fs::hard_link(&target, &hard)",
        "Permissions::from_mode(0o644)",
        "Permissions::from_mode(0o755)",
        "OfflineAuthorizationState::DENIED",
    ] {
        assert!(
            test.contains(required),
            "reviewer trust test lost `{required}`"
        );
    }
}

#[test]
fn phase_a_reviewer_trust_policy_v1_sources_have_exact_audit_hashes() {
    use sha2::Digest as _;

    for (path, source, expected_sha256) in [
        (
            "crates/reap-pm-controlled-trial/src/reviewed_phase_a_reviewer_trust_policy_v1.rs",
            REVIEWED_PHASE_A_REVIEWER_TRUST_POLICY_V1,
            "54fca25dcf7c2286aff63717ff0b6d25fd080d732aea4033ef7049453005bcea",
        ),
        (
            "crates/reap-pm-controlled-trial/tests/reviewed_phase_a_reviewer_trust_policy_v1.rs",
            REVIEWED_PHASE_A_REVIEWER_TRUST_POLICY_V1_TEST,
            "91ee203821f08a73b5b1434322ffb3a918c17868e5fa26eb633ed95b24e5dceb",
        ),
    ] {
        let actual_sha256 = format!("{:x}", sha2::Sha256::digest(source.as_bytes()));
        assert_eq!(
            actual_sha256, expected_sha256,
            "reviewer trust policy audit source changed at exact path {path}"
        );
    }
}
