use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
#[cfg(test)]
use reap_backtest::ResearchOpeningAccountEvidence;
use reap_core::PINNED_JAVA_REVISION;
use reap_live::{
    EconomicReconciliationTolerances, FillStatementTolerances, LiveConfigFileEvidence,
    TradingEnvironment, current_executable_sha256, host_identity_sha256,
};
use serde::{Deserialize, Serialize};

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
use canonical::{failure_sort_key, serialized_sha256, sha256_bytes};
use manifest::{load_manifest, resolve_manifest, validate_manifest};
#[cfg(test)]
use manifest::{resolve_regular_file, resolve_unique_paths};
use policy_time::{FreshnessInputs, evaluate_freshness, unix_time_ms};
#[cfg(test)]
use policy_time::{check_fault_proxy_live_session, push_freshness};
use report::{GateInputs, build_gate_reports, expected_identity, summarize_sources};
use source_verifiers::{
    VerifiedEconomicInput, VerifiedFaultProxyRun, VerifiedFillInput, VerifiedTimedLiveSource,
    load_initial_configs, reconstruct_sources, reopen_verified_configs,
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

    let initial = load_initial_configs(&paths)?;
    let sources = reconstruct_sources(&paths)?;
    let reopened = reopen_verified_configs(&paths, &loaded)?;
    let summaries = summarize_sources(&sources);
    let verified_at_ms = unix_time_ms()?;
    let (freshness_observations, freshness_failures) = evaluate_freshness(FreshnessInputs {
        policy: &loaded.value.freshness,
        verified_at_ms,
        demo_soak_path: &paths.demo_soak_report,
        demo_soak: &sources.demo_soak,
        fault_matrix: &sources.fault_matrix,
        fault_live_sources: &sources.fault_live_sources,
        fault_proxy_runs: &sources.fault_proxy_runs,
        latency_live_sources: &sources.latency_live_sources,
        account_artifacts: &sources.account_artifacts,
        deadman_artifacts: &sources.deadman_artifacts,
        emergency_path: &paths.emergency_cancel_report,
        emergency: &sources.emergency,
        fill_inputs: &sources.fill_inputs,
        economic_inputs: &sources.economic_inputs,
    });
    let gates = build_gate_reports(GateInputs {
        paths: &paths,
        reopened: &reopened,
        sources: &sources,
        freshness_observations: &freshness_observations,
        freshness_failures: &freshness_failures,
    })?;

    let bindings = BindingInputs {
        expected: &expected,
        verifier: &verifier,
        demo_start: (&initial.demo_config, &initial.demo_file),
        production_start: (&initial.production_config, &initial.production_file),
        fault_start: &initial.fault_file,
        fault_proxy_start: &initial.fault_proxy_evidence,
        demo: (&reopened.demo_config, &reopened.demo_evidence),
        production: (&reopened.production_config, &reopened.production_evidence),
        fault: (&reopened.fault_config, &reopened.fault_evidence),
        fault_proxy: &reopened.fault_proxy_evidence,
        expected_fault_config_fingerprint: &reopened.expected_fault_config_fingerprint,
        fault_config_derived: reopened.fault_config_derived,
        transition: &sources.transition,
        research: &sources.research,
        demo_soak: &sources.demo_soak,
        fault_matrix: &sources.fault_matrix,
        fault_live_sources: &sources.fault_live_sources,
        fault_proxy_runs: &sources.fault_proxy_runs,
        latency: &sources.latency,
        account_artifacts: &sources.account_artifacts,
        deadman_artifacts: &sources.deadman_artifacts,
        emergency: &sources.emergency,
        fill_inputs: &sources.fill_inputs,
        economic_inputs: &sources.economic_inputs,
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
        fault_proxy_runs: summaries.observed_fault_proxy_runs,
        verifier,
        demo_config: reopened.demo_evidence,
        production_config: reopened.production_evidence,
        fault_demo_config: reopened.fault_evidence,
        observed_demo_identity: summaries.observed_demo_identity,
        observed_production_account_identity_sha256s: summaries.observed_production_accounts,
        observed_deployment_candidate_id: sources.research.deployment_candidate_id.clone(),
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
