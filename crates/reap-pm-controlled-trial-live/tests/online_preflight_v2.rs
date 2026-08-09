mod support;

use std::{fs, io::Write as _, os::unix::fs::PermissionsExt as _, path::Path};

use reap_pm_controlled_trial::{
    ClobLivenessHealthObservationRequirementV2, FreshStatusAnnouncementObservationRequirementV2,
    OnlineAttemptScopeApprovalV2, OnlineAuthorizationApprovalV2, OnlineAuthorizationBuildBindingV2,
    OnlineAuthorizationHostBindingV2, OnlineAuthorizationPurposeV2,
    OnlineAuthorizationRuntimeBindingV2, OnlineAuthorizationV2, OnlineCleanupApprovalV2,
    OnlineFillRiskApprovalV2, OnlinePhaseScopeApprovalV2, OnlinePolicyPinsV2, OnlinePolicyV2,
    OnlinePostOnlySemanticsApprovalV2, OnlineProxyConcurrencyApprovalV2,
    OnlineSourceSeparationApprovalV2, OperationalObservationProfileV2,
    PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_CLAIM_FILE_V2, PreparedOnlineAuthorizationConsumptionV2,
    ReviewedDestinationIndependentNatV2, ReviewedLinuxEgressProfileV2,
    ReviewedMarketClassificationV2, ReviewedMarketEvidenceV2,
    ReviewedMarketObservationRequirementV2, ReviewedRepositoryStateV2,
    ReviewedStatusClobComponentV2, ReviewedStatusHistoryObservationRequirementV2,
    ReviewedStatusNoticeHistoryCutV2, ReviewedStatusNoticeHistoryFindingV2,
    ReviewedStatusNoticeHistorySourceV2, SameAccountClosedOnlyObservationRequirementV2, TrialPhase,
    V1ConfigPinsV2, load_canonical_online_authorization_v2, load_canonical_online_policy_v2,
    prepare_online_authorization_consumption_v2,
};
use reap_pm_controlled_trial_live::{
    PM_PHASE_A_ONLINE_PREFLIGHT_SIDECAR_FILE_V2, PmControlledTrialLiveJournals,
    PmPhaseAOnlinePreflightEvidenceManifestV2, PmPhaseAOnlinePreflightInspectionV2,
    PmPlacePreparationV1, PmPlaceResultKindV1, PmTrialLiveRecoveryClassificationV1,
    create_phase_a_online_preflight_basis_v2, inspect_phase_a_online_preflight_v2,
    verify_controlled_trial_live_recovery,
};
use sha2::{Digest as _, Sha256};

use support::{Fixture, hex, l2_time};

const PLACE_TIME: &str = "2026-08-09T12:05:04Z";

