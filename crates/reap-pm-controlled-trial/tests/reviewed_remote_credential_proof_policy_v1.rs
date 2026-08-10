use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
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
    PM_T2_REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_FILE_V1,
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
    ReviewedRemoteCredentialProofPolicyContextV1, ReviewedRemoteCredentialProofPolicyV1,
    ReviewedRemoteCredentialProofProtocolPolicyV1, ReviewedRemoteCredentialProofRequestPolicyV1,
    ReviewedRemoteCredentialProofResponsePolicyV1,
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
    verify_reviewed_remote_credential_proof_policy_v1,
};
use tempfile::TempDir;

const SIGNER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const FUNDER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const REVIEWED_CLOB_COMPONENT_NAME: &str = "  Trading API (CLOB)";
const REVIEWED_CREDENTIAL_DIRECTORY: &str = "/run/reap/pm-t2/credentials/pm-t2-slot-1";
const REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_GOLDEN_CANONICAL_JSON: &str = concat!(
    r#"{"schema_version":1,"policy_id":"pm-t2-reviewed-remote-credential-proof-policy-v1","reviewer_label":"operator-reviewer","reviewed_at_utc":"2026-08-09T12:00:00Z","valid_not_before_utc":"2026-08-09T12:00:00Z","valid_not_after_utc":"2026-08-09T12:20:00Z","v1_config":{"canonical_config_sha256":"23056742d14de855cd263e14a0e4549fb5d172f1a71683f525863c9cb92e465f","canonical_config_length":2450,"canonical_config_fingerprint":"f2b3c0ec4445c7c02a8cd2ee692edc47f0564c74c398ff6253e0959e31063df7","trial_plan_fingerprint":"c422f334503711dac5c91174e8f6f2274f54a71cebace3ea8297a85ff3afd008"},"#,
    r#""online_policy":{"canonical_sha256":"8656a900122f2278e187d28e86503168577c3ab660813b907c009f0ca022649d","canonical_length":1278,"fingerprint":"58a8a69165a6690dc2394b43d7e96e12f8016cb47fe39aa65104c91df4800589"},"#,
    r#""online_authorization":{"authorization_id":"pm-t2-online-authorization-v2","canonical_sha256":"20966b61e04a40b90daccf97085082bbba7657699a32eb6c67fc868d4997938c","canonical_length":2663,"fingerprint":"e880375bda260b557c8c8360fba2678500c37c4e46f6aaaa29a07ec478e81ccb"},"#,
    r#""reviewed_destination":{"schema_version":1,"profile_id":"pm-t2-reviewed-production-destinations-v1","canonical_sha256":"dc3c0dc61ca669c8e0f6390608e9efa6873139965de567bade87f632685e612f","canonical_length":2370,"fingerprint":"d3323ab9e5b8a839451521d7d380f1461c5ee818dc549889a04f3eed855e50f0"},"#,
    r#""reviewed_fresh_credential_locator":{"schema_version":1,"locator_id":"pm-t2-reviewed-fresh-credential-slot-locator-v1","canonical_sha256":"a52ed7c70e681fff9ae747513c29db43aa9cf2084085597f70f0b9648930aacc","canonical_length":1422,"fingerprint":"52db4cb5f58e7e1cd830a7d16e5b0c737271b16e2f07c2968d9f328d5e447e3b"},"#,
    r#""fresh_credential_delivery":{"schema_version":1,"binding_id":"pm-t2-fresh-credential-delivery-binding-v1","canonical_sha256":"cf7526d00a1c24543ab9319060a10cf4fbda4652247bf79eb0222991e38e838e","canonical_length":1939,"fingerprint":"74276425237900ab146e7a43b9824c6adadcfb6ca930193b4e3fa0f485fc4726"},"#,
    r#""reviewed_signer_proxy_identity":{"schema_version":1,"identity_id":"pm-t2-account-v1","canonical_sha256":"c0f4bc76d0cf57218ab66bd79793cdc6608d9f48f415c84040a17e87a37901b3","canonical_length":1887,"fingerprint":"c6a1fdb9b16522568ee6efbf99ad092f3f82d66f745ece1ee1df92eb80357800"},"#,
    r#""official_sources":{"manifest":{"schema_family":"reap-pm-controlled-trial-official-sources","schema_version":1,"retrieved_at_utc":"2026-08-09T10:17:00Z","byte_length":9103,"sha256":"ebd07e0dfbb7ee0dd825b7b435b303826130761d156e2f23b6c3428f1486e910"},"#,
    r#""api_authentication":{"id":"api_authentication","requested_url":"https://docs.polymarket.com/getting-started/api.md","final_url":"https://docs.polymarket.com/getting-started/api.md","retrieved_at_utc":"2026-08-09T10:17:00Z","content_type":"text/markdown; charset=utf-8","byte_length":9391,"sha256":"6c397c66109852220b3f5d8033ea274061b3fc44b426edc9faa60673ecbef8fc"},"#,
    r#""manage_orders":{"id":"manage_orders","requested_url":"https://docs.polymarket.com/trading/manage-orders.md","final_url":"https://docs.polymarket.com/trading/manage-orders.md","retrieved_at_utc":"2026-08-09T10:17:00Z","content_type":"text/markdown; charset=utf-8","byte_length":52401,"sha256":"e4a0238db31d5137b4d0da0d4333b1fb90be8f7c7b47d92968edfd993c8c4482"}},"#,
    r#""authentication_acceptance_contract_status":"unavailable_in_frozen_sources_v1","protocol":{"endpoint":{"scheme":"https","dns_name":"clob.polymarket.com","tls_server_name":"clob.polymarket.com","http_host":"clob.polymarket.com","tcp_port":443,"selected_peer_ip":"1.1.1.1","network_namespace_device":4,"network_namespace_inode":4026531999,"interface_name":"wg0","interface_index":7,"local_egress_ip":"10.0.0.2","dedicated_tunnel_or_gateway_profile_reference":"reviewed-egress:dedicated-wg0-v2","dedicated_tunnel_or_gateway_profile_sha256":"abababababababababababababababababababababababababababababababab"},"#,
    r#""request":{"method":"GET","path":"/auth/ban-status/closed-only","query":"absent","body":"absent","content_type":"absent","accept":"application/json","accept_encoding":"identity","sensitive_header_names":{"poly_address":"POLY_ADDRESS","poly_signature":"POLY_SIGNATURE","poly_timestamp":"POLY_TIMESTAMP","poly_api_key":"POLY_API_KEY","poly_passphrase":"POLY_PASSPHRASE"},"#,
    r#""hmac_preimage":{"hmac_algorithm":"hmac_sha256","hmac_key_source":"same_loaded_l2_credential_holder_decoded_url_safe_base64_secret_bytes","l2_secret_input_encoding":"rfc4648_url_safe_base64_with_padding_canonical","maximum_l2_secret_encoded_bytes":172,"minimum_hmac_key_bytes":1,"maximum_hmac_key_bytes":128,"ordered_variant":"decimal_timestamp_then_uppercase_get_then_exact_path_no_separators_v1","timestamp_component":"fresh_clob_server_unix_seconds_decimal_ascii","timestamp_decimal_digits":10,"minimum_timestamp_unix_seconds":1000000000,"maximum_timestamp_unix_seconds":9999999999,"method_component":"GET","path_component":"/auth/ban-status/closed-only","separator":"none","query_component":"absent","body_component":"absent","poly_address_component":"excluded_from_preimage_header_only","poly_api_key_component":"excluded_from_preimage_header_only","poly_passphrase_component":"excluded_from_preimage_header_only","l2_secret_role":"hmac_key_only_not_preimage","signature_encoding":"rfc4648_url_safe_base64_with_padding","signature_decoded_length":32,"signature_encoded_length":44,"signature_terminal_padding":"="},"poly_address_source":"canonical_config_signer_eip55","poly_timestamp_source":"same_canonical_l2_timestamp_as_hmac_preimage","poly_signature_source":"exact_hmac_output","poly_api_key_source":"same_loaded_l2_credential_holder","poly_passphrase_source":"same_loaded_l2_credential_holder"},"#,
    r#""freshness":{"server_time_path":"/time","server_time_sample_required":true,"server_time_sample_must_precede_authenticated_dispatch":true,"maximum_server_time_sample_to_dispatch_age_ms":5000,"maximum_proof_observation_age_ms":5000},"#,
    r#""dispatch":{"maximum_authenticated_dispatch_count":1,"connect_timeout_ms":3000,"request_timeout_ms":5000,"redirects_allowed":false,"retries_allowed":false,"forward_proxy_allowed":false,"destination_fallback_allowed":false,"connected_peer_check_before_status_and_body_required":true,"ambiguous_outcome_requires_durable_burn":true},"#,
    r#""response":{"required_status_code":200,"content_type_header_required":true,"required_content_type_essence":"application/json","allowed_content_type_charset":"none_or_single_charset_utf8_ascii_case_insensitive","allowed_content_encoding":"absent_or_single_identity_ascii_case_insensitive","maximum_body_bytes":64,"required_json_object_field_count":1,"required_json_field_name":"closed_only","required_json_field_type":"boolean","allowed_json_boolean_values":"true_or_false_shape_only","closed_only_false_semantics":"placement_candidate_evaluated_separately","closed_only_true_semantics":"hard_block","authentication_semantics":"neither_value_proves_authentication_acceptance_or_failure"}}}"#,
);
const REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_GOLDEN_CANONICAL_LENGTH: u64 = 6_841;
const REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_GOLDEN_CANONICAL_SHA256: &str =
    "9566e5867e660c97fda8076527611177605e660f342d14a24ca4af45e5927497";
const REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_GOLDEN_FINGERPRINT: &str =
    "c4e060a2960436001d6e3da2701f0bdb01ad5cd1e6da80b2c8ad16d1a62ee462";

#[test]
fn exact_remote_policy_is_canonical_nonprojecting_and_permanently_denied() {
    let fixture = Fixture::new();
    let record = reviewed_remote_policy(&fixture);
    let path = fixture.write_json(
        PM_T2_REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_FILE_V1,
        &record,
    );
    let reviewed_policy = load_canonical_reviewed_remote_credential_proof_policy_v1(&path).unwrap();
    let context = fixture.context();
    let verification =
        verify_reviewed_remote_credential_proof_policy_v1(&context, &reviewed_policy).unwrap();

    assert!(verification.exact_config_policy_authorization_pins_structurally_valid);
    assert!(verification.exact_destination_locator_delivery_identity_pins_structurally_valid);
    assert!(verification.exact_official_source_manifest_and_entries_structurally_valid);
    assert!(verification.exact_closed_only_protocol_policy_structurally_valid);
    assert!(verification.validity_envelope_nested_within_online_authorization_v2);
    assert!(verification.selected_peer_and_local_egress_labels_match_bound_records);
    assert_all_runtime_and_authority_claims_false(&verification);
    assert_ne!(
        reviewed_policy.canonical_sha256(),
        reviewed_policy.fingerprint()
    );
    assert_eq!(
        reviewed_policy.canonical_length(),
        fs::metadata(path).unwrap().len()
    );
    assert_eq!(
        format!("{record:?}"),
        "ReviewedRemoteCredentialProofPolicyV1(<reviewer-labeled-protocol-and-source-pins; route-address-and-request-redacted; acceptance-contract-unavailable; denied>)"
    );
    assert_eq!(
        format!("{reviewed_policy:?}"),
        "CanonicalReviewedRemoteCredentialProofPolicyV1(<exact-protected-canonical-bytes; no-value-route-address-source-or-request-projection; redacted; denied>)"
    );

    let display = serde_json::to_string(&verification).unwrap();
    for nonprojecting_detail in [
        SIGNER,
        FUNDER,
        "1.1.1.1",
        "10.0.0.2",
        "/auth/ban-status/closed-only",
        "https://docs.polymarket.com/getting-started/api.md",
        "6c397c66109852220b3f5d8033ea274061b3fc44b426edc9faa60673ecbef8fc",
        "POLY_ADDRESS",
    ] {
        assert!(
            !display.contains(nonprojecting_detail),
            "verification projected `{nonprojecting_detail}`"
        );
    }

    fn assert_send<T: Send>() {}
    assert_send::<reap_pm_controlled_trial::CanonicalReviewedRemoteCredentialProofPolicyV1>();
}

