use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use reap_core::PINNED_JAVA_REVISION;
use reap_fault::{FaultProxyConfig, FaultProxyConfigEvidence};
use reap_live::{
    AccountCertificationArtifact, DeadmanExpiryCertificationArtifact,
    EmergencyCancelVerificationOptions, FillStatementCoverage, FillStatementReconciliationOptions,
    FillStatementTolerances, LiveConfig, LiveConfigFileEvidence, LiveMode, TradingEnvironment,
    current_executable_sha256, host_identity_sha256, reconcile_okx_fill_collection_paths,
    verify_account_certification_artifact_path, verify_deadman_expiry_certification_artifact_path,
    verify_emergency_cancel_paths, verify_fill_collection_manifest_path,
    verify_live_fault_matrix_paths, verify_live_run_paths, verify_production_transition_paths,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::deployment::{ResearchDeploymentVerificationReport, verify_research_deployment_paths};
use crate::latency::{LatencyCalibrationVerificationReport, verify_latency_calibration};

pub(crate) const PRODUCTION_EVIDENCE_MANIFEST_SCHEMA_VERSION: u16 = 1;
pub(crate) const PRODUCTION_EVIDENCE_REPORT_FORMAT_VERSION: u16 = 1;
const MAX_PRODUCTION_EVIDENCE_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_PRODUCTION_EVIDENCE_ACCOUNTS: usize = 32;
const MAX_PRODUCTION_EVIDENCE_LATENCY_REPORTS: usize = 128;
const MAX_PRODUCTION_EVIDENCE_CANDIDATE_ID_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceFillInput {
    pub collection_manifest: PathBuf,
    pub journal: PathBuf,
    pub minimum_fills: u64,
    #[serde(default)]
    pub price_tolerance: f64,
    #[serde(default)]
    pub quantity_tolerance: f64,
    #[serde(default)]
    pub fee_tolerance: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceDeadmanInput {
    pub artifact: PathBuf,
    pub journal: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceManifest {
    pub schema_version: u16,
    pub expected_reap_version: String,
    pub expected_live_executable_sha256: String,
    pub expected_host_identity_sha256: String,
    pub expected_deployment_candidate_id: String,
    pub expected_demo_account_identity_sha256s: BTreeMap<String, String>,
    pub expected_production_account_identity_sha256s: BTreeMap<String, String>,
    pub demo_config: PathBuf,
    pub production_config: PathBuf,
    pub fault_demo_config: PathBuf,
    pub fault_proxy_config: PathBuf,
    pub demo_soak_report: PathBuf,
    pub fault_matrix_manifest: PathBuf,
    pub latency_calibration_artifact: PathBuf,
    pub latency_source_reports: Vec<PathBuf>,
    pub research_manifest: PathBuf,
    pub research_report: PathBuf,
    pub account_certifications: Vec<PathBuf>,
    pub deadman_certifications: Vec<ProductionEvidenceDeadmanInput>,
    pub emergency_cancel_report: PathBuf,
    pub fill_reconciliations: Vec<ProductionEvidenceFillInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceFileEvidence {
    pub source_path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceExpectedIdentity {
    pub reap_version: String,
    pub live_executable_sha256: String,
    pub host_identity_sha256: String,
    pub deployment_candidate_id: String,
    pub demo_account_identity_sha256s: BTreeMap<String, String>,
    pub production_account_identity_sha256s: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceVerifierIdentity {
    pub reap_version: String,
    pub executable_sha256: String,
    pub host_identity_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceLiveIdentity {
    pub reap_version: String,
    pub executable_sha256: String,
    pub host_identity_sha256: String,
    pub account_identity_sha256s: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceConfigEvidence {
    pub file: LiveConfigFileEvidence,
    pub config_fingerprint: String,
    pub evidence_config_fingerprint: String,
    pub environment: TradingEnvironment,
    pub account_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProductionEvidenceGate {
    Verifier,
    ProductionTransition,
    ResearchDeployment,
    DemoSoak,
    FaultConfiguration,
    FaultMatrix,
    LatencyCalibration,
    AccountCertification,
    DeadmanCertification,
    EmergencyCancel,
    FillReconciliation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceGateReport {
    pub gate: ProductionEvidenceGate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub source_paths: Vec<PathBuf>,
    pub reconstructed_sha256: String,
    pub acceptance_passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub(crate) enum ProductionEvidenceFailure {
    ConfigChangedDuringVerification {
        role: String,
    },
    ConfigEnvironmentMismatch {
        expected: TradingEnvironment,
        actual: TradingEnvironment,
    },
    ConfigAccountSetMismatch {
        demo: Vec<String>,
        production: Vec<String>,
    },
    AccountCoverageMismatch {
        gate: ProductionEvidenceGate,
        expected: Vec<String>,
        actual: Vec<String>,
    },
    DuplicateAccountEvidence {
        gate: ProductionEvidenceGate,
        account_id: String,
    },
    GateRejected {
        gate: ProductionEvidenceGate,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<String>,
    },
    BindingMismatch {
        gate: ProductionEvidenceGate,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<String>,
        field: String,
        expected: String,
        actual: String,
    },
    DemoSoakSessionReusedByFaultCampaign {
        session_id: String,
    },
    RequiredTypedFaultProxyEvidenceMissing {
        scenario: reap_live::LiveFaultScenario,
    },
    DuplicateFaultProxySession {
        proxy_session_id: String,
    },
    DuplicateFaultCommand {
        command_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceVerificationReport {
    pub format_version: u16,
    pub manifest_schema_version: u16,
    pub java_reference_revision: String,
    pub verifier_reap_version: String,
    pub manifest: ProductionEvidenceFileEvidence,
    pub expected: ProductionEvidenceExpectedIdentity,
    pub verifier: ProductionEvidenceVerifierIdentity,
    pub demo_config: ProductionEvidenceConfigEvidence,
    pub production_config: ProductionEvidenceConfigEvidence,
    pub fault_demo_config: ProductionEvidenceConfigEvidence,
    pub observed_demo_identity: ProductionEvidenceLiveIdentity,
    pub observed_production_account_identity_sha256s: BTreeMap<String, String>,
    pub observed_deployment_candidate_id: Option<String>,
    pub gates: Vec<ProductionEvidenceGateReport>,
    pub failures: Vec<ProductionEvidenceFailure>,
    pub limitations: Vec<String>,
    pub evidence_bundle_passed: bool,
    pub production_order_entry_authorized: bool,
}

struct LoadedManifest {
    evidence: ProductionEvidenceFileEvidence,
    value: ProductionEvidenceManifest,
    base: PathBuf,
}

struct ResolvedManifest {
    demo_config: PathBuf,
    production_config: PathBuf,
    fault_demo_config: PathBuf,
    fault_proxy_config: PathBuf,
    demo_soak_report: PathBuf,
    fault_matrix_manifest: PathBuf,
    latency_calibration_artifact: PathBuf,
    latency_source_reports: Vec<PathBuf>,
    research_manifest: PathBuf,
    research_report: PathBuf,
    account_certifications: Vec<PathBuf>,
    deadman_certifications: Vec<ResolvedDeadmanInput>,
    emergency_cancel_report: PathBuf,
    fill_reconciliations: Vec<ResolvedFillInput>,
}

struct ResolvedDeadmanInput {
    artifact: PathBuf,
    journal: PathBuf,
}

struct ResolvedFillInput {
    collection_manifest: PathBuf,
    journal: PathBuf,
    minimum_fills: u64,
    tolerances: FillStatementTolerances,
}

struct VerifiedFillInput {
    collection_manifest: PathBuf,
    journal: PathBuf,
    manifest: reap_live::FillCollectionManifest,
    report: reap_live::FillStatementReconciliationReport,
}

pub(crate) fn verify_production_evidence_manifest_path(
    manifest_path: &Path,
) -> Result<ProductionEvidenceVerificationReport> {
    let loaded = load_manifest(manifest_path)?;
    validate_manifest(&loaded.value)?;
    let paths = resolve_manifest(&loaded)?;

    let expected = expected_identity(&loaded.value);
    let verifier = ProductionEvidenceVerifierIdentity {
        reap_version: env!("CARGO_PKG_VERSION").to_string(),
        executable_sha256: current_executable_sha256()
            .map_err(anyhow::Error::msg)
            .context("failed to fingerprint production-evidence verifier executable")?,
        host_identity_sha256: host_identity_sha256()
            .map_err(anyhow::Error::msg)
            .context("failed to fingerprint production-evidence verifier host")?,
    };

    let (demo_config_start, demo_file_start) =
        LiveConfig::load_with_evidence(&paths.demo_config)
            .context("failed to load exact demo config for production evidence")?;
    let (production_config_start, production_file_start) =
        LiveConfig::load_with_evidence(&paths.production_config)
            .context("failed to load exact production config for production evidence")?;
    let (_fault_config_start, fault_file_start) =
        LiveConfig::load_with_evidence(&paths.fault_demo_config)
            .context("failed to load exact routed fault config for production evidence")?;
    let (_, fault_proxy_evidence_start) = FaultProxyConfig::load(&paths.fault_proxy_config)
        .context("failed to load exact fault-proxy config for production evidence")?;

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

    let emergency = verify_emergency_cancel_paths(
        &paths.demo_config,
        &paths.emergency_cancel_report,
        EmergencyCancelVerificationOptions {
            require_all_configured_accounts: true,
        },
    )
    .context("failed to reconstruct emergency-cancel evidence")?;

    let mut fill_inputs = Vec::with_capacity(paths.fill_reconciliations.len());
    for input in &paths.fill_reconciliations {
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
        fill_inputs.push(VerifiedFillInput {
            collection_manifest: input.collection_manifest.clone(),
            journal: input.journal.clone(),
            manifest,
            report,
        });
    }

    // Reopen all configs after the expensive source reconstructions. Every
    // subordinate report is compared to this final exact-file observation.
    let (demo_config, demo_file) = LiveConfig::load_with_evidence(&paths.demo_config)
        .context("failed to reload exact demo config after production-evidence verification")?;
    let (production_config, production_file) = LiveConfig::load_with_evidence(
        &paths.production_config,
    )
    .context("failed to reload exact production config after production-evidence verification")?;
    let (fault_config, fault_file) = LiveConfig::load_with_evidence(&paths.fault_demo_config)
        .context("failed to reload exact routed fault config after verification")?;
    let (fault_proxy_config, fault_proxy_evidence) =
        FaultProxyConfig::load(&paths.fault_proxy_config)
            .context("failed to reload exact fault-proxy config after verification")?;
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

    let observed_demo_identity = ProductionEvidenceLiveIdentity {
        reap_version: demo_soak.reap_version.clone(),
        executable_sha256: demo_soak.executable_sha256.clone(),
        host_identity_sha256: demo_soak.host_identity_sha256.clone().unwrap_or_default(),
        account_identity_sha256s: demo_soak.account_identity_sha256s.clone(),
    };
    let observed_production_accounts = account_artifacts
        .iter()
        .map(|(_, artifact)| {
            (
                artifact.summary.account_id.clone(),
                artifact.summary.account_identity_sha256.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();

    let mut gates = vec![
        gate_report(
            ProductionEvidenceGate::ProductionTransition,
            None,
            vec![paths.demo_config.clone(), paths.production_config.clone()],
            &transition,
            transition.acceptance_passed,
        )?,
        gate_report(
            ProductionEvidenceGate::ResearchDeployment,
            None,
            vec![
                paths.production_config.clone(),
                paths.research_manifest.clone(),
                paths.research_report.clone(),
            ],
            &research,
            research.acceptance_passed,
        )?,
        gate_report(
            ProductionEvidenceGate::DemoSoak,
            None,
            vec![paths.demo_config.clone(), paths.demo_soak_report.clone()],
            &demo_soak,
            demo_soak.acceptance_passed,
        )?,
        gate_report(
            ProductionEvidenceGate::FaultConfiguration,
            None,
            vec![
                paths.demo_config.clone(),
                paths.fault_proxy_config.clone(),
                paths.fault_demo_config.clone(),
            ],
            &(
                &fault_proxy_evidence,
                &expected_fault_config,
                &fault_config,
                &fault_evidence,
            ),
            fault_config_derived,
        )?,
        gate_report(
            ProductionEvidenceGate::FaultMatrix,
            None,
            vec![
                paths.fault_demo_config.clone(),
                paths.fault_matrix_manifest.clone(),
            ],
            &fault_matrix,
            fault_matrix.live_fault_matrix_passed,
        )?,
    ];
    let mut latency_paths = vec![
        paths.demo_config.clone(),
        paths.latency_calibration_artifact.clone(),
    ];
    latency_paths.extend(paths.latency_source_reports.iter().cloned());
    gates.push(gate_report(
        ProductionEvidenceGate::LatencyCalibration,
        None,
        latency_paths,
        &latency,
        latency.acceptance_passed,
    )?);
    for (path, artifact) in &account_artifacts {
        gates.push(gate_report(
            ProductionEvidenceGate::AccountCertification,
            Some(artifact.summary.account_id.clone()),
            vec![path.clone()],
            artifact,
            artifact.summary.passed,
        )?);
    }
    for (input, artifact) in &deadman_artifacts {
        gates.push(gate_report(
            ProductionEvidenceGate::DeadmanCertification,
            Some(artifact.summary.account_id.clone()),
            vec![input.artifact.clone(), input.journal.clone()],
            artifact,
            artifact.summary.passed,
        )?);
    }
    gates.push(gate_report(
        ProductionEvidenceGate::EmergencyCancel,
        None,
        vec![
            paths.demo_config.clone(),
            paths.emergency_cancel_report.clone(),
        ],
        &emergency,
        emergency.acceptance_passed,
    )?);
    for input in &fill_inputs {
        gates.push(gate_report(
            ProductionEvidenceGate::FillReconciliation,
            Some(input.manifest.account_id.clone()),
            vec![input.collection_manifest.clone(), input.journal.clone()],
            &(&input.manifest, &input.report),
            input.report.passed,
        )?);
    }
    gates.sort_by(|left, right| {
        left.gate
            .cmp(&right.gate)
            .then_with(|| left.subject.cmp(&right.subject))
    });

    let bindings = BindingInputs {
        expected: &expected,
        verifier: &verifier,
        demo_start: (&demo_config_start, &demo_file_start),
        production_start: (&production_config_start, &production_file_start),
        fault_start: &fault_file_start,
        fault_proxy_start: &fault_proxy_evidence_start,
        demo: (&demo_config, &demo_evidence),
        production: (&production_config, &production_evidence),
        fault: (&fault_config, &fault_evidence),
        fault_proxy: &fault_proxy_evidence,
        expected_fault_config_fingerprint: &expected_fault_config_fingerprint,
        fault_config_derived,
        transition: &transition,
        research: &research,
        demo_soak: &demo_soak,
        fault_matrix: &fault_matrix,
        latency: &latency,
        account_artifacts: &account_artifacts,
        deadman_artifacts: &deadman_artifacts,
        emergency: &emergency,
        fill_inputs: &fill_inputs,
    };
    let failures = evaluate_bindings(bindings);
    let evidence_bundle_passed =
        failures.is_empty() && gates.iter().all(|gate| gate.acceptance_passed);

    Ok(ProductionEvidenceVerificationReport {
        format_version: PRODUCTION_EVIDENCE_REPORT_FORMAT_VERSION,
        manifest_schema_version: loaded.value.schema_version,
        java_reference_revision: PINNED_JAVA_REVISION.to_string(),
        verifier_reap_version: env!("CARGO_PKG_VERSION").to_string(),
        manifest: loaded.evidence,
        expected,
        verifier,
        demo_config: demo_evidence,
        production_config: production_evidence,
        fault_demo_config: fault_evidence,
        observed_demo_identity,
        observed_production_account_identity_sha256s: observed_production_accounts,
        observed_deployment_candidate_id: research.deployment_candidate_id.clone(),
        gates,
        failures,
        limitations: vec![
            "a passing bundle reconstructs the implemented source gates and binds exact configs, candidate, build, host, and account identities; it does not prove that the evidence is recent enough for a particular approval window"
                .to_string(),
            "host and exchange-account identity hashes are provenance identifiers, not remote attestation; operators must independently control the manifest and target host"
                .to_string(),
            "account certification is point-in-time and fill reconciliation covers fills and fees only; complete economic statements, funding, transfers, tax, and profitability review remain external gates"
                .to_string(),
            "supervision, paging, credential permissions, venue announcements, rollout/rollback review, and explicit human approval remain required"
                .to_string(),
            "typed fault injector records are bound, but separate fault-proxy run-report clean shutdown and supervisor timing still require archived operator review"
                .to_string(),
            "this verifier never authorizes or enables production order entry".to_string(),
        ],
        evidence_bundle_passed,
        production_order_entry_authorized: false,
    })
}

struct BindingInputs<'a> {
    expected: &'a ProductionEvidenceExpectedIdentity,
    verifier: &'a ProductionEvidenceVerifierIdentity,
    demo_start: (&'a LiveConfig, &'a LiveConfigFileEvidence),
    production_start: (&'a LiveConfig, &'a LiveConfigFileEvidence),
    fault_start: &'a LiveConfigFileEvidence,
    fault_proxy_start: &'a FaultProxyConfigEvidence,
    demo: (&'a LiveConfig, &'a ProductionEvidenceConfigEvidence),
    production: (&'a LiveConfig, &'a ProductionEvidenceConfigEvidence),
    fault: (&'a LiveConfig, &'a ProductionEvidenceConfigEvidence),
    fault_proxy: &'a FaultProxyConfigEvidence,
    expected_fault_config_fingerprint: &'a str,
    fault_config_derived: bool,
    transition: &'a reap_live::ProductionTransitionReport,
    research: &'a ResearchDeploymentVerificationReport,
    demo_soak: &'a reap_live::LiveRunVerificationReport,
    fault_matrix: &'a reap_live::LiveFaultMatrixVerificationReport,
    latency: &'a LatencyCalibrationVerificationReport,
    account_artifacts: &'a [(PathBuf, AccountCertificationArtifact)],
    deadman_artifacts: &'a [(&'a ResolvedDeadmanInput, DeadmanExpiryCertificationArtifact)],
    emergency: &'a reap_live::EmergencyCancelVerificationReport,
    fill_inputs: &'a [VerifiedFillInput],
}

fn evaluate_bindings(input: BindingInputs<'_>) -> Vec<ProductionEvidenceFailure> {
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

    failures.sort_by_key(failure_sort_key);
    failures.dedup();
    failures
}

fn check_fault_proxy_entries<'a>(
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
        let scenario_name = serde_json::to_value(scenario)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| format!("{scenario:?}"));
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
fn check_live_identity(
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

fn check_account_coverage(
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

fn failure_sort_key(failure: &ProductionEvidenceFailure) -> String {
    serde_json::to_string(failure).unwrap_or_else(|_| format!("{failure:?}"))
}

fn gate_report<T: Serialize>(
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

fn config_evidence(
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

fn account_ids(config: &LiveConfig) -> BTreeSet<String> {
    config
        .accounts
        .iter()
        .map(|account| account.id.clone())
        .collect()
}

fn expected_identity(manifest: &ProductionEvidenceManifest) -> ProductionEvidenceExpectedIdentity {
    ProductionEvidenceExpectedIdentity {
        reap_version: manifest.expected_reap_version.clone(),
        live_executable_sha256: manifest.expected_live_executable_sha256.clone(),
        host_identity_sha256: manifest.expected_host_identity_sha256.clone(),
        deployment_candidate_id: manifest.expected_deployment_candidate_id.clone(),
        demo_account_identity_sha256s: manifest.expected_demo_account_identity_sha256s.clone(),
        production_account_identity_sha256s: manifest
            .expected_production_account_identity_sha256s
            .clone(),
    }
}

fn validate_manifest(manifest: &ProductionEvidenceManifest) -> Result<()> {
    if manifest.schema_version != PRODUCTION_EVIDENCE_MANIFEST_SCHEMA_VERSION {
        bail!(
            "production evidence manifest schema must be {}, got {}",
            PRODUCTION_EVIDENCE_MANIFEST_SCHEMA_VERSION,
            manifest.schema_version
        );
    }
    if manifest.expected_reap_version.is_empty()
        || manifest.expected_reap_version.trim() != manifest.expected_reap_version
    {
        bail!("expected_reap_version must be non-empty without surrounding whitespace");
    }
    for (field, value) in [
        (
            "expected_live_executable_sha256",
            manifest.expected_live_executable_sha256.as_str(),
        ),
        (
            "expected_host_identity_sha256",
            manifest.expected_host_identity_sha256.as_str(),
        ),
    ] {
        if !is_lower_sha256(value) {
            bail!("{field} must be a lower-case SHA-256");
        }
    }
    if manifest.expected_deployment_candidate_id.is_empty()
        || manifest.expected_deployment_candidate_id.trim()
            != manifest.expected_deployment_candidate_id
        || manifest.expected_deployment_candidate_id.len()
            > MAX_PRODUCTION_EVIDENCE_CANDIDATE_ID_BYTES
    {
        bail!("expected_deployment_candidate_id is invalid");
    }
    validate_expected_account_map(
        "expected_demo_account_identity_sha256s",
        &manifest.expected_demo_account_identity_sha256s,
    )?;
    validate_expected_account_map(
        "expected_production_account_identity_sha256s",
        &manifest.expected_production_account_identity_sha256s,
    )?;
    validate_count(
        "latency_source_reports",
        manifest.latency_source_reports.len(),
        MAX_PRODUCTION_EVIDENCE_LATENCY_REPORTS,
    )?;
    validate_count(
        "account_certifications",
        manifest.account_certifications.len(),
        MAX_PRODUCTION_EVIDENCE_ACCOUNTS,
    )?;
    validate_count(
        "deadman_certifications",
        manifest.deadman_certifications.len(),
        MAX_PRODUCTION_EVIDENCE_ACCOUNTS,
    )?;
    validate_count(
        "fill_reconciliations",
        manifest.fill_reconciliations.len(),
        MAX_PRODUCTION_EVIDENCE_ACCOUNTS,
    )?;
    for fill in &manifest.fill_reconciliations {
        if fill.minimum_fills == 0 {
            bail!("every fill reconciliation must require at least one fill");
        }
        for (field, value) in [
            ("price_tolerance", fill.price_tolerance),
            ("quantity_tolerance", fill.quantity_tolerance),
            ("fee_tolerance", fill.fee_tolerance),
        ] {
            if !value.is_finite() || value < 0.0 {
                bail!("fill reconciliation {field} must be finite and non-negative");
            }
        }
    }
    Ok(())
}

fn validate_expected_account_map(label: &str, values: &BTreeMap<String, String>) -> Result<()> {
    validate_count(label, values.len(), MAX_PRODUCTION_EVIDENCE_ACCOUNTS)?;
    for (account_id, sha256) in values {
        if account_id.is_empty() || account_id.trim() != account_id || account_id.len() > 128 {
            bail!("{label} contains an invalid account id");
        }
        if !is_lower_sha256(sha256) {
            bail!("{label}.{account_id} must be a lower-case SHA-256");
        }
    }
    Ok(())
}

fn validate_count(label: &str, actual: usize, maximum: usize) -> Result<()> {
    if actual == 0 || actual > maximum {
        bail!("{label} must contain 1..={maximum} entries, got {actual}");
    }
    Ok(())
}

fn resolve_manifest(loaded: &LoadedManifest) -> Result<ResolvedManifest> {
    let value = &loaded.value;
    let base = &loaded.base;
    let demo_config = resolve_regular_file(base, &value.demo_config, "demo config")?;
    let production_config =
        resolve_regular_file(base, &value.production_config, "production config")?;
    if demo_config == production_config {
        bail!("demo and production configs resolve to the same file");
    }
    let fault_demo_config =
        resolve_regular_file(base, &value.fault_demo_config, "routed fault demo config")?;
    let fault_proxy_config =
        resolve_regular_file(base, &value.fault_proxy_config, "fault-proxy config")?;
    if fault_demo_config == demo_config
        || fault_demo_config == production_config
        || fault_proxy_config == demo_config
        || fault_proxy_config == production_config
        || fault_proxy_config == fault_demo_config
    {
        bail!("demo, production, routed-fault, and fault-proxy configs must be distinct files");
    }
    let demo_soak_report = resolve_regular_file(base, &value.demo_soak_report, "demo soak report")?;
    let fault_matrix_manifest =
        resolve_regular_file(base, &value.fault_matrix_manifest, "fault matrix manifest")?;
    let latency_calibration_artifact = resolve_regular_file(
        base,
        &value.latency_calibration_artifact,
        "latency calibration artifact",
    )?;
    let latency_source_reports =
        resolve_unique_paths(base, &value.latency_source_reports, "latency source report")?;
    let research_manifest =
        resolve_regular_file(base, &value.research_manifest, "research manifest")?;
    let research_report = resolve_regular_file(base, &value.research_report, "research report")?;
    let account_certifications =
        resolve_unique_paths(base, &value.account_certifications, "account certification")?;
    let mut deadman_certifications = Vec::with_capacity(value.deadman_certifications.len());
    let mut deadman_artifacts = HashSet::new();
    for input in &value.deadman_certifications {
        let artifact = resolve_regular_file(base, &input.artifact, "deadman artifact")?;
        let journal = resolve_regular_file(base, &input.journal, "deadman journal")?;
        if artifact == journal {
            bail!("deadman artifact and journal resolve to the same file");
        }
        if !deadman_artifacts.insert(artifact.clone()) {
            bail!("duplicate deadman artifact {}", artifact.display());
        }
        deadman_certifications.push(ResolvedDeadmanInput { artifact, journal });
    }
    let emergency_cancel_report = resolve_regular_file(
        base,
        &value.emergency_cancel_report,
        "emergency cancel report",
    )?;
    let mut fill_reconciliations = Vec::with_capacity(value.fill_reconciliations.len());
    let mut fill_manifests = HashSet::new();
    for input in &value.fill_reconciliations {
        let collection_manifest =
            resolve_regular_file(base, &input.collection_manifest, "fill collection manifest")?;
        let journal = resolve_regular_file(base, &input.journal, "fill journal")?;
        if collection_manifest == journal {
            bail!("fill collection manifest and journal resolve to the same file");
        }
        if !fill_manifests.insert(collection_manifest.clone()) {
            bail!(
                "duplicate fill collection manifest {}",
                collection_manifest.display()
            );
        }
        fill_reconciliations.push(ResolvedFillInput {
            collection_manifest,
            journal,
            minimum_fills: input.minimum_fills,
            tolerances: FillStatementTolerances {
                price_abs: input.price_tolerance,
                quantity_abs: input.quantity_tolerance,
                fee_abs: input.fee_tolerance,
            },
        });
    }
    Ok(ResolvedManifest {
        demo_config,
        production_config,
        fault_demo_config,
        fault_proxy_config,
        demo_soak_report,
        fault_matrix_manifest,
        latency_calibration_artifact,
        latency_source_reports,
        research_manifest,
        research_report,
        account_certifications,
        deadman_certifications,
        emergency_cancel_report,
        fill_reconciliations,
    })
}

fn resolve_unique_paths(
    base: &Path,
    values: &[PathBuf],
    label: &'static str,
) -> Result<Vec<PathBuf>> {
    let mut resolved = Vec::with_capacity(values.len());
    let mut unique = HashSet::new();
    for value in values {
        let path = resolve_regular_file(base, value, label)?;
        if !unique.insert(path.clone()) {
            bail!("duplicate {label} path {}", path.display());
        }
        resolved.push(path);
    }
    Ok(resolved)
}

fn resolve_regular_file(base: &Path, value: &Path, label: &'static str) -> Result<PathBuf> {
    let path = if value.is_absolute() {
        value.to_path_buf()
    } else {
        base.join(value)
    };
    let metadata = std::fs::symlink_metadata(&path)
        .with_context(|| format!("invalid {label} path {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "{label} {} must be a regular file and not a symbolic link",
            path.display()
        );
    }
    std::fs::canonicalize(&path)
        .with_context(|| format!("failed to canonicalize {label} {}", path.display()))
}

fn load_manifest(path: &Path) -> Result<LoadedManifest> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("invalid production evidence manifest {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "production evidence manifest {} must be a regular file and not a symbolic link",
            path.display()
        );
    }
    if metadata.len() > MAX_PRODUCTION_EVIDENCE_MANIFEST_BYTES {
        bail!(
            "production evidence manifest is {} bytes; limit is {}",
            metadata.len(),
            MAX_PRODUCTION_EVIDENCE_MANIFEST_BYTES
        );
    }
    let source_path = std::fs::canonicalize(path).with_context(|| {
        format!(
            "failed to canonicalize production evidence manifest {}",
            path.display()
        )
    })?;
    let bytes = std::fs::read(&source_path).with_context(|| {
        format!(
            "failed to read production evidence manifest {}",
            source_path.display()
        )
    })?;
    if bytes.len() as u64 > MAX_PRODUCTION_EVIDENCE_MANIFEST_BYTES {
        bail!(
            "production evidence manifest is {} bytes after reading; limit is {}",
            bytes.len(),
            MAX_PRODUCTION_EVIDENCE_MANIFEST_BYTES
        );
    }
    let text = std::str::from_utf8(&bytes).context("production evidence manifest is not UTF-8")?;
    let value: ProductionEvidenceManifest =
        toml::from_str(text).context("failed to parse strict production evidence manifest")?;
    let base = source_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    Ok(LoadedManifest {
        evidence: ProductionEvidenceFileEvidence {
            source_path,
            bytes: bytes.len() as u64,
            sha256: sha256_bytes(&bytes),
        },
        value,
        base,
    })
}

fn serialized_sha256<T: Serialize>(value: &T) -> Result<String> {
    let bytes = serde_json::to_vec(value).context("failed to serialize reconstructed evidence")?;
    Ok(sha256_bytes(&bytes))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_toml(extra: &str) -> String {
        format!(
            r#"
schema_version = 1
expected_reap_version = "0.1.0"
expected_live_executable_sha256 = "{}"
expected_host_identity_sha256 = "{}"
expected_deployment_candidate_id = "candidate-a"
demo_config = "demo.toml"
production_config = "production.toml"
fault_demo_config = "fault-demo.toml"
fault_proxy_config = "fault-proxy.toml"
demo_soak_report = "soak.json"
fault_matrix_manifest = "faults.toml"
latency_calibration_artifact = "latency.json"
latency_source_reports = ["latency-source.json"]
research_manifest = "research.toml"
research_report = "research.json"
account_certifications = ["account.json"]
emergency_cancel_report = "emergency.json"

[expected_demo_account_identity_sha256s]
main = "{}"

[expected_production_account_identity_sha256s]
main = "{}"

[[deadman_certifications]]
artifact = "deadman.json"
journal = "journal.jsonl"

[[fill_reconciliations]]
collection_manifest = "fills/manifest.json"
journal = "journal.jsonl"
minimum_fills = 1
{extra}
"#,
            "1".repeat(64),
            "2".repeat(64),
            "3".repeat(64),
            "4".repeat(64),
        )
    }

    #[test]
    fn strict_manifest_accepts_complete_shape_and_rejects_unknown_fields() {
        let parsed: ProductionEvidenceManifest = toml::from_str(&manifest_toml("")).unwrap();
        validate_manifest(&parsed).unwrap();

        let template: ProductionEvidenceManifest =
            toml::from_str(include_str!("../../../examples/production-evidence.toml")).unwrap();
        validate_manifest(&template).unwrap();

        let error = toml::from_str::<ProductionEvidenceManifest>(&manifest_toml(
            "unknown_release_switch = true",
        ))
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn manifest_rejects_zero_fill_threshold_and_invalid_identity() {
        let mut parsed: ProductionEvidenceManifest = toml::from_str(&manifest_toml("")).unwrap();
        parsed.fill_reconciliations[0].minimum_fills = 0;
        assert!(validate_manifest(&parsed).is_err());

        parsed.fill_reconciliations[0].minimum_fills = 1;
        parsed.expected_host_identity_sha256 = "ABC".to_string();
        assert!(validate_manifest(&parsed).is_err());
    }

    #[test]
    fn identity_binding_reports_each_wrong_boundary() {
        let mut failures = Vec::new();
        let expected_accounts = BTreeMap::from([("main".to_string(), "4".repeat(64))]);
        let observed_accounts = BTreeMap::from([("main".to_string(), "5".repeat(64))]);
        check_live_identity(
            &mut failures,
            ProductionEvidenceGate::DemoSoak,
            None,
            "wrong-version",
            &"6".repeat(64),
            &"7".repeat(64),
            &observed_accounts,
            "0.1.0",
            &"1".repeat(64),
            &"2".repeat(64),
            &expected_accounts,
        );
        assert_eq!(failures.len(), 4);
    }

    #[test]
    fn production_bundle_requires_unique_exact_proxy_evidence() {
        let exact = reap_live::LiveFaultProxyEvidenceSummary {
            format_version: 1,
            proxy_session_id: "proxy-one".to_string(),
            proxy_config_fingerprint: "a".repeat(64),
            command_id: "command-one".to_string(),
            command_kind: "disconnect_websockets".to_string(),
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
            failures.iter().any(|failure| matches!(
                failure,
                ProductionEvidenceFailure::BindingMismatch { .. }
            ))
        );
    }

    #[test]
    fn account_coverage_is_exact() {
        let expected = BTreeSet::from(["a".to_string(), "b".to_string()]);
        let actual = BTreeSet::from(["a".to_string()]);
        let mut failures = Vec::new();
        check_account_coverage(
            &mut failures,
            ProductionEvidenceGate::AccountCertification,
            &expected,
            &actual,
        );
        assert_eq!(
            failures,
            [ProductionEvidenceFailure::AccountCoverageMismatch {
                gate: ProductionEvidenceGate::AccountCertification,
                expected: vec!["a".to_string(), "b".to_string()],
                actual: vec!["a".to_string()],
            }]
        );
    }
}