#[test]
fn basis_then_v2_first_burn_then_v1_a3_conjunct_is_denied_and_dnd_capable() {
    let (fixture, v1_consumption) = Fixture::new();
    let v2_consumption = prepare_v2(&fixture);
    let mut journals = bind_preflight(&fixture);
    let prepared = record_place_prepared(&fixture, &mut journals);
    let pending = create_phase_a_online_preflight_basis_v2(
        &fixture.config,
        &fixture.authorization,
        &mut journals,
        prepared,
        v1_consumption,
        v2_consumption,
        evidence_manifest(),
    )
    .expect("durable online Basis");

    assert_denied(pending.authorization());
    assert!(!pending.preparation().mutation_authority());
    assert_eq!(
        inspect_phase_a_online_preflight_v2(&fixture.config).unwrap(),
        PmPhaseAOnlinePreflightInspectionV2::BasisOnly {
            basis_record_fingerprint: pending.basis_record_fingerprint().to_owned(),
        }
    );
    assert!(
        !fixture
            .path(PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_CLAIM_FILE_V2)
            .exists()
    );
    assert!(
        !fixture
            .path(
                &fixture
                    .config
                    .value()
                    .journal
                    .authorization_consumption_claim_file,
            )
            .exists()
    );

    let mut owner = pending
        .burn_and_record_a3(
            &mut journals,
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime(PLACE_TIME),
            &v2_runtime("2026-08-09T12:05:03Z"),
        )
        .expect("complete V2 conjunct");
    owner.revalidate_held_evidence().unwrap();
    assert_denied(owner.authorization());
    assert_eq!(
        owner.public_request_identity(),
        fixture.config.exact_place_public_request_identity()
    );
    assert_eq!(owner.l2_timestamp_seconds(), l2_time(PLACE_TIME));
    assert_eq!(
        fs::read(fixture.path(PM_PHASE_A_ONLINE_PREFLIGHT_SIDECAR_FILE_V2))
            .unwrap()
            .iter()
            .filter(|byte| **byte == b'\n')
            .count(),
        2
    );
    assert_eq!(
        inspect_phase_a_online_preflight_v2(&fixture.config).unwrap(),
        PmPhaseAOnlinePreflightInspectionV2::Complete {
            basis_record_fingerprint: owner.basis_record_fingerprint().to_owned(),
            a3_conjunct_record_fingerprint: owner.a3_conjunct_record_fingerprint().to_owned(),
        }
    );
    assert_exact_complete_bindings(&fixture);

    let network_owner = journals
        .revalidate_phase_a_online_preflight_v2_for_network_dispatch(owner)
        .expect("fresh V2 then V1 final durable recheck");
    assert_eq!(
        network_owner.public_request_identity(),
        fixture.config.exact_place_public_request_identity()
    );
    assert_eq!(network_owner.l2_timestamp_seconds(), l2_time(PLACE_TIME));
    let dnd = network_owner
        .into_definitely_not_dispatched()
        .expect("exact-rechecked V2 and V1 DND conversion");
    let (result, consumed_v1) = journals
        .record_phase_a_place_definitely_not_dispatched(dnd)
        .expect("unchanged V1 DND transition");
    assert_eq!(
        result.outcome(),
        PmPlaceResultKindV1::DefinitelyNotDispatched
    );
    drop(consumed_v1);
}

#[test]
fn post_a3_v2_revalidation_failure_retains_only_the_unchanged_v1_dnd_path() {
    let (fixture, v1_consumption) = Fixture::new();
    let v2_consumption = prepare_v2(&fixture);
    let mut journals = bind_preflight(&fixture);
    let prepared = record_place_prepared(&fixture, &mut journals);
    let pending = create_phase_a_online_preflight_basis_v2(
        &fixture.config,
        &fixture.authorization,
        &mut journals,
        prepared,
        v1_consumption,
        v2_consumption,
        evidence_manifest(),
    )
    .unwrap();
    let mut owner = pending
        .burn_and_record_a3(
            &mut journals,
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime(PLACE_TIME),
            &v2_runtime("2026-08-09T12:05:03Z"),
        )
        .unwrap();

    fs::OpenOptions::new()
        .append(true)
        .open(fixture.path(PM_PHASE_A_ONLINE_PREFLIGHT_SIDECAR_FILE_V2))
        .unwrap()
        .write_all(b"post-a3-drift")
        .unwrap();
    assert!(owner.revalidate_held_evidence().is_err());
    let dnd = owner.into_definitely_not_dispatched();
    let (result, consumed_v1) = journals
        .record_phase_a_place_definitely_not_dispatched(dnd)
        .expect("V2 drift cannot suppress V1 DND");
    assert_eq!(
        result.outcome(),
        PmPlaceResultKindV1::DefinitelyNotDispatched
    );
    drop(consumed_v1);
}

