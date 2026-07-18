use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use reap_backtest::ResearchOpeningAccountEvidence;
use reap_emergency_core::EmergencyCancelVerificationReport;
use reap_fault::FaultProxyConfigEvidence;
use reap_live::{
    AccountCertificationArtifact, DeadmanExpiryCertificationArtifact, FillStatementCoverage,
    LiveConfig, LiveConfigFileEvidence, TradingEnvironment,
};

use crate::deployment::ResearchDeploymentVerificationReport;
use crate::latency::LatencyCalibrationVerificationReport;

use super::canonical::{failure_sort_key, scenario_name, serialized_sha256};
use super::{
    ProductionEvidenceConfigEvidence, ProductionEvidenceExpectedIdentity,
    ProductionEvidenceFailure, ProductionEvidenceGate, ProductionEvidenceVerifierIdentity,
    ResolvedDeadmanInput, VerifiedEconomicInput, VerifiedFaultProxyRun, VerifiedFillInput,
    VerifiedTimedLiveSource,
};

pub(super) struct BindingInputs<'a> {
    pub(super) expected: &'a ProductionEvidenceExpectedIdentity,
    pub(super) verifier: &'a ProductionEvidenceVerifierIdentity,
    pub(super) demo_start: (&'a LiveConfig, &'a LiveConfigFileEvidence),
    pub(super) production_start: (&'a LiveConfig, &'a LiveConfigFileEvidence),
    pub(super) fault_start: &'a LiveConfigFileEvidence,
    pub(super) fault_proxy_start: &'a FaultProxyConfigEvidence,
    pub(super) demo: (&'a LiveConfig, &'a ProductionEvidenceConfigEvidence),
    pub(super) production: (&'a LiveConfig, &'a ProductionEvidenceConfigEvidence),
    pub(super) fault: (&'a LiveConfig, &'a ProductionEvidenceConfigEvidence),
    pub(super) fault_proxy: &'a FaultProxyConfigEvidence,
    pub(super) expected_fault_config_fingerprint: &'a str,
    pub(super) fault_config_derived: bool,
    pub(super) transition: &'a reap_live::ProductionTransitionReport,
    pub(super) research: &'a ResearchDeploymentVerificationReport,
    pub(super) demo_soak: &'a reap_live::LiveRunVerificationReport,
    pub(super) fault_matrix: &'a reap_live::LiveFaultMatrixVerificationReport,
    pub(super) fault_live_sources: &'a [VerifiedTimedLiveSource],
    pub(super) fault_proxy_runs: &'a [VerifiedFaultProxyRun],
    pub(super) latency: &'a LatencyCalibrationVerificationReport,
    pub(super) account_artifacts: &'a [(PathBuf, AccountCertificationArtifact)],
    pub(super) deadman_artifacts:
        &'a [(&'a ResolvedDeadmanInput, DeadmanExpiryCertificationArtifact)],
    pub(super) emergency: &'a EmergencyCancelVerificationReport,
    pub(super) fill_inputs: &'a [VerifiedFillInput],
    pub(super) economic_inputs: &'a [VerifiedEconomicInput],
}

