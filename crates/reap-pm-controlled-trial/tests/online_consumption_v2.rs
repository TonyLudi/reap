use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
};

use reap_pm_controlled_trial::{
    ClobLivenessHealthObservationRequirementV2, FreshStatusAnnouncementObservationRequirementV2,
    OnlineAttemptScopeApprovalV2, OnlineAuthorizationApprovalV2, OnlineAuthorizationBuildBindingV2,
    OnlineAuthorizationConsumptionAttemptV2, OnlineAuthorizationConsumptionStateV2,
    OnlineAuthorizationCrashRecoveryV2, OnlineAuthorizationHostBindingV2,
    OnlineAuthorizationPlacementReuseV2, OnlineAuthorizationPurposeV2,
    OnlineAuthorizationRuntimeBindingV2, OnlineAuthorizationV2, OnlineCleanupApprovalV2,
    OnlineFillRiskApprovalV2, OnlinePhaseScopeApprovalV2, OnlinePolicyPinsV2, OnlinePolicyV2,
    OnlinePostOnlySemanticsApprovalV2, OnlineProxyConcurrencyApprovalV2,
    OnlineSourceSeparationApprovalV2, OperationalObservationProfileV2,
    PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_CLAIM_FILE_V2,
    PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_LEDGER_FILE_V2, PM_T2_ONLINE_PREFLIGHT_SIDECAR_FILE_V2,
    PmOnlineAuthorizationConsumptionV2Error, ReviewedDestinationIndependentNatV2,
    ReviewedLinuxEgressProfileV2, ReviewedMarketClassificationV2, ReviewedMarketEvidenceV2,
    ReviewedMarketObservationRequirementV2, ReviewedRepositoryStateV2,
    ReviewedStatusClobComponentV2, ReviewedStatusHistoryObservationRequirementV2,
    ReviewedStatusNoticeHistoryCutV2, ReviewedStatusNoticeHistoryFindingV2,
    ReviewedStatusNoticeHistorySourceV2, SameAccountClosedOnlyObservationRequirementV2,
    TrialAccount, TrialConfig, TrialCredentialSlot, TrialDomain, TrialJournalBinding, TrialMarket,
    TrialOrder, TrialOrderType, TrialPhase, TrialSide, TrialTimeLimits, V1ConfigPinsV2,
    load_canonical_online_authorization_v2, load_canonical_online_policy_v2,
    load_canonical_trial_config, prepare_online_authorization_consumption_v2,
    verify_online_authorization_consumption_v2,
};
use tempfile::TempDir;

const SIGNER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const FUNDER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

