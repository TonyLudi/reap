use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use reap_pm_controlled_trial::{
    ClobLivenessHealthObservationRequirementV2, FreshStatusAnnouncementObservationRequirementV2,
    OnlineAttemptScopeApprovalV2, OnlineAuthorizationApprovalV2, OnlineAuthorizationBuildBindingV2,
    OnlineAuthorizationHostBindingV2, OnlineAuthorizationPurposeV2, OnlineAuthorizationV2,
    OnlineCleanupApprovalV2, OnlineFillRiskApprovalV2, OnlinePhaseScopeApprovalV2,
    OnlinePolicyPinsV2, OnlinePolicyV2, OnlinePostOnlySemanticsApprovalV2,
    OnlineProxyConcurrencyApprovalV2, OnlineSourceSeparationApprovalV2,
    OperationalObservationProfileV2, ReviewedMarketClassificationV2, ReviewedMarketEvidenceV2,
    ReviewedMarketObservationRequirementV2, ReviewedRepositoryStateV2,
    ReviewedStatusClobComponentV2, ReviewedStatusHistoryObservationRequirementV2,
    ReviewedStatusNoticeHistoryCutV2, ReviewedStatusNoticeHistoryFindingV2,
    ReviewedStatusNoticeHistorySourceV2, SameAccountClosedOnlyObservationRequirementV2,
    TrialAccount, TrialConfig, TrialCredentialSlot, TrialDomain, TrialJournalBinding, TrialMarket,
    TrialOrder, TrialOrderType, TrialPhase, TrialSide, TrialTimeLimits, V1ConfigPinsV2,
    load_canonical_authorization, load_canonical_online_authorization_v2,
    load_canonical_online_policy_v2, load_canonical_trial_config, verify_online_authorization_v2,
    verify_online_policy_v2,
};
use tempfile::TempDir;

const SIGNER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const FUNDER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const REVIEWED_CLOB_COMPONENT_NAME: &str = "  Trading API (CLOB)";

#[test]
fn exact_v2_policy_and_authorization_are_distinct_move_only_denied_evidence() {
    let fixture = Fixture::new();
    let policy_record = online_policy(&fixture.config);
    let policy_path = fixture.write_json("online-policy-v2.json", &policy_record);
    let policy = load_canonical_online_policy_v2(&policy_path).unwrap();
    let policy_verification = verify_online_policy_v2(&fixture.config, &policy).unwrap();
    assert!(policy_verification.exact_v1_config_binding_structurally_valid);
    assert!(policy_verification.five_source_profile_structurally_valid);
    assert!(
        !policy_verification
            .authorization
            .production_order_entry_authorized
    );
    assert!(
        !policy_verification
            .authorization
            .real_order_submission_authorized
    );
    assert_eq!(
        policy_verification.authorization.place_dispatch_allowance,
        0
    );
    assert_eq!(
        policy.value().reviewed_status_clob_component.component_name,
        REVIEWED_CLOB_COMPONENT_NAME,
        "legitimate production leading spaces must remain exact",
    );
    assert_ne!(policy.canonical_sha256(), policy.fingerprint());
    assert_eq!(
        policy.canonical_length(),
        fs::metadata(&policy_path).unwrap().len()
    );
    assert_eq!(
        format!("{policy:?}"),
        "CanonicalOnlinePolicyV2(<reviewed-evidence; exact-canonical-bytes>)"
    );

    let authorization_record = online_authorization(&fixture.config, &policy);
    let authorization_path =
        fixture.write_json("online-authorization-v2.json", &authorization_record);
    let authorization = load_canonical_online_authorization_v2(&authorization_path).unwrap();
    let verification = verify_online_authorization_v2(
        &fixture.config,
        &policy,
        &authorization,
        utc("2026-08-09T12:05:00Z"),
    )
    .unwrap();
    assert!(verification.exact_bindings_structurally_valid);
    assert!(verification.within_short_lived_window_at_verification);
    assert!(verification.component_scoped_history_window_structurally_valid);
    assert!(!verification.authorization_consumption_checked);
    assert!(!verification.authorization.production_order_entry_authorized);
    assert!(!verification.authorization.real_order_submission_authorized);
    assert_eq!(verification.authorization.place_dispatch_allowance, 0);
    assert_ne!(
        authorization.canonical_sha256(),
        authorization.fingerprint()
    );
    assert_eq!(
        format!("{authorization:?}"),
        "CanonicalOnlineAuthorizationV2(<reviewed-evidence; exact-canonical-bytes>)"
    );
}

