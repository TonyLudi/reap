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
    PM_T2_FRESH_API_KEY_ENTRY_V1, PM_T2_FRESH_CREDENTIAL_DELIVERY_BINDING_FILE_V1,
    PM_T2_FRESH_L2_SECRET_ENTRY_V1, PM_T2_FRESH_PASSPHRASE_ENTRY_V1,
    PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1, PM_T2_REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_FILE_V1,
    REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_SCHEMA_VERSION, ReviewedDestinationIndependentNatV2,
    ReviewedFreshCredentialFilesV1, ReviewedFreshCredentialSlotLocatorV1,
    ReviewedLinuxEgressProfileV2, ReviewedMarketClassificationV2, ReviewedMarketEvidenceV2,
    ReviewedMarketObservationRequirementV2, ReviewedOnlineAuthorizationPinsV1,
    ReviewedRepositoryStateV2, ReviewedStatusClobComponentV2,
    ReviewedStatusHistoryObservationRequirementV2, ReviewedStatusNoticeHistoryCutV2,
    ReviewedStatusNoticeHistoryFindingV2, ReviewedStatusNoticeHistorySourceV2,
    SameAccountClosedOnlyObservationRequirementV2, TrialAccount, TrialConfig, TrialCredentialSlot,
    TrialDomain, TrialJournalBinding, TrialMarket, TrialOrder, TrialOrderType, TrialPhase,
    TrialSide, TrialTimeLimits, UnattestedFreshCredentialProviderGenerationV1, V1ConfigPinsV2,
    bind_fresh_credential_delivery_binding_v1, load_canonical_fresh_credential_delivery_binding_v1,
    load_canonical_online_authorization_v2, load_canonical_online_policy_v2,
    load_canonical_reviewed_fresh_credential_slot_locator_v1, load_canonical_trial_config,
    verify_fresh_credential_delivery_binding_evidence_v1,
    verify_fresh_credential_delivery_binding_v1,
};
use tempfile::TempDir;

const SIGNER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const FUNDER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const REVIEWED_CLOB_COMPONENT_NAME: &str = "  Trading API (CLOB)";
const REVIEWED_CREDENTIAL_DIRECTORY: &str = "/run/reap/pm-t2/credentials/pm-t2-slot-1";
const FRESH_CREDENTIAL_DELIVERY_BINDING_V1_GOLDEN_CANONICAL_JSON: &str = concat!(
    r#"{"schema_version":1,"binding_id":"pm-t2-fresh-credential-delivery-binding-v1","unattested_delivery_recorded_at_utc":"2026-08-09T12:00:00Z","unattested_valid_not_before_utc":"2026-08-09T12:00:00Z","unattested_valid_not_after_utc":"2026-08-09T12:20:00Z","reviewed_fresh_credential_slot_locator":{"locator_id":"pm-t2-reviewed-fresh-credential-slot-locator-v1","canonical_sha256":"f29a954206ae875bdbd13240c46f815b60873c7b833052766fc0de170431babf","canonical_length":1422,"fingerprint":"9ab3382959e8156f2ef6fa4d174ae7018239e830f26429517c579f9d260441f1"},"#,
    r#""unattested_provider_generation":{"provider_id":"provider-a","provider_key_id":"provider-key-a","rotation_namespace_id":"rotation-a","delivery_id":"delivery-a","rotation_generation":7},"#,
    r#""unattested_linux_objects":{"directory":{"filesystem_device":41,"inode":424242,"owner_uid":1000,"permission_mode":448,"modified_seconds":1754741999,"modified_nanoseconds":100,"status_changed_seconds":1754741999,"status_changed_nanoseconds":200},"files":{"private_key":{"filesystem_device":41,"inode":424243,"owner_uid":1000,"permission_mode":384,"hard_link_count":1,"modified_seconds":1754741999,"modified_nanoseconds":300,"status_changed_seconds":1754741999,"status_changed_nanoseconds":400},"#,
    r#""api_key":{"filesystem_device":41,"inode":424244,"owner_uid":1000,"permission_mode":384,"hard_link_count":1,"modified_seconds":1754741999,"modified_nanoseconds":300,"status_changed_seconds":1754741999,"status_changed_nanoseconds":400},"#,
    r#""l2_secret":{"filesystem_device":41,"inode":424245,"owner_uid":1000,"permission_mode":384,"hard_link_count":1,"modified_seconds":1754741999,"modified_nanoseconds":300,"status_changed_seconds":1754741999,"status_changed_nanoseconds":400},"#,
    r#""passphrase":{"filesystem_device":41,"inode":424246,"owner_uid":1000,"permission_mode":384,"hard_link_count":1,"modified_seconds":1754741999,"modified_nanoseconds":300,"status_changed_seconds":1754741999,"status_changed_nanoseconds":400}}}}"#,
);
const FRESH_CREDENTIAL_DELIVERY_BINDING_V1_GOLDEN_CANONICAL_LENGTH: u64 = 1_939;
const FRESH_CREDENTIAL_DELIVERY_BINDING_V1_GOLDEN_CANONICAL_SHA256: &str =
    "3747dcf7f14feca5928c79e149123c5d7f02eb1df8379b6ae795bdb24f76aca1";