#[test]
fn prepared_to_atomic_claim_to_consumed_is_denied_move_only_evidence() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.canonical_v2();
    let mut prepared = prepare_online_authorization_consumption_v2(
        &fixture.config,
        policy,
        authorization,
        &runtime("2026-08-09T12:04:00Z"),
    )
    .unwrap();

    assert!(matches!(
        prepared.evidence().consumption,
        OnlineAuthorizationConsumptionStateV2::Prepared { .. }
    ));
    assert_denied(prepared.evidence().authorization);
    assert_ne!(
        prepared.binding_fingerprint(),
        prepared.prepared_record_fingerprint()
    );
    assert_eq!(
        format!("{prepared:?}"),
        "PreparedOnlineAuthorizationConsumptionV2(<denied-evidence; held-canonical-v2-records>)"
    );
    assert!(fixture.ledger_path().is_file());
    assert!(!fixture.claim_path().exists());
    assert!(!fixture.v1_ledger_path().exists());
    assert!(!fixture.v1_claim_path().exists());

    let verification = verify_online_authorization_consumption_v2(
        &fixture.config,
        prepared.policy(),
        prepared.authorization(),
    )
    .unwrap();
    assert_eq!(verification.ledger_record_count, 1);
    assert!(!verification.atomic_consumption_claim_durable);
    assert!(!verification.consumed_ledger_record_durable);
    assert!(!verification.ambiguous_tail);
    assert_denied(verification.authorization);

    write_0600(
        &fixture.root.join(PM_T2_ONLINE_PREFLIGHT_SIDECAR_FILE_V2),
        b"{\"basis\":\"denied-structural-evidence\"}\n",
    );
    prepared.refresh_after_bound_artifact_create().unwrap();
    let prepared_fingerprint = prepared.prepared_record_fingerprint().to_owned();
    let mut consumed = prepared
        .consume(&fixture.config, &runtime("2026-08-09T12:05:00Z"), attempt())
        .unwrap();
    consumed.revalidate_held_consumption_evidence().unwrap();
    assert_eq!(consumed.prepared_record_fingerprint(), prepared_fingerprint);
    assert_ne!(
        consumed.atomic_claim_fingerprint(),
        consumed.consumed_record_fingerprint()
    );
    assert_eq!(
        format!("{consumed:?}"),
        "ConsumedOnlineAuthorizationConsumptionV2(<denied-burn-evidence; held-canonical-v2-records>)"
    );
    match &consumed.evidence().consumption {
        OnlineAuthorizationConsumptionStateV2::Consumed {
            attempt: attempt_evidence,
            placement_reuse,
            crash_recovery,
            ..
        } => {
            assert_eq!(attempt_evidence, &attempt());
            assert_eq!(
                *placement_reuse,
                OnlineAuthorizationPlacementReuseV2::PermanentlyBurned
            );
            assert_eq!(
                *crash_recovery,
                OnlineAuthorizationCrashRecoveryV2::ExistingV1LifecycleOnlyNoPlacementResume
            );
        }
        _ => panic!("expected Consumed V2 evidence"),
    }
    assert_denied(consumed.evidence().authorization);

    let verification = verify_online_authorization_consumption_v2(
        &fixture.config,
        consumed.policy(),
        consumed.authorization(),
    )
    .unwrap();
    assert_eq!(verification.ledger_record_count, 2);
    assert!(verification.atomic_consumption_claim_durable);
    assert!(verification.consumed_ledger_record_durable);
    assert!(verification.atomic_claim_fingerprint.is_some());
    assert_denied(verification.authorization);

    drop(consumed);
    let (policy, authorization) = fixture.canonical_v2();
    assert!(matches!(
        prepare_online_authorization_consumption_v2(
            &fixture.config,
            policy,
            authorization,
            &runtime("2026-08-09T12:06:00Z"),
        ),
        Err(PmOnlineAuthorizationConsumptionV2Error::AlreadyConsumed(_))
    ));
}

#[test]
fn prepare_rejects_every_runtime_build_host_egress_and_window_drift() {
    let mutations: [fn(&mut OnlineAuthorizationRuntimeBindingV2); 14] = [
        |value| value.release_binary_sha256 = "89".repeat(32),
        |value| value.release_binary_length += 1,
        |value| value.uts_nodename = "other-host".into(),
        |value| value.boot_id = "11234567-89ab-cdef-0123-456789abcdef".into(),
        |value| value.nss_username = "other-user".into(),
        |value| value.linux_euid += 1,
        |value| value.network_namespace_device += 1,
        |value| value.network_namespace_inode += 1,
        |value| value.interface_name = "eth0".into(),
        |value| value.interface_index += 1,
        |value| value.local_source_ip = "10.0.0.3".into(),
        |value| value.geoblock_reported_public_ip = "1.1.1.1".into(),
        |value| value.observed_at_utc = "2026-08-09T11:59:59Z".into(),
        |value| value.observed_at_utc = "2026-08-09T12:15:00Z".into(),
    ];
    for mutate in mutations {
        let fixture = Fixture::new();
        let (policy, authorization) = fixture.canonical_v2();
        let mut observation = runtime("2026-08-09T12:04:00Z");
        mutate(&mut observation);
        assert!(
            prepare_online_authorization_consumption_v2(
                &fixture.config,
                policy,
                authorization,
                &observation,
            )
            .is_err()
        );
        assert!(!fixture.ledger_path().exists());
        assert!(!fixture.claim_path().exists());
    }
}