#[test]
fn v1_and_v2_loaders_never_mix_or_fall_back() {
    let fixture = Fixture::new();
    let policy_record = online_policy(&fixture.config);
    let policy_path = fixture.write_json("policy-for-mixing.json", &policy_record);
    let policy = load_canonical_online_policy_v2(&policy_path).unwrap();
    assert!(load_canonical_trial_config(&policy_path).is_err());
    assert!(load_canonical_authorization(&policy_path).is_err());

    let config_path = fixture.write_json("config-for-mixing.json", &trial_config());
    assert!(load_canonical_online_policy_v2(&config_path).is_err());
    assert!(load_canonical_online_authorization_v2(&config_path).is_err());

    let authorization_record = online_authorization(&fixture.config, &policy);
    let authorization_path =
        fixture.write_json("authorization-for-mixing.json", &authorization_record);
    assert!(load_canonical_trial_config(&authorization_path).is_err());
    assert!(load_canonical_authorization(&authorization_path).is_err());
    assert!(load_canonical_online_policy_v2(&authorization_path).is_err());
}

#[test]
fn policy_loader_rejects_duplicate_unknown_noncanonical_null_type_and_closed_enum_drift() {
    let fixture = Fixture::new();
    let canonical = serde_json::to_vec(&online_policy(&fixture.config)).unwrap();

    let mut duplicate = b"{\"schema_version\":2,".to_vec();
    duplicate.extend_from_slice(&canonical[1..]);
    assert_policy_bytes_rejected(&duplicate);

    let mut unknown = canonical[..canonical.len() - 1].to_vec();
    unknown.extend_from_slice(b",\"unknown\":null}");
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

    let canonical_text = String::from_utf8(canonical).unwrap();
    assert_policy_bytes_rejected(
        canonical_text
            .replacen(
                "\"policy_id\":\"pm-t2-online-policy-v2\"",
                "\"policy_id\":null",
                1,
            )
            .as_bytes(),
    );
    assert_policy_bytes_rejected(
        canonical_text
            .replacen(
                "\"maximum_observation_age_ms\":5000",
                "\"maximum_observation_age_ms\":\"5000\"",
                1,
            )
            .as_bytes(),
    );
    assert_policy_bytes_rejected(
        canonical_text
            .replacen("\"non_sports\"", "\"sports\"", 1)
            .as_bytes(),
    );
    assert_policy_bytes_rejected(
        canonical_text
            .replacen(
                "\"require_fixed_clob_get_ok_liveness_only\"",
                "\"require_fresh_summary_and_components\"",
                1,
            )
            .as_bytes(),
    );
}

#[test]
fn policy_rejects_phase_pin_age_quiet_interval_and_component_drift() {
    let fixture = Fixture::new();

    let pin_mutations: [fn(&mut OnlinePolicyV2); 4] = [
        |policy: &mut OnlinePolicyV2| policy.v1_config.canonical_config_sha256 = "aa".repeat(32),
        |policy: &mut OnlinePolicyV2| policy.v1_config.canonical_config_length += 1,
        |policy: &mut OnlinePolicyV2| {
            policy.v1_config.canonical_config_fingerprint = "bb".repeat(32)
        },
        |policy: &mut OnlinePolicyV2| policy.v1_config.trial_plan_fingerprint = "cc".repeat(32),
    ];
    for mutate in pin_mutations {
        let mut record = online_policy(&fixture.config);
        mutate(&mut record);
        let path = fixture.write_json("drift-policy.json", &record);
        let loaded = load_canonical_online_policy_v2(&path).unwrap();
        assert!(verify_online_policy_v2(&fixture.config, &loaded).is_err());
    }

    for invalid_age in [0, 5_001] {
        let mut record = online_policy(&fixture.config);
        record.maximum_observation_age_ms = invalid_age;
        assert_policy_record_rejected(&record);
    }
    for invalid_quiet in [86_399, 7_776_001] {
        let mut record = online_policy(&fixture.config);
        record.minimum_notice_history_quiet_interval_seconds = invalid_quiet;
        assert_policy_record_rejected(&record);
    }
    for invalid_name in ["", "CLOB\n"] {
        let mut record = online_policy(&fixture.config);
        record.reviewed_status_clob_component.component_name = invalid_name.into();
        assert_policy_record_rejected(&record);
    }

    let mut phase_b = trial_config();
    phase_b.phase = TrialPhase::BFillPosition;
    let phase_b_fixture = Fixture::from_config(phase_b);
    let phase_b_policy = online_policy(&phase_b_fixture.config);
    let path = phase_b_fixture.write_json("phase-b-policy.json", &phase_b_policy);
    let loaded = load_canonical_online_policy_v2(&path).unwrap();
    assert!(verify_online_policy_v2(&phase_b_fixture.config, &loaded).is_err());

    let mut tighter_config = trial_config();
    tighter_config
        .time_limits
        .maximum_preflight_observation_age_ms = 4_000;
    let tighter_fixture = Fixture::from_config(tighter_config);
    let policy_record = online_policy(&tighter_fixture.config);
    let path = tighter_fixture.write_json("too-old-for-v1.json", &policy_record);
    let loaded = load_canonical_online_policy_v2(&path).unwrap();
    assert!(verify_online_policy_v2(&tighter_fixture.config, &loaded).is_err());
}