#[test]
fn basis_only_or_complete_sidecar_never_reopens_placement_and_never_changes_v1_recovery() {
    let (fixture, v1_consumption) = Fixture::new();
    let v2_consumption = prepare_v2(&fixture);
    let mut journals = bind_preflight(&fixture);
    let prepared = record_place_prepared(&fixture, &mut journals);
    let pending = create_phase_a_online_preflight_basis_v2(
        &fixture.config,
        &fixture.authorization,
        &mut journals,
        prepared,
        v1_consumption,
        v2_consumption,
        evidence_manifest(),
    )
    .unwrap();
    let owner = pending
        .burn_and_record_a3(
            &mut journals,
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime(PLACE_TIME),
            &v2_runtime("2026-08-09T12:05:03Z"),
        )
        .unwrap();
    let network_owner = journals
        .revalidate_phase_a_online_preflight_v2_for_network_dispatch(owner)
        .unwrap();
    let may_have = network_owner.into_may_have_been_dispatched().unwrap();
    drop(may_have);
    drop(journals);

    let projection =
        verify_controlled_trial_live_recovery(&fixture.config, &fixture.authorization).unwrap();
    assert_eq!(
        projection.classification(),
        &PmTrialLiveRecoveryClassificationV1::PlaceMayHaveBeenSentNoResend
    );
    assert!(!projection.production_order_entry_authorized());
    assert!(!projection.real_order_submission_authorized());
    assert_eq!(projection.place_dispatch_allowance(), 0);
    assert!(!projection.placement_resumption_allowed());
    assert!(matches!(
        inspect_phase_a_online_preflight_v2(&fixture.config).unwrap(),
        PmPhaseAOnlinePreflightInspectionV2::Complete { .. }
    ));
}

#[test]
fn zero_torn_basis_only_and_extra_line_inspection_never_reopen_placement() {
    let (absent_fixture, _v1_consumption) = Fixture::new();
    assert_eq!(
        inspect_phase_a_online_preflight_v2(&absent_fixture.config).unwrap(),
        PmPhaseAOnlinePreflightInspectionV2::Absent
    );

    let (fixture, v1_consumption) = Fixture::new();
    let v2_consumption = prepare_v2(&fixture);
    let mut journals = bind_preflight(&fixture);
    let prepared = record_place_prepared(&fixture, &mut journals);
    let pending = create_phase_a_online_preflight_basis_v2(
        &fixture.config,
        &fixture.authorization,
        &mut journals,
        prepared,
        v1_consumption,
        v2_consumption,
        evidence_manifest(),
    )
    .unwrap();
    let basis_fingerprint = pending.basis_record_fingerprint().to_owned();
    drop(pending);
    let path = fixture.path(PM_PHASE_A_ONLINE_PREFLIGHT_SIDECAR_FILE_V2);
    let basis = fs::read(&path).unwrap();
    assert_eq!(
        inspect_phase_a_online_preflight_v2(&fixture.config).unwrap(),
        PmPhaseAOnlinePreflightInspectionV2::BasisOnly {
            basis_record_fingerprint: basis_fingerprint,
        }
    );

    fs::write(&path, []).unwrap();
    assert_eq!(
        inspect_phase_a_online_preflight_v2(&fixture.config).unwrap(),
        PmPhaseAOnlinePreflightInspectionV2::Ambiguous
    );

    fs::write(&path, [basis.as_slice(), b"torn"].concat()).unwrap();
    assert_eq!(
        inspect_phase_a_online_preflight_v2(&fixture.config).unwrap(),
        PmPhaseAOnlinePreflightInspectionV2::Ambiguous
    );

    fs::write(&path, [basis.as_slice(), basis.as_slice()].concat()).unwrap();
    assert_eq!(
        inspect_phase_a_online_preflight_v2(&fixture.config).unwrap(),
        PmPhaseAOnlinePreflightInspectionV2::Ambiguous
    );
}

