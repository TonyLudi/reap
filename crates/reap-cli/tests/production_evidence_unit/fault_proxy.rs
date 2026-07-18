use super::super::*;

#[test]
fn production_bundle_requires_unique_exact_proxy_evidence() {
    let exact = reap_live::LiveFaultProxyEvidenceSummary {
        format_version: 1,
        proxy_session_id: "proxy-one".to_string(),
        proxy_config_fingerprint: "a".repeat(64),
        command_id: "command-one".to_string(),
        command_kind: "disconnect_websockets".to_string(),
        armed_at_ms: 100,
        completed_at_ms: 101,
        effect_count: 1,
        passed: true,
    };
    let mut duplicate = exact.clone();
    duplicate.proxy_config_fingerprint = "b".repeat(64);
    let mut failures = Vec::new();
    check_fault_proxy_entries(
        &mut failures,
        &"a".repeat(64),
        [
            (reap_live::LiveFaultScenario::PublicReconnect, Some(&exact)),
            (
                reap_live::LiveFaultScenario::PrivateReconnect,
                Some(&duplicate),
            ),
            (reap_live::LiveFaultScenario::PartialFill, None),
            (reap_live::LiveFaultScenario::ExchangeClockFailure, None),
        ],
    );
    assert_eq!(failures.len(), 4, "{failures:#?}");
    assert!(failures.iter().any(|failure| matches!(
        failure,
        ProductionEvidenceFailure::RequiredTypedFaultProxyEvidenceMissing {
            scenario: reap_live::LiveFaultScenario::ExchangeClockFailure
        }
    )));
    assert!(failures.iter().any(|failure| matches!(
        failure,
        ProductionEvidenceFailure::DuplicateFaultProxySession { .. }
    )));
    assert!(failures.iter().any(|failure| matches!(
        failure,
        ProductionEvidenceFailure::DuplicateFaultCommand { .. }
    )));
    assert!(
        failures
            .iter()
            .any(|failure| matches!(failure, ProductionEvidenceFailure::BindingMismatch { .. }))
    );

    let mut timing_failures = Vec::new();
    check_fault_proxy_live_session(
        &mut timing_failures,
        reap_live::LiveFaultScenario::PublicReconnect,
        &exact,
        100,
        1,
    );
    assert!(timing_failures.is_empty());
    let mut outside = exact;
    outside.completed_at_ms = 102;
    check_fault_proxy_live_session(
        &mut timing_failures,
        reap_live::LiveFaultScenario::PublicReconnect,
        &outside,
        100,
        1,
    );
    assert!(matches!(
        timing_failures.as_slice(),
        [ProductionEvidenceFailure::FaultProxyOutsideLiveSession { .. }]
    ));
}

#[test]
fn fault_proxy_run_interval_must_enclose_exactly_one_assigned_session() {
    use reap_live::LiveFaultScenario::{PrivateReconnect, PublicReconnect};

    let sessions = [(PublicReconnect, 100, 10), (PrivateReconnect, 200, 10)];
    assert_eq!(
        enclosed_fault_scenarios(90, 150, sessions),
        [PublicReconnect]
    );
    assert!(enclosed_fault_scenarios(101, 150, sessions).is_empty());
    assert_eq!(
        enclosed_fault_scenarios(90, 250, sessions),
        [PublicReconnect, PrivateReconnect]
    );
    assert!(enclosed_fault_scenarios(0, u64::MAX, [(PublicReconnect, u64::MAX, 1)]).is_empty());
}
