use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use reap_emergency_core::{
    EmergencyCancelVerificationOptions, EmergencyCancelVerificationReport,
    verify_emergency_cancel_paths,
};
use reap_fault::{
    FaultProxyConfig, FaultProxyConfigEvidence, FaultProxyRunVerificationReport,
    verify_fault_proxy_run_paths,
};
use reap_live::{
    AccountCertificationArtifact, DeadmanExpiryCertificationArtifact,
    EconomicReconciliationOptions, FillStatementReconciliationOptions, LiveConfig,
    LiveConfigFileEvidence, LiveMode, load_live_config_with_evidence,
    reconcile_okx_economics_paths, reconcile_okx_fill_collection_paths,
    verify_account_certification_artifact_path, verify_bill_collection_manifest_path,
    verify_deadman_expiry_certification_artifact_path, verify_fill_collection_manifest_path,
    verify_live_fault_matrix_paths, verify_live_run_paths, verify_production_transition_paths,
};

use crate::deployment::{ResearchDeploymentVerificationReport, verify_research_deployment_paths};
use crate::latency::LatencyCalibrationVerificationReport;
use crate::latency::verify_latency_calibration;

use super::bindings::account_ids;
use super::canonical::scenario_name;
use super::manifest::{LoadedManifest, ResolvedManifest};
use super::{
    ProductionEvidenceConfigEvidence, ProductionEvidenceGate, ResolvedDeadmanInput,
    ResolvedEconomicInput, ResolvedFillInput,
};

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

pub(super) struct InitialConfigs {
    pub(super) demo_config: LiveConfig,
    pub(super) demo_file: LiveConfigFileEvidence,
    pub(super) production_config: LiveConfig,
    pub(super) production_file: LiveConfigFileEvidence,
    pub(super) fault_file: LiveConfigFileEvidence,
    pub(super) fault_proxy_evidence: FaultProxyConfigEvidence,
}

pub(super) struct VerifiedSources<'a> {
    pub(super) transition: reap_live::ProductionTransitionReport,
    pub(super) research: ResearchDeploymentVerificationReport,
    pub(super) demo_soak: reap_live::LiveRunVerificationReport,
    pub(super) fault_matrix: reap_live::LiveFaultMatrixVerificationReport,
    pub(super) latency: LatencyCalibrationVerificationReport,
    pub(super) fault_live_sources: Vec<VerifiedTimedLiveSource>,
    pub(super) fault_proxy_runs: Vec<VerifiedFaultProxyRun>,
    pub(super) latency_live_sources: Vec<VerifiedTimedLiveSource>,
    pub(super) account_artifacts: Vec<(PathBuf, AccountCertificationArtifact)>,
    pub(super) deadman_artifacts:
        Vec<(&'a ResolvedDeadmanInput, DeadmanExpiryCertificationArtifact)>,
    pub(super) emergency: EmergencyCancelVerificationReport,
    pub(super) fill_inputs: Vec<VerifiedFillInput>,
    pub(super) economic_inputs: Vec<VerifiedEconomicInput>,
}

pub(super) struct ReopenedConfigs {
    pub(super) demo_config: LiveConfig,
    pub(super) production_config: LiveConfig,
    pub(super) fault_config: LiveConfig,
    pub(super) fault_proxy_evidence: FaultProxyConfigEvidence,
    pub(super) expected_fault_config: LiveConfig,
    pub(super) expected_fault_config_fingerprint: String,
    pub(super) fault_config_derived: bool,
    pub(super) demo_evidence: ProductionEvidenceConfigEvidence,
    pub(super) production_evidence: ProductionEvidenceConfigEvidence,
    pub(super) fault_evidence: ProductionEvidenceConfigEvidence,
}

pub(super) fn load_initial_configs(paths: &ResolvedManifest) -> Result<InitialConfigs> {
    let (demo_config, demo_file) = load_live_config_with_evidence(&paths.demo_config)
        .context("failed to load exact demo config for production evidence")?;
    let (production_config, production_file) =
        load_live_config_with_evidence(&paths.production_config)
            .context("failed to load exact production config for production evidence")?;
    let (_fault_config, fault_file) = load_live_config_with_evidence(&paths.fault_demo_config)
        .context("failed to load exact routed fault config for production evidence")?;
    let (_, fault_proxy_evidence) = FaultProxyConfig::load(&paths.fault_proxy_config)
        .context("failed to load exact fault-proxy config for production evidence")?;
    Ok(InitialConfigs {
        demo_config,
        demo_file,
        production_config,
        production_file,
        fault_file,
        fault_proxy_evidence,
    })
}