const FRESH_CREDENTIAL_DELIVERY_BINDING_V1_GOLDEN_FINGERPRINT: &str =
    "fe2a790d35720152eb0b115c9d1e1c2cd91df0b9e598e6dd3cf7332899f9a7c2";

#[test]
fn canonical_delivery_binding_v1_has_stable_golden_bytes_length_sha_and_domain_fingerprint() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let locator_record = credential_locator(&fixture.config, &policy, &authorization);
    let locator_path = fixture.write_json("golden-locator.json", &locator_record);
    let locator = load_canonical_reviewed_fresh_credential_slot_locator_v1(&locator_path).unwrap();
    let record = delivery_binding(&locator_record.locator_id, &locator);
    let canonical = serde_json::to_vec(&record).unwrap();
    assert_eq!(
        canonical.as_slice(),
        FRESH_CREDENTIAL_DELIVERY_BINDING_V1_GOLDEN_CANONICAL_JSON.as_bytes()
    );
    assert_eq!(
        canonical.len() as u64,
        FRESH_CREDENTIAL_DELIVERY_BINDING_V1_GOLDEN_CANONICAL_LENGTH
    );
    let binding_path = fixture.write_json("golden-binding.json", &record);
    let binding = load_canonical_fresh_credential_delivery_binding_v1(&binding_path).unwrap();
    assert_eq!(
        binding.canonical_length(),
        FRESH_CREDENTIAL_DELIVERY_BINDING_V1_GOLDEN_CANONICAL_LENGTH
    );
    assert_eq!(
        binding.canonical_sha256(),
        FRESH_CREDENTIAL_DELIVERY_BINDING_V1_GOLDEN_CANONICAL_SHA256
    );
    assert_eq!(
        binding.fingerprint(),
        FRESH_CREDENTIAL_DELIVERY_BINDING_V1_GOLDEN_FINGERPRINT
    );
}