#[test]
fn authorization_loader_rejects_duplicate_unknown_noncanonical_null_type_and_source_drift() {
    let fixture = Fixture::new();
    let policy_path = fixture.write_json("policy.json", &online_policy(&fixture.config));
    let policy = load_canonical_online_policy_v2(&policy_path).unwrap();
    let canonical = serde_json::to_vec(&online_authorization(&fixture.config, &policy)).unwrap();

    let mut duplicate = b"{\"schema_version\":2,".to_vec();
    duplicate.extend_from_slice(&canonical[1..]);
    assert_authorization_bytes_rejected(&duplicate);

    let mut unknown = canonical[..canonical.len() - 1].to_vec();
    unknown.extend_from_slice(b",\"unknown\":false}");
    assert_authorization_bytes_rejected(&unknown);

    let mut trailing = canonical.clone();
    trailing.push(b' ');
    assert_authorization_bytes_rejected(&trailing);

    let mut reordered: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
    let object = reordered.as_object_mut().unwrap();
    let id = object.remove("authorization_id").unwrap();
    object.insert("authorization_id".into(), id);
    let reordered = serde_json::to_vec(&reordered).unwrap();
    assert_ne!(reordered, canonical);
    assert_authorization_bytes_rejected(&reordered);

    let canonical_text = String::from_utf8(canonical).unwrap();
    for drifted in [
        canonical_text.replacen(
            "\"authorization_id\":\"pm-t2-online-authorization-v2\"",
            "\"authorization_id\":null",
            1,
        ),
        canonical_text.replacen("\"linux_euid\":1000", "\"linux_euid\":\"1000\"", 1),
        canonical_text.replacen(
            "\"official_polymarket_status_history\"",
            "\"unreviewed_status_source\"",
            1,
        ),
        canonical_text.replacen("\"exactly_one_place_dispatch\"", "true", 1),
    ] {
        assert_authorization_bytes_rejected(drifted.as_bytes());
    }
}

