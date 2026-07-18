use super::super::*;

#[test]
fn failure_json_uses_exact_tag_and_field_names() {
    let cases = [
        (
            ProductionEvidenceFailure::GateRejected {
                gate: ProductionEvidenceGate::Freshness,
                subject: None,
            },
            r#"{"code":"gate_rejected","gate":"freshness"}"#,
        ),
        (
            ProductionEvidenceFailure::BindingMismatch {
                gate: ProductionEvidenceGate::DemoSoak,
                subject: Some("main".to_string()),
                field: "executable_sha256".to_string(),
                expected: "expected".to_string(),
                actual: "actual".to_string(),
            },
            r#"{"code":"binding_mismatch","gate":"demo_soak","subject":"main","field":"executable_sha256","expected":"expected","actual":"actual"}"#,
        ),
        (
            ProductionEvidenceFailure::EvidenceTimestampInFuture {
                gate: ProductionEvidenceGate::AccountCertification,
                subject: None,
                completed_at_ms: 11,
                verified_at_ms: 10,
                future_tolerance_ms: 0,
            },
            r#"{"code":"evidence_timestamp_in_future","gate":"account_certification","completed_at_ms":11,"verified_at_ms":10,"future_tolerance_ms":0}"#,
        ),
        (
            ProductionEvidenceFailure::RequiredTypedFaultProxyEvidenceMissing {
                scenario: reap_live::LiveFaultScenario::PublicReconnect,
            },
            r#"{"code":"required_typed_fault_proxy_evidence_missing","scenario":"public_reconnect"}"#,
        ),
        (
            ProductionEvidenceFailure::FaultProxyRunCoverageMismatch {
                expected: vec![
                    reap_live::LiveFaultScenario::PublicReconnect,
                    reap_live::LiveFaultScenario::PrivateReconnect,
                ],
                actual: vec![reap_live::LiveFaultScenario::PublicReconnect],
            },
            r#"{"code":"fault_proxy_run_coverage_mismatch","expected":["public_reconnect","private_reconnect"],"actual":["public_reconnect"]}"#,
        ),
    ];

    for (failure, expected) in cases {
        assert_eq!(serde_json::to_string(&failure).unwrap(), expected);
        assert_eq!(
            serde_json::from_str::<ProductionEvidenceFailure>(expected).unwrap(),
            failure
        );
    }
}