pub(super) fn evaluate_bindings(input: BindingInputs<'_>) -> Vec<ProductionEvidenceFailure> {
    let mut failures = Vec::new();
    check_binding(
        &mut failures,
        ProductionEvidenceGate::Verifier,
        None,
        "reap_version",
        &input.expected.reap_version,
        &input.verifier.reap_version,
    );
    check_binding(
        &mut failures,
        ProductionEvidenceGate::Verifier,
        None,
        "executable_sha256",
        &input.expected.live_executable_sha256,
        &input.verifier.executable_sha256,
    );
    check_binding(
        &mut failures,
        ProductionEvidenceGate::Verifier,
        None,
        "host_identity_sha256",
        &input.expected.host_identity_sha256,
        &input.verifier.host_identity_sha256,
    );

    if input.demo_start.1 != &input.demo.1.file {
        failures.push(ProductionEvidenceFailure::ConfigChangedDuringVerification {
            role: "demo_config".to_string(),
        });
    }
    if input.production_start.1 != &input.production.1.file {
        failures.push(ProductionEvidenceFailure::ConfigChangedDuringVerification {
            role: "production_config".to_string(),
        });
    }
    if input.fault_start != &input.fault.1.file {
        failures.push(ProductionEvidenceFailure::ConfigChangedDuringVerification {
            role: "fault_demo_config".to_string(),
        });
    }
    if input.fault_proxy_start != input.fault_proxy {
        failures.push(ProductionEvidenceFailure::ConfigChangedDuringVerification {
            role: "fault_proxy_config".to_string(),
        });
    }
    check_environment(
        &mut failures,
        input.demo.0.venue.environment,
        TradingEnvironment::Demo,
    );
    check_environment(
        &mut failures,
        input.production.0.venue.environment,
        TradingEnvironment::Production,
    );
    check_environment(
        &mut failures,
        input.fault.0.venue.environment,
        TradingEnvironment::Demo,
    );
    let demo_connection_pacer = input
        .demo
        .0
        .runtime
        .connection_attempt_pacer_path
        .as_deref()
        .unwrap_or_else(|| Path::new(""));
    let production_connection_pacer = input
        .production
        .0
        .runtime
        .connection_attempt_pacer_path
        .as_deref()
        .unwrap_or_else(|| Path::new(""));
    check_binding(
        &mut failures,
        ProductionEvidenceGate::Verifier,
        None,
        "connection_attempt_pacer_path",
        &demo_connection_pacer.to_string_lossy(),
        &production_connection_pacer.to_string_lossy(),
    );
    for (role, path) in [
        (
            "demo_connection_attempt_pacer_path_absolute",
            demo_connection_pacer,
        ),
        (
            "production_connection_attempt_pacer_path_absolute",
            production_connection_pacer,
        ),
    ] {
        check_binding(
            &mut failures,
            ProductionEvidenceGate::Verifier,
            None,
            role,
            "true",
            if path.is_absolute() { "true" } else { "false" },
        );
    }

    let demo_accounts = account_ids(input.demo.0);
    let production_accounts = account_ids(input.production.0);
    if demo_accounts != production_accounts {
        failures.push(ProductionEvidenceFailure::ConfigAccountSetMismatch {
            demo: demo_accounts.iter().cloned().collect(),
            production: production_accounts.iter().cloned().collect(),
        });
    }
    check_account_coverage(
        &mut failures,
        ProductionEvidenceGate::Verifier,
        &demo_accounts,
        &input
            .expected
            .demo_account_identity_sha256s
            .keys()
            .cloned()
            .collect(),
    );

    reject_gate(
        &mut failures,
        ProductionEvidenceGate::FaultConfiguration,
        None,
        input.fault_config_derived,
    );
    check_binding(
        &mut failures,
        ProductionEvidenceGate::FaultConfiguration,
        None,
        "fault_config_evidence_fingerprint",
        input.expected_fault_config_fingerprint,
        &input.fault.1.evidence_config_fingerprint,
    );
    check_account_coverage(
        &mut failures,
        ProductionEvidenceGate::FaultConfiguration,
        &demo_accounts,
        &account_ids(input.fault.0),
    );
    check_account_coverage(
        &mut failures,
        ProductionEvidenceGate::Verifier,
        &production_accounts,
        &input
            .expected
            .production_account_identity_sha256s
            .keys()
            .cloned()
            .collect(),
    );

    reject_gate(
        &mut failures,
        ProductionEvidenceGate::ProductionTransition,
        None,
        input.transition.acceptance_passed,
    );
    check_binding(
        &mut failures,
        ProductionEvidenceGate::ProductionTransition,
        None,
        "demo_config_sha256",
        &input.demo.1.file.sha256,
        &input.transition.demo.file.sha256,
    );
    check_binding(
        &mut failures,
        ProductionEvidenceGate::ProductionTransition,
        None,
        "production_config_sha256",
        &input.production.1.file.sha256,
        &input.transition.production.file.sha256,
    );

    reject_gate(
        &mut failures,
        ProductionEvidenceGate::ResearchDeployment,
        None,
        input.research.acceptance_passed,
    );
    check_binding(
        &mut failures,
        ProductionEvidenceGate::ResearchDeployment,
        None,
        "production_config_sha256",
        &input.production.1.file.sha256,
        &input.research.production_config.file.sha256,
    );
    check_binding(
        &mut failures,
        ProductionEvidenceGate::ResearchDeployment,
        None,
        "deployment_candidate_id",
        &input.expected.deployment_candidate_id,
        input
            .research
            .deployment_candidate_id
            .as_deref()
            .unwrap_or(""),
    );
    check_research_opening_accounts(
        &mut failures,
        &input.research.research.artifact_opening_accounts,
        &production_accounts,
        &input.production.1.file.sha256,
        &input.expected.live_executable_sha256,
        &input.expected.host_identity_sha256,
        &input.expected.production_account_identity_sha256s,
    );

    reject_gate(
        &mut failures,
        ProductionEvidenceGate::DemoSoak,
        None,
        input.demo_soak.acceptance_passed,
    );
    check_binding(
        &mut failures,
        ProductionEvidenceGate::DemoSoak,
        None,
        "demo_config_sha256",
        &input.demo.1.file.sha256,
        &input.demo_soak.config.sha256,
    );
    check_live_identity(
        &mut failures,
        ProductionEvidenceGate::DemoSoak,
        None,
        &input.demo_soak.reap_version,
        &input.demo_soak.executable_sha256,
        input
            .demo_soak
            .host_identity_sha256
            .as_deref()
            .unwrap_or(""),
        &input.demo_soak.account_identity_sha256s,
        &input.expected.reap_version,
        &input.expected.live_executable_sha256,
        &input.expected.host_identity_sha256,
        &input.expected.demo_account_identity_sha256s,
    );

    reject_gate(
        &mut failures,
        ProductionEvidenceGate::FaultMatrix,
        None,
        input.fault_matrix.live_fault_matrix_passed,
    );
    check_binding(
        &mut failures,
        ProductionEvidenceGate::FaultMatrix,
        None,
        "fault_demo_config_sha256",
        &input.fault.1.file.sha256,
        &input.fault_matrix.config.sha256,
    );
    if let Some(identity) = &input.fault_matrix.identity {
        check_live_identity(
            &mut failures,
            ProductionEvidenceGate::FaultMatrix,
            None,
            &identity.reap_version,
            &identity.executable_sha256,
            &identity.host_identity_sha256,
            &identity.account_identity_sha256s,
            &input.expected.reap_version,
            &input.expected.live_executable_sha256,
            &input.expected.host_identity_sha256,
            &input.expected.demo_account_identity_sha256s,
        );
    } else {
        reject_gate(
            &mut failures,
            ProductionEvidenceGate::FaultMatrix,
            None,
            false,
        );
    }
    check_fault_proxy_entries(
        &mut failures,
        &input.fault_proxy.effective_fingerprint,
        input
            .fault_matrix
            .runs
            .iter()
            .map(|run| (run.scenario, run.reap_fault_proxy_evidence.as_ref())),
    );
    check_fault_proxy_runs(
        &mut failures,
        input.expected,
        input.fault_proxy,
        input.fault_matrix,
        input.fault_live_sources,
        input.fault_proxy_runs,
    );
    if let Some(session_id) = &input.demo_soak.session_id
        && input
            .fault_matrix
            .runs
            .iter()
            .any(|run| run.session_id.as_ref() == Some(session_id))
    {
        failures.push(
            ProductionEvidenceFailure::DemoSoakSessionReusedByFaultCampaign {
                session_id: session_id.clone(),
            },
        );
    }

    reject_gate(
        &mut failures,
        ProductionEvidenceGate::LatencyCalibration,
        None,
        input.latency.acceptance_passed,
    );
    check_binding(
        &mut failures,
        ProductionEvidenceGate::LatencyCalibration,
        None,
        "demo_config_sha256",
        &input.demo.1.file.sha256,
        &input.latency.config.sha256,
    );
    check_live_identity(
        &mut failures,
        ProductionEvidenceGate::LatencyCalibration,
        None,
        &input.latency.artifact_reap_version,
        &input.latency.live_executable_sha256,
        &input.latency.host_identity_sha256,
        &input.latency.account_identity_sha256s,
        &input.expected.reap_version,
        &input.expected.live_executable_sha256,
        &input.expected.host_identity_sha256,
        &input.expected.demo_account_identity_sha256s,
    );

    let mut production_certified_accounts = BTreeSet::new();
    for (_, artifact) in input.account_artifacts {
        let account_id = artifact.summary.account_id.as_str();
        reject_gate(
            &mut failures,
            ProductionEvidenceGate::AccountCertification,
            Some(account_id),
            artifact.summary.passed,
        );
        if !production_certified_accounts.insert(account_id.to_string()) {
            failures.push(ProductionEvidenceFailure::DuplicateAccountEvidence {
                gate: ProductionEvidenceGate::AccountCertification,
                account_id: account_id.to_string(),
            });
        }
        check_binding(
            &mut failures,
            ProductionEvidenceGate::AccountCertification,
            Some(account_id),
            "production_config_sha256",
            &input.production.1.file.sha256,
            &artifact.config.sha256,
        );
        check_binding(
            &mut failures,
            ProductionEvidenceGate::AccountCertification,
            Some(account_id),
            "reap_version",
            &input.expected.reap_version,
            &artifact.reap_version,
        );
        check_binding(
            &mut failures,
            ProductionEvidenceGate::AccountCertification,
            Some(account_id),
            "executable_sha256",
            &input.expected.live_executable_sha256,
            &artifact.executable_sha256,
        );
        check_binding(
            &mut failures,
            ProductionEvidenceGate::AccountCertification,
            Some(account_id),
            "host_identity_sha256",
            &input.expected.host_identity_sha256,
            &artifact.host_identity_sha256,
        );
        check_binding(
            &mut failures,
            ProductionEvidenceGate::AccountCertification,
            Some(account_id),
            "account_identity_sha256",
            input
                .expected
                .production_account_identity_sha256s
                .get(account_id)
                .map(String::as_str)
                .unwrap_or(""),
            &artifact.summary.account_identity_sha256,
        );
    }
    check_account_coverage(
        &mut failures,
        ProductionEvidenceGate::AccountCertification,
        &production_accounts,
        &production_certified_accounts,
    );

    let mut deadman_accounts = BTreeSet::new();
    for (_, artifact) in input.deadman_artifacts {
        let account_id = artifact.summary.account_id.as_str();
        reject_gate(
            &mut failures,
            ProductionEvidenceGate::DeadmanCertification,
            Some(account_id),
            artifact.summary.passed,
        );
        if !deadman_accounts.insert(account_id.to_string()) {
            failures.push(ProductionEvidenceFailure::DuplicateAccountEvidence {
                gate: ProductionEvidenceGate::DeadmanCertification,
                account_id: account_id.to_string(),
            });
        }
        check_demo_artifact_identity(
            &mut failures,
            ProductionEvidenceGate::DeadmanCertification,
            account_id,
            &artifact.config.sha256,
            &artifact.reap_version,
            &artifact.executable_sha256,
            &artifact.host_identity_sha256,
            &artifact.summary.account_identity_sha256,
            &input,
        );
    }
    check_account_coverage(
        &mut failures,
        ProductionEvidenceGate::DeadmanCertification,
        &demo_accounts,
        &deadman_accounts,
    );

    reject_gate(
        &mut failures,
        ProductionEvidenceGate::EmergencyCancel,
        None,
        input.emergency.acceptance_passed,
    );
    check_binding(
        &mut failures,
        ProductionEvidenceGate::EmergencyCancel,
        None,
        "demo_config_sha256",
        &input.demo.1.file.sha256,
        &input.emergency.config.sha256,
    );
    check_live_identity(
        &mut failures,
        ProductionEvidenceGate::EmergencyCancel,
        None,
        &input.emergency.reap_version,
        input.emergency.executable_sha256.as_deref().unwrap_or(""),
        input
            .emergency
            .host_identity_sha256
            .as_deref()
            .unwrap_or(""),
        &input.emergency.account_identity_sha256s,
        &input.expected.reap_version,
        &input.expected.live_executable_sha256,
        &input.expected.host_identity_sha256,
        &input.expected.demo_account_identity_sha256s,
    );

    let mut fill_accounts = BTreeSet::new();
    for fill in input.fill_inputs {
        let account_id = fill.manifest.account_id.as_str();
        let accepted = fill.report.passed
            && fill.report.coverage == FillStatementCoverage::AuthenticatedRecentFillCollection;
        reject_gate(
            &mut failures,
            ProductionEvidenceGate::FillReconciliation,
            Some(account_id),
            accepted,
        );
        if !fill_accounts.insert(account_id.to_string()) {
            failures.push(ProductionEvidenceFailure::DuplicateAccountEvidence {
                gate: ProductionEvidenceGate::FillReconciliation,
                account_id: account_id.to_string(),
            });
        }
        check_demo_artifact_identity(
            &mut failures,
            ProductionEvidenceGate::FillReconciliation,
            account_id,
            &fill.manifest.config_file.sha256,
            &fill.manifest.reap_version,
            &fill.manifest.executable_sha256,
            &fill.manifest.host_identity_sha256,
            &fill.manifest.account_identity_sha256,
            &input,
        );
    }
    check_account_coverage(
        &mut failures,
        ProductionEvidenceGate::FillReconciliation,
        &demo_accounts,
        &fill_accounts,
    );

    let mut economic_accounts = BTreeSet::new();
    for economic in input.economic_inputs {
        let account_id = economic.bill_manifest.account_id.as_str();
        let cash_continuity_passed = economic.report.counts.cash_balance_currencies > 0
            && economic.report.counts.cash_balance_currencies
                == economic.report.counts.cash_balance_currencies_validated
            && economic.report.counts.cash_balance_chain_links > 0
            && economic.report.counts.cash_balance_chain_links
                == economic.report.counts.cash_balance_chain_links_validated
            && !economic.report.currency_balance_continuity.is_empty()
            && economic
                .report
                .currency_balance_continuity
                .iter()
                .all(|sample| sample.validated);
        reject_gate(
            &mut failures,
            ProductionEvidenceGate::EconomicReconciliation,
            Some(account_id),
            economic.report.passed && cash_continuity_passed,
        );
        if !economic_accounts.insert(account_id.to_string()) {
            failures.push(ProductionEvidenceFailure::DuplicateAccountEvidence {
                gate: ProductionEvidenceGate::EconomicReconciliation,
                account_id: account_id.to_string(),
            });
        }
        check_demo_artifact_identity(
            &mut failures,
            ProductionEvidenceGate::EconomicReconciliation,
            account_id,
            &economic.fill_manifest.config_file.sha256,
            &economic.fill_manifest.reap_version,
            &economic.fill_manifest.executable_sha256,
            &economic.fill_manifest.host_identity_sha256,
            &economic.fill_manifest.account_identity_sha256,
            &input,
        );
        for artifact in [&economic.opening_account, &economic.closing_account] {
            check_demo_artifact_identity(
                &mut failures,
                ProductionEvidenceGate::EconomicReconciliation,
                account_id,
                &artifact.config.sha256,
                &artifact.reap_version,
                &artifact.executable_sha256,
                &artifact.host_identity_sha256,
                &artifact.summary.account_identity_sha256,
                &input,
            );
            check_binding(
                &mut failures,
                ProductionEvidenceGate::EconomicReconciliation,
                Some(account_id),
                "boundary_account_id",
                account_id,
                &artifact.summary.account_id,
            );
        }
        check_demo_artifact_identity(
            &mut failures,
            ProductionEvidenceGate::EconomicReconciliation,
            account_id,
            &economic.bill_manifest.config_file.sha256,
            &economic.bill_manifest.reap_version,
            &economic.bill_manifest.executable_sha256,
            &economic.bill_manifest.host_identity_sha256,
            &economic.bill_manifest.account_identity_sha256,
            &input,
        );
        check_binding(
            &mut failures,
            ProductionEvidenceGate::EconomicReconciliation,
            Some(account_id),
            "report_account_id",
            account_id,
            &economic.report.account_id,
        );
        check_binding(
            &mut failures,
            ProductionEvidenceGate::EconomicReconciliation,
            Some(account_id),
            "report_config_sha256",
            &input.demo.1.file.sha256,
            &economic.report.config_file.sha256,
        );
        check_binding(
            &mut failures,
            ProductionEvidenceGate::EconomicReconciliation,
            Some(account_id),
            "report_account_identity_sha256",
            input
                .expected
                .demo_account_identity_sha256s
                .get(account_id)
                .map(String::as_str)
                .unwrap_or(""),
            &economic.report.account_identity_sha256,
        );
        check_binding(
            &mut failures,
            ProductionEvidenceGate::EconomicReconciliation,
            Some(account_id),
            "opening_account_certification_path",
            &economic.opening_account_certification.to_string_lossy(),
            &economic
                .report
                .opening_account_boundary
                .certification_file
                .path,
        );
        check_binding(
            &mut failures,
            ProductionEvidenceGate::EconomicReconciliation,
            Some(account_id),
            "closing_account_certification_path",
            &economic.closing_account_certification.to_string_lossy(),
            &economic
                .report
                .closing_account_boundary
                .certification_file
                .path,
        );
        let matching_fill = input
            .fill_inputs
            .iter()
            .find(|fill| fill.manifest.account_id == account_id);
        if let Some(fill) = matching_fill {
            check_binding(
                &mut failures,
                ProductionEvidenceGate::EconomicReconciliation,
                Some(account_id),
                "fill_collection_manifest",
                &fill.collection_manifest.to_string_lossy(),
                &economic.fill_collection_manifest.to_string_lossy(),
            );
            if let Some(fill_manifest_evidence) = fill.report.collection_manifest.as_ref() {
                check_binding(
                    &mut failures,
                    ProductionEvidenceGate::EconomicReconciliation,
                    Some(account_id),
                    "fill_manifest_evidence_path",
                    &fill_manifest_evidence.path,
                    &economic.report.fill_collection_manifest.path,
                );
                check_binding(
                    &mut failures,
                    ProductionEvidenceGate::EconomicReconciliation,
                    Some(account_id),
                    "fill_manifest_evidence_bytes",
                    &fill_manifest_evidence.bytes.to_string(),
                    &economic.report.fill_collection_manifest.bytes.to_string(),
                );
                check_binding(
                    &mut failures,
                    ProductionEvidenceGate::EconomicReconciliation,
                    Some(account_id),
                    "fill_manifest_evidence_sha256",
                    &fill_manifest_evidence.sha256,
                    &economic.report.fill_collection_manifest.sha256,
                );
            } else {
                reject_gate(
                    &mut failures,
                    ProductionEvidenceGate::EconomicReconciliation,
                    Some(account_id),
                    false,
                );
            }
            check_binding(
                &mut failures,
                ProductionEvidenceGate::EconomicReconciliation,
                Some(account_id),
                "journal",
                &fill.journal.to_string_lossy(),
                &economic.journal.to_string_lossy(),
            );
            check_binding(
                &mut failures,
                ProductionEvidenceGate::EconomicReconciliation,
                Some(account_id),
                "journal_evidence_path",
                &fill.report.journal.path,
                &economic.report.journal.path,
            );
            check_binding(
                &mut failures,
                ProductionEvidenceGate::EconomicReconciliation,
                Some(account_id),
                "journal_evidence_bytes",
                &fill.report.journal.bytes.to_string(),
                &economic.report.journal.bytes.to_string(),
            );
            check_binding(
                &mut failures,
                ProductionEvidenceGate::EconomicReconciliation,
                Some(account_id),
                "journal_evidence_sha256",
                &fill.report.journal.sha256,
                &economic.report.journal.sha256,
            );
        } else {
            reject_gate(
                &mut failures,
                ProductionEvidenceGate::EconomicReconciliation,
                Some(account_id),
                false,
            );
        }
    }
    check_account_coverage(
        &mut failures,
        ProductionEvidenceGate::EconomicReconciliation,
        &demo_accounts,
        &economic_accounts,
    );

    failures.sort_by_key(failure_sort_key);
    failures.dedup();
    failures
}

