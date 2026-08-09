use std::{
    fs::{self, OpenOptions},
    io::Write as _,
    os::unix::fs::{PermissionsExt as _, symlink},
    path::Path,
    process::Command,
};

use chrono::{DateTime, Utc};
use reap_pm_controlled_trial::{
    AuthorizationApproval, AuthorizationBuildBinding, AuthorizationConsumptionState,
    AuthorizationHostBinding, AuthorizationRuntimeBinding, CustodyPaths, TerminalDisposition,
    TrialAccount, TrialAuthorization, TrialConfig, TrialCredentialSlot, TrialDomain,
    TrialJournalBinding, TrialMarket, TrialOrder, TrialOrderType, TrialPhase, TrialSide,
    TrialTimeLimits, claim_prepared_authorization_consumption, inspect_custody,
    load_canonical_authorization, load_canonical_trial_config, prepare_authorization_consumption,
    verify_authorization, verify_authorization_consumption, verify_plan,
};
use tempfile::TempDir;

const KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const SIGNER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const FUNDER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

#[test]
fn exact_config_and_authorization_remain_structural_and_explicitly_unauthorized() {
    let directory = protected_dir();
    let config_path = directory.path().join("config.json");
    write_0600(&config_path, &serde_json::to_vec(&trial_config()).unwrap());
    let config = load_canonical_trial_config(&config_path).unwrap();
    let plan = verify_plan(&config);
    assert!(plan.configured_profile_structurally_valid);
    assert!(!plan.authorization.production_order_entry_authorized);
    assert!(!plan.authorization.real_order_submission_authorized);
    assert_eq!(plan.authorization.place_dispatch_allowance, 0);
    assert_ne!(plan.config_fingerprint, plan.plan_fingerprint);
    assert_ne!(plan.config_sha256, plan.config_fingerprint);

    let authorization_path = directory.path().join("authorization.json");
    let authorization = authorization(&config);
    write_0600(
        &authorization_path,
        &serde_json::to_vec(&authorization).unwrap(),
    );
    let authorization = load_canonical_authorization(&authorization_path).unwrap();
    let verified =
        verify_authorization(&config, &authorization, parse_time("2026-08-09T12:05:00Z")).unwrap();
    assert!(verified.exact_bindings_structurally_valid);
    assert!(verified.within_short_lived_window_at_verification);
    assert!(!verified.authorization_consumption_checked);
    assert!(!verified.authorization.production_order_entry_authorized);
    assert!(!verified.authorization.real_order_submission_authorized);
    assert_eq!(verified.authorization.place_dispatch_allowance, 0);
    assert_ne!(
        verified.authorization_fingerprint,
        verified.config_fingerprint
    );
}

#[test]
fn canonical_reader_rejects_duplicate_unknown_reordered_and_trailing_bytes() {
    let directory = protected_dir();
    let canonical = serde_json::to_vec(&trial_config()).unwrap();

    let trailing = directory.path().join("trailing.json");
    let mut bytes = canonical.clone();
    bytes.push(b'\n');
    write_0600(&trailing, &bytes);
    assert!(load_canonical_trial_config(&trailing).is_err());

    let duplicate = directory.path().join("duplicate.json");
    let mut bytes = b"{\"schema_version\":1,".to_vec();
    bytes.extend_from_slice(&canonical[1..]);
    write_0600(&duplicate, &bytes);
    assert!(load_canonical_trial_config(&duplicate).is_err());

    let unknown = directory.path().join("unknown.json");
    let mut bytes = canonical[..canonical.len() - 1].to_vec();
    bytes.extend_from_slice(b",\"unknown\":false}");
    write_0600(&unknown, &bytes);
    assert!(load_canonical_trial_config(&unknown).is_err());

    let reordered = directory.path().join("reordered.json");
    let mut value: serde_json::Value = serde_json::from_slice(&canonical).unwrap();
    let object = value.as_object_mut().unwrap();
    let schema = object.remove("schema_version").unwrap();
    object.insert("schema_version".into(), schema);
    let reordered_bytes = serde_json::to_vec(&value).unwrap();
    assert_ne!(reordered_bytes, canonical);
    write_0600(&reordered, &reordered_bytes);
    assert!(load_canonical_trial_config(&reordered).is_err());
}