#[test]
fn consume_rechecks_runtime_time_and_exact_nonzero_sidecar_basis_before_claim() {
    for case in 0..3 {
        let fixture = Fixture::new();
        let (policy, authorization) = fixture.canonical_v2();
        let prepared = prepare_online_authorization_consumption_v2(
            &fixture.config,
            policy,
            authorization,
            &runtime("2026-08-09T12:05:00Z"),
        )
        .unwrap();
        let mut observation = runtime("2026-08-09T12:06:00Z");
        let mut attempt = attempt();
        match case {
            0 => observation.observed_at_utc = "2026-08-09T12:04:59Z".into(),
            1 => observation.network_namespace_inode += 1,
            2 => {
                attempt.online_preflight_basis_record_fingerprint = "00".repeat(32);
            }
            _ => unreachable!(),
        }
        assert!(
            prepared
                .consume(&fixture.config, &observation, attempt)
                .is_err()
        );
        assert!(!fixture.claim_path().exists());
        let (policy, authorization) = fixture.canonical_v2();
        let verification =
            verify_online_authorization_consumption_v2(&fixture.config, &policy, &authorization)
                .unwrap();
        assert!(matches!(
            verification.state,
            OnlineAuthorizationConsumptionStateV2::Prepared { .. }
        ));
    }
}

#[test]
fn full_cleanup_runway_accepts_equality_and_rejects_one_millisecond_shortfall_and_overflow() {
    let fixture = Fixture::with_cleanup_runway(120_000, "2026-08-09T12:15:00Z");
    let (policy, authorization) = fixture.canonical_v2();
    let prepared = prepare_online_authorization_consumption_v2(
        &fixture.config,
        policy,
        authorization,
        &runtime("2026-08-09T12:13:00Z"),
    )
    .unwrap();
    drop(prepared);

    let fixture = Fixture::with_cleanup_runway(120_001, "2026-08-09T12:15:00Z");
    let (policy, authorization) = fixture.canonical_v2();
    assert!(
        prepare_online_authorization_consumption_v2(
            &fixture.config,
            policy,
            authorization,
            &runtime("2026-08-09T12:13:00Z"),
        )
        .is_err()
    );
    assert!(!fixture.ledger_path().exists());

    let fixture = Fixture::with_cleanup_runway(120_001, "2026-08-09T12:15:00Z");
    let (policy, authorization) = fixture.canonical_v2();
    let prepared = prepare_online_authorization_consumption_v2(
        &fixture.config,
        policy,
        authorization,
        &runtime("2026-08-09T12:12:59Z"),
    )
    .unwrap();
    assert!(
        prepared
            .consume(&fixture.config, &runtime("2026-08-09T12:13:00Z"), attempt(),)
            .is_err()
    );
    assert!(!fixture.claim_path().exists());

    let fixture = Fixture::with_cleanup_runway(u64::MAX, "2026-08-09T12:20:00Z");
    let (policy, authorization) = fixture.canonical_v2();
    assert!(
        prepare_online_authorization_consumption_v2(
            &fixture.config,
            policy,
            authorization,
            &runtime("2026-08-09T12:04:00Z"),
        )
        .is_err()
    );
    assert!(!fixture.ledger_path().exists());
}

