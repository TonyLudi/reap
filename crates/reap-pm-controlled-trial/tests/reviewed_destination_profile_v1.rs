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
    OperationalObservationProfileV2, ReviewedDestinationIndependentNatV2,
    ReviewedDnsAnswerEvidenceV1, ReviewedDnsAnswerSourceV1, ReviewedFixedTlsDestinationV1,
    ReviewedFixedWebSocketDestinationV1, ReviewedLinuxEgressProfileV2,
    ReviewedMarketClassificationV2, ReviewedMarketEvidenceV2,
    ReviewedMarketObservationRequirementV2, ReviewedOnlineAuthorizationPinsV1,
    ReviewedProductionDestinationProfileV1, ReviewedProductionDestinationsV1,
    ReviewedRepositoryStateV2, ReviewedStatusClobComponentV2,
    ReviewedStatusHistoryObservationRequirementV2, ReviewedStatusNoticeHistoryCutV2,
    ReviewedStatusNoticeHistoryFindingV2, ReviewedStatusNoticeHistorySourceV2,
    SameAccountClosedOnlyObservationRequirementV2, TrialAccount, TrialConfig, TrialCredentialSlot,
    TrialDomain, TrialJournalBinding, TrialMarket, TrialOrder, TrialOrderType, TrialPhase,
    TrialSide, TrialTimeLimits, V1ConfigPinsV2, load_canonical_online_authorization_v2,
    load_canonical_online_policy_v2, load_canonical_reviewed_production_destination_profile_v1,
    load_canonical_trial_config, verify_reviewed_production_destination_profile_v1,
};
use tempfile::TempDir;

const SIGNER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const FUNDER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const REVIEWED_CLOB_COMPONENT_NAME: &str = "  Trading API (CLOB)";

#[test]
fn exact_profile_is_move_only_canonical_bound_and_denied_only() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let record = destination_profile(&fixture.config, &policy, &authorization);
    let path = fixture.write_json("reviewed-production-destinations-v1.json", &record);
    let profile = load_canonical_reviewed_production_destination_profile_v1(&path).unwrap();
    let verification = verify_reviewed_production_destination_profile_v1(
        &fixture.config,
        &policy,
        &authorization,
        &profile,
    )
    .unwrap();

    assert!(verification.exact_v2_bindings_structurally_valid);
    assert!(verification.fixed_six_destination_profile_structurally_valid);
    assert!(!verification.live_dns_observation_checked);
    assert!(!verification.destination_nat_equivalence_checked);
    assert!(!verification.authorization_consumption_checked);
    assert!(!verification.authorization.production_order_entry_authorized);
    assert!(!verification.authorization.real_order_submission_authorized);
    assert_eq!(verification.authorization.place_dispatch_allowance, 0);
    assert_eq!(
        serde_json::to_value(&verification).unwrap()["production_order_entry_authorized"],
        false
    );
    assert_eq!(
        profile.value().destinations.clob_websocket_wss.public_path,
        "/ws/market"
    );
    assert_eq!(
        profile.value().destinations.clob_websocket_wss.user_path,
        "/ws/user"
    );
    assert_eq!(
        serde_json::to_value(&profile.value().destinations)
            .unwrap()
            .as_object()
            .unwrap()
            .len(),
        6
    );
    assert_ne!(profile.canonical_sha256(), profile.fingerprint());
    assert_eq!(
        profile.canonical_length(),
        fs::metadata(path).unwrap().len()
    );
    assert_eq!(
        format!("{profile:?}"),
        "CanonicalReviewedProductionDestinationProfileV1(<reviewed-evidence; exact-canonical-bytes>)"
    );
}

#[test]
fn loader_rejects_duplicate_unknown_noncanonical_and_unprotected_bytes() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let canonical = serde_json::to_vec(&destination_profile(
        &fixture.config,
        &policy,
        &authorization,
    ))
    .unwrap();

    let mut duplicate = b"{\"schema_version\":1,".to_vec();
    duplicate.extend_from_slice(&canonical[1..]);
    assert_profile_bytes_rejected(&duplicate);

    let mut unknown = canonical[..canonical.len() - 1].to_vec();
    unknown.extend_from_slice(b",\"alternate_peer_ips\":[\"8.8.8.8\"]}");
    assert_profile_bytes_rejected(&unknown);

    let mut trailing = canonical.clone();
    trailing.push(b'\n');
    assert_profile_bytes_rejected(&trailing);

    let mut reordered: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
    let object = reordered.as_object_mut().unwrap();
    let schema = object.remove("schema_version").unwrap();
    object.insert("schema_version".into(), schema);
    let reordered = serde_json::to_vec(&reordered).unwrap();
    assert_ne!(reordered, canonical);
    assert_profile_bytes_rejected(&reordered);

    let directory = protected_dir();
    let path = directory.path().join("wrong-mode.json");
    fs::write(&path, canonical).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(load_canonical_reviewed_production_destination_profile_v1(&path).is_err());
}