#[test]
fn authorization_rejects_cross_config_expiry_and_more_than_fifteen_minutes() {
    let directory = protected_dir();
    let config_path = directory.path().join("config.json");
    write_0600(&config_path, &serde_json::to_vec(&trial_config()).unwrap());
    let config = load_canonical_trial_config(&config_path).unwrap();

    let mut record = authorization(&config);
    record.expires_at_utc = "2026-08-09T12:16:00Z".into();
    record.cleanup_not_after_utc = "2026-08-09T12:20:00Z".into();
    let path = directory.path().join("too-long.json");
    write_0600(&path, &serde_json::to_vec(&record).unwrap());
    let loaded = load_canonical_authorization(&path).unwrap();
    assert!(verify_authorization(&config, &loaded, parse_time("2026-08-09T12:05:00Z")).is_err());

    let record = authorization(&config);
    let path = directory.path().join("expired.json");
    write_0600(&path, &serde_json::to_vec(&record).unwrap());
    let loaded = load_canonical_authorization(&path).unwrap();
    assert!(verify_authorization(&config, &loaded, parse_time("2026-08-09T12:15:00Z")).is_err());
}

#[test]
fn authorization_reader_rejects_unknown_trailing_and_hard_linked_records() {
    let directory = protected_dir();
    let config_path = directory.path().join("config.json");
    write_0600(&config_path, &serde_json::to_vec(&trial_config()).unwrap());
    let config = load_canonical_trial_config(&config_path).unwrap();
    let canonical = serde_json::to_vec(&authorization(&config)).unwrap();

    let trailing = directory.path().join("authorization-trailing.json");
    let mut bytes = canonical.clone();
    bytes.push(b' ');
    write_0600(&trailing, &bytes);
    assert!(load_canonical_authorization(&trailing).is_err());

    let unknown = directory.path().join("authorization-unknown.json");
    let mut bytes = canonical[..canonical.len() - 1].to_vec();
    bytes.extend_from_slice(b",\"credentials\":\"forbidden\"}");
    write_0600(&unknown, &bytes);
    assert!(load_canonical_authorization(&unknown).is_err());

    let linked = directory.path().join("authorization-linked.json");
    let second_link = directory.path().join("authorization-second-link.json");
    write_0600(&linked, &canonical);
    fs::hard_link(&linked, second_link).unwrap();
    assert!(load_canonical_authorization(&linked).is_err());
}

#[test]
fn order_profile_rejects_type_zero_equal_identity_wrong_amount_and_phase_b_sell() {
    let mutations: [fn(&mut TrialConfig); 4] = [
        |config: &mut TrialConfig| config.account.signature_type = 0,
        |config: &mut TrialConfig| config.account.funder = config.account.signer.clone(),
        |config: &mut TrialConfig| config.order.maker_amount = "2499999".into(),
        |config: &mut TrialConfig| {
            config.phase = TrialPhase::BFillPosition;
            config.order.side = TrialSide::Sell;
        },
    ];
    for mutate in mutations {
        let directory = protected_dir();
        let path = directory.path().join("invalid.json");
        let mut config = trial_config();
        mutate(&mut config);
        write_0600(&path, &serde_json::to_vec(&config).unwrap());
        assert!(load_canonical_trial_config(&path).is_err());
    }
}

#[test]
fn canonical_config_binds_exact_fee_tuple_and_five_second_preflight_age_cap() {
    let mutations: [fn(&mut TrialConfig); 6] = [
        |config| config.time_limits.maximum_preflight_observation_age_ms = 5_001,
        |config| config.market.maker_base_fee_bps = 1,
        |config| config.market.taker_base_fee_bps = 10_001,
        |config| config.market.fee_rate = "1e-2".into(),
        |config| config.market.fee_exponent = "2.".into(),
        |config| config.market.fee_taker_only = false,
    ];
    for mutate in mutations {
        let directory = protected_dir();
        let path = directory.path().join("invalid-fee-or-age.json");
        let mut config = trial_config();
        mutate(&mut config);
        write_0600(&path, &serde_json::to_vec(&config).unwrap());
        assert!(load_canonical_trial_config(&path).is_err());
    }
}

#[test]
fn canonical_config_derives_one_exact_no_secret_public_place_identity() {
    let directory = protected_dir();
    let path = directory.path().join("identity-config.json");
    write_0600(&path, &serde_json::to_vec(&trial_config()).unwrap());
    let config = load_canonical_trial_config(&path).unwrap();
    let first = config.exact_place_public_request_identity();
    let second = config.exact_place_public_request_identity();
    assert_eq!(first, second);
    assert_ne!(
        first.expected_order_id().bytes(),
        first.semantic_request_commitment().bytes()
    );
    assert_eq!(first.expected_order_id().to_string().len(), 66);
}

