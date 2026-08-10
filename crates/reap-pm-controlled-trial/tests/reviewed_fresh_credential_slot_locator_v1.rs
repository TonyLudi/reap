use std::{
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
    OperationalObservationProfileV2, PM_T2_FRESH_API_KEY_ENTRY_V1, PM_T2_FRESH_L2_SECRET_ENTRY_V1,
    PM_T2_FRESH_PASSPHRASE_ENTRY_V1, PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1,
    PM_T2_REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_FILE_V1,
    REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_V1_SCHEMA_VERSION, ReviewedDestinationIndependentNatV2,
    ReviewedFreshCredentialFilesV1, ReviewedFreshCredentialSlotLocatorV1,
    ReviewedLinuxEgressProfileV2, ReviewedMarketClassificationV2, ReviewedMarketEvidenceV2,
    ReviewedMarketObservationRequirementV2, ReviewedOnlineAuthorizationPinsV1,
    ReviewedRepositoryStateV2, ReviewedStatusClobComponentV2,
    ReviewedStatusHistoryObservationRequirementV2, ReviewedStatusNoticeHistoryCutV2,
    ReviewedStatusNoticeHistoryFindingV2, ReviewedStatusNoticeHistorySourceV2,
    SameAccountClosedOnlyObservationRequirementV2, TrialAccount, TrialConfig, TrialCredentialSlot,
    TrialDomain, TrialJournalBinding, TrialMarket, TrialOrder, TrialOrderType, TrialPhase,
    TrialSide, TrialTimeLimits, V1ConfigPinsV2, bind_reviewed_fresh_credential_slot_locator_v1,
    load_canonical_online_authorization_v2, load_canonical_online_policy_v2,
    load_canonical_reviewed_fresh_credential_slot_locator_v1, load_canonical_trial_config,
    verify_reviewed_fresh_credential_slot_locator_evidence_v1,
    verify_reviewed_fresh_credential_slot_locator_v1,
};
use tempfile::TempDir;

const SIGNER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const FUNDER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const REVIEWED_CLOB_COMPONENT_NAME: &str = "  Trading API (CLOB)";
const REVIEWED_CREDENTIAL_DIRECTORY: &str = "/run/reap/pm-t2/credentials/pm-t2-slot-1";