fn check_fault_proxy_runs(
    failures: &mut Vec<ProductionEvidenceFailure>,
    expected: &ProductionEvidenceExpectedIdentity,
    expected_config: &FaultProxyConfigEvidence,
    matrix: &reap_live::LiveFaultMatrixVerificationReport,
    live_sources: &[VerifiedTimedLiveSource],
    proxy_runs: &[VerifiedFaultProxyRun],
) {
    let expected_scenarios = reap_live::LiveFaultScenario::REQUIRED
        .into_iter()
        .collect::<BTreeSet<_>>();
    let actual_scenarios = proxy_runs
        .iter()
        .map(|run| run.scenario)
        .collect::<BTreeSet<_>>();
    if expected_scenarios != actual_scenarios || proxy_runs.len() != expected_scenarios.len() {
        failures.push(ProductionEvidenceFailure::FaultProxyRunCoverageMismatch {
            expected: expected_scenarios.iter().copied().collect(),
            actual: actual_scenarios.iter().copied().collect(),
        });
    }

    let mut sessions = BTreeSet::new();
    for proxy in proxy_runs {
        let scenario = proxy.scenario;
        let subject = scenario_name(scenario);
        reject_gate(
            failures,
            ProductionEvidenceGate::FaultProxyRun,
            Some(&subject),
            proxy.report.acceptance_passed,
        );
        for (field, expected_value, actual) in [
            (
                "proxy_config_sha256",
                expected_config.sha256.as_str(),
                proxy.report.config.sha256.as_str(),
            ),
            (
                "proxy_config_fingerprint",
                expected_config.effective_fingerprint.as_str(),
                proxy.report.config.effective_fingerprint.as_str(),
            ),
            (
                "reap_version",
                expected.reap_version.as_str(),
                proxy.report.reap_version.as_str(),
            ),
            (
                "executable_sha256",
                expected.live_executable_sha256.as_str(),
                proxy.report.executable_sha256.as_str(),
            ),
            (
                "host_identity_sha256",
                expected.host_identity_sha256.as_str(),
                proxy.report.host_identity_sha256.as_str(),
            ),
        ] {
            check_binding(
                failures,
                ProductionEvidenceGate::FaultProxyRun,
                Some(&subject),
                field,
                expected_value,
                actual,
            );
        }
        if !sessions.insert(proxy.report.proxy_session_id.clone()) {
            failures.push(ProductionEvidenceFailure::DuplicateFaultProxyRunSession {
                proxy_session_id: proxy.report.proxy_session_id.clone(),
            });
        }

        let Some((matrix_run, live)) = matrix
            .runs
            .iter()
            .zip(live_sources)
            .find(|(run, _)| run.scenario == scenario)
        else {
            continue;
        };
        if let Some(live_completed_at_ms) = live
            .report
            .session_started_at_ms
            .checked_add(live.report.elapsed_ms)
            && (proxy.report.started_at_ms > live.report.session_started_at_ms
                || proxy.report.stopped_at_ms < live_completed_at_ms)
        {
            failures.push(
                ProductionEvidenceFailure::FaultProxyRunDoesNotEncloseLiveSession {
                    scenario,
                    proxy_started_at_ms: proxy.report.started_at_ms,
                    proxy_stopped_at_ms: proxy.report.stopped_at_ms,
                    live_started_at_ms: live.report.session_started_at_ms,
                    live_completed_at_ms,
                },
            );
        }
        let enclosed_scenarios = enclosed_fault_scenarios(
            proxy.report.started_at_ms,
            proxy.report.stopped_at_ms,
            matrix.runs.iter().zip(live_sources).map(|(run, live)| {
                (
                    run.scenario,
                    live.report.session_started_at_ms,
                    live.report.elapsed_ms,
                )
            }),
        );
        if enclosed_scenarios.as_slice() != [scenario] {
            failures.push(
                ProductionEvidenceFailure::FaultProxyRunAmbiguousLiveCoverage {
                    scenario,
                    enclosed_scenarios,
                },
            );
        }
        let expected_completed_faults = u64::from(matrix_run.reap_fault_proxy_evidence.is_some());
        if proxy.report.completed_faults != expected_completed_faults {
            failures.push(
                ProductionEvidenceFailure::FaultProxyCompletedFaultCountMismatch {
                    scenario,
                    expected: expected_completed_faults,
                    actual: proxy.report.completed_faults,
                },
            );
        }
        if let Some(injector) = &matrix_run.reap_fault_proxy_evidence {
            check_binding(
                failures,
                ProductionEvidenceGate::FaultProxyRun,
                Some(&subject),
                "proxy_session_id",
                &injector.proxy_session_id,
                &proxy.report.proxy_session_id,
            );
        }
    }
}