#[test]
fn loader_rejects_duplicate_unknown_noncanonical_unavailable_status_and_unprotected_bytes() {
    let fixture = Fixture::new();
    let record = reviewed_remote_policy(&fixture);
    let canonical = serde_json::to_vec(&record).unwrap();

    let mut duplicate = b"{\"schema_version\":1,".to_vec();
    duplicate.extend_from_slice(&canonical[1..]);
    assert_policy_bytes_rejected(&duplicate);

    let mut unknown = canonical[..canonical.len() - 1].to_vec();
    unknown.extend_from_slice(b",\"proof_token\":\"forbidden\"}");
    assert_policy_bytes_rejected(&unknown);

    let mut trailing = canonical.clone();
    trailing.push(b'\n');
    assert_policy_bytes_rejected(&trailing);

    let mut reordered: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
    let object = reordered.as_object_mut().unwrap();
    let schema = object.remove("schema_version").unwrap();
    object.insert("schema_version".into(), schema);
    let reordered = serde_json::to_vec(&reordered).unwrap();
    assert_ne!(reordered, canonical);
    assert_policy_bytes_rejected(&reordered);

    let mut wrong_status = serde_json::to_value(&record).unwrap();
    wrong_status["authentication_acceptance_contract_status"] =
        serde_json::json!("caller_digest_supplied");
    assert_policy_bytes_rejected(&serde_json::to_vec(&wrong_status).unwrap());

    let directory = protected_dir();
    let path = directory.path().join("wrong-mode.json");
    fs::write(&path, canonical).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(load_canonical_reviewed_remote_credential_proof_policy_v1(&path).is_err());
}

