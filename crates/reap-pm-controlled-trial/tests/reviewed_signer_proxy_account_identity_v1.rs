use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
};

use reap_pm_controlled_trial::{
    ClobLivenessHealthObservationRequirementV2, OnlineAttemptScopeApprovalV2,
    OnlineAuthorizationApprovalV2, OnlineAuthorizationBuildBindingV2,
    OnlineAuthorizationHostBindingV2, OnlineAuthorizationPurposeV2, OnlineAuthorizationV2,
    OnlineCleanupApprovalV2, OnlineFillRiskApprovalV2, OnlinePhaseScopeApprovalV2,
    OnlinePolicyPinsV2, OnlinePolicyV2, OnlinePostOnlySemanticsApprovalV2,
    OnlineProxyConcurrencyApprovalV2, OnlineSourceSeparationApprovalV2,
    OperationalObservationProfileV2, PM_T2_OFFICIAL_SOURCE_MANIFEST_BYTE_LENGTH_V1,
    PM_T2_OFFICIAL_SOURCE_MANIFEST_RETRIEVED_AT_UTC_V1,
    PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_FAMILY_V1,
    PM_T2_OFFICIAL_SOURCE_MANIFEST_SCHEMA_VERSION_V1, PM_T2_OFFICIAL_SOURCE_MANIFEST_SHA256_V1,
    PM_T2_REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_FILE_V1,
    REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_SCHEMA_VERSION, ReviewedDestinationIndependentNatV2,
    ReviewedLinuxEgressProfileV2, ReviewedMarketClassificationV2, ReviewedMarketEvidenceV2,
    ReviewedMarketObservationRequirementV2, ReviewedOfficialSourceManifestPinsV1,
    ReviewedOnlineAuthorizationPinsV1, ReviewedRepositoryStateV2,
    ReviewedSignerProxyAccountEvidenceKindV1, ReviewedSignerProxyAccountIdentityV1,
    ReviewedSignerProxyClaimedAccountV1, ReviewedStatusClobComponentV2,
    ReviewedStatusHistoryObservationRequirementV2, ReviewedStatusNoticeHistoryCutV2,
    ReviewedStatusNoticeHistoryFindingV2, ReviewedStatusNoticeHistorySourceV2,
    SameAccountClosedOnlyObservationRequirementV2, TrialAccount, TrialConfig, TrialCredentialSlot,
    TrialDomain, TrialJournalBinding, TrialMarket, TrialOrder, TrialOrderType, TrialPhase,
    TrialSide, TrialTimeLimits, UnattestedReviewedSignerProxyAccountEvidenceV1, V1ConfigPinsV2,
    load_canonical_online_authorization_v2, load_canonical_online_policy_v2,
    load_canonical_reviewed_signer_proxy_account_identity_v1, load_canonical_trial_config,
    verify_reviewed_signer_proxy_account_identity_v1,
};
use tempfile::TempDir;

const SIGNER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const FUNDER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const REVIEWED_CLOB_COMPONENT_NAME: &str = "  Trading API (CLOB)";
const REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_GOLDEN_CANONICAL_JSON: &str = concat!(
    r#"{"schema_version":1,"identity_id":"pm-t2-account-v1","reviewer_label":"operator-reviewer","reviewed_at_utc":"2026-08-09T12:00:00Z","valid_not_before_utc":"2026-08-09T12:00:00Z","valid_not_after_utc":"2026-08-09T12:20:00Z","v1_config":{"canonical_config_sha256":"23056742d14de855cd263e14a0e4549fb5d172f1a71683f525863c9cb92e465f","canonical_config_length":2450,"canonical_config_fingerprint":"f2b3c0ec4445c7c02a8cd2ee692edc47f0564c74c398ff6253e0959e31063df7","trial_plan_fingerprint":"c422f334503711dac5c91174e8f6f2274f54a71cebace3ea8297a85ff3afd008"},"#,
    r#""online_policy":{"canonical_sha256":"8656a900122f2278e187d28e86503168577c3ab660813b907c009f0ca022649d","canonical_length":1278,"fingerprint":"58a8a69165a6690dc2394b43d7e96e12f8016cb47fe39aa65104c91df4800589"},"#,
    r#""online_authorization":{"authorization_id":"pm-t2-online-authorization-v2","canonical_sha256":"20966b61e04a40b90daccf97085082bbba7657699a32eb6c67fc868d4997938c","canonical_length":2663,"fingerprint":"e880375bda260b557c8c8360fba2678500c37c4e46f6aaaa29a07ec478e81ccb"},"#,
    r#""official_source_manifest":{"schema_family":"reap-pm-controlled-trial-official-sources","schema_version":1,"retrieved_at_utc":"2026-08-09T10:17:00Z","byte_length":9103,"sha256":"ebd07e0dfbb7ee0dd825b7b435b303826130761d156e2f23b6c3428f1486e910"},"#,
    r#""evidence":{"evidence_kind":"unattested_reviewed_account_source_v1","evidence_id_label":"pm-t2-account-evidence-v1","issuer_label":"account-custodian","source_reference_label":"reviewed-account-record:pm-t2-account-v1","observed_at_utc":"2026-08-09T11:58:00Z","payload_media_type_label":"application/json","payload_byte_length":1024,"payload_sha256":"cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd","claimed_account":{"chain_id":137,"wallet_profile":"poly_proxy","signature_type":1,"signer":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266","proxy_funder":"0x70997970C51812dc3A010C7d01b50e0d17dc79C8"}}}"#,
);
const REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_GOLDEN_CANONICAL_LENGTH: u64 = 1_887;
const REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_GOLDEN_CANONICAL_SHA256: &str =
    "c0f4bc76d0cf57218ab66bd79793cdc6608d9f48f415c84040a17e87a37901b3";
const REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_GOLDEN_FINGERPRINT: &str =
    "c6a1fdb9b16522568ee6efbf99ad092f3f82d66f745ece1ee1df92eb80357800";

#[test]
fn reviewed_signer_proxy_account_identity_v1_has_stable_golden_vector() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let record = reviewed_account_identity(&fixture.config, &policy, &authorization);
    let canonical = serde_json::to_vec(&record).unwrap();
    assert_eq!(
        canonical.as_slice(),
        REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_GOLDEN_CANONICAL_JSON.as_bytes()
    );
    assert_eq!(
        canonical.len() as u64,
        REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_GOLDEN_CANONICAL_LENGTH
    );
    let path = fixture.write_json("golden-account-identity.json", &record);
    let identity = load_canonical_reviewed_signer_proxy_account_identity_v1(&path).unwrap();
    assert_eq!(
        identity.canonical_length(),
        REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_GOLDEN_CANONICAL_LENGTH
    );
    assert_eq!(
        identity.canonical_sha256(),
        REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_GOLDEN_CANONICAL_SHA256
    );
    assert_eq!(
        identity.fingerprint(),
        REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_GOLDEN_FINGERPRINT
    );
}

#[test]
fn exact_account_identity_is_canonical_move_only_redacted_and_permanently_denied() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let record = reviewed_account_identity(&fixture.config, &policy, &authorization);
    let path = fixture.write_json(
        PM_T2_REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_FILE_V1,
        &record,
    );
    let identity = load_canonical_reviewed_signer_proxy_account_identity_v1(&path).unwrap();
    let verification = verify_reviewed_signer_proxy_account_identity_v1(
        &fixture.config,
        &policy,
        &authorization,
        &identity,
    )
    .unwrap();

    assert!(verification.exact_config_policy_authorization_pins_structurally_valid);
    assert!(verification.exact_official_source_manifest_pin_structurally_valid);
    assert!(verification.official_source_manifest_sha256_matches_config_label);
    assert!(verification.exact_claimed_account_tuple_matches_config);
    assert!(verification.identity_id_matches_config_evidence_reference_label);
    assert!(verification.source_reference_matches_config_label);
    assert_all_semantic_claims_false(&verification);
    assert_ne!(identity.canonical_sha256(), identity.fingerprint());
    assert_eq!(
        identity.canonical_length(),
        fs::metadata(path).unwrap().len()
    );
    assert_eq!(
        format!("{record:?}"),
        "ReviewedSignerProxyAccountIdentityV1(<unsigned-reviewer-and-issuer-labels; account-and-evidence-redacted; denied>)"
    );
    assert_eq!(
        format!("{identity:?}"),
        "CanonicalReviewedSignerProxyAccountIdentityV1(<exact-protected-canonical-bytes; no-value-address-reference-or-path-projection; redacted; denied>)"
    );

    let display = serde_json::to_string(&verification).unwrap();
    for private_or_unattested_detail in [
        SIGNER,
        FUNDER,
        "operator-reviewer",
        "account-custodian",
        "reviewed-account-record:pm-t2-account-v1",
        "cdcdcdcd",
    ] {
        assert!(!display.contains(private_or_unattested_detail));
    }

    fn assert_send<T: Send>() {}
    assert_send::<reap_pm_controlled_trial::CanonicalReviewedSignerProxyAccountIdentityV1>();
}