#[test]
fn exact_unsigned_binding_is_canonical_move_only_redacted_and_permanently_denied() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let locator_record = credential_locator(&fixture.config, &policy, &authorization);
    let locator_path = fixture.write_json(
        PM_T2_REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_FILE_V1,
        &locator_record,
    );
    let locator = load_canonical_reviewed_fresh_credential_slot_locator_v1(&locator_path).unwrap();
    let binding_record = delivery_binding(&locator_record.locator_id, &locator);
    let binding_path = fixture.write_json(
        PM_T2_FRESH_CREDENTIAL_DELIVERY_BINDING_FILE_V1,
        &binding_record,
    );
    let binding = load_canonical_fresh_credential_delivery_binding_v1(&binding_path).unwrap();
    let verification = verify_fresh_credential_delivery_binding_v1(
        &fixture.config,
        &policy,
        &authorization,
        &locator,
        &binding,
    )
    .unwrap();

    assert!(verification.exact_reviewed_locator_pins_structurally_valid);
    assert!(verification.unattested_provider_generation_labels_structurally_valid);
    assert!(verification.unattested_linux_object_metadata_labels_structurally_valid);
    assert!(verification.unattested_validity_labels_nested_within_v2);
    assert_all_authority_claims_false(&verification);
    assert_ne!(binding.canonical_sha256(), binding.fingerprint());
    assert_eq!(
        binding.canonical_length(),
        fs::metadata(binding_path).unwrap().len()
    );
    assert_eq!(
        format!("{binding_record:?}"),
        "FreshCredentialDeliveryBindingV1(<unsigned-provider-generation-and-linux-metadata-labels; redacted; denied>)"
    );
    assert_eq!(
        format!("{binding:?}"),
        "CanonicalFreshCredentialDeliveryBindingV1(<unsigned-exact-canonical-bytes; redacted; denied>)"
    );

    let display = serde_json::to_string(&verification).unwrap();
    for private_or_unattested_detail in [
        REVIEWED_CREDENTIAL_DIRECTORY,
        SIGNER,
        "provider-a",
        "provider-key-a",
        "rotation-a",
        "delivery-a",
        "424242",
    ] {
        assert!(!display.contains(private_or_unattested_detail));
    }

    fn assert_send<T: Send>() {}
    assert_send::<reap_pm_controlled_trial::CanonicalFreshCredentialDeliveryBindingV1>();
    assert_send::<reap_pm_controlled_trial::CanonicalFreshCredentialDeliveryBindingEvidenceV1>();
    assert_send::<reap_pm_controlled_trial::FreshCredentialDeliveryLoadTokenV1>();

    let canonical_sha256 = binding.canonical_sha256().to_owned();
    let canonical_length = binding.canonical_length();
    let fingerprint = binding.fingerprint().to_owned();
    let expected_linux_objects = binding_record.unattested_linux_objects.clone();
    let (evidence, token) = bind_fresh_credential_delivery_binding_v1(
        &fixture.config,
        &policy,
        &authorization,
        locator,
        binding,
    )
    .unwrap();
    assert_eq!(evidence.canonical_sha256(), canonical_sha256);
    assert_eq!(evidence.canonical_length(), canonical_length);
    assert_eq!(evidence.fingerprint(), fingerprint);
    assert_eq!(
        format!("{evidence:?}"),
        "CanonicalFreshCredentialDeliveryBindingEvidenceV1(<retained-exact-evidence; no-path-or-provider-projection; redacted; denied>)"
    );
    assert_eq!(
        format!("{token:?}"),
        "FreshCredentialDeliveryLoadTokenV1(<one-shot-local-load; path-signer-and-metadata-redacted; denied>)"
    );

    let reverified = verify_fresh_credential_delivery_binding_evidence_v1(
        &fixture.config,
        &policy,
        &authorization,
        &evidence,
    )
    .unwrap();
    assert_eq!(reverified, verification);
    assert_all_authority_claims_false(&reverified);

    let (locator_token, linux_objects, token_fingerprint) = token.into_parts();
    assert!(linux_objects == expected_linux_objects);
    assert_eq!(token_fingerprint, fingerprint);
    let (directory, configured_signer) = locator_token.into_parts();
    assert_eq!(directory, Path::new(REVIEWED_CREDENTIAL_DIRECTORY));
    assert_eq!(configured_signer, fixture.config.value().account.signer);
}

