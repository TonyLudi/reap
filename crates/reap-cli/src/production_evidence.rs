use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
#[cfg(test)]
use reap_backtest::ResearchOpeningAccountEvidence;
use reap_core::PINNED_JAVA_REVISION;
use reap_emergency_core::{EmergencyCancelVerificationOptions, verify_emergency_cancel_paths};
use reap_fault::{FaultProxyConfig, verify_fault_proxy_run_paths};
use reap_live::{
    EconomicReconciliationOptions, EconomicReconciliationTolerances,
    FillStatementReconciliationOptions, FillStatementTolerances, LiveConfigFileEvidence, LiveMode,
    TradingEnvironment, current_executable_sha256, host_identity_sha256,
    load_live_config_with_evidence, reconcile_okx_economics_paths,
    reconcile_okx_fill_collection_paths, verify_account_certification_artifact_path,
    verify_bill_collection_manifest_path, verify_deadman_expiry_certification_artifact_path,
    verify_fill_collection_manifest_path, verify_live_fault_matrix_paths, verify_live_run_paths,
    verify_production_transition_paths,
};
use serde::{Deserialize, Serialize};

use crate::deployment::verify_research_deployment_paths;
use crate::latency::verify_latency_calibration;

mod bindings;
mod canonical;
mod manifest;
mod policy_time;
mod report;
mod source_verifiers;

use bindings::{BindingInputs, evaluate_bindings};
#[cfg(test)]
use bindings::{
    check_account_coverage, check_fault_proxy_entries, check_live_identity,
    check_research_opening_accounts, enclosed_fault_scenarios,
};
use canonical::{failure_sort_key, scenario_name, serialized_sha256, sha256_bytes};
use manifest::{load_manifest, resolve_manifest, validate_manifest};
#[cfg(test)]
use manifest::{resolve_regular_file, resolve_unique_paths};
use policy_time::{FreshnessInputs, evaluate_freshness, unix_time_ms};
#[cfg(test)]
use policy_time::{check_fault_proxy_live_session, push_freshness};
use report::{expected_identity, gate_report};
use source_verifiers::{
    VerifiedEconomicInput, VerifiedFaultProxyRun, VerifiedFillInput, VerifiedTimedLiveSource,
    config_evidence, verify_fault_live_sources, verify_latency_live_sources,
};