#[test]
fn loader_rejects_duplicate_unknown_noncanonical_kind_and_unprotected_bytes() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let record = reviewed_account_identity(&fixture.config, &policy, &authorization);
    let canonical = serde_json::to_vec(&record).unwrap();

    let mut duplicate = b"{\"schema_version\":1,".to_vec();
    duplicate.extend_from_slice(&canonical[1..]);
    assert_identity_bytes_rejected(&duplicate);

    let mut unknown = canonical[..canonical.len() - 1].to_vec();
    unknown.extend_from_slice(b",\"signer_controls_proxy_attested\":true}");
    assert_identity_bytes_rejected(&unknown);

    let mut trailing = canonical.clone();
    trailing.push(b'\n');
    assert_identity_bytes_rejected(&trailing);

    let mut reordered: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
    let object = reordered.as_object_mut().unwrap();
    let schema = object.remove("schema_version").unwrap();
    object.insert("schema_version".into(), schema);
    let reordered = serde_json::to_vec(&reordered).unwrap();
    assert_ne!(reordered, canonical);
    assert_identity_bytes_rejected(&reordered);

    let mut wrong_kind = serde_json::to_value(&record).unwrap();
    wrong_kind["evidence"]["evidence_kind"] = serde_json::json!("reviewed_account_source_v1");
    assert_identity_bytes_rejected(&serde_json::to_vec(&wrong_kind).unwrap());

    let directory = protected_dir();
    let path = directory.path().join("wrong-mode.json");
    fs::write(&path, canonical).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(load_canonical_reviewed_signer_proxy_account_identity_v1(&path).is_err());
}

#[test]
fn official_source_manifest_pin_is_closed_to_the_exact_raw_pretty_json_freeze() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let base = reviewed_account_identity(&fixture.config, &policy, &authorization);
    let mutations: [fn(&mut ReviewedSignerProxyAccountIdentityV1); 5] = [
        |identity| identity.official_source_manifest.schema_family = "other-family".into(),
        |identity| identity.official_source_manifest.schema_version = 2,
        |identity| {
            identity.official_source_manifest.retrieved_at_utc = "2026-08-09T10:17:01Z".into()
        },
        |identity| identity.official_source_manifest.byte_length += 1,
        |identity| identity.official_source_manifest.sha256 = "aa".repeat(32),
    ];
    for mutate in mutations {
        let mut record = base.clone();
        mutate(&mut record);
        assert_identity_record_rejected(&record);
    }
}

#[test]
fn verifier_requires_exact_pins_account_tuple_and_reference_label_correlation() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let base = reviewed_account_identity(&fixture.config, &policy, &authorization);
    let drift_mutations: [fn(&mut ReviewedSignerProxyAccountIdentityV1); 8] = [
        |identity| identity.v1_config.canonical_config_sha256 = "a1".repeat(32),
        |identity| identity.online_policy.fingerprint = "b2".repeat(32),
        |identity| identity.online_authorization.fingerprint = "c3".repeat(32),
        |identity| {
            std::mem::swap(
                &mut identity.evidence.claimed_account.signer,
                &mut identity.evidence.claimed_account.proxy_funder,
            )
        },
        |identity| {
            identity.evidence.source_reference_label = "reviewed-account-record:other".into()
        },
        |identity| identity.identity_id = "other-account-v1".into(),
        |identity| identity.valid_not_before_utc = "2026-08-09T12:00:01Z".into(),
        |identity| identity.valid_not_after_utc = "2026-08-09T12:20:01Z".into(),
    ];
    for mutate in drift_mutations {
        let mut record = base.clone();
        mutate(&mut record);
        let path = fixture.write_json("drifted-account-identity.json", &record);
        let identity = load_canonical_reviewed_signer_proxy_account_identity_v1(&path).unwrap();
        assert!(
            verify_reviewed_signer_proxy_account_identity_v1(
                &fixture.config,
                &policy,
                &authorization,
                &identity,
            )
            .is_err()
        );
    }

    let mut mismatched_source_manifest_config = trial_config();
    mismatched_source_manifest_config.source_pin_manifest_sha256 = "aa".repeat(32);
    let mismatched_fixture = Fixture::with_trial_config(mismatched_source_manifest_config);
    let (mismatched_policy, mismatched_authorization) = mismatched_fixture.online_records();
    let mismatched_record = reviewed_account_identity(
        &mismatched_fixture.config,
        &mismatched_policy,
        &mismatched_authorization,
    );
    let mismatched_path =
        mismatched_fixture.write_json("mismatched-source-manifest.json", &mismatched_record);
    let mismatched_identity =
        load_canonical_reviewed_signer_proxy_account_identity_v1(&mismatched_path).unwrap();
    assert!(
        verify_reviewed_signer_proxy_account_identity_v1(
            &mismatched_fixture.config,
            &mismatched_policy,
            &mismatched_authorization,
            &mismatched_identity,
        )
        .is_err()
    );
}