#[test]
fn custody_is_four_file_pinned_redacted_and_signer_bound() {
    let directory = protected_dir();
    let config_path = directory.path().join("config.json");
    write_0600(&config_path, &serde_json::to_vec(&trial_config()).unwrap());
    let config = load_canonical_trial_config(&config_path).unwrap();
    let paths = write_credentials(directory.path());
    let inspection = inspect_custody(&config, paths).unwrap();
    let summary = inspection.summary();
    assert_eq!(summary.signer, SIGNER);
    assert!(summary.directory_mode_0700);
    assert!(summary.four_regular_single_link_mode_0600_files);
    assert!(summary.no_follow_descriptor_pinning_and_stable_reresolution);
    assert!(summary.private_key_derived_signer_matches_config);
    assert!(summary.l2_bundle_structurally_bound_to_signer);
    assert!(!summary.remote_api_key_owner_attested);
    assert!(!summary.secret_values_exposed);
    assert!(!summary.authorization.production_order_entry_authorized);
    assert_eq!(summary.authorization.place_dispatch_allowance, 0);
}

#[test]
fn custody_rejects_symlink_hardlink_mode_and_wrong_signer_without_secret_diagnostics() {
    let directory = protected_dir();
    let config_path = directory.path().join("config.json");
    write_0600(&config_path, &serde_json::to_vec(&trial_config()).unwrap());
    let config = load_canonical_trial_config(&config_path).unwrap();

    let paths = write_credentials(directory.path());
    fs::set_permissions(&paths.api_key, fs::Permissions::from_mode(0o640)).unwrap();
    let error = inspect_custody(&config, paths).err().unwrap().to_string();
    assert!(error.contains("all four staged files remain unchanged"));
    assert!(!error.contains(KEY));

    let paths = write_credentials(directory.path());
    let link = directory.path().join("secret-hardlink");
    fs::hard_link(&paths.l2_secret, &link).unwrap();
    assert!(inspect_custody(&config, paths).is_err());
    fs::remove_file(link).unwrap();

    let paths = write_credentials(directory.path());
    let target = paths.passphrase.clone();
    fs::remove_file(&paths.passphrase).unwrap();
    symlink(target, &paths.passphrase).unwrap();
    assert!(inspect_custody(&config, paths).is_err());

    let paths = write_credentials(directory.path());
    write_0600(
        &paths.private_key,
        b"0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d",
    );
    let error = inspect_custody(&config, paths).err().unwrap().to_string();
    assert!(!error.contains("59c6995e"));
}

#[test]
fn durable_take_once_sequence_is_prepared_consumed_terminal_and_always_unauthorized() {
    let fixture = consumption_fixture();
    let prepared = prepare_authorization_consumption(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:00Z"),
    )
    .unwrap();
    assert!(matches!(
        &prepared.evidence().consumption,
        AuthorizationConsumptionState::Prepared { .. }
    ));
    assert_eq!(
        prepared.evidence().authorization,
        reap_pm_controlled_trial::OfflineAuthorizationState::DENIED
    );
    let verified =
        verify_authorization_consumption(&fixture.config, &fixture.authorization).unwrap();
    assert!(matches!(
        &verified.state,
        AuthorizationConsumptionState::Prepared { .. }
    ));
    assert!(!verified.atomic_consumption_claim_durable);
    assert!(!verified.authorization.production_order_entry_authorized);
    assert_eq!(verified.authorization.place_dispatch_allowance, 0);

    let consumed = prepared
        .consume(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:06:00Z"),
        )
        .unwrap();
    assert!(matches!(
        &consumed.evidence().consumption,
        AuthorizationConsumptionState::Consumed {
            burned_before_dispatch_authority: true,
            crash_allows_recovery_cancel_only: true,
            placement_can_never_resume: true,
            ..
        }
    ));
    let verified =
        verify_authorization_consumption(&fixture.config, &fixture.authorization).unwrap();
    assert!(verified.atomic_consumption_claim_durable);
    assert!(verified.consumed_ledger_record_durable);
    assert_eq!(verified.ledger_record_count, 2);
    assert!(!verified.authorization.real_order_submission_authorized);

    let terminal = consumed
        .terminal("2026-08-09T12:20:00Z".into(), TerminalDisposition::Stopped)
        .unwrap();
    assert!(matches!(
        &terminal.evidence().consumption,
        AuthorizationConsumptionState::Terminal {
            terminal_is_evidence_not_authority: true,
            ..
        }
    ));
    let verified =
        verify_authorization_consumption(&fixture.config, &fixture.authorization).unwrap();
    assert_eq!(verified.ledger_record_count, 3);
    assert!(matches!(
        &verified.state,
        AuthorizationConsumptionState::Terminal { .. }
    ));
    assert!(
        prepare_authorization_consumption(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:07:00Z"),
        )
        .is_err()
    );
}