#[test]
fn claim_only_crash_prefix_is_burned_evidence_and_two_records_require_claim() {
    let fixture = Fixture::new();
    consume_fixture(&fixture);
    let bytes = fs::read(fixture.ledger_path()).unwrap();
    let first_end = bytes.iter().position(|byte| *byte == b'\n').unwrap() + 1;
    fs::write(fixture.ledger_path(), &bytes[..first_end]).unwrap();
    let (policy, authorization) = fixture.canonical_v2();
    let verification =
        verify_online_authorization_consumption_v2(&fixture.config, &policy, &authorization)
            .unwrap();
    assert_eq!(verification.ledger_record_count, 1);
    assert!(verification.atomic_consumption_claim_durable);
    assert!(!verification.consumed_ledger_record_durable);
    assert!(matches!(
        verification.state,
        OnlineAuthorizationConsumptionStateV2::Consumed {
            placement_reuse: OnlineAuthorizationPlacementReuseV2::PermanentlyBurned,
            crash_recovery:
                OnlineAuthorizationCrashRecoveryV2::ExistingV1LifecycleOnlyNoPlacementResume,
            ..
        }
    ));
    assert_denied(verification.authorization);

    let fixture = Fixture::new();
    consume_fixture(&fixture);
    fs::remove_file(fixture.claim_path()).unwrap();
    let (policy, authorization) = fixture.canonical_v2();
    assert!(
        verify_online_authorization_consumption_v2(&fixture.config, &policy, &authorization,)
            .is_err()
    );
}

#[test]
fn verifier_rejects_duplicate_unknown_noncanonical_null_type_torn_and_extra_records() {
    for case in 0..8 {
        let fixture = Fixture::new();
        prepare_fixture(&fixture);
        let canonical = fs::read(fixture.ledger_path()).unwrap();
        let mutated = match case {
            0 => {
                let mut value = b"{\"schema_version\":2,".to_vec();
                value.extend_from_slice(&canonical[1..]);
                value
            }
            1 => {
                let mut value = canonical[..canonical.len() - 2].to_vec();
                value.extend_from_slice(b",\"unknown\":null}\n");
                value
            }
            2 => {
                let mut value = b" ".to_vec();
                value.extend_from_slice(&canonical);
                value
            }
            3 => String::from_utf8(canonical)
                .unwrap()
                .replacen("\"sequence\":0", "\"sequence\":null", 1)
                .into_bytes(),
            4 => String::from_utf8(canonical)
                .unwrap()
                .replacen("\"sequence\":0", "\"sequence\":\"0\"", 1)
                .into_bytes(),
            5 => canonical[..canonical.len() - 1].to_vec(),
            6 => canonical.repeat(3),
            7 => {
                let mut value = canonical.clone();
                value.push(b'\n');
                value.extend_from_slice(&canonical);
                value
            }
            _ => unreachable!(),
        };
        fs::write(fixture.ledger_path(), mutated).unwrap();
        let (policy, authorization) = fixture.canonical_v2();
        assert!(
            verify_online_authorization_consumption_v2(&fixture.config, &policy, &authorization,)
                .is_err(),
            "malformed ledger case {case} was accepted",
        );
    }
}

#[test]
fn verifier_rejects_duplicate_unknown_noncanonical_null_type_time_binding_and_mode_claims() {
    for case in 0..9 {
        let fixture = Fixture::new();
        consume_fixture(&fixture);
        let canonical = fs::read(fixture.claim_path()).unwrap();
        if case == 8 {
            fs::set_permissions(fixture.claim_path(), fs::Permissions::from_mode(0o644)).unwrap();
        } else {
            let mutated = match case {
                0 => {
                    let mut value = b"{\"schema_version\":2,".to_vec();
                    value.extend_from_slice(&canonical[1..]);
                    value
                }
                1 => {
                    let mut value = canonical[..canonical.len() - 1].to_vec();
                    value.extend_from_slice(b",\"unknown\":null}");
                    value
                }
                2 => {
                    let mut value = b" ".to_vec();
                    value.extend_from_slice(&canonical);
                    value
                }
                3 => String::from_utf8(canonical)
                    .unwrap()
                    .replacen("\"schema_version\":2", "\"schema_version\":null", 1)
                    .into_bytes(),
                4 => String::from_utf8(canonical)
                    .unwrap()
                    .replacen("\"schema_version\":2", "\"schema_version\":\"2\"", 1)
                    .into_bytes(),
                5 => String::from_utf8(canonical)
                    .unwrap()
                    .replacen(
                        "\"consumed_at_utc\":\"2026-08-09T12:05:00Z\"",
                        "\"consumed_at_utc\":\"2026-08-09T12:03:00Z\"",
                        1,
                    )
                    .into_bytes(),
                6 => String::from_utf8(canonical)
                    .unwrap()
                    .replacen("\"interface_index\":7", "\"interface_index\":8", 1)
                    .into_bytes(),
                7 => canonical[..canonical.len() - 1].to_vec(),
                _ => unreachable!(),
            };
            fs::write(fixture.claim_path(), mutated).unwrap();
        }
        let (policy, authorization) = fixture.canonical_v2();
        assert!(
            verify_online_authorization_consumption_v2(&fixture.config, &policy, &authorization,)
                .is_err(),
            "malformed claim case {case} was accepted",
        );
    }
}