#[test]
fn official_sources_and_closed_protocol_are_exact_and_intrinsically_strict() {
    let fixture = Fixture::new();
    let base = reviewed_remote_policy(&fixture);
    let intrinsic_mutations: [fn(&mut ReviewedRemoteCredentialProofPolicyV1); 38] = [
        |policy| policy.official_sources.manifest.byte_length += 1,
        |policy| policy.official_sources.api_authentication.byte_length += 1,
        |policy| policy.official_sources.manage_orders.sha256 = "aa".repeat(32),
        |policy| policy.protocol.endpoint.scheme = "http".into(),
        |policy| policy.protocol.endpoint.tls_server_name = "other.example".into(),
        |policy| policy.protocol.request.method = "POST".into(),
        |policy| policy.protocol.request.query = "present".into(),
        |policy| policy.protocol.request.body = "present".into(),
        |policy| policy.protocol.request.content_type = "application/json".into(),
        |policy| policy.protocol.request.accept_encoding = "gzip".into(),
        |policy| policy.protocol.request.hmac_preimage.hmac_algorithm = "hmac_sha512".into(),
        |policy| {
            policy.protocol.request.hmac_preimage.hmac_key_source = "caller_secret_bytes".into()
        },
        |policy| {
            policy
                .protocol
                .request
                .hmac_preimage
                .l2_secret_input_encoding = "unpadded_base64".into()
        },
        |policy| {
            policy
                .protocol
                .request
                .hmac_preimage
                .maximum_l2_secret_encoded_bytes = 173
        },
        |policy| policy.protocol.request.hmac_preimage.minimum_hmac_key_bytes = 0,
        |policy| {
            policy
                .protocol
                .request
                .hmac_preimage
                .timestamp_decimal_digits = 9
        },
        |policy| {
            policy
                .protocol
                .request
                .hmac_preimage
                .minimum_timestamp_unix_seconds -= 1
        },
        |policy| policy.protocol.request.hmac_preimage.separator = ":".into(),
        |policy| {
            policy.protocol.request.hmac_preimage.poly_address_component =
                "included_in_preimage".into()
        },
        |policy| {
            policy.protocol.request.hmac_preimage.l2_secret_role =
                "serialized_preimage_component".into()
        },
        |policy| policy.protocol.request.hmac_preimage.signature_encoding = "hex".into(),
        |policy| {
            policy
                .protocol
                .request
                .hmac_preimage
                .signature_decoded_length = 31
        },
        |policy| {
            policy
                .protocol
                .request
                .hmac_preimage
                .signature_encoded_length = 43
        },
        |policy| {
            policy
                .protocol
                .request
                .hmac_preimage
                .signature_terminal_padding = "none".into()
        },
        |policy| policy.protocol.request.poly_timestamp_source = "different_timestamp".into(),
        |policy| policy.protocol.request.poly_api_key_source = "different_holder".into(),
        |policy| {
            policy
                .protocol
                .freshness
                .maximum_server_time_sample_to_dispatch_age_ms += 1
        },
        |policy| policy.protocol.freshness.maximum_proof_observation_age_ms += 1,
        |policy| {
            policy
                .protocol
                .dispatch
                .maximum_authenticated_dispatch_count = 2
        },
        |policy| policy.protocol.dispatch.retries_allowed = true,
        |policy| {
            policy
                .protocol
                .dispatch
                .ambiguous_outcome_requires_durable_burn = false
        },
        |policy| policy.protocol.response.required_status_code = 201,
        |policy| policy.protocol.response.content_type_header_required = false,
        |policy| policy.protocol.response.allowed_content_type_charset = "any".into(),
        |policy| policy.protocol.response.allowed_content_encoding = "gzip".into(),
        |policy| policy.protocol.response.required_json_object_field_count = 2,
        |policy| policy.protocol.response.closed_only_false_semantics = "auth-success".into(),
        |policy| policy.protocol.response.authentication_semantics = "http-200-proves-auth".into(),
    ];
    for mutate in intrinsic_mutations {
        let mut record = base.clone();
        mutate(&mut record);
        assert_policy_record_rejected(&record);
    }

    let time_mutations: [fn(&mut ReviewedRemoteCredentialProofPolicyV1); 3] = [
        |policy| policy.reviewed_at_utc = "2026-08-09T10:16:59Z".into(),
        |policy| policy.reviewed_at_utc = "2026-08-09T12:00:01Z".into(),
        |policy| policy.valid_not_after_utc = "2026-08-09T12:00:00Z".into(),
    ];
    for mutate in time_mutations {
        let mut record = base.clone();
        mutate(&mut record);
        assert_policy_record_rejected(&record);
    }
}