#[test]
fn two_restart_processes_race_and_exactly_one_reaches_consumed() {
    let fixture = consumption_fixture();
    let prepared = prepare_authorization_consumption(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:00Z"),
    )
    .unwrap();
    drop(prepared);

    let executable = std::env::current_exe().unwrap();
    let mut children = Vec::new();
    for _ in 0..2 {
        children.push(
            Command::new(&executable)
                .arg("--exact")
                .arg("subprocess_claim_helper")
                .env("REAP_PM_T2_CLAIM_HELPER", "1")
                .env("REAP_PM_T2_CONFIG_PATH", &fixture.config_path)
                .env("REAP_PM_T2_AUTHORIZATION_PATH", &fixture.authorization_path)
                .spawn()
                .unwrap(),
        );
    }
    let successes = children
        .into_iter()
        .map(|mut child| usize::from(child.wait().unwrap().success()))
        .sum::<usize>();
    assert_eq!(successes, 1);
    let verified =
        verify_authorization_consumption(&fixture.config, &fixture.authorization).unwrap();
    assert!(verified.atomic_consumption_claim_durable);
    assert!(matches!(
        &verified.state,
        AuthorizationConsumptionState::Consumed { .. }
    ));
    assert!(!verified.authorization.production_order_entry_authorized);
}

#[test]
fn subprocess_claim_helper() {
    if std::env::var_os("REAP_PM_T2_CLAIM_HELPER").is_none() {
        return;
    }
    let config = load_canonical_trial_config(Path::new(
        &std::env::var_os("REAP_PM_T2_CONFIG_PATH").unwrap(),
    ))
    .unwrap();
    let authorization = load_canonical_authorization(Path::new(
        &std::env::var_os("REAP_PM_T2_AUTHORIZATION_PATH").unwrap(),
    ))
    .unwrap();
    let runtime = AuthorizationRuntimeBinding {
        release_binary_sha256: authorization.value().build.release_binary_sha256.clone(),
        release_binary_length: authorization.value().build.release_binary_length,
        host: authorization.value().host.clone(),
        observed_at_utc: "2026-08-09T12:06:00Z".into(),
    };
    if claim_prepared_authorization_consumption(&config, &authorization, &runtime).is_err() {
        std::process::exit(23);
    }
}

#[test]
fn claim_only_crash_tail_is_irreversibly_consumed_and_never_resumable() {
    let fixture = consumption_fixture();
    let prepared = prepare_authorization_consumption(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:00Z"),
    )
    .unwrap();
    let consumed = prepared
        .consume(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:06:00Z"),
        )
        .unwrap();
    drop(consumed);

    // Simulate death after the atomic claim was fsynced but before the
    // redundant Consumed ledger append completed.
    let ledger = fixture.ledger_path();
    let bytes = fs::read(&ledger).unwrap();
    let first_record_end = bytes.iter().position(|byte| *byte == b'\n').unwrap() + 1;
    fs::write(&ledger, &bytes[..first_record_end]).unwrap();
    fs::set_permissions(&ledger, fs::Permissions::from_mode(0o600)).unwrap();

    let verified =
        verify_authorization_consumption(&fixture.config, &fixture.authorization).unwrap();
    assert_eq!(verified.ledger_record_count, 1);
    assert!(verified.atomic_consumption_claim_durable);
    assert!(!verified.consumed_ledger_record_durable);
    assert!(matches!(
        &verified.state,
        AuthorizationConsumptionState::Consumed {
            placement_can_never_resume: true,
            crash_allows_recovery_cancel_only: true,
            ..
        }
    ));
    assert!(
        claim_prepared_authorization_consumption(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:07:00Z"),
        )
        .is_err()
    );
}

#[test]
fn ambiguous_atomic_claim_file_burns_retry_and_fails_offline_verification() {
    let fixture = consumption_fixture();
    let prepared = prepare_authorization_consumption(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:00Z"),
    )
    .unwrap();
    drop(prepared);
    write_0600(&fixture.claim_path(), b"");
    assert!(
        claim_prepared_authorization_consumption(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:06:00Z"),
        )
        .is_err()
    );
    assert!(verify_authorization_consumption(&fixture.config, &fixture.authorization).is_err());
}