fn assert_exact_complete_bindings(fixture: &Fixture) {
    const V1_CONSUMPTION_RECORD: &[u8] = b"reap.pm-t2.authorization-consumption.record.v1\0";
    const V1_CONSUMPTION_CLAIM: &[u8] = b"reap.pm-t2.authorization-consumption.claim.v1\0";
    const V1_DISPATCH_RECORD: &[u8] = b"reap.pm-t2.controlled-trial-live.dispatch-record.v1\0";
    const V2_CONSUMPTION_RECORD: &[u8] =
        b"reap.pm-t2.controlled-trial.online-authorization-consumption.record.v2\0";
    const V2_CONSUMPTION_CLAIM: &[u8] =
        b"reap.pm-t2.controlled-trial.online-authorization-consumption.claim.v2\0";

    let sidecar = fs::read(fixture.path(PM_PHASE_A_ONLINE_PREFLIGHT_SIDECAR_FILE_V2)).unwrap();
    let lines = complete_lines(&sidecar);
    let basis: serde_json::Value = serde_json::from_slice(lines[0]).unwrap();
    let conjunct: serde_json::Value = serde_json::from_slice(lines[1]).unwrap();
    let manifest = &conjunct["conjunct"]["durable_manifest"];
    assert_eq!(
        conjunct["conjunct"]["basis_record_fingerprint"],
        basis["record_fingerprint"]
    );
    assert_eq!(
        manifest["online_preflight_basis_record_fingerprint"],
        basis["record_fingerprint"]
    );

    let v1_ledger = fs::read(
        fixture.path(
            &fixture
                .config
                .value()
                .journal
                .authorization_consumption_ledger_file,
        ),
    )
    .unwrap();
    let v1_records = complete_lines(&v1_ledger);
    assert_eq!(
        manifest["v1_consumption_prepared_record_fingerprint"],
        fingerprint(V1_CONSUMPTION_RECORD, v1_records[0])
    );
    assert_eq!(
        manifest["v1_consumption_consumed_record_fingerprint"],
        fingerprint(V1_CONSUMPTION_RECORD, v1_records[1])
    );
    assert_eq!(
        manifest["v1_consumption_claim_fingerprint"],
        fingerprint(
            V1_CONSUMPTION_CLAIM,
            &fs::read(
                fixture.path(
                    &fixture
                        .config
                        .value()
                        .journal
                        .authorization_consumption_claim_file,
                )
            )
            .unwrap(),
        )
    );

    let v2_ledger = fs::read(
        fixture
            .path(reap_pm_controlled_trial::PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_LEDGER_FILE_V2),
    )
    .unwrap();
    let v2_records = complete_lines(&v2_ledger);
    assert_eq!(
        manifest["v2_consumption_prepared_record_fingerprint"],
        fingerprint(V2_CONSUMPTION_RECORD, v2_records[0])
    );
    assert_eq!(
        manifest["v2_consumption_consumed_record_fingerprint"],
        fingerprint(V2_CONSUMPTION_RECORD, v2_records[1])
    );
    assert_eq!(
        manifest["v2_consumption_claim_fingerprint"],
        fingerprint(
            V2_CONSUMPTION_CLAIM,
            &fs::read(fixture.path(
                reap_pm_controlled_trial::PM_T2_ONLINE_AUTHORIZATION_CONSUMPTION_CLAIM_FILE_V2,
            ))
            .unwrap(),
        )
    );

    let dispatch =
        fs::read(fixture.path(reap_pm_controlled_trial_live::PM_TRIAL_LIVE_DISPATCH_FILE_V1))
            .unwrap();
    let dispatch_records = complete_lines(&dispatch);
    assert_eq!(
        manifest["v1_place_prepared_record_fingerprint"],
        fingerprint(V1_DISPATCH_RECORD, dispatch_records[2])
    );
    assert_eq!(
        manifest["v1_dispatch_authorized_record_fingerprint"],
        fingerprint(V1_DISPATCH_RECORD, dispatch_records[3])
    );
    let barrier: serde_json::Value = serde_json::from_slice(
        fs::read(
            fixture.path(reap_pm_controlled_trial_live::PM_PHASE_A_LIVE_DISPATCH_BARRIER_FILE_V1),
        )
        .unwrap()
        .strip_suffix(b"\n")
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        manifest["v1_dispatch_barrier_record_fingerprint"],
        barrier["record_fingerprint"]
    );
}

fn complete_lines(bytes: &[u8]) -> Vec<&[u8]> {
    bytes
        .strip_suffix(b"\n")
        .unwrap()
        .split(|byte| *byte == b'\n')
        .collect()
}