#[test]
fn loader_rejects_noncanonical_unknown_duplicate_and_unprotected_binding_bytes() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let locator_record = credential_locator(&fixture.config, &policy, &authorization);
    let locator_path = fixture.write_json("canonical-locator.json", &locator_record);
    let locator = load_canonical_reviewed_fresh_credential_slot_locator_v1(&locator_path).unwrap();
    let canonical =
        serde_json::to_vec(&delivery_binding(&locator_record.locator_id, &locator)).unwrap();

    let mut duplicate = b"{\"schema_version\":1,".to_vec();
    duplicate.extend_from_slice(&canonical[1..]);
    assert_binding_bytes_rejected(&duplicate);

    let mut unknown = canonical[..canonical.len() - 1].to_vec();
    unknown.extend_from_slice(b",\"provider_signature_verified\":true}");
    assert_binding_bytes_rejected(&unknown);

    let mut trailing = canonical.clone();
    trailing.push(b'\n');
    assert_binding_bytes_rejected(&trailing);

    let mut reordered: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
    let object = reordered.as_object_mut().unwrap();
    let schema = object.remove("schema_version").unwrap();
    object.insert("schema_version".into(), schema);
    let reordered = serde_json::to_vec(&reordered).unwrap();
    assert_ne!(reordered, canonical);
    assert_binding_bytes_rejected(&reordered);

    let directory = protected_dir();
    let path = directory.path().join("wrong-mode.json");
    fs::write(&path, canonical).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(load_canonical_fresh_credential_delivery_binding_v1(&path).is_err());
}

#[test]
fn verifier_requires_all_four_exact_locator_pins_and_exact_authorized_owner() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let locator_record = credential_locator(&fixture.config, &policy, &authorization);
    let locator_path = fixture.write_json("pin-test-locator.json", &locator_record);
    let locator = load_canonical_reviewed_fresh_credential_slot_locator_v1(&locator_path).unwrap();
    let base = delivery_binding(&locator_record.locator_id, &locator);

    let drift_mutations: [fn(&mut FreshCredentialDeliveryBindingV1); 4] = [
        |binding| {
            binding.reviewed_fresh_credential_slot_locator.locator_id = "other-locator".into()
        },
        |binding| {
            binding
                .reviewed_fresh_credential_slot_locator
                .canonical_sha256 = "a1".repeat(32)
        },
        |binding| {
            binding
                .reviewed_fresh_credential_slot_locator
                .canonical_length += 1
        },
        |binding| binding.reviewed_fresh_credential_slot_locator.fingerprint = "b2".repeat(32),
    ];
    for mutate in drift_mutations {
        let mut record = base.clone();
        mutate(&mut record);
        let path = fixture.write_json("drifted-delivery-binding.json", &record);
        let binding = load_canonical_fresh_credential_delivery_binding_v1(&path).unwrap();
        assert!(
            verify_fresh_credential_delivery_binding_v1(
                &fixture.config,
                &policy,
                &authorization,
                &locator,
                &binding,
            )
            .is_err()
        );
    }

    let mut wrong_owner = base;
    wrong_owner.unattested_linux_objects.directory.owner_uid = 1_001;
    for file in linux_files_mut(&mut wrong_owner.unattested_linux_objects.files) {
        file.owner_uid = 1_001;
    }
    let path = fixture.write_json("wrong-owner-delivery-binding.json", &wrong_owner);
    let binding = load_canonical_fresh_credential_delivery_binding_v1(&path).unwrap();
    assert!(
        verify_fresh_credential_delivery_binding_v1(
            &fixture.config,
            &policy,
            &authorization,
            &locator,
            &binding,
        )
        .is_err()
    );
}

