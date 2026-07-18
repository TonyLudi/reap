use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use reap_emergency_core::EmergencyCancelVerificationReport;
use reap_live::{AccountCertificationArtifact, DeadmanExpiryCertificationArtifact};

use super::canonical::{failure_sort_key, scenario_name};
use super::{
    ProductionEvidenceFailure, ProductionEvidenceFreshnessObservation,
    ProductionEvidenceFreshnessPolicy, ProductionEvidenceGate, ResolvedDeadmanInput,
    VerifiedEconomicInput, VerifiedFaultProxyRun, VerifiedFillInput, VerifiedTimedLiveSource,
};

pub(super) struct FreshnessInputs<'a> {
    pub(super) policy: &'a ProductionEvidenceFreshnessPolicy,
    pub(super) verified_at_ms: u64,
    pub(super) demo_soak_path: &'a Path,
    pub(super) demo_soak: &'a reap_live::LiveRunVerificationReport,
    pub(super) fault_matrix: &'a reap_live::LiveFaultMatrixVerificationReport,
    pub(super) fault_live_sources: &'a [VerifiedTimedLiveSource],
    pub(super) fault_proxy_runs: &'a [VerifiedFaultProxyRun],
    pub(super) latency_live_sources: &'a [VerifiedTimedLiveSource],
    pub(super) account_artifacts: &'a [(PathBuf, AccountCertificationArtifact)],
    pub(super) deadman_artifacts:
        &'a [(&'a ResolvedDeadmanInput, DeadmanExpiryCertificationArtifact)],
    pub(super) emergency_path: &'a Path,
    pub(super) emergency: &'a EmergencyCancelVerificationReport,
    pub(super) fill_inputs: &'a [VerifiedFillInput],
    pub(super) economic_inputs: &'a [VerifiedEconomicInput],
}

pub(super) fn evaluate_freshness(
    input: FreshnessInputs<'_>,
) -> (
    Vec<ProductionEvidenceFreshnessObservation>,
    Vec<ProductionEvidenceFailure>,
) {
    let mut observations = Vec::new();
    let mut failures = Vec::new();
    push_live_freshness(
        &mut observations,
        &mut failures,
        input.policy,
        input.verified_at_ms,
        ProductionEvidenceGate::DemoSoak,
        input.demo_soak.session_id.clone(),
        input.demo_soak_path,
        input.demo_soak,
        input.policy.demo_soak_max_age_ms,
    );
    for (run, source) in input.fault_matrix.runs.iter().zip(input.fault_live_sources) {
        let subject = Some(scenario_name(run.scenario));
        push_live_freshness(
            &mut observations,
            &mut failures,
            input.policy,
            input.verified_at_ms,
            source.gate,
            subject.clone(),
            &source.report.run_report.source_path,
            &source.report,
            input.policy.fault_run_max_age_ms,
        );
        if let (Some(proxy), Some(evidence)) = (
            run.reap_fault_proxy_evidence.as_ref(),
            run.injector_evidence.as_ref(),
        ) {
            push_freshness(
                &mut observations,
                &mut failures,
                input.policy,
                input.verified_at_ms,
                ProductionEvidenceGate::FaultMatrix,
                subject,
                &evidence.source_path,
                proxy.armed_at_ms,
                Some(proxy.completed_at_ms),
                input.policy.fault_run_max_age_ms,
            );
            check_fault_proxy_live_session(
                &mut failures,
                run.scenario,
                proxy,
                source.report.session_started_at_ms,
                source.report.elapsed_ms,
            );
        }
    }
    for proxy in input.fault_proxy_runs {
        push_freshness(
            &mut observations,
            &mut failures,
            input.policy,
            input.verified_at_ms,
            ProductionEvidenceGate::FaultProxyRun,
            Some(scenario_name(proxy.scenario)),
            &proxy.report.run_report.source_path,
            proxy.report.started_at_ms,
            Some(proxy.report.stopped_at_ms),
            input.policy.fault_run_max_age_ms,
        );
    }
    for source in input.latency_live_sources {
        push_live_freshness(
            &mut observations,
            &mut failures,
            input.policy,
            input.verified_at_ms,
            source.gate,
            source.subject.clone(),
            &source.report.run_report.source_path,
            &source.report,
            input.policy.latency_source_max_age_ms,
        );
    }
    for (path, artifact) in input.account_artifacts {
        push_freshness(
            &mut observations,
            &mut failures,
            input.policy,
            input.verified_at_ms,
            ProductionEvidenceGate::AccountCertification,
            Some(artifact.summary.account_id.clone()),
            path,
            artifact.start_clock.server_ms,
            Some(artifact.finish_clock.server_ms),
            input.policy.production_account_certification_max_age_ms,
        );
    }
    for (source, artifact) in input.deadman_artifacts {
        push_freshness(
            &mut observations,
            &mut failures,
            input.policy,
            input.verified_at_ms,
            ProductionEvidenceGate::DeadmanCertification,
            Some(artifact.summary.account_id.clone()),
            &source.artifact,
            artifact.start_clock.server_ms,
            Some(artifact.finish_clock.server_ms),
            input.policy.deadman_certification_max_age_ms,
        );
    }
    push_freshness(
        &mut observations,
        &mut failures,
        input.policy,
        input.verified_at_ms,
        ProductionEvidenceGate::EmergencyCancel,
        None,
        input.emergency_path,
        input.emergency.started_at_ms,
        input
            .emergency
            .started_at_ms
            .checked_add(input.emergency.elapsed_ms),
        input.policy.emergency_cancel_max_age_ms,
    );
    for fill in input.fill_inputs {
        push_freshness(
            &mut observations,
            &mut failures,
            input.policy,
            input.verified_at_ms,
            ProductionEvidenceGate::FillReconciliation,
            Some(fill.manifest.account_id.clone()),
            &fill.collection_manifest,
            fill.manifest.window.begin_ms,
            Some(fill.manifest.window.end_ms),
            input.policy.fill_collection_max_age_ms,
        );
    }
    for economic in input.economic_inputs {
        push_freshness(
            &mut observations,
            &mut failures,
            input.policy,
            input.verified_at_ms,
            ProductionEvidenceGate::EconomicReconciliation,
            Some(economic.bill_manifest.account_id.clone()),
            &economic.bill_collection_manifest,
            economic.bill_manifest.window.begin_ms,
            Some(economic.bill_manifest.window.end_ms),
            input.policy.bill_collection_max_age_ms,
        );
        for (path, artifact) in [
            (
                &economic.opening_account_certification,
                &economic.opening_account,
            ),
            (
                &economic.closing_account_certification,
                &economic.closing_account,
            ),
        ] {
            push_freshness(
                &mut observations,
                &mut failures,
                input.policy,
                input.verified_at_ms,
                ProductionEvidenceGate::EconomicReconciliation,
                Some(economic.bill_manifest.account_id.clone()),
                path,
                artifact.start_clock.server_ms,
                Some(artifact.finish_clock.server_ms),
                input.policy.bill_collection_max_age_ms,
            );
        }
    }
    observations.sort_by(|left, right| {
        left.gate
            .cmp(&right.gate)
            .then_with(|| left.subject.cmp(&right.subject))
            .then_with(|| left.source_path.cmp(&right.source_path))
    });
    failures.sort_by_key(failure_sort_key);
    failures.dedup();
    (observations, failures)
}