#[test]
fn protected_ledger_mode_tamper_and_v1_v2_filename_collision_fail_closed() {
    let fixture = Fixture::new();
    prepare_fixture(&fixture);
    fs::set_permissions(fixture.ledger_path(), fs::Permissions::from_mode(0o644)).unwrap();
    let (policy, authorization) = fixture.canonical_v2();
    assert!(
        verify_online_authorization_consumption_v2(&fixture.config, &policy, &authorization,)
            .is_err()
    );

    let fixture = Fixture::new();
    let (policy, authorization) = fixture.canonical_v2();
    let mut prepared = prepare_online_authorization_consumption_v2(
        &fixture.config,
        policy,
        authorization,
        &runtime("2026-08-09T12:04:00Z"),
    )
    .unwrap();
    let mut bytes = fs::read(fixture.ledger_path()).unwrap();
    bytes.push(b' ');
    fs::write(fixture.ledger_path(), bytes).unwrap();
    assert!(prepared.revalidate_held_consumption_evidence().is_err());

    let directory = protected_dir();
    let mut colliding_v1 = trial_config(directory.path());
    colliding_v1.journal.authorization_consumption_ledger_file =
        PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_LEDGER_FILE_V2.into();
    let path = directory.path().join("colliding-v1-config.json");
    write_0600(&path, &serde_json::to_vec(&colliding_v1).unwrap());
    assert!(load_canonical_trial_config(&path).is_err());
}

fn prepare_fixture(fixture: &Fixture) {
    let (policy, authorization) = fixture.canonical_v2();
    let prepared = prepare_online_authorization_consumption_v2(
        &fixture.config,
        policy,
        authorization,
        &runtime("2026-08-09T12:04:00Z"),
    )
    .unwrap();
    drop(prepared);
}

fn consume_fixture(fixture: &Fixture) {
    let (policy, authorization) = fixture.canonical_v2();
    let prepared = prepare_online_authorization_consumption_v2(
        &fixture.config,
        policy,
        authorization,
        &runtime("2026-08-09T12:04:00Z"),
    )
    .unwrap();
    let consumed = prepared
        .consume(&fixture.config, &runtime("2026-08-09T12:05:00Z"), attempt())
        .unwrap();
    drop(consumed);
}

fn attempt() -> OnlineAuthorizationConsumptionAttemptV2 {
    OnlineAuthorizationConsumptionAttemptV2 {
        online_preflight_basis_record_fingerprint: "cd".repeat(32),
    }
}

fn runtime(observed_at_utc: &str) -> OnlineAuthorizationRuntimeBindingV2 {
    OnlineAuthorizationRuntimeBindingV2 {
        release_binary_sha256: "88".repeat(32),
        release_binary_length: 1_000_000,
        uts_nodename: "trial-host-1".into(),
        boot_id: "01234567-89ab-cdef-0123-456789abcdef".into(),
        nss_username: "reap-trial".into(),
        linux_euid: 1_000,
        network_namespace_device: 4,
        network_namespace_inode: 4_026_531_999,
        interface_name: "wg0".into(),
        interface_index: 7,
        local_source_ip: "10.0.0.2".into(),
        geoblock_reported_public_ip: "8.8.8.8".into(),
        observed_at_utc: observed_at_utc.into(),
    }
}