#[test]
fn exact_locator_is_move_only_canonical_bound_redacted_and_denied_only() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let record = credential_locator(&fixture.config, &policy, &authorization);
    let path = fixture.write_json(
        PM_T2_REVIEWED_FRESH_CREDENTIAL_SLOT_LOCATOR_FILE_V1,
        &record,
    );
    let locator = load_canonical_reviewed_fresh_credential_slot_locator_v1(&path).unwrap();
    let verification = verify_reviewed_fresh_credential_slot_locator_v1(
        &fixture.config,
        &policy,
        &authorization,
        &locator,
    )
    .unwrap();

    assert!(verification.exact_v2_bindings_structurally_valid);
    assert!(verification.canonical_credential_slot_binding_structurally_valid);
    assert!(verification.fixed_fresh_credential_locator_structurally_valid);
    assert!(!verification.source_owned_current_time_checked);
    assert!(!verification.protected_credential_directory_and_four_files_checked);
    assert!(!verification.loaded_bundle_matches_credential_slot_generation);
    assert!(!verification.remote_api_key_owner_attested);
    assert!(!verification.locator_fingerprint_pinned_by_v2);
    assert!(!verification.reviewer_authorship_attested);
    assert!(!verification.load_token_consumption_durably_recorded);
    assert!(!verification.authorization_consumption_checked);
    assert!(!verification.authorization.production_order_entry_authorized);
    assert!(!verification.authorization.real_order_submission_authorized);
    assert_eq!(verification.authorization.place_dispatch_allowance, 0);
    assert_eq!(
        &record.files,
        &ReviewedFreshCredentialFilesV1 {
            private_key_entry: PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1.into(),
            api_key_entry: PM_T2_FRESH_API_KEY_ENTRY_V1.into(),
            l2_secret_entry: PM_T2_FRESH_L2_SECRET_ENTRY_V1.into(),
            passphrase_entry: PM_T2_FRESH_PASSPHRASE_ENTRY_V1.into(),
        }
    );
    assert_ne!(locator.canonical_sha256(), locator.fingerprint());
    assert_eq!(
        locator.canonical_length(),
        fs::metadata(path).unwrap().len()
    );
    assert_eq!(
        format!("{record:?}"),
        "ReviewedFreshCredentialSlotLocatorV1(<reviewed-nonsecret-locator; redacted>)"
    );
    assert_eq!(
        format!("{locator:?}"),
        "CanonicalReviewedFreshCredentialSlotLocatorV1(<reviewed-locator-evidence; exact-canonical-bytes; redacted>)"
    );

    let display = serde_json::to_string(&verification).unwrap();
    for private_topology in [
        REVIEWED_CREDENTIAL_DIRECTORY,
        PM_T2_FRESH_PRIVATE_KEY_ENTRY_V1,
        PM_T2_FRESH_API_KEY_ENTRY_V1,
        PM_T2_FRESH_L2_SECRET_ENTRY_V1,
        PM_T2_FRESH_PASSPHRASE_ENTRY_V1,
        "pm-t2-slot-1",
    ] {
        assert!(!display.contains(private_topology));
    }

    fn assert_send<T: Send>() {}
    assert_send::<reap_pm_controlled_trial::CanonicalReviewedFreshCredentialSlotLocatorV1>();
    assert_send::<reap_pm_controlled_trial::CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1>(
    );
    assert_send::<reap_pm_controlled_trial::ReviewedFreshCredentialLoadTokenV1>();

    let canonical_sha256 = locator.canonical_sha256().to_owned();
    let fingerprint = locator.fingerprint().to_owned();
    let canonical_length = locator.canonical_length();
    let (evidence, token) = bind_reviewed_fresh_credential_slot_locator_v1(
        &fixture.config,
        &policy,
        &authorization,
        locator,
    )
    .unwrap();
    assert_eq!(evidence.canonical_sha256(), canonical_sha256.as_str());
    assert_eq!(evidence.fingerprint(), fingerprint.as_str());
    assert_eq!(evidence.canonical_length(), canonical_length);
    assert_eq!(
        format!("{evidence:?}"),
        "CanonicalReviewedFreshCredentialSlotLocatorEvidenceV1(<retained-canonical-evidence; no-directory-authority; redacted>)"
    );
    assert_eq!(
        format!("{token:?}"),
        "ReviewedFreshCredentialLoadTokenV1(<one-shot-local-load; path-and-signer-redacted; denied>)"
    );
    let reverified = verify_reviewed_fresh_credential_slot_locator_evidence_v1(
        &fixture.config,
        &policy,
        &authorization,
        &evidence,
    )
    .unwrap();
    assert_eq!(reverified, verification);
    assert!(!reverified.authorization.production_order_entry_authorized);
    assert!(!reverified.locator_fingerprint_pinned_by_v2);
    assert!(!reverified.reviewer_authorship_attested);
    assert!(!reverified.load_token_consumption_durably_recorded);

    let (directory, configured_signer) = token.into_parts();
    // One-shot means one projection from this loaded holder. The projected
    // values are ordinary cloneable scalars; only the runner's private staged
    // loader can impose a linear single-load composition after this boundary.
    let copied_directory = directory.clone();
    let copied_signer = configured_signer.clone();
    assert_eq!(
        directory.as_path(),
        Path::new(REVIEWED_CREDENTIAL_DIRECTORY)
    );
    assert_eq!(
        configured_signer.as_str(),
        fixture.config.value().account.signer.as_str()
    );
    assert_eq!(copied_directory, directory);
    assert_eq!(copied_signer, configured_signer);
}