pub(super) fn reconstruct_sources<'a>(paths: &'a ResolvedManifest) -> Result<VerifiedSources<'a>> {
    let transition =
        verify_production_transition_paths(&paths.demo_config, &paths.production_config)
            .context("failed to reconstruct production-transition evidence")?;
    let research = verify_research_deployment_paths(
        &paths.production_config,
        &paths.research_manifest,
        &paths.research_report,
    )
    .context("failed to reconstruct research deployment evidence")?;
    let demo_soak = verify_live_run_paths(
        &paths.demo_config,
        &paths.demo_soak_report,
        Some(LiveMode::Demo),
    )
    .context("failed to verify the dedicated demo soak report")?;
    let fault_matrix =
        verify_live_fault_matrix_paths(&paths.fault_demo_config, &paths.fault_matrix_manifest)
            .context("failed to reconstruct the live fault matrix")?;
    let latency = verify_latency_calibration(
        &paths.demo_config,
        &paths.latency_calibration_artifact,
        &paths.latency_source_reports,
    )
    .context("failed to reconstruct latency calibration")?;
    let fault_live_sources = verify_fault_live_sources(&paths.fault_demo_config, &fault_matrix)
        .context("failed to bind fault-matrix source timestamps")?;
    let fault_proxy_runs = verify_fault_proxy_runs(paths)?;
    let latency_live_sources = verify_latency_live_sources(&paths.demo_config, &latency)
        .context("failed to bind latency source timestamps")?;
    let account_artifacts = verify_account_artifacts(paths)?;
    let deadman_artifacts = verify_deadman_artifacts(paths)?;
    let emergency = verify_emergency_cancel_paths(
        &paths.demo_config,
        &paths.emergency_cancel_report,
        EmergencyCancelVerificationOptions {
            require_all_configured_accounts: true,
        },
    )
    .context("failed to reconstruct emergency-cancel evidence")?;
    let fill_inputs = verify_fill_inputs(paths)?;
    let economic_inputs = verify_economic_inputs(paths)?;
    Ok(VerifiedSources {
        transition,
        research,
        demo_soak,
        fault_matrix,
        latency,
        fault_live_sources,
        fault_proxy_runs,
        latency_live_sources,
        account_artifacts,
        deadman_artifacts,
        emergency,
        fill_inputs,
        economic_inputs,
    })
}

fn verify_fault_proxy_runs(paths: &ResolvedManifest) -> Result<Vec<VerifiedFaultProxyRun>> {
    let mut fault_proxy_runs = Vec::with_capacity(paths.fault_proxy_runs.len());
    for input in &paths.fault_proxy_runs {
        let report = verify_fault_proxy_run_paths(&paths.fault_proxy_config, &input.report)
            .with_context(|| {
                format!(
                    "failed to reconstruct fault-proxy run evidence for {}",
                    scenario_name(input.scenario)
                )
            })?;
        fault_proxy_runs.push(VerifiedFaultProxyRun {
            scenario: input.scenario,
            report,
        });
    }
    Ok(fault_proxy_runs)
}

fn verify_account_artifacts(
    paths: &ResolvedManifest,
) -> Result<Vec<(PathBuf, AccountCertificationArtifact)>> {
    let mut account_artifacts = Vec::with_capacity(paths.account_certifications.len());
    for path in &paths.account_certifications {
        let artifact = verify_account_certification_artifact_path(path).with_context(|| {
            format!(
                "failed to reconstruct account certification {}",
                path.display()
            )
        })?;
        account_artifacts.push((path.clone(), artifact));
    }
    Ok(account_artifacts)
}

fn verify_deadman_artifacts(
    paths: &ResolvedManifest,
) -> Result<Vec<(&ResolvedDeadmanInput, DeadmanExpiryCertificationArtifact)>> {
    let mut deadman_artifacts = Vec::with_capacity(paths.deadman_certifications.len());
    for input in &paths.deadman_certifications {
        let artifact =
            verify_deadman_expiry_certification_artifact_path(&input.artifact, &input.journal)
                .with_context(|| {
                    format!(
                        "failed to reconstruct deadman certification {}",
                        input.artifact.display()
                    )
                })?;
        deadman_artifacts.push((input, artifact));
    }
    Ok(deadman_artifacts)
}