#[test]
fn account_profile_address_and_time_labels_are_intrinsically_strict_only() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let base = reviewed_account_identity(&fixture.config, &policy, &authorization);
    let intrinsic_mutations: [fn(&mut ReviewedSignerProxyAccountIdentityV1); 10] = [
        |identity| identity.evidence.claimed_account.chain_id = 1,
        |identity| identity.evidence.claimed_account.signature_type = 0,
        |identity| identity.evidence.claimed_account.wallet_profile = "eoa".into(),
        |identity| identity.evidence.claimed_account.signer = SIGNER.to_ascii_lowercase(),
        |identity| identity.evidence.claimed_account.proxy_funder = SIGNER.into(),
        |identity| identity.evidence.payload_byte_length = 0,
        |identity| identity.evidence.payload_sha256 = "not-a-sha".into(),
        |identity| identity.evidence.observed_at_utc = "2026-08-09T12:00:01Z".into(),
        |identity| identity.reviewed_at_utc = "2026-08-09T12:00:01Z".into(),
        |identity| identity.valid_not_after_utc = "2026-08-09T12:00:00Z".into(),
    ];
    for mutate in intrinsic_mutations {
        let mut record = base.clone();
        mutate(&mut record);
        assert_identity_record_rejected(&record);
    }

    let mut before_authorization_review = base;
    before_authorization_review.reviewed_at_utc = "2026-08-09T11:59:59Z".into();
    let path = fixture.write_json("early-account-review.json", &before_authorization_review);
    let identity = load_canonical_reviewed_signer_proxy_account_identity_v1(&path).unwrap();
    assert!(
        verify_reviewed_signer_proxy_account_identity_v1(
            &fixture.config,
            &policy,
            &authorization,
            &identity,
        )
        .is_err()
    );
}

#[test]
fn schema_has_metadata_digest_and_public_tuple_but_no_credential_or_api_secret_field() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let value = serde_json::to_value(reviewed_account_identity(
        &fixture.config,
        &policy,
        &authorization,
    ))
    .unwrap();
    let evidence = value["evidence"].as_object().unwrap();
    assert_eq!(
        evidence.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "claimed_account",
            "evidence_id_label",
            "evidence_kind",
            "issuer_label",
            "observed_at_utc",
            "payload_byte_length",
            "payload_media_type_label",
            "payload_sha256",
            "source_reference_label",
        ])
    );
    assert_eq!(
        evidence["claimed_account"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "chain_id",
            "proxy_funder",
            "signature_type",
            "signer",
            "wallet_profile",
        ])
    );
    let canonical = serde_json::to_string(&value).unwrap();
    for forbidden in [
        "private_key",
        "api_key",
        "l2_secret",
        "passphrase",
        "hmac",
        "authorization_header",
        "signed_body",
    ] {
        assert!(!canonical.contains(forbidden));
    }
}

fn assert_all_semantic_claims_false(
    verification: &reap_pm_controlled_trial::ReviewedSignerProxyAccountIdentityVerificationV1,
) {
    assert!(!verification.official_source_manifest_bytes_loaded_and_hash_verified);
    assert!(!verification.reviewed_account_evidence_bytes_loaded_and_hash_verified);
    assert!(!verification.official_source_manifest_publisher_authorship_attested);
    assert!(!verification.reviewer_authorship_attested);
    assert!(!verification.source_authorship_attested);
    assert!(!verification.issuer_signature_verified);
    assert!(!verification.evidence_source_tls_and_server_identity_verified);
    assert!(!verification.signer_on_chain_eoa_status_verified);
    assert!(!verification.proxy_on_chain_contract_status_verified);
    assert!(!verification.on_chain_account_state_checked);
    assert!(!verification.on_chain_finality_checked);
    assert!(!verification.proxy_factory_semantics_verified);
    assert!(!verification.signer_controls_proxy_attested);
    assert!(!verification.signer_proxy_relationship_current);
    assert!(!verification.signer_proxy_relationship_unrevoked);
    assert!(!verification.account_specific_evidence_reference_resolved_and_authenticated);
    assert!(!verification.source_owned_current_time_checked);
    assert!(!verification.remote_api_key_owner_attested);
    assert!(!verification.private_key_derived_signer_matches_config_checked);
    assert!(!verification.l2_credentials_match_configured_signer_checked);
    assert!(!verification.identity_fingerprint_pinned_by_online_authorization_v2);
    assert!(!verification.identity_fingerprint_pinned_by_v3);
    assert!(!verification.identity_consumption_durably_recorded);
    assert!(!verification.authorization_consumption_checked);
    assert!(!verification.credential_mutation_authority_attested);
    assert!(!verification.authorization.production_order_entry_authorized);
    assert!(!verification.authorization.real_order_submission_authorized);
    assert_eq!(verification.authorization.place_dispatch_allowance, 0);
}

