use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::Result;
use serde::Serialize;

use super::canonical::{scenario_name, serialized_sha256};
use super::manifest::ResolvedManifest;
use super::source_verifiers::{ReopenedConfigs, VerifiedSources};
use super::{
    ProductionEvidenceExpectedIdentity, ProductionEvidenceFailure,
    ProductionEvidenceFaultProxyRunSummary, ProductionEvidenceFreshnessObservation,
    ProductionEvidenceGate, ProductionEvidenceGateReport, ProductionEvidenceLiveIdentity,
    ProductionEvidenceManifest,
};

pub(super) fn gate_report<T: Serialize>(
    gate: ProductionEvidenceGate,
    subject: Option<String>,
    source_paths: Vec<PathBuf>,
    reconstructed: &T,
    acceptance_passed: bool,
) -> Result<ProductionEvidenceGateReport> {
    Ok(ProductionEvidenceGateReport {
        gate,
        subject,
        source_paths,
        reconstructed_sha256: serialized_sha256(reconstructed)?,
        acceptance_passed,
    })
}

pub(super) fn expected_identity(
    manifest: &ProductionEvidenceManifest,
) -> ProductionEvidenceExpectedIdentity {
    ProductionEvidenceExpectedIdentity {
        reap_version: manifest.expected_reap_version.clone(),
        live_executable_sha256: manifest.expected_live_executable_sha256.clone(),
        host_identity_sha256: manifest.expected_host_identity_sha256.clone(),
        approval_policy_sha256: manifest.expected_approval_policy_sha256.clone(),
        deployment_candidate_id: manifest.expected_deployment_candidate_id.clone(),
        demo_account_identity_sha256s: manifest.expected_demo_account_identity_sha256s.clone(),
        production_account_identity_sha256s: manifest
            .expected_production_account_identity_sha256s
            .clone(),
    }
}

pub(super) struct EvidenceSummaries {
    pub(super) observed_demo_identity: ProductionEvidenceLiveIdentity,
    pub(super) observed_production_accounts: BTreeMap<String, String>,
    pub(super) observed_fault_proxy_runs: Vec<ProductionEvidenceFaultProxyRunSummary>,
}