#[test]
fn verifier_rejects_every_typed_artifact_pin_and_selected_route_drift() {
    let fixture = Fixture::new();
    let base = reviewed_remote_policy(&fixture);
    let mutations: [fn(&mut ReviewedRemoteCredentialProofPolicyV1); 14] = [
        |policy| policy.v1_config.canonical_config_sha256 = "a1".repeat(32),
        |policy| policy.online_policy.fingerprint = "b2".repeat(32),
        |policy| policy.online_authorization.fingerprint = "c3".repeat(32),
        |policy| policy.reviewed_destination.profile_id = "other-profile".into(),
        |policy| policy.reviewed_destination.fingerprint = "d4".repeat(32),
        |policy| policy.reviewed_fresh_credential_locator.locator_id = "other-locator".into(),
        |policy| policy.reviewed_fresh_credential_locator.canonical_length += 1,
        |policy| policy.fresh_credential_delivery.binding_id = "other-binding".into(),
        |policy| policy.fresh_credential_delivery.canonical_sha256 = "e5".repeat(32),
        |policy| policy.reviewed_signer_proxy_identity.identity_id = "other-identity".into(),
        |policy| policy.protocol.endpoint.selected_peer_ip = "1.1.1.2".into(),
        |policy| policy.protocol.endpoint.local_egress_ip = "10.0.0.3".into(),
        |policy| policy.protocol.endpoint.interface_index = 8,
        |policy| {
            policy
                .protocol
                .endpoint
                .dedicated_tunnel_or_gateway_profile_sha256 = "f6".repeat(32)
        },
    ];
    for (index, mutate) in mutations.into_iter().enumerate() {
        let mut record = base.clone();
        mutate(&mut record);
        let path = fixture.write_json(&format!("drift-{index}.json"), &record);
        let reviewed_policy =
            load_canonical_reviewed_remote_credential_proof_policy_v1(&path).unwrap();
        assert!(
            verify_reviewed_remote_credential_proof_policy_v1(
                &fixture.context(),
                &reviewed_policy,
            )
            .is_err()
        );
    }

    let mut early_review = base;
    early_review.reviewed_at_utc = "2026-08-09T11:59:59Z".into();
    let path = fixture.write_json("before-authorization-review.json", &early_review);
    let reviewed_policy = load_canonical_reviewed_remote_credential_proof_policy_v1(&path).unwrap();
    assert!(
        verify_reviewed_remote_credential_proof_policy_v1(&fixture.context(), &reviewed_policy,)
            .is_err()
    );
}