fn verify_fill_inputs(paths: &ResolvedManifest) -> Result<Vec<VerifiedFillInput>> {
    let mut fill_inputs = Vec::with_capacity(paths.fill_reconciliations.len());
    for input in &paths.fill_reconciliations {
        fill_inputs.push(verify_fill_input(input)?);
    }
    Ok(fill_inputs)
}

fn verify_fill_input(input: &ResolvedFillInput) -> Result<VerifiedFillInput> {
    let verified_before = verify_fill_collection_manifest_path(&input.collection_manifest)
        .with_context(|| {
            format!(
                "failed to reconstruct fill collection {}",
                input.collection_manifest.display()
            )
        })?;
    let manifest = verified_before.manifest.clone();
    let report = reconcile_okx_fill_collection_paths(
        &input.journal,
        &input.collection_manifest,
        FillStatementReconciliationOptions {
            account_id: manifest.account_id.clone(),
            begin_ms: manifest.window.begin_ms,
            end_ms: manifest.window.end_ms,
            minimum_fills: input.minimum_fills,
            tolerances: input.tolerances,
            statement_account_and_window_completeness_attested: false,
        },
    )
    .with_context(|| {
        format!(
            "failed to reconstruct fill reconciliation for {}",
            manifest.account_id
        )
    })?;
    let verified_after = verify_fill_collection_manifest_path(&input.collection_manifest)
        .with_context(|| {
            format!(
                "failed to recheck fill collection {} after reconciliation",
                input.collection_manifest.display()
            )
        })?;
    if verified_before != verified_after {
        bail!(
            "fill collection {} changed while it was being reconciled",
            input.collection_manifest.display()
        );
    }
    Ok(VerifiedFillInput {
        collection_manifest: input.collection_manifest.clone(),
        journal: input.journal.clone(),
        manifest,
        report,
    })
}

fn verify_economic_inputs(paths: &ResolvedManifest) -> Result<Vec<VerifiedEconomicInput>> {
    let mut economic_inputs = Vec::with_capacity(paths.economic_reconciliations.len());
    for input in &paths.economic_reconciliations {
        economic_inputs.push(verify_economic_input(input)?);
    }
    Ok(economic_inputs)
}