pub(super) fn enclosed_fault_scenarios(
    proxy_started_at_ms: u64,
    proxy_stopped_at_ms: u64,
    sessions: impl IntoIterator<Item = (reap_live::LiveFaultScenario, u64, u64)>,
) -> Vec<reap_live::LiveFaultScenario> {
    sessions
        .into_iter()
        .filter_map(|(scenario, started_at_ms, elapsed_ms)| {
            let completed_at_ms = started_at_ms.checked_add(elapsed_ms)?;
            (proxy_started_at_ms <= started_at_ms && proxy_stopped_at_ms >= completed_at_ms)
                .then_some(scenario)
        })
        .collect()
}

pub(super) fn check_fault_proxy_entries<'a>(
    failures: &mut Vec<ProductionEvidenceFailure>,
    expected_fingerprint: &str,
    entries: impl IntoIterator<
        Item = (
            reap_live::LiveFaultScenario,
            Option<&'a reap_live::LiveFaultProxyEvidenceSummary>,
        ),
    >,
) {
    let mut proxy_sessions = BTreeSet::new();
    let mut proxy_commands = BTreeSet::new();
    for (scenario, proxy) in entries {
        let typed_required = !matches!(
            scenario,
            reap_live::LiveFaultScenario::CleanObserve
                | reap_live::LiveFaultScenario::CleanDemo
                | reap_live::LiveFaultScenario::PartialFill
                | reap_live::LiveFaultScenario::RestoredSafetyLatch
        );
        let Some(proxy) = proxy else {
            if typed_required {
                failures.push(
                    ProductionEvidenceFailure::RequiredTypedFaultProxyEvidenceMissing { scenario },
                );
            }
            continue;
        };
        let scenario_name = scenario_name(scenario);
        check_binding(
            failures,
            ProductionEvidenceGate::FaultConfiguration,
            Some(&scenario_name),
            "proxy_config_fingerprint",
            expected_fingerprint,
            &proxy.proxy_config_fingerprint,
        );
        if !proxy_sessions.insert(proxy.proxy_session_id.clone()) {
            failures.push(ProductionEvidenceFailure::DuplicateFaultProxySession {
                proxy_session_id: proxy.proxy_session_id.clone(),
            });
        }
        if !proxy_commands.insert(proxy.command_id.clone()) {
            failures.push(ProductionEvidenceFailure::DuplicateFaultCommand {
                command_id: proxy.command_id.clone(),
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn check_demo_artifact_identity(
    failures: &mut Vec<ProductionEvidenceFailure>,
    gate: ProductionEvidenceGate,
    account_id: &str,
    config_sha256: &str,
    reap_version: &str,
    executable_sha256: &str,
    host_identity_sha256: &str,
    account_identity_sha256: &str,
    input: &BindingInputs<'_>,
) {
    check_binding(
        failures,
        gate,
        Some(account_id),
        "demo_config_sha256",
        &input.demo.1.file.sha256,
        config_sha256,
    );
    check_binding(
        failures,
        gate,
        Some(account_id),
        "reap_version",
        &input.expected.reap_version,
        reap_version,
    );
    check_binding(
        failures,
        gate,
        Some(account_id),
        "executable_sha256",
        &input.expected.live_executable_sha256,
        executable_sha256,
    );
    check_binding(
        failures,
        gate,
        Some(account_id),
        "host_identity_sha256",
        &input.expected.host_identity_sha256,
        host_identity_sha256,
    );
    check_binding(
        failures,
        gate,
        Some(account_id),
        "account_identity_sha256",
        input
            .expected
            .demo_account_identity_sha256s
            .get(account_id)
            .map(String::as_str)
            .unwrap_or(""),
        account_identity_sha256,
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn check_live_identity(
    failures: &mut Vec<ProductionEvidenceFailure>,
    gate: ProductionEvidenceGate,
    subject: Option<&str>,
    reap_version: &str,
    executable_sha256: &str,
    host_identity_sha256: &str,
    account_identity_sha256s: &BTreeMap<String, String>,
    expected_reap_version: &str,
    expected_executable_sha256: &str,
    expected_host_identity_sha256: &str,
    expected_account_identity_sha256s: &BTreeMap<String, String>,
) {
    check_binding(
        failures,
        gate,
        subject,
        "reap_version",
        expected_reap_version,
        reap_version,
    );
    check_binding(
        failures,
        gate,
        subject,
        "executable_sha256",
        expected_executable_sha256,
        executable_sha256,
    );
    check_binding(
        failures,
        gate,
        subject,
        "host_identity_sha256",
        expected_host_identity_sha256,
        host_identity_sha256,
    );
    check_binding(
        failures,
        gate,
        subject,
        "account_identity_set_sha256",
        &serialized_sha256(expected_account_identity_sha256s).unwrap_or_default(),
        &serialized_sha256(account_identity_sha256s).unwrap_or_default(),
    );
}

fn check_binding(
    failures: &mut Vec<ProductionEvidenceFailure>,
    gate: ProductionEvidenceGate,
    subject: Option<&str>,
    field: &str,
    expected: &str,
    actual: &str,
) {
    if expected != actual {
        failures.push(ProductionEvidenceFailure::BindingMismatch {
            gate,
            subject: subject.map(str::to_string),
            field: field.to_string(),
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
}

fn check_environment(
    failures: &mut Vec<ProductionEvidenceFailure>,
    actual: TradingEnvironment,
    expected: TradingEnvironment,
) {
    if actual != expected {
        failures.push(ProductionEvidenceFailure::ConfigEnvironmentMismatch { expected, actual });
    }
}

pub(super) fn check_account_coverage(
    failures: &mut Vec<ProductionEvidenceFailure>,
    gate: ProductionEvidenceGate,
    expected: &BTreeSet<String>,
    actual: &BTreeSet<String>,
) {
    if expected != actual {
        failures.push(ProductionEvidenceFailure::AccountCoverageMismatch {
            gate,
            expected: expected.iter().cloned().collect(),
            actual: actual.iter().cloned().collect(),
        });
    }
}

pub(super) fn check_research_opening_accounts(
    failures: &mut Vec<ProductionEvidenceFailure>,
    openings: &[ResearchOpeningAccountEvidence],
    expected_accounts: &BTreeSet<String>,
    expected_live_config_sha256: &str,
    expected_executable_sha256: &str,
    expected_host_identity_sha256: &str,
    expected_account_identity_sha256s: &BTreeMap<String, String>,
) {
    reject_gate(
        failures,
        ProductionEvidenceGate::ResearchDeployment,
        None,
        !openings.is_empty(),
    );
    let mut observed_accounts = BTreeSet::new();
    for opening in openings {
        let account_id = opening.account_id.as_str();
        observed_accounts.insert(opening.account_id.clone());
        check_binding(
            failures,
            ProductionEvidenceGate::ResearchDeployment,
            Some(account_id),
            "opening_live_config_sha256",
            expected_live_config_sha256,
            &opening.live_config_sha256,
        );
        check_binding(
            failures,
            ProductionEvidenceGate::ResearchDeployment,
            Some(account_id),
            "opening_executable_sha256",
            expected_executable_sha256,
            &opening.executable_sha256,
        );
        check_binding(
            failures,
            ProductionEvidenceGate::ResearchDeployment,
            Some(account_id),
            "opening_host_identity_sha256",
            expected_host_identity_sha256,
            &opening.host_identity_sha256,
        );
        check_binding(
            failures,
            ProductionEvidenceGate::ResearchDeployment,
            Some(account_id),
            "opening_account_identity_sha256",
            expected_account_identity_sha256s
                .get(account_id)
                .map(String::as_str)
                .unwrap_or(""),
            &opening.account_identity_sha256,
        );
    }
    check_account_coverage(
        failures,
        ProductionEvidenceGate::ResearchDeployment,
        expected_accounts,
        &observed_accounts,
    );
}

fn reject_gate(
    failures: &mut Vec<ProductionEvidenceFailure>,
    gate: ProductionEvidenceGate,
    subject: Option<&str>,
    passed: bool,
) {
    if !passed {
        failures.push(ProductionEvidenceFailure::GateRejected {
            gate,
            subject: subject.map(str::to_string),
        });
    }
}

pub(super) fn account_ids(config: &LiveConfig) -> BTreeSet<String> {
    config
        .accounts
        .iter()
        .map(|account| account.id.clone())
        .collect()
}
