use super::super::*;

#[test]
fn freshness_rejects_invalid_future_and_stale_sources() {
    let policy = ProductionEvidenceFreshnessPolicy {
        future_tolerance_ms: 10,
        demo_soak_max_age_ms: 100,
        fault_run_max_age_ms: 100,
        latency_source_max_age_ms: 100,
        production_account_certification_max_age_ms: 100,
        deadman_certification_max_age_ms: 100,
        emergency_cancel_max_age_ms: 100,
        fill_collection_max_age_ms: 100,
        bill_collection_max_age_ms: 100,
    };
    let mut observations = Vec::new();
    let mut failures = Vec::new();
    for (subject, started, completed) in [
        ("invalid", 0, Some(1)),
        ("future", 100, Some(1_011)),
        ("stale", 100, Some(899)),
        ("current", 900, Some(900)),
    ] {
        push_freshness(
            &mut observations,
            &mut failures,
            &policy,
            1_000,
            ProductionEvidenceGate::DemoSoak,
            Some(subject.to_string()),
            Path::new("source.json"),
            started,
            completed,
            100,
        );
    }
    assert_eq!(observations.len(), 4);
    assert_eq!(observations.iter().filter(|entry| entry.passed).count(), 1);
    assert!(failures.iter().any(|failure| matches!(
        failure,
        ProductionEvidenceFailure::EvidenceTimestampInvalid { .. }
    )));
    assert!(failures.iter().any(|failure| matches!(
        failure,
        ProductionEvidenceFailure::EvidenceTimestampInFuture { .. }
    )));
    assert!(
        failures
            .iter()
            .any(|failure| matches!(failure, ProductionEvidenceFailure::EvidenceStale { .. }))
    );
}