#[test]
fn unattested_labels_are_strict_but_never_provider_generation_or_freshness_proof() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let locator_record = credential_locator(&fixture.config, &policy, &authorization);
    let locator_path = fixture.write_json("label-test-locator.json", &locator_record);
    let locator = load_canonical_reviewed_fresh_credential_slot_locator_v1(&locator_path).unwrap();
    let base = delivery_binding(&locator_record.locator_id, &locator);

    let intrinsic_mutations: [fn(&mut FreshCredentialDeliveryBindingV1); 11] = [
        |binding| binding.unattested_provider_generation.provider_id = "provider label".into(),
        |binding| binding.unattested_provider_generation.rotation_generation = 0,
        |binding| binding.unattested_valid_not_before_utc = "2026-08-09T12:00:00+00:00".into(),
        |binding| binding.unattested_linux_objects.directory.permission_mode = 0o755,
        |binding| {
            binding
                .unattested_linux_objects
                .files
                .private_key
                .permission_mode = 0o640
        },
        |binding| {
            binding
                .unattested_linux_objects
                .files
                .api_key
                .hard_link_count = 2
        },
        |binding| binding.unattested_linux_objects.files.l2_secret.inode = 0,
        |binding| {
            binding
                .unattested_linux_objects
                .files
                .passphrase
                .modified_nanoseconds = 1_000_000_000
        },
        |binding| {
            binding.unattested_linux_objects.files.passphrase.inode =
                binding.unattested_linux_objects.files.private_key.inode
        },
        |binding| binding.unattested_delivery_recorded_at_utc = "2026-08-09T12:00:01Z".into(),
        |binding| binding.unattested_valid_not_after_utc = "2026-08-09T12:00:00Z".into(),
    ];
    for mutate in intrinsic_mutations {
        let mut record = base.clone();
        mutate(&mut record);
        assert_binding_record_rejected(&record);
    }

    for (recorded_at, not_before, not_after) in [
        (
            "2026-08-09T11:59:00Z",
            "2026-08-09T11:59:00Z",
            "2026-08-09T12:20:00Z",
        ),
        (
            "2026-08-09T12:00:00Z",
            "2026-08-09T12:00:00Z",
            "2026-08-09T12:20:01Z",
        ),
    ] {
        let mut record = base.clone();
        record.unattested_delivery_recorded_at_utc = recorded_at.into();
        record.unattested_valid_not_before_utc = not_before.into();
        record.unattested_valid_not_after_utc = not_after.into();
        let path = fixture.write_json("outside-v2-time-labels.json", &record);
        let binding = load_canonical_fresh_credential_delivery_binding_v1(&path).unwrap();
        assert!(
            verify_fresh_credential_delivery_binding_v1(
                &fixture.config,
                &policy,
                &authorization,
                &locator,
                &binding,
            )
            .is_err()
        );
    }

    // A label may cover only post-placement cleanup. Nesting through V2
    // cleanup_not_after is not a current-time, active placement, provider
    // freshness, signature, or unrevoked-lease check.
    let mut cleanup_only = base;
    cleanup_only.unattested_delivery_recorded_at_utc = "2026-08-09T12:15:00Z".into();
    cleanup_only.unattested_valid_not_before_utc = "2026-08-09T12:16:00Z".into();
    cleanup_only.unattested_valid_not_after_utc = "2026-08-09T12:20:00Z".into();
    let path = fixture.write_json("cleanup-only-time-labels.json", &cleanup_only);
    let binding = load_canonical_fresh_credential_delivery_binding_v1(&path).unwrap();
    let verification = verify_fresh_credential_delivery_binding_v1(
        &fixture.config,
        &policy,
        &authorization,
        &locator,
        &binding,
    )
    .unwrap();
    assert!(verification.unattested_validity_labels_nested_within_v2);
    assert_all_authority_claims_false(&verification);
}

#[test]
fn four_role_metadata_contains_no_credential_file_length_or_content_hash_field() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let locator_record = credential_locator(&fixture.config, &policy, &authorization);
    let locator_path = fixture.write_json("privacy-test-locator.json", &locator_record);
    let locator = load_canonical_reviewed_fresh_credential_slot_locator_v1(&locator_path).unwrap();
    let serialized =
        serde_json::to_value(delivery_binding(&locator_record.locator_id, &locator)).unwrap();

    // canonical_length is required for the exact locator pin. No Linux file
    // identity has a length, credential digest, or content hash.
    assert!(
        serialized["reviewed_fresh_credential_slot_locator"]
            .get("canonical_length")
            .is_some()
    );
    let expected_keys = BTreeSet::from([
        "filesystem_device",
        "hard_link_count",
        "inode",
        "modified_nanoseconds",
        "modified_seconds",
        "owner_uid",
        "permission_mode",
        "status_changed_nanoseconds",
        "status_changed_seconds",
    ]);
    let files = serialized["unattested_linux_objects"]["files"]
        .as_object()
        .unwrap();
    assert_eq!(
        files.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        BTreeSet::from(["api_key", "l2_secret", "passphrase", "private_key"])
    );
    for identity in files.values() {
        assert_eq!(
            identity
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            expected_keys
        );
    }
}

