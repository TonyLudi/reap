use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use reap_fault::FaultProxyRunVerificationReport;
use reap_live::{
    AccountCertificationArtifact, LiveConfig, LiveConfigFileEvidence, verify_live_run_paths,
};

use crate::latency::LatencyCalibrationVerificationReport;

use super::bindings::account_ids;
use super::canonical::scenario_name;
use super::{ProductionEvidenceConfigEvidence, ProductionEvidenceGate};

pub(super) struct VerifiedFillInput {
    pub(super) collection_manifest: PathBuf,
    pub(super) journal: PathBuf,
    pub(super) manifest: reap_live::FillCollectionManifest,
    pub(super) report: reap_live::FillStatementReconciliationReport,
}

pub(super) struct VerifiedEconomicInput {
    pub(super) fill_collection_manifest: PathBuf,
    pub(super) bill_collection_manifest: PathBuf,
    pub(super) opening_account_certification: PathBuf,
    pub(super) closing_account_certification: PathBuf,
    pub(super) journal: PathBuf,
    pub(super) fill_manifest: reap_live::FillCollectionManifest,
    pub(super) bill_manifest: reap_live::BillCollectionManifest,
    pub(super) opening_account: AccountCertificationArtifact,
    pub(super) closing_account: AccountCertificationArtifact,
    pub(super) report: reap_live::EconomicReconciliationReport,
}

pub(super) struct VerifiedTimedLiveSource {
    pub(super) gate: ProductionEvidenceGate,
    pub(super) subject: Option<String>,
    pub(super) report: reap_live::LiveRunVerificationReport,
}

pub(super) struct VerifiedFaultProxyRun {
    pub(super) scenario: reap_live::LiveFaultScenario,
    pub(super) report: FaultProxyRunVerificationReport,
}

pub(super) fn verify_fault_live_sources(
    config_path: &Path,
    matrix: &reap_live::LiveFaultMatrixVerificationReport,
) -> Result<Vec<VerifiedTimedLiveSource>> {
    let mut sources = Vec::with_capacity(matrix.runs.len());
    for run in &matrix.runs {
        let report = verify_live_run_paths(config_path, &run.report.source_path, None)
            .with_context(|| {
                format!(
                    "failed to reverify fault source report {}",
                    run.report.source_path.display()
                )
            })?;
        if report.run_report != run.report || report.session_id != run.session_id {
            bail!(
                "fault source report {} changed after matrix reconstruction",
                run.report.source_path.display()
            );
        }
        sources.push(VerifiedTimedLiveSource {
            gate: ProductionEvidenceGate::FaultMatrix,
            subject: Some(scenario_name(run.scenario)),
            report,
        });
    }
    Ok(sources)
}

pub(super) fn verify_latency_live_sources(
    config_path: &Path,
    latency: &LatencyCalibrationVerificationReport,
) -> Result<Vec<VerifiedTimedLiveSource>> {
    let mut sources = Vec::with_capacity(latency.source_reports.len());
    for source in &latency.source_reports {
        let report =
            verify_live_run_paths(config_path, &source.source_path, None).with_context(|| {
                format!(
                    "failed to reverify latency source report {}",
                    source.source_path.display()
                )
            })?;
        if report.run_report.source_path != source.source_path
            || report.run_report.bytes != source.bytes
            || report.run_report.sha256 != source.sha256
        {
            bail!(
                "latency source report {} changed after calibration reconstruction",
                source.source_path.display()
            );
        }
        sources.push(VerifiedTimedLiveSource {
            gate: ProductionEvidenceGate::LatencyCalibration,
            subject: report.session_id.clone(),
            report,
        });
    }
    Ok(sources)
}

pub(super) fn config_evidence(
    config: &LiveConfig,
    file: LiveConfigFileEvidence,
) -> Result<ProductionEvidenceConfigEvidence> {
    Ok(ProductionEvidenceConfigEvidence {
        file,
        config_fingerprint: config.fingerprint()?,
        evidence_config_fingerprint: config.evidence_fingerprint()?,
        environment: config.venue.environment,
        account_ids: account_ids(config).into_iter().collect(),
    })
}