#[test]
fn loader_rejects_duplicate_unknown_noncanonical_and_unprotected_bytes() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let canonical = serde_json::to_vec(&credential_locator(
        &fixture.config,
        &policy,
        &authorization,
    ))
    .unwrap();

    let mut duplicate = b"{\"schema_version\":1,".to_vec();
    duplicate.extend_from_slice(&canonical[1..]);
    assert_locator_bytes_rejected(&duplicate);

    let mut unknown = canonical[..canonical.len() - 1].to_vec();
    unknown.extend_from_slice(b",\"credential_provider_owner_attested\":true}");
    assert_locator_bytes_rejected(&unknown);

    let mut trailing = canonical.clone();
    trailing.push(b'\n');
    assert_locator_bytes_rejected(&trailing);

    let mut reordered: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
    let object = reordered.as_object_mut().unwrap();
    let schema = object.remove("schema_version").unwrap();
    object.insert("schema_version".into(), schema);
    let reordered = serde_json::to_vec(&reordered).unwrap();
    assert_ne!(reordered, canonical);
    assert_locator_bytes_rejected(&reordered);

    let directory = protected_dir();
    let path = directory.path().join("wrong-mode.json");
    fs::write(&path, canonical).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(load_canonical_reviewed_fresh_credential_slot_locator_v1(&path).is_err());
}

#[test]
fn directory_and_four_role_names_are_closed_lexical_locator_facts_only() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let base = credential_locator(&fixture.config, &policy, &authorization);

    let basename_mutations: [fn(&mut ReviewedFreshCredentialSlotLocatorV1); 4] = [
        |locator| locator.files.private_key_entry = "signer-key".into(),
        |locator| locator.files.api_key_entry = "clob-api-key".into(),
        |locator| locator.files.l2_secret_entry = "secret".into(),
        |locator| locator.files.passphrase_entry = "password".into(),
    ];
    for mutate in basename_mutations {
        let mut record = base.clone();
        mutate(&mut record);
        assert_locator_record_rejected(&record);
    }

    for invalid_directory in [
        "run/reap/pm-t2/credentials",
        "/",
        "/run/reap/../credentials",
        "/run/reap/./credentials",
        "/run//reap/credentials",
        "/run/reap/credentials/",
        "/run/reap/credentials\nslot",
    ] {
        let mut record = base.clone();
        record.protected_fresh_credential_directory = invalid_directory.into();
        assert_locator_record_rejected(&record);
    }

    // Structural verification deliberately performs no lookup. The protected
    // sidecar and absolute directory remain caller-supplied and unattested by
    // frozen V2; protected-file custody does not attest reviewer authorship. A
    // reviewed lexical locator may be checked before the fresh runtime
    // directory is provisioned, and the result must preserve all filesystem
    // claims false.
    let not_provisioned = fixture.root.join("not-provisioned-credentials");
    assert!(!not_provisioned.exists());
    let mut lexical_only = base;
    lexical_only.protected_fresh_credential_directory =
        not_provisioned.to_str().unwrap().to_owned();
    let path = fixture.write_json("nonexistent-target-locator.json", &lexical_only);
    let locator = load_canonical_reviewed_fresh_credential_slot_locator_v1(&path).unwrap();
    let verification = verify_reviewed_fresh_credential_slot_locator_v1(
        &fixture.config,
        &policy,
        &authorization,
        &locator,
    )
    .unwrap();
    assert!(!verification.protected_credential_directory_and_four_files_checked);
    assert!(!verification.loaded_bundle_matches_credential_slot_generation);
    assert!(!verification.remote_api_key_owner_attested);
}