#[test]
fn exact_hosts_sni_hosts_ports_paths_and_scalar_public_peers_are_closed() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let base = destination_profile(&fixture.config, &policy, &authorization);

    let mutations: [fn(&mut ReviewedProductionDestinationProfileV1); 8] = [
        |profile| profile.destinations.geoblock_https.dns_name = "www.polymarket.com".into(),
        |profile| profile.destinations.clob_https.tls_server_name = "polymarket.com".into(),
        |profile| profile.destinations.status_https.http_host = "STATUS.polymarket.com".into(),
        |profile| profile.destinations.data_api_https.tcp_port = 8443,
        |profile| profile.destinations.polygon_rpc_https.peer_ip = "10.0.0.1".into(),
        |profile| profile.destinations.clob_websocket_wss.dns_name = "clob.polymarket.com".into(),
        |profile| profile.destinations.clob_websocket_wss.public_path = "/ws/public".into(),
        |profile| profile.destinations.clob_websocket_wss.user_path = "/ws/private".into(),
    ];
    for mutate in mutations {
        let mut record = base.clone();
        mutate(&mut record);
        assert_profile_record_rejected(&record);
    }

    for invalid_peer in [
        "0.0.0.0",
        "127.0.0.1",
        "100.64.0.1",
        "169.254.1.1",
        "192.0.2.1",
        "198.51.100.1",
        "203.0.113.1",
        "224.0.0.1",
        "::ffff:8.8.8.8",
        "2001:db8::1",
        "2606:4700:4700:0000:0000:0000:0000:1111",
    ] {
        let mut record = base.clone();
        record.destinations.geoblock_https.peer_ip = invalid_peer.into();
        assert_profile_record_rejected(&record);
    }

    let mut value = serde_json::to_value(base).unwrap();
    value["destinations"]["geoblock_https"]["peer_ips"] = serde_json::json!(["8.8.8.8", "1.1.1.1"]);
    assert_profile_bytes_rejected(&serde_json::to_vec(&value).unwrap());
}

#[test]
fn verifier_requires_exact_three_way_pins_envelope_age_and_address_family() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let base = destination_profile(&fixture.config, &policy, &authorization);

    let drift_mutations: [fn(&mut ReviewedProductionDestinationProfileV1); 6] = [
        |profile| profile.v1_config.canonical_config_sha256 = "a1".repeat(32),
        |profile| profile.online_policy.fingerprint = "b2".repeat(32),
        |profile| profile.online_authorization.fingerprint = "c3".repeat(32),
        |profile| profile.valid_not_before_utc = "2026-08-09T12:00:01Z".into(),
        |profile| profile.valid_not_after_utc = "2026-08-09T12:20:01Z".into(),
        |profile| profile.reviewed_at_utc = "2026-08-09T11:59:59Z".into(),
    ];
    for mutate in drift_mutations {
        let mut record = base.clone();
        mutate(&mut record);
        let path = fixture.write_json("verification-drift-profile.json", &record);
        let loaded = load_canonical_reviewed_production_destination_profile_v1(&path).unwrap();
        assert!(
            verify_reviewed_production_destination_profile_v1(
                &fixture.config,
                &policy,
                &authorization,
                &loaded,
            )
            .is_err()
        );
    }

    let boundary_path = fixture.write_json("age-boundary-profile.json", &base);
    let boundary =
        load_canonical_reviewed_production_destination_profile_v1(&boundary_path).unwrap();
    assert!(
        verify_reviewed_production_destination_profile_v1(
            &fixture.config,
            &policy,
            &authorization,
            &boundary,
        )
        .is_ok()
    );

    let mut too_old = base.clone();
    too_old.dns_review.resolved_at_utc = "2026-08-09T11:54:59Z".into();
    let path = fixture.write_json("too-old-profile.json", &too_old);
    let loaded = load_canonical_reviewed_production_destination_profile_v1(&path).unwrap();
    assert!(
        verify_reviewed_production_destination_profile_v1(
            &fixture.config,
            &policy,
            &authorization,
            &loaded,
        )
        .is_err()
    );

    let mut mixed_family = base;
    mixed_family.destinations.clob_websocket_wss.peer_ip = "2606:4700:4700::1111".into();
    let path = fixture.write_json("mixed-family-profile.json", &mixed_family);
    let loaded = load_canonical_reviewed_production_destination_profile_v1(&path).unwrap();
    assert!(
        verify_reviewed_production_destination_profile_v1(
            &fixture.config,
            &policy,
            &authorization,
            &loaded,
        )
        .is_err()
    );
}

#[test]
fn destination_profile_is_additive_and_never_falls_back_to_v2_records() {
    let fixture = Fixture::new();
    let (policy, authorization) = fixture.online_records();
    let profile_path = fixture.write_json(
        "distinct-profile.json",
        &destination_profile(&fixture.config, &policy, &authorization),
    );
    assert!(load_canonical_online_policy_v2(&profile_path).is_err());
    assert!(load_canonical_online_authorization_v2(&profile_path).is_err());

    let policy_path = fixture.write_json("distinct-policy.json", policy.value());
    assert!(load_canonical_reviewed_production_destination_profile_v1(&policy_path).is_err());
    let authorization_path =
        fixture.write_json("distinct-authorization.json", authorization.value());
    assert!(
        load_canonical_reviewed_production_destination_profile_v1(&authorization_path).is_err()
    );
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

fn assert_profile_record_rejected(record: &ReviewedProductionDestinationProfileV1) {
    assert_profile_bytes_rejected(&serde_json::to_vec(record).unwrap());
}

fn assert_profile_bytes_rejected(bytes: &[u8]) {
    let directory = protected_dir();
    let path = directory.path().join("rejected-profile.json");
    write_0600(&path, bytes);
    assert!(load_canonical_reviewed_production_destination_profile_v1(&path).is_err());
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