pub(crate) const PRODUCTION_EVIDENCE_MANIFEST_SCHEMA_VERSION: u16 = 8;
pub(crate) const PRODUCTION_EVIDENCE_REPORT_FORMAT_VERSION: u16 = 9;
pub(crate) const PRODUCTION_EVIDENCE_APPROVAL_SUBJECT_FORMAT_VERSION: u16 = 1;
const MAX_PRODUCTION_EVIDENCE_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_PRODUCTION_EVIDENCE_ACCOUNTS: usize = 32;
const MAX_PRODUCTION_EVIDENCE_LATENCY_REPORTS: usize = 128;
const MAX_PRODUCTION_EVIDENCE_CANDIDATE_ID_BYTES: usize = 128;
const MAX_FUTURE_TOLERANCE_MS: u64 = 5 * 60 * 1_000;
const MAX_DEMO_SOAK_AGE_MS: u64 = 24 * 60 * 60 * 1_000;
const MAX_FAULT_RUN_AGE_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const MAX_LATENCY_SOURCE_AGE_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const MAX_PRODUCTION_ACCOUNT_CERTIFICATION_AGE_MS: u64 = 15 * 60 * 1_000;
const MAX_DEADMAN_CERTIFICATION_AGE_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const MAX_EMERGENCY_CANCEL_AGE_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const MAX_FILL_COLLECTION_AGE_MS: u64 = 24 * 60 * 60 * 1_000;
const MAX_BILL_COLLECTION_AGE_MS: u64 = 24 * 60 * 60 * 1_000;
const MAX_PRODUCTION_ECONOMIC_QUANTITY_TOLERANCE: f64 = 1e-8;
const MAX_PRODUCTION_ECONOMIC_FEE_TOLERANCE: f64 = 1e-10;
const MAX_PRODUCTION_ECONOMIC_BALANCE_TOLERANCE: f64 = 1e-8;
const MAX_PRODUCTION_ECONOMIC_TRADE_PNL_ABSOLUTE_TOLERANCE: f64 = 1e-8;
const MAX_PRODUCTION_ECONOMIC_TRADE_PNL_RELATIVE_TOLERANCE: f64 = 1e-6;
const MAX_PRODUCTION_ECONOMIC_FUNDING_ABSOLUTE_TOLERANCE: f64 = 1e-8;
const MAX_PRODUCTION_ECONOMIC_FUNDING_RELATIVE_TOLERANCE: f64 = 1e-6;
const MAX_PRODUCTION_FUNDING_MARK_BRACKET_DISTANCE_MS: u64 = 2_000;
const MAX_PRODUCTION_ACCOUNT_BOUNDARY_GAP_MS: u64 = 60_000;
const MAX_PRODUCTION_ECONOMIC_FUNDING_MARK_ABSOLUTE_TOLERANCE: f64 = 1e-8;
const MAX_PRODUCTION_ECONOMIC_FUNDING_MARK_RELATIVE_TOLERANCE: f64 = 1e-4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceFreshnessPolicy {
    pub future_tolerance_ms: u64,
    pub demo_soak_max_age_ms: u64,
    pub fault_run_max_age_ms: u64,
    pub latency_source_max_age_ms: u64,
    pub production_account_certification_max_age_ms: u64,
    pub deadman_certification_max_age_ms: u64,
    pub emergency_cancel_max_age_ms: u64,
    pub fill_collection_max_age_ms: u64,
    pub bill_collection_max_age_ms: u64,
}

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceEconomicInput {
    pub fill_collection_manifest: PathBuf,
    pub bill_collection_manifest: PathBuf,
    pub opening_account_certification: PathBuf,
    pub closing_account_certification: PathBuf,
    pub journal: PathBuf,
    pub minimum_trade_bills: u64,
    pub minimum_derivative_close_bills: u64,
    pub minimum_funding_bills: u64,
    pub maximum_trade_bill_delay_ms: u64,
    pub maximum_funding_bill_delay_ms: u64,
    pub maximum_funding_mark_bracket_distance_ms: u64,
    pub maximum_account_boundary_gap_ms: u64,
    #[serde(default)]
    pub price_tolerance: f64,
    #[serde(default)]
    pub quantity_tolerance: f64,
    #[serde(default)]
    pub fee_tolerance: f64,
    #[serde(default)]
    pub balance_tolerance: f64,
    #[serde(default)]
    pub trade_pnl_absolute_tolerance: f64,
    #[serde(default)]
    pub trade_pnl_relative_tolerance: f64,
    #[serde(default)]
    pub funding_pnl_absolute_tolerance: f64,
    #[serde(default)]
    pub funding_pnl_relative_tolerance: f64,
    #[serde(default)]
    pub funding_mark_absolute_tolerance: f64,
    #[serde(default)]
    pub funding_mark_relative_tolerance: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceDeadmanInput {
    pub artifact: PathBuf,
    pub journal: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceFaultProxyRunInput {
    pub scenario: reap_live::LiveFaultScenario,
    pub report: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceManifest {
    pub schema_version: u16,
    pub expected_reap_version: String,
    pub expected_live_executable_sha256: String,
    pub expected_host_identity_sha256: String,
    pub expected_approval_policy_sha256: String,
    pub expected_deployment_candidate_id: String,
    pub expected_demo_account_identity_sha256s: BTreeMap<String, String>,
    pub expected_production_account_identity_sha256s: BTreeMap<String, String>,
    pub freshness: ProductionEvidenceFreshnessPolicy,
    pub demo_config: PathBuf,
    pub production_config: PathBuf,
    pub fault_demo_config: PathBuf,
    pub fault_proxy_config: PathBuf,
    pub demo_soak_report: PathBuf,
    pub fault_matrix_manifest: PathBuf,
    pub fault_proxy_runs: Vec<ProductionEvidenceFaultProxyRunInput>,
    pub latency_calibration_artifact: PathBuf,
    pub latency_source_reports: Vec<PathBuf>,
    pub research_manifest: PathBuf,
    pub research_report: PathBuf,
    pub account_certifications: Vec<PathBuf>,
    pub deadman_certifications: Vec<ProductionEvidenceDeadmanInput>,
    pub emergency_cancel_report: PathBuf,
    pub fill_reconciliations: Vec<ProductionEvidenceFillInput>,
    pub economic_reconciliations: Vec<ProductionEvidenceEconomicInput>,
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
    pub approval_policy_sha256: String,
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
    Freshness,
    FaultProxyRun,
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
    EconomicReconciliation,
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
    EvidenceTimestampInvalid {
        gate: ProductionEvidenceGate,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<String>,
        started_at_ms: u64,
        completed_at_ms: u64,
    },
    EvidenceTimestampInFuture {
        gate: ProductionEvidenceGate,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<String>,
        completed_at_ms: u64,
        verified_at_ms: u64,
        future_tolerance_ms: u64,
    },
    EvidenceStale {
        gate: ProductionEvidenceGate,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<String>,
        age_ms: u64,
        maximum_age_ms: u64,
    },
    FaultProxyOutsideLiveSession {
        scenario: reap_live::LiveFaultScenario,
        proxy_armed_at_ms: u64,
        proxy_completed_at_ms: u64,
        live_started_at_ms: u64,
        live_completed_at_ms: u64,
    },
    FaultProxyRunCoverageMismatch {
        expected: Vec<reap_live::LiveFaultScenario>,
        actual: Vec<reap_live::LiveFaultScenario>,
    },
    DuplicateFaultProxyRunSession {
        proxy_session_id: String,
    },
    FaultProxyRunDoesNotEncloseLiveSession {
        scenario: reap_live::LiveFaultScenario,
        proxy_started_at_ms: u64,
        proxy_stopped_at_ms: u64,
        live_started_at_ms: u64,
        live_completed_at_ms: u64,
    },
    FaultProxyRunAmbiguousLiveCoverage {
        scenario: reap_live::LiveFaultScenario,
        enclosed_scenarios: Vec<reap_live::LiveFaultScenario>,
    },
    FaultProxyCompletedFaultCountMismatch {
        scenario: reap_live::LiveFaultScenario,
        expected: u64,
        actual: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceFreshnessObservation {
    pub gate: ProductionEvidenceGate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub source_path: PathBuf,
    pub started_at_ms: u64,
    pub completed_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_ms: Option<u64>,
    pub maximum_age_ms: u64,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceFaultProxyRunSummary {
    pub scenario: reap_live::LiveFaultScenario,
    pub run_report: reap_fault::FaultProxyRunFileEvidence,
    pub proxy_session_id: String,
    pub started_at_ms: u64,
    pub stopped_at_ms: u64,
    pub completed_faults: u64,
    pub acceptance_passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceVerificationReport {
    pub format_version: u16,
    pub manifest_schema_version: u16,
    pub java_reference_revision: String,
    pub verifier_reap_version: String,
    pub verified_at_ms: u64,
    pub manifest: ProductionEvidenceFileEvidence,
    pub expected: ProductionEvidenceExpectedIdentity,
    pub freshness_policy: ProductionEvidenceFreshnessPolicy,
    pub freshness_observations: Vec<ProductionEvidenceFreshnessObservation>,
    pub fault_proxy_runs: Vec<ProductionEvidenceFaultProxyRunSummary>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceApprovalFreshnessObservation {
    pub gate: ProductionEvidenceGate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub source_path: PathBuf,
    pub started_at_ms: u64,
    pub completed_at_ms: u64,
    pub maximum_age_ms: u64,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceApprovalGate {
    pub gate: ProductionEvidenceGate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub source_paths: Vec<PathBuf>,
    pub reconstructed_sha256: String,
    pub acceptance_passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductionEvidenceApprovalSubject {
    pub format_version: u16,
    pub production_evidence_report_format_version: u16,
    pub manifest_schema_version: u16,
    pub java_reference_revision: String,
    pub manifest: ProductionEvidenceFileEvidence,
    pub expected: ProductionEvidenceExpectedIdentity,
    pub freshness_policy: ProductionEvidenceFreshnessPolicy,
    pub freshness_observations: Vec<ProductionEvidenceApprovalFreshnessObservation>,
    pub fault_proxy_runs: Vec<ProductionEvidenceFaultProxyRunSummary>,
    pub verifier: ProductionEvidenceVerifierIdentity,
    pub demo_config: ProductionEvidenceConfigEvidence,
    pub production_config: ProductionEvidenceConfigEvidence,
    pub fault_demo_config: ProductionEvidenceConfigEvidence,
    pub observed_demo_identity: ProductionEvidenceLiveIdentity,
    pub observed_production_account_identity_sha256s: BTreeMap<String, String>,
    pub observed_deployment_candidate_id: Option<String>,
    pub gates: Vec<ProductionEvidenceApprovalGate>,
    pub limitations: Vec<String>,
    pub evidence_bundle_passed: bool,
    pub production_order_entry_authorized: bool,
}

impl ProductionEvidenceApprovalSubject {
    pub(crate) fn from_report(report: &ProductionEvidenceVerificationReport) -> Result<Self> {
        if report.format_version != PRODUCTION_EVIDENCE_REPORT_FORMAT_VERSION
            || report.manifest_schema_version != PRODUCTION_EVIDENCE_MANIFEST_SCHEMA_VERSION
            || report.java_reference_revision != PINNED_JAVA_REVISION
            || !report.evidence_bundle_passed
            || !report.failures.is_empty()
            || report.gates.iter().any(|gate| !gate.acceptance_passed)
            || report.production_order_entry_authorized
        {
            bail!(
                "only a passing, unauthorized current-format production bundle can become an approval subject"
            );
        }
        let freshness_observations = report
            .freshness_observations
            .iter()
            .map(
                |observation| ProductionEvidenceApprovalFreshnessObservation {
                    gate: observation.gate,
                    subject: observation.subject.clone(),
                    source_path: observation.source_path.clone(),
                    started_at_ms: observation.started_at_ms,
                    completed_at_ms: observation.completed_at_ms,
                    maximum_age_ms: observation.maximum_age_ms,
                    passed: observation.passed,
                },
            )
            .collect::<Vec<_>>();
        let stable_freshness_sha256 = serialized_sha256(&freshness_observations)?;
        let gates = report
            .gates
            .iter()
            .map(|gate| ProductionEvidenceApprovalGate {
                gate: gate.gate,
                subject: gate.subject.clone(),
                source_paths: gate.source_paths.clone(),
                reconstructed_sha256: if gate.gate == ProductionEvidenceGate::Freshness {
                    stable_freshness_sha256.clone()
                } else {
                    gate.reconstructed_sha256.clone()
                },
                acceptance_passed: gate.acceptance_passed,
            })
            .collect();
        Ok(Self {
            format_version: PRODUCTION_EVIDENCE_APPROVAL_SUBJECT_FORMAT_VERSION,
            production_evidence_report_format_version: report.format_version,
            manifest_schema_version: report.manifest_schema_version,
            java_reference_revision: report.java_reference_revision.clone(),
            manifest: report.manifest.clone(),
            expected: report.expected.clone(),
            freshness_policy: report.freshness_policy.clone(),
            freshness_observations,
            fault_proxy_runs: report.fault_proxy_runs.clone(),
            verifier: report.verifier.clone(),
            demo_config: report.demo_config.clone(),
            production_config: report.production_config.clone(),
            fault_demo_config: report.fault_demo_config.clone(),
            observed_demo_identity: report.observed_demo_identity.clone(),
            observed_production_account_identity_sha256s: report
                .observed_production_account_identity_sha256s
                .clone(),
            observed_deployment_candidate_id: report.observed_deployment_candidate_id.clone(),
            gates,
            limitations: report.limitations.clone(),
            evidence_bundle_passed: report.evidence_bundle_passed,
            production_order_entry_authorized: report.production_order_entry_authorized,
        })
    }

    pub(crate) fn sha256(&self) -> Result<String> {
        serialized_sha256(self)
    }
}

struct ResolvedDeadmanInput {
    artifact: PathBuf,
    journal: PathBuf,
}

struct ResolvedFaultProxyRunInput {
    scenario: reap_live::LiveFaultScenario,
    report: PathBuf,
}

struct ResolvedFillInput {
    collection_manifest: PathBuf,
    journal: PathBuf,
    minimum_fills: u64,
    tolerances: FillStatementTolerances,
}

struct ResolvedEconomicInput {
    fill_collection_manifest: PathBuf,
    bill_collection_manifest: PathBuf,
    opening_account_certification: PathBuf,
    closing_account_certification: PathBuf,
    journal: PathBuf,
    minimum_trade_bills: u64,
    minimum_derivative_close_bills: u64,
    minimum_funding_bills: u64,
    maximum_trade_bill_delay_ms: u64,
    maximum_funding_bill_delay_ms: u64,
    maximum_funding_mark_bracket_distance_ms: u64,
    maximum_account_boundary_gap_ms: u64,
    tolerances: EconomicReconciliationTolerances,
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
        load_live_config_with_evidence(&paths.demo_config)
            .context("failed to load exact demo config for production evidence")?;
    let (production_config_start, production_file_start) =
        load_live_config_with_evidence(&paths.production_config)
            .context("failed to load exact production config for production evidence")?;
    let (_fault_config_start, fault_file_start) =
        load_live_config_with_evidence(&paths.fault_demo_config)
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
    let fault_live_sources = verify_fault_live_sources(&paths.fault_demo_config, &fault_matrix)
        .context("failed to bind fault-matrix source timestamps")?;
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
    let latency_live_sources = verify_latency_live_sources(&paths.demo_config, &latency)
        .context("failed to bind latency source timestamps")?;

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

    let mut economic_inputs = Vec::with_capacity(paths.economic_reconciliations.len());
    for input in &paths.economic_reconciliations {
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
        economic_inputs.push(VerifiedEconomicInput {
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
        });
    }

    // Reopen all configs after the expensive source reconstructions. Every
    // subordinate report is compared to this final exact-file observation.
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
    let manifest_final = load_manifest(&loaded.evidence.source_path)
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
    let mut observed_fault_proxy_runs = fault_proxy_runs
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
    let verified_at_ms = unix_time_ms()?;
    let (freshness_observations, freshness_failures) = evaluate_freshness(FreshnessInputs {
        policy: &loaded.value.freshness,
        verified_at_ms,
        demo_soak_path: &paths.demo_soak_report,
        demo_soak: &demo_soak,
        fault_matrix: &fault_matrix,
        fault_live_sources: &fault_live_sources,
        fault_proxy_runs: &fault_proxy_runs,
        latency_live_sources: &latency_live_sources,
        account_artifacts: &account_artifacts,
        deadman_artifacts: &deadman_artifacts,
        emergency_path: &paths.emergency_cancel_report,
        emergency: &emergency,
        fill_inputs: &fill_inputs,
        economic_inputs: &economic_inputs,
    });

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
    for proxy in &fault_proxy_runs {
        gates.push(gate_report(
            ProductionEvidenceGate::FaultProxyRun,
            Some(scenario_name(proxy.scenario)),
            vec![
                paths.fault_proxy_config.clone(),
                proxy.report.run_report.source_path.clone(),
            ],
            &proxy.report,
            proxy.report.acceptance_passed,
        )?);
    }
    let freshness_paths = freshness_observations
        .iter()
        .map(|observation| observation.source_path.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    gates.push(gate_report(
        ProductionEvidenceGate::Freshness,
        None,
        freshness_paths,
        &freshness_observations,
        freshness_failures.is_empty(),
    )?);
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
    for input in &economic_inputs {
        gates.push(gate_report(
            ProductionEvidenceGate::EconomicReconciliation,
            Some(input.bill_manifest.account_id.clone()),
            vec![
                input.fill_collection_manifest.clone(),
                input.bill_collection_manifest.clone(),
                input.opening_account_certification.clone(),
                input.closing_account_certification.clone(),
                input.journal.clone(),
            ],
            &(
                &input.fill_manifest,
                &input.bill_manifest,
                &input.opening_account,
                &input.closing_account,
                &input.report,
            ),
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
        fault_live_sources: &fault_live_sources,
        fault_proxy_runs: &fault_proxy_runs,
        latency: &latency,
        account_artifacts: &account_artifacts,
        deadman_artifacts: &deadman_artifacts,
        emergency: &emergency,
        fill_inputs: &fill_inputs,
        economic_inputs: &economic_inputs,
    };
    let mut failures = evaluate_bindings(bindings);
    failures.extend(freshness_failures);
    failures.sort_by_key(failure_sort_key);
    failures.dedup();
    let evidence_bundle_passed =
        failures.is_empty() && gates.iter().all(|gate| gate.acceptance_passed);

    Ok(ProductionEvidenceVerificationReport {
        format_version: PRODUCTION_EVIDENCE_REPORT_FORMAT_VERSION,
        manifest_schema_version: loaded.value.schema_version,
        java_reference_revision: PINNED_JAVA_REVISION.to_string(),
        verifier_reap_version: env!("CARGO_PKG_VERSION").to_string(),
        verified_at_ms,
        manifest: loaded.evidence,
        expected,
        freshness_policy: loaded.value.freshness,
        freshness_observations,
        fault_proxy_runs: observed_fault_proxy_runs,
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
            "a passing bundle reconstructs the implemented source gates, binds exact configs, candidate, build, host, and account identities, and enforces the manifest freshness policy within hard upper bounds"
                .to_string(),
            "source timestamps and verifier wall time are validated artifact fields but are not remotely attested; operators must independently control clock synchronization, the manifest, and target host"
                .to_string(),
            "trade/funding reconciliation rejects unexplained account bills, proves controlled-window cash continuity, checks endpoint equity conversion, and binds funding to the journaled signed position plus two-sided mark bracket; exact internal valuation ticks, total-equity attribution, taxes, and profitability review remain external gates"
                .to_string(),
            "supervision, paging, credential permissions, venue announcements, rollout/rollback review, and explicit human approval remain required"
                .to_string(),
            "the bound absolute connection pacer coordinates Reap processes on the declared host only; another host sharing the same egress IP requires isolated egress or an external IP-wide coordinator"
                .to_string(),
            "partial-fill and restored-latch roles may use opaque external injector evidence; freshness is enforced on their verified live reports, while external causality remains an operator-reviewed gate"
                .to_string(),
            "this verifier never authorizes or enables production order entry".to_string(),
        ],
        evidence_bundle_passed,
        production_order_entry_authorized: false,
    })
}

#[cfg(test)]
#[path = "../tests/production_evidence_unit/mod.rs"]
mod tests;