#[test]
fn verifier_requires_exact_three_way_slot_and_authorization_envelope_bindings() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let base = credential_locator(&fixture.config, &policy, &authorization);

    let drift_mutations: [fn(&mut ReviewedFreshCredentialSlotLocatorV1); 8] = [
        |locator| locator.v1_config.canonical_config_sha256 = "a1".repeat(32),
        |locator| locator.online_policy.fingerprint = "b2".repeat(32),
        |locator| locator.online_authorization.fingerprint = "c3".repeat(32),
        |locator| locator.credential_slot_id = "pm-t2-slot-2".into(),
        |locator| locator.credential_slot_nonsecret_fingerprint_sha256 = "d4".repeat(32),
        |locator| locator.valid_not_before_utc = "2026-08-09T12:00:01Z".into(),
        |locator| locator.valid_not_after_utc = "2026-08-09T12:20:01Z".into(),
        |locator| locator.reviewed_at_utc = "2026-08-09T11:59:59Z".into(),
    ];
    for mutate in drift_mutations {
        let mut record = base.clone();
        mutate(&mut record);
        let path = fixture.write_json("verification-drift-locator.json", &record);
        let loaded = load_canonical_reviewed_fresh_credential_slot_locator_v1(&path).unwrap();
        assert!(
            verify_reviewed_fresh_credential_slot_locator_v1(
                &fixture.config,
                &policy,
                &authorization,
                &loaded,
            )
            .is_err()
        );
    }
}

#[test]
fn one_shot_token_is_per_loaded_holder_not_v2_consumption_or_global_uniqueness() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let path = fixture.write_json(
        "repeatable-denied-locator.json",
        &credential_locator(&fixture.config, &policy, &authorization),
    );
    let first = load_canonical_reviewed_fresh_credential_slot_locator_v1(&path).unwrap();
    let second = load_canonical_reviewed_fresh_credential_slot_locator_v1(&path).unwrap();
    let (first_evidence, first_token) = bind_reviewed_fresh_credential_slot_locator_v1(
        &fixture.config,
        &policy,
        &authorization,
        first,
    )
    .unwrap();
    let (second_evidence, second_token) = bind_reviewed_fresh_credential_slot_locator_v1(
        &fixture.config,
        &policy,
        &authorization,
        second,
    )
    .unwrap();

    assert_eq!(first_evidence.fingerprint(), second_evidence.fingerprint());
    let verification = verify_reviewed_fresh_credential_slot_locator_evidence_v1(
        &fixture.config,
        &policy,
        &authorization,
        &first_evidence,
    )
    .unwrap();
    assert!(!verification.locator_fingerprint_pinned_by_v2);
    assert!(!verification.reviewer_authorship_attested);
    assert!(!verification.load_token_consumption_durably_recorded);
    assert!(!verification.authorization.production_order_entry_authorized);

    // Each token is one-shot only for its loaded holder. Reloading the same
    // protected, caller-supplied sidecar remains possible and is not durable
    // consumption, global uniqueness, credential delivery, or V2 authority.
    drop(first_token);
    drop(second_token);
    drop(first_evidence);
    drop(second_evidence);
}

#[test]
fn locator_is_additive_and_cannot_be_substituted_for_frozen_v2_records() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let locator_path = fixture.write_json(
        "distinct-locator.json",
        &credential_locator(&fixture.config, &policy, &authorization),
    );
    assert!(load_canonical_online_policy_v2(&locator_path).is_err());
    assert!(load_canonical_online_authorization_v2(&locator_path).is_err());

    let policy_path = fixture.write_json("distinct-policy.json", policy.value());
    assert!(load_canonical_reviewed_fresh_credential_slot_locator_v1(&policy_path).is_err());
    let authorization_path =
        fixture.write_json("distinct-authorization.json", authorization.value());
    assert!(load_canonical_reviewed_fresh_credential_slot_locator_v1(&authorization_path).is_err());
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

fn assert_locator_record_rejected(record: &ReviewedFreshCredentialSlotLocatorV1) {
    assert_locator_bytes_rejected(&serde_json::to_vec(record).unwrap());
}

fn assert_locator_bytes_rejected(bytes: &[u8]) {
    let directory = protected_dir();
    let path = directory.path().join("rejected-locator.json");
    write_0600(&path, bytes);
    assert!(load_canonical_reviewed_fresh_credential_slot_locator_v1(&path).is_err());
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