#[test]
fn expiry_boundary_rejects_consumption_without_claim_and_terminal_requires_consumed_time_order() {
    let fixture = consumption_fixture();
    let prepared = prepare_authorization_consumption(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:00:00Z"),
    )
    .unwrap();
    assert!(
        prepared
            .consume(
                &fixture.config,
                &fixture.authorization,
                &fixture.runtime("2026-08-09T12:15:00Z"),
            )
            .is_err()
    );
    assert!(!fixture.claim_path().exists());
    let consumed = claim_prepared_authorization_consumption(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:14:59Z"),
    )
    .unwrap();
    assert!(
        consumed
            .terminal(
                "2026-08-09T12:14:58Z".into(),
                TerminalDisposition::Completed,
            )
            .is_err()
    );
    let verified =
        verify_authorization_consumption(&fixture.config, &fixture.authorization).unwrap();
    assert!(matches!(
        &verified.state,
        AuthorizationConsumptionState::Consumed { .. }
    ));
}

#[test]
fn canonical_config_binds_one_absolute_parent_and_the_closed_journal_profile() {
    let mutations: [fn(&mut TrialConfig); 6] = [
        |config| config.journal.artifact_directory = "relative/artifacts".into(),
        |config| config.journal.authorization_consumption_ledger_file = "nested/ledger".into(),
        |config| {
            config.journal.authorization_consumption_claim_file =
                config.journal.authorization_consumption_ledger_file.clone();
        },
        |config| config.journal.journal_family = "foreign-family".into(),
        |config| config.journal.journal_version = 2,
        |config| {
            config.journal.authorization_consumption_claim_file = "foreign.claim".into();
        },
    ];
    for mutate in mutations {
        let directory = protected_dir();
        let path = directory.path().join("invalid-consumption-path.json");
        let mut config = trial_config();
        mutate(&mut config);
        write_0600(&path, &serde_json::to_vec(&config).unwrap());
        assert!(load_canonical_trial_config(&path).is_err());
    }
}

#[test]
fn ambiguous_tail_and_existing_ledger_fail_closed_without_a_second_claim() {
    let fixture = consumption_fixture();
    let prepared = prepare_authorization_consumption(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:00Z"),
    )
    .unwrap();
    drop(prepared);
    let ledger = fixture.ledger_path();
    OpenOptions::new()
        .append(true)
        .open(&ledger)
        .unwrap()
        .write_all(b"{\"partial\"")
        .unwrap();
    assert!(verify_authorization_consumption(&fixture.config, &fixture.authorization).is_err());
    assert!(
        claim_prepared_authorization_consumption(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:06:00Z"),
        )
        .is_err()
    );
    assert!(!fixture.claim_path().exists());
    assert!(
        prepare_authorization_consumption(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:06:00Z"),
        )
        .is_err()
    );
}