#[test]
fn schema_contains_only_policy_grammar_and_enumerates_both_boolean_shapes() {
    let fixture = Fixture::new();
    let value = serde_json::to_value(reviewed_remote_policy(&fixture)).unwrap();
    let request = value["protocol"]["request"].as_object().unwrap();
    assert_eq!(request["query"], "absent");
    assert_eq!(request["body"], "absent");
    assert_eq!(request["content_type"], "absent");
    assert_eq!(request["accept_encoding"], "identity");
    let hmac = request["hmac_preimage"].as_object().unwrap();
    assert_eq!(hmac["hmac_algorithm"], "hmac_sha256");
    assert_eq!(
        hmac["hmac_key_source"],
        "same_loaded_l2_credential_holder_decoded_url_safe_base64_secret_bytes"
    );
    assert_eq!(
        hmac["l2_secret_input_encoding"],
        "rfc4648_url_safe_base64_with_padding_canonical"
    );
    assert_eq!(hmac["maximum_l2_secret_encoded_bytes"], 172);
    assert_eq!(hmac["minimum_hmac_key_bytes"], 1);
    assert_eq!(hmac["maximum_hmac_key_bytes"], 128);
    assert_eq!(hmac["timestamp_decimal_digits"], 10);
    assert_eq!(hmac["minimum_timestamp_unix_seconds"], 1_000_000_000u64);
    assert_eq!(hmac["maximum_timestamp_unix_seconds"], 9_999_999_999u64);
    assert_eq!(
        hmac["signature_encoding"],
        "rfc4648_url_safe_base64_with_padding"
    );
    assert_eq!(hmac["signature_decoded_length"], 32);
    assert_eq!(hmac["signature_encoded_length"], 44);
    assert_eq!(hmac["signature_terminal_padding"], "=");
    assert_eq!(
        request["poly_timestamp_source"],
        "same_canonical_l2_timestamp_as_hmac_preimage"
    );
    assert_eq!(request["poly_signature_source"], "exact_hmac_output");
    assert_eq!(
        request["poly_api_key_source"],
        "same_loaded_l2_credential_holder"
    );
    assert_eq!(
        request["poly_passphrase_source"],
        "same_loaded_l2_credential_holder"
    );
    assert_eq!(
        request["sensitive_header_names"]
            .as_object()
            .unwrap()
            .values()
            .map(|value| value.as_str().unwrap())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "POLY_ADDRESS",
            "POLY_API_KEY",
            "POLY_PASSPHRASE",
            "POLY_SIGNATURE",
            "POLY_TIMESTAMP",
        ])
    );
    let response = value["protocol"]["response"].as_object().unwrap();
    assert_eq!(response["content_type_header_required"], true);
    assert_eq!(
        response["allowed_content_type_charset"],
        "none_or_single_charset_utf8_ascii_case_insensitive"
    );
    assert_eq!(
        response["allowed_content_encoding"],
        "absent_or_single_identity_ascii_case_insensitive"
    );
    assert_eq!(
        response["allowed_json_boolean_values"],
        "true_or_false_shape_only"
    );
    assert_eq!(
        response["closed_only_false_semantics"],
        "placement_candidate_evaluated_separately"
    );
    assert_eq!(response["closed_only_true_semantics"], "hard_block");
    assert_eq!(
        response["authentication_semantics"],
        "neither_value_proves_authentication_acceptance_or_failure"
    );
    assert_eq!(
        value["authentication_acceptance_contract_status"],
        "unavailable_in_frozen_sources_v1"
    );

    let canonical = serde_json::to_string(&value).unwrap();
    for forbidden in [
        "private_key_value",
        "api_key_value",
        "l2_secret_value",
        "passphrase_value",
        "signature_bytes",
        "header_bytes",
        "hmac_value",
        "request_body_bytes",
        "acceptance_contract_sha256",
        "expected_closed_only",
        "observed_closed_only",
        "proof_token",
    ] {
        assert!(!canonical.contains(forbidden));
    }
}