fn assert_denied(value: reap_pm_controlled_trial::OfflineAuthorizationState) {
    assert!(!value.production_order_entry_authorized);
    assert!(!value.real_order_submission_authorized);
    assert_eq!(value.place_dispatch_allowance, 0);
}

struct Fixture {
    _directory: TempDir,
    root: PathBuf,
    config: reap_pm_controlled_trial::CanonicalTrialConfig,
    policy_path: PathBuf,
    authorization_path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        Self::configured(120_000, "2026-08-09T12:20:00Z")
    }

    fn with_cleanup_runway(cleanup_runway_ms: u64, authorization_cleanup: &str) -> Self {
        Self::configured(cleanup_runway_ms, authorization_cleanup)
    }

    fn configured(cleanup_runway_ms: u64, authorization_cleanup: &str) -> Self {
        let directory = protected_dir();
        let root = directory.path().to_owned();
        let mut config_record = trial_config(&root);
        config_record.time_limits.cleanup_not_after_ms = cleanup_runway_ms;
        let config_path = root.join("canonical-config.json");
        write_0600(&config_path, &serde_json::to_vec(&config_record).unwrap());
        let config = load_canonical_trial_config(&config_path).unwrap();
        let policy_path = root.join("online-policy-v2.json");
        write_0600(
            &policy_path,
            &serde_json::to_vec(&online_policy(&config)).unwrap(),
        );
        let policy = load_canonical_online_policy_v2(&policy_path).unwrap();
        let authorization_path = root.join("online-authorization-v2.json");
        let mut authorization = online_authorization(&config, &policy);
        authorization.cleanup_not_after_utc = authorization_cleanup.into();
        write_0600(
            &authorization_path,
            &serde_json::to_vec(&authorization).unwrap(),
        );
        Self {
            _directory: directory,
            root,
            config,
            policy_path,
            authorization_path,
        }
    }

    fn canonical_v2(
        &self,
    ) -> (
        reap_pm_controlled_trial::CanonicalOnlinePolicyV2,
        reap_pm_controlled_trial::CanonicalOnlineAuthorizationV2,
    ) {
        (
            load_canonical_online_policy_v2(&self.policy_path).unwrap(),
            load_canonical_online_authorization_v2(&self.authorization_path).unwrap(),
        )
    }

    fn ledger_path(&self) -> PathBuf {
        self.root
            .join(PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_LEDGER_FILE_V2)
    }

    fn claim_path(&self) -> PathBuf {
        self.root
            .join(PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_CLAIM_FILE_V2)
    }

    fn v1_ledger_path(&self) -> PathBuf {
        self.root.join(
            &self
                .config
                .value()
                .journal
                .authorization_consumption_ledger_file,
        )
    }

    fn v1_claim_path(&self) -> PathBuf {
        self.root.join(
            &self
                .config
                .value()
                .journal
                .authorization_consumption_claim_file,
        )
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
                FreshStatusAnnouncementObservationRequirementV2::RequireFreshSummaryAndComponents,
            clob_ok_liveness_health:
                ClobLivenessHealthObservationRequirementV2::RequireFixedClobGetOkLivenessOnly,
            same_account_closed_only:
                SameAccountClosedOnlyObservationRequirementV2::RequireSignerAuthenticatedFalseForExactAccount,
        },
        reviewed_status_clob_component: ReviewedStatusClobComponentV2 {
            component_id: "clobapi1".into(),
            component_name: "  Trading API (CLOB)".into(),
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

fn trial_config(root: &Path) -> TrialConfig {
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
            artifact_directory: root.to_str().unwrap().into(),
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