#[test]
fn consume_rechecks_binary_host_and_window_before_atomic_burn() {
    let fixture = consumption_fixture();
    let mut wrong_host = fixture.runtime("2026-08-09T12:05:00Z");
    wrong_host.host.boot_identity = "foreign-boot".into();
    assert!(
        prepare_authorization_consumption(&fixture.config, &fixture.authorization, &wrong_host)
            .is_err()
    );
    assert!(!fixture.ledger_path().exists());

    let prepared = prepare_authorization_consumption(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:05:00Z"),
    )
    .unwrap();
    let mut wrong_binary = fixture.runtime("2026-08-09T12:06:00Z");
    wrong_binary.release_binary_sha256 = "99".repeat(32);
    assert!(
        prepared
            .consume(&fixture.config, &fixture.authorization, &wrong_binary)
            .is_err()
    );
    assert!(!fixture.claim_path().exists());
    let consumed = claim_prepared_authorization_consumption(
        &fixture.config,
        &fixture.authorization,
        &fixture.runtime("2026-08-09T12:06:00Z"),
    )
    .unwrap();
    drop(consumed);
    assert!(
        claim_prepared_authorization_consumption(
            &fixture.config,
            &fixture.authorization,
            &fixture.runtime("2026-08-09T12:07:00Z"),
        )
        .is_err()
    );
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

fn authorization(config: &reap_pm_controlled_trial::CanonicalTrialConfig) -> TrialAuthorization {
    TrialAuthorization {
        schema_version: 1,
        authorization_id: "pm-t2-a-authorization-1".into(),
        phase: TrialPhase::APlaceCancel,
        issuing_reviewer: "operator-reviewer".into(),
        reviewed_at_utc: "2026-08-09T11:59:00Z".into(),
        purpose: "one_exact_pm_t2_phase_a_passive_place_cancel_attempt".into(),
        not_before_utc: "2026-08-09T12:00:00Z".into(),
        expires_at_utc: "2026-08-09T12:15:00Z".into(),
        cleanup_not_after_utc: "2026-08-09T12:20:00Z".into(),
        build: AuthorizationBuildBinding {
            repository_commit: "66".repeat(20),
            clean_tree_attested: true,
            cargo_lock_sha256: "77".repeat(32),
            release_binary_sha256: "88".repeat(32),
            release_binary_length: 1_000_000,
            canonical_config_sha256: config.canonical_sha256().into(),
            canonical_config_length: config.canonical_length(),
            canonical_config_fingerprint: config.fingerprint().into(),
        },
        host: AuthorizationHostBinding {
            host_identity: "trial-host-1".into(),
            boot_identity: "01234567-89ab-cdef-0123-456789abcdef".into(),
            runtime_user: "reap-trial".into(),
            egress_identity: "203.0.113.7".into(),
        },
        trial: config.value().clone(),
        trial_plan_fingerprint: config.plan_fingerprint().into(),
        approval: AuthorizationApproval {
            only_named_phase: true,
            exactly_one_attempt: true,
            one_possible_fill_is_within_loss_cap: true,
            post_only_does_not_mean_no_fill: true,
            no_concurrent_proxy_trading_attested: true,
            independent_cleanup_method_reviewed: true,
        },
    }
}

fn protected_dir() -> TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    directory
}

fn write_credentials(directory: &Path) -> CustodyPaths {
    let paths = CustodyPaths {
        private_key: directory.join("private-key"),
        api_key: directory.join("api-key"),
        l2_secret: directory.join("l2-secret"),
        passphrase: directory.join("passphrase"),
    };
    for path in [
        &paths.private_key,
        &paths.api_key,
        &paths.l2_secret,
        &paths.passphrase,
    ] {
        let _ = fs::remove_file(path);
    }
    write_0600(&paths.private_key, KEY.as_bytes());
    write_0600(&paths.api_key, b"00000000-0000-0000-0000-000000000001");
    write_0600(&paths.l2_secret, b"c2VjcmV0");
    write_0600(&paths.passphrase, b"synthetic-passphrase");
    paths
}

fn write_0600(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn parse_time(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

struct ConsumptionFixture {
    _directory: TempDir,
    config_path: std::path::PathBuf,
    authorization_path: std::path::PathBuf,
    config: reap_pm_controlled_trial::CanonicalTrialConfig,
    authorization: reap_pm_controlled_trial::CanonicalAuthorization,
}

impl ConsumptionFixture {
    fn runtime(&self, observed_at_utc: &str) -> AuthorizationRuntimeBinding {
        AuthorizationRuntimeBinding {
            release_binary_sha256: self
                .authorization
                .value()
                .build
                .release_binary_sha256
                .clone(),
            release_binary_length: self.authorization.value().build.release_binary_length,
            host: self.authorization.value().host.clone(),
            observed_at_utc: observed_at_utc.into(),
        }
    }

    fn ledger_path(&self) -> std::path::PathBuf {
        Path::new(&self.config.value().journal.artifact_directory).join(
            &self
                .config
                .value()
                .journal
                .authorization_consumption_ledger_file,
        )
    }

    fn claim_path(&self) -> std::path::PathBuf {
        Path::new(&self.config.value().journal.artifact_directory).join(
            &self
                .config
                .value()
                .journal
                .authorization_consumption_claim_file,
        )
    }
}

fn consumption_fixture() -> ConsumptionFixture {
    let directory = protected_dir();
    let mut raw_config = trial_config();
    raw_config.journal.artifact_directory = directory.path().to_str().unwrap().into();
    let config_path = directory.path().join("config-consumption.json");
    write_0600(&config_path, &serde_json::to_vec(&raw_config).unwrap());
    let config = load_canonical_trial_config(&config_path).unwrap();
    let authorization_path = directory.path().join("authorization-consumption.json");
    write_0600(
        &authorization_path,
        &serde_json::to_vec(&authorization(&config)).unwrap(),
    );
    let authorization = load_canonical_authorization(&authorization_path).unwrap();
    ConsumptionFixture {
        _directory: directory,
        config_path,
        authorization_path,
        config,
        authorization,
    }
}