#[test]
fn reviewed_remote_credential_proof_policy_v1_has_stable_golden_vector() {
    let fixture = Fixture::new();
    let record = reviewed_remote_policy(&fixture);
    let canonical = serde_json::to_vec(&record).unwrap();
    assert_eq!(
        canonical.as_slice(),
        REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_GOLDEN_CANONICAL_JSON.as_bytes()
    );
    assert_eq!(
        canonical.len() as u64,
        REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_GOLDEN_CANONICAL_LENGTH
    );
    let path = fixture.write_json("golden-remote-policy.json", &record);
    let reviewed_policy = load_canonical_reviewed_remote_credential_proof_policy_v1(&path).unwrap();
    assert_eq!(
        reviewed_policy.canonical_length(),
        REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_GOLDEN_CANONICAL_LENGTH
    );
    assert_eq!(
        reviewed_policy.canonical_sha256(),
        REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_GOLDEN_CANONICAL_SHA256
    );
    assert_eq!(
        reviewed_policy.fingerprint(),
        REVIEWED_REMOTE_CREDENTIAL_PROOF_POLICY_V1_GOLDEN_FINGERPRINT
    );
}

fn assert_all_runtime_and_authority_claims_false(
    verification: &reap_pm_controlled_trial::ReviewedRemoteCredentialProofPolicyVerificationV1,
) {
    assert!(!verification.official_source_manifest_bytes_loaded_and_hash_verified);
    assert!(!verification.api_authentication_source_bytes_loaded_and_hash_verified);
    assert!(!verification.manage_orders_source_bytes_loaded_and_hash_verified);
    assert!(!verification.official_source_publisher_authorship_attested);
    assert!(!verification.official_source_manifest_publisher_authorship_attested);
    assert!(!verification.api_authentication_source_publisher_authorship_attested);
    assert!(!verification.manage_orders_source_publisher_authorship_attested);
    assert!(!verification.reviewer_authorship_attested);
    assert!(!verification.remote_api_key_owner_attested);
    assert!(!verification.authoritative_authentication_acceptance_contract_available);
    assert!(!verification.api_key_mismatch_rejected_before_http_200_attested);
    assert!(!verification.l2_secret_mismatch_rejected_before_http_200_attested);
    assert!(!verification.passphrase_mismatch_rejected_before_http_200_attested);
    assert!(!verification.poly_address_mismatch_rejected_before_http_200_attested);
    assert!(!verification.timestamp_mismatch_rejected_before_http_200_attested);
    assert!(!verification.authentication_precedes_closed_only_handler_attested);
    assert!(!verification.response_not_shared_or_cache_derived_attested);
    assert!(!verification.strict_http_200_implies_live_credential_tuple_acceptance_attested);
    assert!(!verification.credential_provider_authorship_attested);
    assert!(!verification.credential_delivery_generation_attested);
    assert!(!verification.same_loaded_credential_holder_attested);
    assert!(!verification.post_load_same_holder_runtime_conjunction_attested);
    assert!(!verification.credential_delivery_and_remote_proof_same_source_generation_attested);
    assert!(!verification.globally_unique_credential_delivery_attested);
    assert!(!verification.rotation_generation_attested);
    assert!(!verification.protected_credential_directory_and_four_objects_checked);
    assert!(!verification.loaded_credentials_match_delivery_binding);
    assert!(!verification.request_l2_tuple_from_same_loaded_credential_holder_checked);
    assert!(!verification.selected_actor_generation_bound);
    assert!(!verification.product_clock_owner_bound);
    assert!(!verification.retained_delivery_evidence_and_load_token_joined_for_selected_actor);
    assert!(!verification.private_key_derived_signer_matches_config_checked);
    assert!(!verification.l2_credentials_match_configured_signer_checked);
    assert!(!verification.signer_controls_proxy_attested);
    assert!(!verification.signer_proxy_relationship_current_and_unrevoked_attested);
    assert!(!verification.server_time_sample_received);
    assert!(!verification.server_time_proof_authenticated_and_fresh);
    assert!(!verification.source_owned_current_time_checked);
    assert!(!verification.server_time_sample_to_dispatch_freshness_checked);
    assert!(!verification.server_time_and_closed_only_same_peer_pairing_checked);
    assert!(!verification.proof_observation_freshness_checked);
    assert!(!verification.response_receive_freshness_checked);
    assert!(!verification.poly_address_header_from_configured_signer_produced);
    assert!(!verification.sensitive_request_headers_produced);
    assert!(!verification.request_query_body_and_content_type_absence_enforced);
    assert!(!verification.request_accept_application_json_header_produced);
    assert!(!verification.accept_encoding_identity_header_produced);
    assert!(!verification.hmac_preimage_produced);
    assert!(!verification.hmac_signature_produced);
    assert!(!verification.fixed_local_egress_selected_and_checked);
    assert!(!verification.fixed_reviewed_peer_selected_and_checked);
    assert!(!verification.network_namespace_and_interface_selected_and_checked);
    assert!(!verification.tunnel_or_gateway_profile_checked);
    assert!(!verification.live_dns_answer_checked);
    assert!(!verification.dnssec_checked);
    assert!(!verification.dns_ttl_freshness_checked);
    assert!(!verification.destination_nat_equivalence_checked);
    assert!(!verification.authorized_public_ip_checked);
    assert!(!verification.connect_and_request_timeouts_enforced);
    assert!(!verification.authenticated_dispatch_performed_once);
    assert!(!verification.redirect_retry_proxy_and_fallback_absence_enforced);
    assert!(!verification.response_received);
    assert!(!verification.connected_peer_checked_before_status_and_body);
    assert!(!verification.tls_server_identity_verified);
    assert!(!verification.http_status_200_checked);
    assert!(!verification.response_content_type_checked);
    assert!(!verification.response_content_encoding_checked);
    assert!(!verification.response_body_length_and_exact_schema_checked);
    assert!(!verification.closed_only_boolean_observed);
    assert!(!verification.closed_only_false_readiness_checked);
    assert!(!verification.closed_only_true_hard_block_checked);
    assert!(!verification.ambiguous_outcome_durable_burn_performed);
    assert!(!verification.live_credential_tuple_accepted_by_provider);
    assert!(!verification.credential_tuple_current_and_unrevoked_attested);
    assert!(!verification.online_authorization_v2_reverse_pins_remote_policy);
    assert!(!verification.reviewed_destination_reverse_pins_remote_policy);
    assert!(!verification.reviewed_locator_reverse_pins_remote_policy);
    assert!(!verification.fresh_delivery_reverse_pins_remote_policy);
    assert!(!verification.reviewed_identity_reverse_pins_remote_policy);
    assert!(!verification.remote_policy_fingerprint_pinned_by_online_authorization_v2);
    assert!(!verification.remote_policy_fingerprint_pinned_by_v3);
    assert!(!verification.remote_policy_consumption_durably_recorded);
    assert!(!verification.authorization_consumption_checked);
    assert!(!verification.credential_mutation_authority_attested);
    assert!(!verification.authorization.production_order_entry_authorized);
    assert!(!verification.authorization.real_order_submission_authorized);
    assert_eq!(verification.authorization.place_dispatch_allowance, 0);
}