#[test]
fn authorization_rejects_config_policy_build_host_and_component_drift() {
    let fixture = Fixture::new();
    let policy_path = fixture.write_json("policy.json", &online_policy(&fixture.config));
    let policy = load_canonical_online_policy_v2(&policy_path).unwrap();

    let binding_mutations: [fn(&mut OnlineAuthorizationV2); 7] = [
        |record: &mut OnlineAuthorizationV2| {
            record.v1_config.canonical_config_sha256 = "aa".repeat(32)
        },
        |record: &mut OnlineAuthorizationV2| record.v1_config.canonical_config_length += 1,
        |record: &mut OnlineAuthorizationV2| {
            record.v1_config.canonical_config_fingerprint = "bb".repeat(32)
        },
        |record: &mut OnlineAuthorizationV2| {
            record.v1_config.trial_plan_fingerprint = "cc".repeat(32)
        },
        |record: &mut OnlineAuthorizationV2| {
            record.online_policy.canonical_sha256 = "dd".repeat(32)
        },
        |record: &mut OnlineAuthorizationV2| record.online_policy.canonical_length += 1,
        |record: &mut OnlineAuthorizationV2| record.online_policy.fingerprint = "ee".repeat(32),
    ];
    for mutate in binding_mutations {
        let mut record = online_authorization(&fixture.config, &policy);
        mutate(&mut record);
        assert_loaded_authorization_fails_verification(&fixture, &policy, &record);
    }

    let intrinsic_mutations: [fn(&mut OnlineAuthorizationV2); 12] = [
        |record: &mut OnlineAuthorizationV2| record.build.repository_commit = "A1".repeat(20),
        |record: &mut OnlineAuthorizationV2| record.build.cargo_lock_sha256 = "GG".repeat(32),
        |record: &mut OnlineAuthorizationV2| record.build.release_binary_sha256 = "00".repeat(31),
        |record: &mut OnlineAuthorizationV2| record.build.release_binary_length = 0,
        |record: &mut OnlineAuthorizationV2| record.host.uts_nodename = " bad-host".into(),
        |record: &mut OnlineAuthorizationV2| record.host.boot_id = "NOT-A-BOOT-ID".into(),
        |record: &mut OnlineAuthorizationV2| record.host.nss_username = "bad:user".into(),
        |record: &mut OnlineAuthorizationV2| record.host.linux_euid = 0,
        |record: &mut OnlineAuthorizationV2| record.host.linux_euid = u32::MAX,
        |record: &mut OnlineAuthorizationV2| record.host.authorized_egress_ip = "not-an-ip".into(),
        |record: &mut OnlineAuthorizationV2| {
            record.host.authorized_egress_ip = "2001:0db8::1".into()
        },
        |record: &mut OnlineAuthorizationV2| {
            record.status_notice_history.clob_component.component_name = "CLOB\n".into()
        },
    ];
    for mutate in intrinsic_mutations {
        let mut record = online_authorization(&fixture.config, &policy);
        mutate(&mut record);
        assert_authorization_record_rejected(&record);
    }

    let mut component_id_drift = online_authorization(&fixture.config, &policy);
    component_id_drift
        .status_notice_history
        .clob_component
        .component_id = "anotherclob".into();
    assert_loaded_authorization_fails_verification(&fixture, &policy, &component_id_drift);

    let mut component_name_drift = online_authorization(&fixture.config, &policy);
    component_name_drift
        .status_notice_history
        .clob_component
        .component_name = "Trading API (CLOB)".into();
    assert_loaded_authorization_fails_verification(&fixture, &policy, &component_name_drift);

    for invalid_id in ["", "Clobapi1", "clob-api-1"] {
        let mut record = online_authorization(&fixture.config, &policy);
        record.status_notice_history.clob_component.component_id = invalid_id.into();
        assert_authorization_record_rejected(&record);
    }
    let mut oversized_id = online_authorization(&fixture.config, &policy);
    oversized_id
        .status_notice_history
        .clob_component
        .component_id = "a".repeat(129);
    assert_authorization_record_rejected(&oversized_id);

    let mut unicode_internal_space_host = online_authorization(&fixture.config, &policy);
    unicode_internal_space_host.host.uts_nodename = "trial host λ".into();
    let path = fixture.write_json(
        "unicode-internal-space-host.json",
        &unicode_internal_space_host,
    );
    let loaded = load_canonical_online_authorization_v2(&path).unwrap();
    assert!(
        verify_online_authorization_v2(
            &fixture.config,
            &policy,
            &loaded,
            utc("2026-08-09T12:05:00Z"),
        )
        .is_ok()
    );
}