fn verify_economic_input(input: &ResolvedEconomicInput) -> Result<VerifiedEconomicInput> {
    let fills_before = verify_fill_collection_manifest_path(&input.fill_collection_manifest)
        .with_context(|| {
            format!(
                "failed to reconstruct economic fill collection {}",
                input.fill_collection_manifest.display()
            )
        })?;
    let bills_before = verify_bill_collection_manifest_path(&input.bill_collection_manifest)
        .with_context(|| {
            format!(
                "failed to reconstruct economic bill collection {}",
                input.bill_collection_manifest.display()
            )
        })?;
    let opening_before =
        verify_account_certification_artifact_path(&input.opening_account_certification)
            .with_context(|| {
                format!(
                    "failed to reconstruct opening economic account certification {}",
                    input.opening_account_certification.display()
                )
            })?;
    let closing_before =
        verify_account_certification_artifact_path(&input.closing_account_certification)
            .with_context(|| {
                format!(
                    "failed to reconstruct closing economic account certification {}",
                    input.closing_account_certification.display()
                )
            })?;
    let report = reconcile_okx_economics_paths(
        &input.journal,
        &input.fill_collection_manifest,
        &input.bill_collection_manifest,
        &input.opening_account_certification,
        &input.closing_account_certification,
        EconomicReconciliationOptions {
            account_id: bills_before.manifest.account_id.clone(),
            begin_ms: bills_before.manifest.window.begin_ms,
            end_ms: bills_before.manifest.window.end_ms,
            minimum_trade_bills: input.minimum_trade_bills,
            minimum_derivative_close_bills: input.minimum_derivative_close_bills,
            minimum_funding_bills: input.minimum_funding_bills,
            maximum_trade_bill_delay_ms: input.maximum_trade_bill_delay_ms,
            maximum_funding_bill_delay_ms: input.maximum_funding_bill_delay_ms,
            maximum_funding_mark_bracket_distance_ms: input
                .maximum_funding_mark_bracket_distance_ms,
            maximum_account_boundary_gap_ms: input.maximum_account_boundary_gap_ms,
            tolerances: input.tolerances,
        },
    )
    .with_context(|| {
        format!(
            "failed to reconstruct economic reconciliation for {}",
            bills_before.manifest.account_id
        )
    })?;
    let fills_after = verify_fill_collection_manifest_path(&input.fill_collection_manifest)
        .with_context(|| {
            format!(
                "failed to recheck economic fill collection {}",
                input.fill_collection_manifest.display()
            )
        })?;
    let bills_after = verify_bill_collection_manifest_path(&input.bill_collection_manifest)
        .with_context(|| {
            format!(
                "failed to recheck economic bill collection {}",
                input.bill_collection_manifest.display()
            )
        })?;
    let opening_after =
        verify_account_certification_artifact_path(&input.opening_account_certification)
            .with_context(|| {
                format!(
                    "failed to recheck opening economic account certification {}",
                    input.opening_account_certification.display()
                )
            })?;
    let closing_after =
        verify_account_certification_artifact_path(&input.closing_account_certification)
            .with_context(|| {
                format!(
                    "failed to recheck closing economic account certification {}",
                    input.closing_account_certification.display()
                )
            })?;
    if fills_before != fills_after
        || bills_before != bills_after
        || opening_before != opening_after
        || closing_before != closing_after
    {
        bail!(
            "economic source collections changed while {} was being reconciled",
            bills_before.manifest.account_id
        );
    }
    Ok(VerifiedEconomicInput {
        fill_collection_manifest: input.fill_collection_manifest.clone(),
        bill_collection_manifest: input.bill_collection_manifest.clone(),
        opening_account_certification: input.opening_account_certification.clone(),
        closing_account_certification: input.closing_account_certification.clone(),
        journal: input.journal.clone(),
        fill_manifest: fills_before.manifest,
        bill_manifest: bills_before.manifest,
        opening_account: opening_before,
        closing_account: closing_before,
        report,
    })
}

pub(super) fn reopen_verified_configs(
    paths: &ResolvedManifest,
    loaded: &LoadedManifest,
) -> Result<ReopenedConfigs> {
    let (demo_config, demo_file) = load_live_config_with_evidence(&paths.demo_config)
        .context("failed to reload exact demo config after production-evidence verification")?;
    let (production_config, production_file) = load_live_config_with_evidence(
        &paths.production_config,
    )
    .context("failed to reload exact production config after production-evidence verification")?;
    let (fault_config, fault_file) = load_live_config_with_evidence(&paths.fault_demo_config)
        .context("failed to reload exact routed fault config after verification")?;
    let (fault_proxy_config, fault_proxy_evidence) =
        FaultProxyConfig::load(&paths.fault_proxy_config)
            .context("failed to reload exact fault-proxy config after verification")?;
    let manifest_final = super::manifest::load_manifest(&loaded.evidence.source_path)
        .context("failed to reload production-evidence manifest after verification")?;
    if manifest_final.evidence != loaded.evidence || manifest_final.value != loaded.value {
        bail!("production-evidence manifest changed while it was being verified");
    }
    let expected_fault_config = fault_proxy_config
        .route_live_config(&demo_config)
        .context("failed to reconstruct routed fault config from exact demo/proxy configs")?;
    let expected_fault_config_fingerprint = expected_fault_config.evidence_fingerprint()?;
    let fault_config_derived = serde_json::to_value(&fault_config)?
        == serde_json::to_value(&expected_fault_config)?
        && fault_config.fingerprint()? == expected_fault_config.fingerprint()?
        && fault_config.evidence_fingerprint()? == expected_fault_config.evidence_fingerprint()?;
    let demo_evidence = config_evidence(&demo_config, demo_file.clone())?;
    let production_evidence = config_evidence(&production_config, production_file.clone())?;
    let fault_evidence = config_evidence(&fault_config, fault_file.clone())?;
    Ok(ReopenedConfigs {
        demo_config,
        production_config,
        fault_config,
        fault_proxy_evidence,
        expected_fault_config,
        expected_fault_config_fingerprint,
        fault_config_derived,
        demo_evidence,
        production_evidence,
        fault_evidence,
    })
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