fn reviewed_account_identity(
    config: &reap_pm_controlled_trial::CanonicalTrialConfig,
    policy: &reap_pm_controlled_trial::CanonicalOnlinePolicyV2,
    authorization: &reap_pm_controlled_trial::CanonicalOnlineAuthorizationV2,
) -> ReviewedSignerProxyAccountIdentityV1 {
    ReviewedSignerProxyAccountIdentityV1 {
        schema_version: REVIEWED_SIGNER_PROXY_ACCOUNT_IDENTITY_V1_SCHEMA_VERSION,
        identity_id: "pm-t2-account-v1".into(),
        reviewer_label: "operator-reviewer".into(),
        reviewed_at_utc: "2026-08-09T12:00:00Z".into(),
        valid_not_before_utc: "2026-08-09T12:00:00Z".into(),
        valid_not_after_utc: "2026-08-09T12:20:00Z".into(),
        v1_config: config_pins(config),
        online_policy: OnlinePolicyPinsV2 {
            canonical_sha256: policy.canonical_sha256().into(),
            canonical_length: policy.canonical_length(),
            fingerprint: policy.fingerprint().into(),
        },
        online_authorization: ReviewedOnlineAuthorizationPinsV1 {
            authorization_id: authorization.value().authorization_id.clone(),
            canonical_sha256: authorization.canonical_sha256().into(),
            canonical_length: authorization.canonical_length(),
            fingerprint: authorization.fingerprint().into(),
        },
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
        online_policy: OnlinePolicyPinsV2 {
            canonical_sha256: policy.canonical_sha256().into(),
            canonical_length: policy.canonical_length(),
            fingerprint: policy.fingerprint().into(),
        },
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

fn assert_identity_record_rejected(record: &ReviewedSignerProxyAccountIdentityV1) {
    assert_identity_bytes_rejected(&serde_json::to_vec(record).unwrap());
}

fn assert_identity_bytes_rejected(bytes: &[u8]) {
    let directory = protected_dir();
    let path = directory.path().join("rejected-account-identity.json");
    write_0600(&path, bytes);
    assert!(load_canonical_reviewed_signer_proxy_account_identity_v1(&path).is_err());
}

struct Fixture {
    _directory: TempDir,
    root: PathBuf,
    config: reap_pm_controlled_trial::CanonicalTrialConfig,
}

impl Fixture {
    fn new() -> Self {
        Self::with_trial_config(trial_config())
    }

    fn with_trial_config(trial_config: TrialConfig) -> Self {
        let directory = protected_dir();
        let root = directory.path().to_owned();
        let config_path = root.join("canonical-config.json");
        write_0600(&config_path, &serde_json::to_vec(&trial_config).unwrap());
        let config = load_canonical_trial_config(&config_path).unwrap();
        Self {
            _directory: directory,
            root,
            config,
        }
    }

    fn online_records(
        &self,
    ) -> (
        reap_pm_controlled_trial::CanonicalOnlinePolicyV2,
        reap_pm_controlled_trial::CanonicalOnlineAuthorizationV2,
    ) {
        let policy_path = self.write_json("online-policy-v2.json", &online_policy(&self.config));
        let policy = load_canonical_online_policy_v2(&policy_path).unwrap();
        let authorization_path = self.write_json(
            "online-authorization-v2.json",
            &online_authorization(&self.config, &policy),
        );
        let authorization = load_canonical_online_authorization_v2(&authorization_path).unwrap();
        (policy, authorization)
    }

    fn write_json<T: serde::Serialize>(&self, name: &str, value: &T) -> PathBuf {
        let path = self.root.join(name);
        write_0600(&path, &serde_json::to_vec(value).unwrap());
        path
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

fn protected_dir() -> TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    directory
}

fn write_0600(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
