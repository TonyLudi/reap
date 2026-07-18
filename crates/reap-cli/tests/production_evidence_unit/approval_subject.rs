use super::super::*;

fn sha256(byte: char) -> String {
    byte.to_string().repeat(64)
}

fn config_evidence(
    name: &str,
    environment: TradingEnvironment,
    byte: char,
) -> ProductionEvidenceConfigEvidence {
    ProductionEvidenceConfigEvidence {
        file: LiveConfigFileEvidence {
            source_path: PathBuf::from(format!("/{name}.toml")),
            bytes: 100,
            sha256: sha256(byte),
        },
        config_fingerprint: sha256(byte),
        evidence_config_fingerprint: sha256(byte),
        environment,
        account_ids: vec!["main".to_string()],
    }
}

fn passing_report() -> ProductionEvidenceVerificationReport {
    let demo_accounts = BTreeMap::from([("main".to_string(), sha256('4'))]);
    let production_accounts = BTreeMap::from([("main".to_string(), sha256('5'))]);
    ProductionEvidenceVerificationReport {
        format_version: PRODUCTION_EVIDENCE_REPORT_FORMAT_VERSION,
        manifest_schema_version: PRODUCTION_EVIDENCE_MANIFEST_SCHEMA_VERSION,
        java_reference_revision: PINNED_JAVA_REVISION.to_string(),
        verifier_reap_version: "0.1.0".to_string(),
        verified_at_ms: 10_000,
        manifest: ProductionEvidenceFileEvidence {
            source_path: PathBuf::from("/production-evidence.toml"),
            bytes: 100,
            sha256: sha256('8'),
        },
        expected: ProductionEvidenceExpectedIdentity {
            reap_version: "0.1.0".to_string(),
            live_executable_sha256: sha256('1'),
            host_identity_sha256: sha256('2'),
            approval_policy_sha256: sha256('9'),
            deployment_candidate_id: "candidate-a".to_string(),
            demo_account_identity_sha256s: demo_accounts.clone(),
            production_account_identity_sha256s: production_accounts.clone(),
        },
        freshness_policy: ProductionEvidenceFreshnessPolicy {
            future_tolerance_ms: 1_000,
            demo_soak_max_age_ms: 9_000,
            fault_run_max_age_ms: 6_000,
            latency_source_max_age_ms: 7_000,
            production_account_certification_max_age_ms: 8_000,
            deadman_certification_max_age_ms: 9_000,
            emergency_cancel_max_age_ms: 10_000,
            fill_collection_max_age_ms: 11_000,
            bill_collection_max_age_ms: 12_000,
        },
        freshness_observations: vec![ProductionEvidenceFreshnessObservation {
            gate: ProductionEvidenceGate::DemoSoak,
            subject: None,
            source_path: PathBuf::from("/soak.json"),
            started_at_ms: 1_000,
            completed_at_ms: 2_000,
            age_ms: Some(8_000),
            maximum_age_ms: 9_000,
            passed: true,
        }],
        fault_proxy_runs: Vec::new(),
        verifier: ProductionEvidenceVerifierIdentity {
            reap_version: "0.1.0".to_string(),
            executable_sha256: sha256('1'),
            host_identity_sha256: sha256('2'),
        },
        demo_config: config_evidence("demo", TradingEnvironment::Demo, 'a'),
        production_config: config_evidence("production", TradingEnvironment::Production, 'b'),
        fault_demo_config: config_evidence("fault", TradingEnvironment::Demo, 'c'),
        observed_demo_identity: ProductionEvidenceLiveIdentity {
            reap_version: "0.1.0".to_string(),
            executable_sha256: sha256('1'),
            host_identity_sha256: sha256('2'),
            account_identity_sha256s: demo_accounts,
        },
        observed_production_account_identity_sha256s: production_accounts,
        observed_deployment_candidate_id: Some("candidate-a".to_string()),
        gates: vec![
            ProductionEvidenceGateReport {
                gate: ProductionEvidenceGate::Freshness,
                subject: None,
                source_paths: vec![PathBuf::from("/soak.json")],
                reconstructed_sha256: sha256('6'),
                acceptance_passed: true,
            },
            ProductionEvidenceGateReport {
                gate: ProductionEvidenceGate::DemoSoak,
                subject: None,
                source_paths: vec![PathBuf::from("/soak.json")],
                reconstructed_sha256: sha256('7'),
                acceptance_passed: true,
            },
        ],
        failures: Vec::new(),
        limitations: vec!["test limitation".to_string()],
        evidence_bundle_passed: true,
        production_order_entry_authorized: false,
    }
}

fn approval_sha256(report: &ProductionEvidenceVerificationReport) -> String {
    ProductionEvidenceApprovalSubject::from_report(report)
        .unwrap()
        .sha256()
        .unwrap()
}

#[test]
fn approval_subject_ignores_verifier_time_and_derived_age_but_binds_stable_evidence() {
    let report = passing_report();
    let original_subject = ProductionEvidenceApprovalSubject::from_report(&report).unwrap();
    let original_sha256 = original_subject.sha256().unwrap();

    let mut later_verification = report.clone();
    later_verification.verified_at_ms = 10_500;
    later_verification.freshness_observations[0].age_ms = Some(8_500);
    later_verification.gates[0].reconstructed_sha256 = sha256('d');
    let later_subject =
        ProductionEvidenceApprovalSubject::from_report(&later_verification).unwrap();

    assert_eq!(later_subject, original_subject);
    assert_eq!(later_subject.sha256().unwrap(), original_sha256);
    assert_ne!(
        later_subject.gates[0].reconstructed_sha256,
        later_verification.gates[0].reconstructed_sha256
    );

    let mut changed_timestamp = report.clone();
    changed_timestamp.freshness_observations[0].completed_at_ms += 1;
    assert_ne!(approval_sha256(&changed_timestamp), original_sha256);

    let mut changed_observation_limit = report.clone();
    changed_observation_limit.freshness_observations[0].maximum_age_ms += 1;
    assert_ne!(approval_sha256(&changed_observation_limit), original_sha256);

    let mut changed_policy_limit = report.clone();
    changed_policy_limit.freshness_policy.demo_soak_max_age_ms += 1;
    assert_ne!(approval_sha256(&changed_policy_limit), original_sha256);

    let mut changed_gate_hash = report;
    changed_gate_hash.gates[1].reconstructed_sha256 = sha256('e');
    assert_ne!(approval_sha256(&changed_gate_hash), original_sha256);
}