fn fingerprint(domain: &[u8], bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn bind_preflight(fixture: &Fixture) -> PmControlledTrialLiveJournals {
    let pending = PmControlledTrialLiveJournals::create_pending_preflight(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:00Z"),
    )
    .unwrap();
    let preflight = fixture.canonical_preflight(pending.lease_evidence().clone());
    pending.bind_preflight(preflight).unwrap()
}

fn record_place_prepared(
    fixture: &Fixture,
    journals: &mut PmControlledTrialLiveJournals,
) -> reap_pm_controlled_trial_live::PmDurablePlacePreparedAckV1 {
    let intent = journals.record_place_intent(PLACE_TIME.into()).unwrap();
    let preparation = PmPlacePreparationV1::for_scoped_plan(
        &fixture.config,
        journals.preflight_binding(),
        fixture.config.exact_place_public_request_identity(),
        l2_time(PLACE_TIME),
    )
    .unwrap();
    journals.record_place_prepared(intent, preparation).unwrap()
}

fn prepare_v2(fixture: &Fixture) -> PreparedOnlineAuthorizationConsumptionV2 {
    let policy_path = fixture.path("online-policy-v2.json");
    write_0600(
        &policy_path,
        &serde_json::to_vec(&online_policy(&fixture.config)).unwrap(),
    );
    let policy = load_canonical_online_policy_v2(&policy_path).unwrap();
    let authorization_path = fixture.path("online-authorization-v2.json");
    write_0600(
        &authorization_path,
        &serde_json::to_vec(&online_authorization(&fixture.config, &policy)).unwrap(),
    );
    let authorization = load_canonical_online_authorization_v2(&authorization_path).unwrap();
    prepare_online_authorization_consumption_v2(
        &fixture.config,
        policy,
        authorization,
        &v2_runtime("2026-08-09T12:04:00Z"),
    )
    .unwrap()
}

fn v2_runtime(observed_at_utc: &str) -> OnlineAuthorizationRuntimeBindingV2 {
    OnlineAuthorizationRuntimeBindingV2 {
        release_binary_sha256: hex(0x88),
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

fn evidence_manifest() -> PmPhaseAOnlinePreflightEvidenceManifestV2 {
    PmPhaseAOnlinePreflightEvidenceManifestV2 {
        observation_started_at_utc: "2026-08-09T12:05:00Z".into(),
        observation_completed_at_utc: "2026-08-09T12:05:03Z".into(),
        canonical_manifest_sha256: hex(0xa0),
        canonical_manifest_length: 4_096,
        reviewed_market_evidence_sha256: hex(0xa1),
        reviewed_status_history_sha256: hex(0xa2),
        fresh_status_announcements_sha256: hex(0xa3),
        clob_ok_liveness_sha256: hex(0xa4),
        same_account_closed_only_sha256: hex(0xa5),
        public_book_cut_sha256: hex(0xa6),
        user_account_cut_sha256: hex(0xa7),
        same_authority_rest_cut_sha256: hex(0xa8),
        finalized_chain_cut_sha256: hex(0xa9),
        data_api_position_cut_sha256: hex(0xaa),
        current_runtime_and_egress_sha256: hex(0xab),
        reviewed_repository_state_sha256: hex(0xac),
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
            review_sha256: hex(0x91),
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
            cargo_lock_sha256: hex(0x77),
            release_binary_sha256: hex(0x88),
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
                dedicated_tunnel_or_gateway_profile_sha256: hex(0xab),
                destination_independent_nat_assumption:
                    ReviewedDestinationIndependentNatV2::OnePublicIpForPolymarketComAndClobPolymarketCom,
                authorized_geoblock_reported_public_ip: "8.8.8.8".into(),
            },
        },
        status_notice_history: ReviewedStatusNoticeHistoryCutV2 {
            source_kind: ReviewedStatusNoticeHistorySourceV2::OfficialPolymarketStatusHistory,
            review_reference: "official-status-history:clob:2026-08-08/09".into(),
            review_sha256: hex(0x99),
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

fn write_0600(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn assert_denied(value: reap_pm_controlled_trial::OfflineAuthorizationState) {
    assert!(!value.production_order_entry_authorized);
    assert!(!value.real_order_submission_authorized);
    assert_eq!(value.place_dispatch_allowance, 0);
}