fn assert_all_authority_claims_false(
    verification: &reap_pm_controlled_trial::FreshCredentialDeliveryBindingVerificationV1,
) {
    assert!(!verification.source_owned_current_time_checked);
    assert!(!verification.protected_credential_directory_and_four_files_checked);
    assert!(!verification.loaded_linux_objects_match_unattested_binding);
    assert!(!verification.same_loaded_holder_attested);
    assert!(!verification.globally_unique_delivery_attested);
    assert!(!verification.provider_authorship_attested);
    assert!(!verification.provider_signature_verified);
    assert!(!verification.provider_lease_fresh_and_unrevoked);
    assert!(!verification.rotation_generation_attested);
    assert!(!verification.delivery_freshness_attested);
    assert!(!verification.loaded_bundle_matches_credential_slot_generation);
    assert!(!verification.remote_api_key_owner_attested);
    assert!(!verification.locator_fingerprint_pinned_by_v2);
    assert!(!verification.delivery_binding_fingerprint_pinned_by_v2);
    assert!(!verification.delivery_consumption_durably_recorded);
    assert!(!verification.authorization_consumption_checked);
    assert!(!verification.credential_mutation_authority_attested);
    assert!(!verification.authorization.production_order_entry_authorized);
    assert!(!verification.authorization.real_order_submission_authorized);
    assert_eq!(verification.authorization.place_dispatch_allowance, 0);
}

fn delivery_binding(
    locator_id: &str,
    locator: &reap_pm_controlled_trial::CanonicalReviewedFreshCredentialSlotLocatorV1,
) -> FreshCredentialDeliveryBindingV1 {
    FreshCredentialDeliveryBindingV1 {
        schema_version:
            reap_pm_controlled_trial::FRESH_CREDENTIAL_DELIVERY_BINDING_V1_SCHEMA_VERSION,
        binding_id: "pm-t2-fresh-credential-delivery-binding-v1".into(),
        unattested_delivery_recorded_at_utc: "2026-08-09T12:00:00Z".into(),
        unattested_valid_not_before_utc: "2026-08-09T12:00:00Z".into(),
        unattested_valid_not_after_utc: "2026-08-09T12:20:00Z".into(),
        reviewed_fresh_credential_slot_locator: FreshCredentialSlotLocatorPinsV1 {
            locator_id: locator_id.into(),
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

fn linux_files_mut(
    files: &mut FreshCredentialLinuxFileIdentitiesV1,
) -> [&mut FreshCredentialLinuxFileIdentityV1; 4] {
    [
        &mut files.private_key,
        &mut files.api_key,
        &mut files.l2_secret,
        &mut files.passphrase,
    ]
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

fn assert_binding_record_rejected(record: &FreshCredentialDeliveryBindingV1) {
    assert_binding_bytes_rejected(&serde_json::to_vec(record).unwrap());
}

fn assert_binding_bytes_rejected(bytes: &[u8]) {
    let directory = protected_dir();
    let path = directory.path().join("rejected-delivery-binding.json");
    write_0600(&path, bytes);
    assert!(load_canonical_fresh_credential_delivery_binding_v1(&path).is_err());
}

struct Fixture {
    _directory: TempDir,
    root: PathBuf,
    config: reap_pm_controlled_trial::CanonicalTrialConfig,
}

impl Fixture {
    fn new() -> Self {
        let directory = protected_dir();
        let root = directory.path().to_owned();
        let config_path = root.join("canonical-config.json");
        write_0600(&config_path, &serde_json::to_vec(&trial_config()).unwrap());
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
        source_pin_manifest_sha256: "11".repeat(32),
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
