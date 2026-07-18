use super::super::*;

#[test]
fn identity_binding_reports_each_wrong_boundary() {
    let mut failures = Vec::new();
    let expected_accounts = BTreeMap::from([("main".to_string(), "4".repeat(64))]);
    let observed_accounts = BTreeMap::from([("main".to_string(), "5".repeat(64))]);
    check_live_identity(
        &mut failures,
        ProductionEvidenceGate::DemoSoak,
        None,
        "wrong-version",
        &"6".repeat(64),
        &"7".repeat(64),
        &observed_accounts,
        "0.1.0",
        &"1".repeat(64),
        &"2".repeat(64),
        &expected_accounts,
    );
    assert_eq!(failures.len(), 4);
}

#[test]
fn research_opening_accounts_bind_target_build_host_and_account() {
    let expected_accounts = BTreeSet::from(["main".to_string()]);
    let expected_identities = BTreeMap::from([("main".to_string(), "c".repeat(64))]);
    let opening = ResearchOpeningAccountEvidence {
        dataset_id: "train".to_string(),
        source_path: PathBuf::from("account.json"),
        source_sha256: "a".repeat(64),
        evidence_sha256: "b".repeat(64),
        executable_sha256: "d".repeat(64),
        host_identity_sha256: "e".repeat(64),
        live_config_sha256: "f".repeat(64),
        live_config_fingerprint: "0".repeat(64),
        account_id: "main".to_string(),
        account_identity_sha256: "c".repeat(64),
        certification_finish_server_ms: 100,
        capture_started_at_ms: 101,
        capture_gap_ms: 1,
    };
    let mut failures = Vec::new();
    check_research_opening_accounts(
        &mut failures,
        std::slice::from_ref(&opening),
        &expected_accounts,
        &"f".repeat(64),
        &"d".repeat(64),
        &"e".repeat(64),
        &expected_identities,
    );
    assert!(failures.is_empty());

    let mut wrong = opening;
    wrong.executable_sha256 = "1".repeat(64);
    wrong.host_identity_sha256 = "2".repeat(64);
    wrong.account_identity_sha256 = "3".repeat(64);
    wrong.live_config_sha256 = "4".repeat(64);
    check_research_opening_accounts(
        &mut failures,
        &[wrong],
        &expected_accounts,
        &"f".repeat(64),
        &"d".repeat(64),
        &"e".repeat(64),
        &expected_identities,
    );
    assert_eq!(failures.len(), 4, "{failures:#?}");
}

#[test]
fn account_coverage_is_exact() {
    let expected = BTreeSet::from(["a".to_string(), "b".to_string()]);
    let actual = BTreeSet::from(["a".to_string()]);
    let mut failures = Vec::new();
    check_account_coverage(
        &mut failures,
        ProductionEvidenceGate::AccountCertification,
        &expected,
        &actual,
    );
    assert_eq!(
        failures,
        [ProductionEvidenceFailure::AccountCoverageMismatch {
            gate: ProductionEvidenceGate::AccountCertification,
            expected: vec!["a".to_string(), "b".to_string()],
            actual: vec!["a".to_string()],
        }]
    );
}