#[test]
fn authorization_rejects_timestamp_order_window_coverage_and_policy_review_drift() {
    let fixture = Fixture::new();
    let policy_path = fixture.write_json("policy.json", &online_policy(&fixture.config));
    let policy = load_canonical_online_policy_v2(&policy_path).unwrap();

    let time_mutations: [fn(&mut OnlineAuthorizationV2); 7] = [
        |record: &mut OnlineAuthorizationV2| {
            record.reviewed_at_utc = "2026-08-09T12:00:00.000Z".into()
        },
        |record: &mut OnlineAuthorizationV2| {
            record.reviewed_at_utc = "2026-08-09T13:00:00+01:00".into()
        },
        |record: &mut OnlineAuthorizationV2| record.reviewed_at_utc = "2026-08-09T12:01:00Z".into(),
        |record: &mut OnlineAuthorizationV2| record.expires_at_utc = "2026-08-09T12:15:01Z".into(),
        |record: &mut OnlineAuthorizationV2| {
            record.cleanup_not_after_utc = "2026-08-09T12:14:59Z".into()
        },
        |record: &mut OnlineAuthorizationV2| {
            record.status_notice_history.reviewed_through_utc = "2026-08-09T11:59:59Z".into()
        },
        |record: &mut OnlineAuthorizationV2| {
            record.status_notice_history.history_window_start_utc = "2026-08-09T12:00:00Z".into()
        },
    ];
    for mutate in time_mutations {
        let mut record = online_authorization(&fixture.config, &policy);
        mutate(&mut record);
        assert_authorization_record_rejected(&record);
    }

    let mut short_history = online_authorization(&fixture.config, &policy);
    short_history.status_notice_history.history_window_start_utc = "2026-08-08T12:00:01Z".into();
    assert_loaded_authorization_fails_verification(&fixture, &policy, &short_history);

    let record = online_authorization(&fixture.config, &policy);
    let path = fixture.write_json("outside-window.json", &record);
    let loaded = load_canonical_online_authorization_v2(&path).unwrap();
    for now in ["2026-08-09T11:59:59Z", "2026-08-09T12:15:00Z"] {
        assert!(
            verify_online_authorization_v2(&fixture.config, &policy, &loaded, utc(now)).is_err()
        );
    }

    let mut later_policy_record = online_policy(&fixture.config);
    later_policy_record.reviewed_at_utc = "2026-08-09T12:00:01Z".into();
    let later_policy_path = fixture.write_json("later-policy.json", &later_policy_record);
    let later_policy = load_canonical_online_policy_v2(&later_policy_path).unwrap();
    let auth_record = online_authorization(&fixture.config, &later_policy);
    let mut auth_record = auth_record;
    auth_record.reviewed_at_utc = "2026-08-09T12:00:00Z".into();
    auth_record.status_notice_history.reviewed_through_utc = "2026-08-09T12:00:00Z".into();
    let path = fixture.write_json("policy-review-order.json", &auth_record);
    let loaded = load_canonical_online_authorization_v2(&path).unwrap();
    assert!(
        verify_online_authorization_v2(
            &fixture.config,
            &later_policy,
            &loaded,
            utc("2026-08-09T12:05:00Z"),
        )
        .is_err()
    );
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
                FreshStatusAnnouncementObservationRequirementV2::RequireFreshSummaryAndComponents,
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
            authorized_egress_ip: "203.0.113.7".into(),
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

fn assert_loaded_authorization_fails_verification(
    fixture: &Fixture,
    policy: &reap_pm_controlled_trial::CanonicalOnlinePolicyV2,
    record: &OnlineAuthorizationV2,
) {
    let path = fixture.write_json("verification-drift-auth.json", record);
    let loaded = load_canonical_online_authorization_v2(&path).unwrap();
    assert!(
        verify_online_authorization_v2(
            &fixture.config,
            policy,
            &loaded,
            utc("2026-08-09T12:05:00Z"),
        )
        .is_err()
    );
}

fn assert_policy_record_rejected(record: &OnlinePolicyV2) {
    assert_policy_bytes_rejected(&serde_json::to_vec(record).unwrap());
}

fn assert_authorization_record_rejected(record: &OnlineAuthorizationV2) {
    assert_authorization_bytes_rejected(&serde_json::to_vec(record).unwrap());
}

fn assert_policy_bytes_rejected(bytes: &[u8]) {
    let directory = protected_dir();
    let path = directory.path().join("rejected-policy.json");
    write_0600(&path, bytes);
    assert!(load_canonical_online_policy_v2(&path).is_err());
}

fn assert_authorization_bytes_rejected(bytes: &[u8]) {
    let directory = protected_dir();
    let path = directory.path().join("rejected-authorization.json");
    write_0600(&path, bytes);
    assert!(load_canonical_online_authorization_v2(&path).is_err());
}

struct Fixture {
    _directory: TempDir,
    root: PathBuf,
    config: reap_pm_controlled_trial::CanonicalTrialConfig,
}

impl Fixture {
    fn new() -> Self {
        Self::from_config(trial_config())
    }

    fn from_config(record: TrialConfig) -> Self {
        let directory = protected_dir();
        let root = directory.path().to_owned();
        let config_path = root.join("canonical-config.json");
        write_0600(&config_path, &serde_json::to_vec(&record).unwrap());
        let config = load_canonical_trial_config(&config_path).unwrap();
        Self {
            _directory: directory,
            root,
            config,
        }
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

fn utc(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}