fn reviewed_remote_policy(fixture: &Fixture) -> ReviewedRemoteCredentialProofPolicyV1 {
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

struct Fixture {
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

impl Fixture {
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

    fn context(&self) -> ReviewedRemoteCredentialProofPolicyContextV1<'_> {
        ReviewedRemoteCredentialProofPolicyContextV1 {
            config: &self.config,
            online_policy: &self.online_policy,
            online_authorization: &self.online_authorization,
            reviewed_destination: &self.reviewed_destination,
            reviewed_fresh_credential_locator: &self.reviewed_fresh_credential_locator,
            fresh_credential_delivery: &self.fresh_credential_delivery,
            reviewed_signer_proxy_identity: &self.reviewed_signer_proxy_identity,
        }
    }

    fn write_json<T: serde::Serialize>(&self, name: &str, value: &T) -> PathBuf {
        write_json(&self.root, name, value)
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

fn assert_policy_record_rejected(record: &ReviewedRemoteCredentialProofPolicyV1) {
    assert_policy_bytes_rejected(&serde_json::to_vec(record).unwrap());
}

fn assert_policy_bytes_rejected(bytes: &[u8]) {
    let directory = protected_dir();
    let path = directory.path().join("rejected-remote-policy.json");
    write_0600(&path, bytes);
    assert!(load_canonical_reviewed_remote_credential_proof_policy_v1(&path).is_err());
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
