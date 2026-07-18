use super::super::*;

fn gate(gate: ProductionEvidenceGate, subject: Option<&str>) -> ProductionEvidenceGateReport {
    ProductionEvidenceGateReport {
        gate,
        subject: subject.map(str::to_string),
        source_paths: vec![PathBuf::from("/evidence.json")],
        reconstructed_sha256: "1".repeat(64),
        acceptance_passed: true,
    }
}

fn freshness(
    gate: ProductionEvidenceGate,
    subject: Option<&str>,
    path: &str,
) -> ProductionEvidenceFreshnessObservation {
    ProductionEvidenceFreshnessObservation {
        gate,
        subject: subject.map(str::to_string),
        source_path: PathBuf::from(path),
        started_at_ms: 1,
        completed_at_ms: 2,
        age_ms: Some(3),
        maximum_age_ms: 4,
        passed: true,
    }
}

#[test]
fn failures_sort_by_canonical_json_and_deduplicate_exact_matches() {
    let binding = ProductionEvidenceFailure::BindingMismatch {
        gate: ProductionEvidenceGate::DemoSoak,
        subject: Some("main".to_string()),
        field: "sha256".to_string(),
        expected: "1".repeat(64),
        actual: "2".repeat(64),
    };
    let stale = ProductionEvidenceFailure::EvidenceStale {
        gate: ProductionEvidenceGate::Freshness,
        subject: None,
        age_ms: 5,
        maximum_age_ms: 4,
    };
    let rejected = ProductionEvidenceFailure::GateRejected {
        gate: ProductionEvidenceGate::FaultMatrix,
        subject: Some("public_reconnect".to_string()),
    };
    let mut failures = vec![
        rejected.clone(),
        stale.clone(),
        binding.clone(),
        stale.clone(),
        binding.clone(),
    ];

    failures.sort_by_key(failure_sort_key);
    failures.dedup();

    assert_eq!(failures, [binding, stale, rejected]);
}

#[test]
fn gates_sort_by_gate_then_subject_without_deduplicating() {
    let duplicate = gate(ProductionEvidenceGate::DemoSoak, Some("beta"));
    let mut gates = vec![
        duplicate.clone(),
        gate(ProductionEvidenceGate::DemoSoak, Some("alpha")),
        gate(ProductionEvidenceGate::Freshness, None),
        gate(ProductionEvidenceGate::DemoSoak, None),
        duplicate.clone(),
    ];

    gates.sort_by(|left, right| {
        left.gate
            .cmp(&right.gate)
            .then_with(|| left.subject.cmp(&right.subject))
    });

    assert_eq!(
        gates,
        [
            gate(ProductionEvidenceGate::Freshness, None),
            gate(ProductionEvidenceGate::DemoSoak, None),
            gate(ProductionEvidenceGate::DemoSoak, Some("alpha")),
            duplicate.clone(),
            duplicate,
        ]
    );
}

#[test]
fn freshness_sorts_by_gate_subject_and_path_without_deduplicating() {
    let duplicate = freshness(ProductionEvidenceGate::DemoSoak, Some("beta"), "/z.json");
    let mut observations = vec![
        duplicate.clone(),
        freshness(ProductionEvidenceGate::DemoSoak, Some("alpha"), "/z.json"),
        freshness(
            ProductionEvidenceGate::Freshness,
            Some("verifier"),
            "/z.json",
        ),
        freshness(ProductionEvidenceGate::DemoSoak, None, "/z.json"),
        freshness(ProductionEvidenceGate::DemoSoak, Some("alpha"), "/a.json"),
        duplicate.clone(),
    ];

    observations.sort_by(|left, right| {
        left.gate
            .cmp(&right.gate)
            .then_with(|| left.subject.cmp(&right.subject))
            .then_with(|| left.source_path.cmp(&right.source_path))
    });

    assert_eq!(
        observations,
        [
            freshness(
                ProductionEvidenceGate::Freshness,
                Some("verifier"),
                "/z.json",
            ),
            freshness(ProductionEvidenceGate::DemoSoak, None, "/z.json"),
            freshness(ProductionEvidenceGate::DemoSoak, Some("alpha"), "/a.json",),
            freshness(ProductionEvidenceGate::DemoSoak, Some("alpha"), "/z.json",),
            duplicate.clone(),
            duplicate,
        ]
    );
}