pub(super) fn summarize_sources(sources: &VerifiedSources<'_>) -> EvidenceSummaries {
    let observed_demo_identity = ProductionEvidenceLiveIdentity {
        reap_version: sources.demo_soak.reap_version.clone(),
        executable_sha256: sources.demo_soak.executable_sha256.clone(),
        host_identity_sha256: sources
            .demo_soak
            .host_identity_sha256
            .clone()
            .unwrap_or_default(),
        account_identity_sha256s: sources.demo_soak.account_identity_sha256s.clone(),
    };
    let observed_production_accounts = sources
        .account_artifacts
        .iter()
        .map(|(_, artifact)| {
            (
                artifact.summary.account_id.clone(),
                artifact.summary.account_identity_sha256.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut observed_fault_proxy_runs = sources
        .fault_proxy_runs
        .iter()
        .map(|proxy| ProductionEvidenceFaultProxyRunSummary {
            scenario: proxy.scenario,
            run_report: proxy.report.run_report.clone(),
            proxy_session_id: proxy.report.proxy_session_id.clone(),
            started_at_ms: proxy.report.started_at_ms,
            stopped_at_ms: proxy.report.stopped_at_ms,
            completed_faults: proxy.report.completed_faults,
            acceptance_passed: proxy.report.acceptance_passed,
        })
        .collect::<Vec<_>>();
    observed_fault_proxy_runs.sort_by_key(|proxy| proxy.scenario);
    EvidenceSummaries {
        observed_demo_identity,
        observed_production_accounts,
        observed_fault_proxy_runs,
    }
}

pub(super) struct GateInputs<'a, 'resolved> {
    pub(super) paths: &'a ResolvedManifest,
    pub(super) reopened: &'a ReopenedConfigs,
    pub(super) sources: &'a VerifiedSources<'resolved>,
    pub(super) freshness_observations: &'a [ProductionEvidenceFreshnessObservation],
    pub(super) freshness_failures: &'a [ProductionEvidenceFailure],
}

pub(super) fn build_gate_reports(
    input: GateInputs<'_, '_>,
) -> Result<Vec<ProductionEvidenceGateReport>> {
    let mut gates = vec![
        gate_report(
            ProductionEvidenceGate::ProductionTransition,
            None,
            vec![
                input.paths.demo_config.clone(),
                input.paths.production_config.clone(),
            ],
            &input.sources.transition,
            input.sources.transition.acceptance_passed,
        )?,
        gate_report(
            ProductionEvidenceGate::ResearchDeployment,
            None,
            vec![
                input.paths.production_config.clone(),
                input.paths.research_manifest.clone(),
                input.paths.research_report.clone(),
            ],
            &input.sources.research,
            input.sources.research.acceptance_passed,
        )?,
        gate_report(
            ProductionEvidenceGate::DemoSoak,
            None,
            vec![
                input.paths.demo_config.clone(),
                input.paths.demo_soak_report.clone(),
            ],
            &input.sources.demo_soak,
            input.sources.demo_soak.acceptance_passed,
        )?,
        gate_report(
            ProductionEvidenceGate::FaultConfiguration,
            None,
            vec![
                input.paths.demo_config.clone(),
                input.paths.fault_proxy_config.clone(),
                input.paths.fault_demo_config.clone(),
            ],
            &(
                &input.reopened.fault_proxy_evidence,
                &input.reopened.expected_fault_config,
                &input.reopened.fault_config,
                &input.reopened.fault_evidence,
            ),
            input.reopened.fault_config_derived,
        )?,
        gate_report(
            ProductionEvidenceGate::FaultMatrix,
            None,
            vec![
                input.paths.fault_demo_config.clone(),
                input.paths.fault_matrix_manifest.clone(),
            ],
            &input.sources.fault_matrix,
            input.sources.fault_matrix.live_fault_matrix_passed,
        )?,
    ];
    for proxy in &input.sources.fault_proxy_runs {
        gates.push(gate_report(
            ProductionEvidenceGate::FaultProxyRun,
            Some(scenario_name(proxy.scenario)),
            vec![
                input.paths.fault_proxy_config.clone(),
                proxy.report.run_report.source_path.clone(),
            ],
            &proxy.report,
            proxy.report.acceptance_passed,
        )?);
    }
    let freshness_paths = input
        .freshness_observations
        .iter()
        .map(|observation| observation.source_path.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    gates.push(gate_report(
        ProductionEvidenceGate::Freshness,
        None,
        freshness_paths,
        &input.freshness_observations,
        input.freshness_failures.is_empty(),
    )?);
    let mut latency_paths = vec![
        input.paths.demo_config.clone(),
        input.paths.latency_calibration_artifact.clone(),
    ];
    latency_paths.extend(input.paths.latency_source_reports.iter().cloned());
    gates.push(gate_report(
        ProductionEvidenceGate::LatencyCalibration,
        None,
        latency_paths,
        &input.sources.latency,
        input.sources.latency.acceptance_passed,
    )?);
    for (path, artifact) in &input.sources.account_artifacts {
        gates.push(gate_report(
            ProductionEvidenceGate::AccountCertification,
            Some(artifact.summary.account_id.clone()),
            vec![path.clone()],
            artifact,
            artifact.summary.passed,
        )?);
    }
    for (deadman, artifact) in &input.sources.deadman_artifacts {
        gates.push(gate_report(
            ProductionEvidenceGate::DeadmanCertification,
            Some(artifact.summary.account_id.clone()),
            vec![deadman.artifact.clone(), deadman.journal.clone()],
            artifact,
            artifact.summary.passed,
        )?);
    }
    gates.push(gate_report(
        ProductionEvidenceGate::EmergencyCancel,
        None,
        vec![
            input.paths.demo_config.clone(),
            input.paths.emergency_cancel_report.clone(),
        ],
        &input.sources.emergency,
        input.sources.emergency.acceptance_passed,
    )?);
    for verified in &input.sources.fill_inputs {
        gates.push(gate_report(
            ProductionEvidenceGate::FillReconciliation,
            Some(verified.manifest.account_id.clone()),
            vec![
                verified.collection_manifest.clone(),
                verified.journal.clone(),
            ],
            &(&verified.manifest, &verified.report),
            verified.report.passed,
        )?);
    }
    for verified in &input.sources.economic_inputs {
        gates.push(gate_report(
            ProductionEvidenceGate::EconomicReconciliation,
            Some(verified.bill_manifest.account_id.clone()),
            vec![
                verified.fill_collection_manifest.clone(),
                verified.bill_collection_manifest.clone(),
                verified.opening_account_certification.clone(),
                verified.closing_account_certification.clone(),
                verified.journal.clone(),
            ],
            &(
                &verified.fill_manifest,
                &verified.bill_manifest,
                &verified.opening_account,
                &verified.closing_account,
                &verified.report,
            ),
            verified.report.passed,
        )?);
    }
    gates.sort_by(|left, right| {
        left.gate
            .cmp(&right.gate)
            .then_with(|| left.subject.cmp(&right.subject))
    });
    Ok(gates)
}
