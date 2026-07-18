use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use reap_backtest::ResearchOpeningAccountEvidence;
use reap_core::PINNED_JAVA_REVISION;
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
    EconomicReconciliationOptions, EconomicReconciliationTolerances, FillStatementCoverage,
    FillStatementReconciliationOptions, FillStatementTolerances, LiveConfig,
    LiveConfigFileEvidence, LiveMode, TradingEnvironment, current_executable_sha256,
    host_identity_sha256, load_live_config_with_evidence, reconcile_okx_economics_paths,
    reconcile_okx_fill_collection_paths, verify_account_certification_artifact_path,
    verify_bill_collection_manifest_path, verify_deadman_expiry_certification_artifact_path,
    verify_fill_collection_manifest_path, verify_live_fault_matrix_paths, verify_live_run_paths,
    verify_production_transition_paths,
};
use serde::{Deserialize, Serialize};

use crate::deployment::{ResearchDeploymentVerificationReport, verify_research_deployment_paths};
use crate::latency::{LatencyCalibrationVerificationReport, verify_latency_calibration};

mod canonical;
mod manifest;
mod policy_time;

use canonical::{failure_sort_key, scenario_name, serialized_sha256, sha256_bytes};
use manifest::{load_manifest, resolve_manifest, validate_manifest};
#[cfg(test)]
use manifest::{resolve_regular_file, resolve_unique_paths};
use policy_time::{FreshnessInputs, evaluate_freshness, unix_time_ms};
#[cfg(test)]
use policy_time::{check_fault_proxy_live_session, push_freshness};

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

struct VerifiedFillInput {
    collection_manifest: PathBuf,
    journal: PathBuf,
    manifest: reap_live::FillCollectionManifest,
    report: reap_live::FillStatementReconciliationReport,
}

struct VerifiedEconomicInput {
    fill_collection_manifest: PathBuf,
    bill_collection_manifest: PathBuf,
    opening_account_certification: PathBuf,
    closing_account_certification: PathBuf,
    journal: PathBuf,
    fill_manifest: reap_live::FillCollectionManifest,
    bill_manifest: reap_live::BillCollectionManifest,
    opening_account: AccountCertificationArtifact,
    closing_account: AccountCertificationArtifact,
    report: reap_live::EconomicReconciliationReport,
}

struct VerifiedTimedLiveSource {
    gate: ProductionEvidenceGate,
    subject: Option<String>,
    report: reap_live::LiveRunVerificationReport,
}

struct VerifiedFaultProxyRun {
    scenario: reap_live::LiveFaultScenario,
    report: FaultProxyRunVerificationReport,
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

fn verify_fault_live_sources(
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

fn verify_latency_live_sources(
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
    fault_live_sources: &'a [VerifiedTimedLiveSource],
    fault_proxy_runs: &'a [VerifiedFaultProxyRun],
    latency: &'a LatencyCalibrationVerificationReport,
    account_artifacts: &'a [(PathBuf, AccountCertificationArtifact)],
    deadman_artifacts: &'a [(&'a ResolvedDeadmanInput, DeadmanExpiryCertificationArtifact)],
    emergency: &'a EmergencyCancelVerificationReport,
    fill_inputs: &'a [VerifiedFillInput],
    economic_inputs: &'a [VerifiedEconomicInput],
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

fn enclosed_fault_scenarios(
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

fn check_research_opening_accounts(
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
        approval_policy_sha256: manifest.expected_approval_policy_sha256.clone(),
        deployment_candidate_id: manifest.expected_deployment_candidate_id.clone(),
        demo_account_identity_sha256s: manifest.expected_demo_account_identity_sha256s.clone(),
        production_account_identity_sha256s: manifest
            .expected_production_account_identity_sha256s
            .clone(),
    }
}

#[cfg(test)]
#[path = "../tests/production_evidence_unit/mod.rs"]
mod tests;