#[allow(clippy::too_many_arguments)]
fn push_live_freshness(
    observations: &mut Vec<ProductionEvidenceFreshnessObservation>,
    failures: &mut Vec<ProductionEvidenceFailure>,
    policy: &ProductionEvidenceFreshnessPolicy,
    verified_at_ms: u64,
    gate: ProductionEvidenceGate,
    subject: Option<String>,
    source_path: &Path,
    report: &reap_live::LiveRunVerificationReport,
    maximum_age_ms: u64,
) {
    push_freshness(
        observations,
        failures,
        policy,
        verified_at_ms,
        gate,
        subject,
        source_path,
        report.session_started_at_ms,
        report.session_started_at_ms.checked_add(report.elapsed_ms),
        maximum_age_ms,
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn push_freshness(
    observations: &mut Vec<ProductionEvidenceFreshnessObservation>,
    failures: &mut Vec<ProductionEvidenceFailure>,
    policy: &ProductionEvidenceFreshnessPolicy,
    verified_at_ms: u64,
    gate: ProductionEvidenceGate,
    subject: Option<String>,
    source_path: &Path,
    started_at_ms: u64,
    completed_at_ms: Option<u64>,
    maximum_age_ms: u64,
) {
    let completed = completed_at_ms.unwrap_or(u64::MAX);
    let mut age_ms = None;
    let mut passed = false;
    if started_at_ms == 0 || completed_at_ms.is_none() || completed < started_at_ms {
        failures.push(ProductionEvidenceFailure::EvidenceTimestampInvalid {
            gate,
            subject: subject.clone(),
            started_at_ms,
            completed_at_ms: completed,
        });
    } else if completed > verified_at_ms.saturating_add(policy.future_tolerance_ms) {
        failures.push(ProductionEvidenceFailure::EvidenceTimestampInFuture {
            gate,
            subject: subject.clone(),
            completed_at_ms: completed,
            verified_at_ms,
            future_tolerance_ms: policy.future_tolerance_ms,
        });
    } else {
        let age = verified_at_ms.saturating_sub(completed);
        age_ms = Some(age);
        if age > maximum_age_ms {
            failures.push(ProductionEvidenceFailure::EvidenceStale {
                gate,
                subject: subject.clone(),
                age_ms: age,
                maximum_age_ms,
            });
        } else {
            passed = true;
        }
    }
    observations.push(ProductionEvidenceFreshnessObservation {
        gate,
        subject,
        source_path: source_path.to_path_buf(),
        started_at_ms,
        completed_at_ms: completed,
        age_ms,
        maximum_age_ms,
        passed,
    });
}

pub(super) fn check_fault_proxy_live_session(
    failures: &mut Vec<ProductionEvidenceFailure>,
    scenario: reap_live::LiveFaultScenario,
    proxy: &reap_live::LiveFaultProxyEvidenceSummary,
    live_started_at_ms: u64,
    live_elapsed_ms: u64,
) {
    let Some(live_completed_at_ms) = live_started_at_ms.checked_add(live_elapsed_ms) else {
        return;
    };
    if proxy.armed_at_ms < live_started_at_ms || proxy.completed_at_ms > live_completed_at_ms {
        failures.push(ProductionEvidenceFailure::FaultProxyOutsideLiveSession {
            scenario,
            proxy_armed_at_ms: proxy.armed_at_ms,
            proxy_completed_at_ms: proxy.completed_at_ms,
            live_started_at_ms,
            live_completed_at_ms,
        });
    }
}

pub(super) fn unix_time_ms() -> Result<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_millis();
    millis
        .try_into()
        .context("current Unix time does not fit in milliseconds")
}
