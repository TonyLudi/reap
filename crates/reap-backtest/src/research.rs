use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use reap_capture::{
    CaptureAnalysisReport, CaptureConfig, CaptureVerificationReport, analyze_capture_path,
    verify_capture_paths,
};
use reap_core::PositionMarginMode;
use reap_feed::{ReplayCheckReport, replay_check_path};
use reap_live_contracts::{
    ACCOUNT_CERTIFICATION_SCHEMA_VERSION, LiveConfig, OkxTradeModeConfig, TradingEnvironment,
};
use reap_venue::okx::{
    OkxAccountBalanceSnapshot, OkxAccountPositionsSnapshot,
    parse_okx_account_balance_response_json, parse_okx_account_positions_response_json,
};
use serde::{Deserialize, Serialize};

mod configuration;
mod execution;
mod reporting;
mod verification;

use configuration::{
    resolve, validate_candidate_funding_evidence, validate_candidate_initial_portfolios,
    validate_named, validate_scenario_currency_rates,
};
use execution::{find_dataset, run_sequence};
use reporting::{
    chronology_failures, cross_fold_chronology_failures, deployment_selection_failure,
    no_less_conservative, select_training_candidate, selection_score,
};
pub use verification::effective_strategy_sha256;
use verification::{
    load_candidates, opening_account_evidence_sha256, sha256_bytes, sha256_path,
    verify_input_hashes, verify_opening_account_certification_path,
};

#[cfg(test)]
use configuration::effective_scenario_execution;
#[cfg(test)]
use reap_live_contracts::MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES;

use crate::{
    BacktestConfig, BacktestExecutionConfig, BacktestInitialBalanceConfig,
    BacktestInitialMarginConfig, BacktestInitialPortfolioConfig, BacktestInitialPositionConfig,
    BacktestReport, LatencyCalibrationArtifact, MAX_LATENCY_CALIBRATION_ARTIFACT_BYTES,
    RawCaptureRecordRange,
};

pub const RESEARCH_SCHEMA_VERSION: u32 = 8;
const MAX_PRODUCTION_OPENING_ACCOUNT_GAP_MS: u64 = 15 * 60 * 1_000;
pub use reap_core::PINNED_JAVA_REVISION;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchMode {
    Smoke,
    ProductionCandidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchDataFormat {
    Csv,
    NormalizedJsonl,
    RawCapture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchScenarioKind {
    Baseline,
    Stress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionMetric {
    NetPnlUsd,
    PnlPerTurnoverBps,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatasetPortfolioSemantics {
    IndependentZeroInitialPortfolio,
    IndependentConfiguredInitialPortfolio,
    IndependentCertifiedDatasetPortfolio,
    SequentialSettledCarry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchManifest {
    pub schema_version: u32,
    pub mode: ResearchMode,
    pub java_reference_revision: String,
    #[serde(default)]
    pub latency_calibration: Option<PathBuf>,
    pub selection_metric: SelectionMetric,
    #[serde(default)]
    pub deployment_candidate_id: Option<String>,
    pub candidates: Vec<ResearchCandidate>,
    pub datasets: Vec<ResearchDataset>,
    pub scenarios: Vec<ResearchScenario>,
    pub folds: Vec<ResearchFold>,
    pub gates: ResearchGates,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchCandidate {
    pub id: String,
    pub config: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchDataset {
    pub id: String,
    pub path: PathBuf,
    pub format: ResearchDataFormat,
    #[serde(default)]
    pub capture_record_range: Option<RawCaptureRecordRange>,
    #[serde(default)]
    pub continuation_of: Option<String>,
    #[serde(default)]
    pub capture_config: Option<PathBuf>,
    #[serde(default)]
    pub capture_report: Option<PathBuf>,
    #[serde(default)]
    pub normalized_path: Option<PathBuf>,
    #[serde(default)]
    pub opening_account: Option<ResearchOpeningAccount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchOpeningAccount {
    pub certification: PathBuf,
    /// Currency to configured spot instrument used to value account inventory.
    #[serde(default)]
    pub spot_valuation_symbols: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchScenario {
    pub id: String,
    pub kind: ResearchScenarioKind,
    pub execution: BacktestExecutionConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchFold {
    pub id: String,
    pub train: Vec<String>,
    pub test: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchGates {
    pub minimum_folds: usize,
    pub minimum_stress_scenarios: usize,
    pub minimum_train_input_events_per_fold: u64,
    pub minimum_train_fills_per_fold: usize,
    pub minimum_train_funding_settlements_per_fold: u64,
    pub minimum_test_input_events_per_fold: u64,
    pub minimum_test_fills_per_fold: usize,
    pub minimum_test_funding_settlements_per_fold: u64,
    pub minimum_test_duration_ns_per_fold: u64,
    pub minimum_test_pnl_usd_per_fold: f64,
    pub minimum_total_baseline_test_pnl_usd: f64,
    pub maximum_test_drawdown_usd: f64,
    pub maximum_test_abs_delta_usd: f64,
    pub maximum_test_final_abs_delta_usd: f64,
    pub maximum_test_abs_pending_delta_usd: f64,
    pub maximum_test_final_abs_pending_delta_usd: f64,
    pub maximum_test_gross_exposure_usd: f64,
    pub maximum_test_final_gross_exposure_usd: f64,
    pub maximum_test_active_orders: usize,
    pub maximum_test_active_order_notional_usd: f64,
    pub maximum_test_final_active_order_notional_usd: f64,
    pub maximum_test_average_abs_delta_usd: f64,
    pub maximum_inventory_open_fraction: f64,
    pub maximum_pending_non_funding_actions_per_fold: usize,
    pub maximum_terminal_pending_orders_per_run: usize,
    pub maximum_terminal_pending_cancel_requests_per_run: usize,
    pub maximum_clock_regressions_per_run: u64,
    pub maximum_opening_account_gap_ms: u64,
    pub minimum_profitable_fold_fraction: f64,
    pub minimum_stress_pass_fraction: f64,
    pub minimum_passing_fold_fraction: f64,
    pub require_complete_accounting: bool,
    pub require_calibrated_execution: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchReport {
    pub schema_version: u32,
    pub mode: ResearchMode,
    pub selection_metric: SelectionMetric,
    #[serde(default)]
    pub deployment_candidate_id: Option<String>,
    pub gates: ResearchGates,
    pub manifest_sha256: String,
    pub executable_sha256: String,
    pub reap_version: String,
    pub java_reference_revision: String,
    pub latency_calibration: Option<LatencyCalibrationProvenance>,
    pub dataset_portfolio_semantics: DatasetPortfolioSemantics,
    pub candidates: Vec<CandidateProvenance>,
    pub datasets: Vec<DatasetProvenance>,
    pub scenarios: Vec<ResearchScenario>,
    pub folds: Vec<FoldReport>,
    pub aggregate: ResearchAggregate,
    pub passed: bool,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateProvenance {
    pub id: String,
    pub config: PathBuf,
    pub config_sha256: String,
    pub effective_strategy_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyCalibrationProvenance {
    pub path: PathBuf,
    pub sha256: String,
    pub schema_version: u32,
    pub reap_version: String,
    pub live_executable_sha256: String,
    pub host_identity_sha256: String,
    pub account_identity_sha256s: std::collections::BTreeMap<String, String>,
    pub live_config_sha256: String,
    pub live_config_fingerprint: String,
    pub live_config_evidence_fingerprint: String,
    pub minimum_samples_per_series: u64,
    pub matching_latency_is_upper_bound: bool,
    pub source_report_sha256s: Vec<String>,
    pub calibrated_series: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetProvenance {
    pub id: String,
    pub path: PathBuf,
    pub format: ResearchDataFormat,
    #[serde(default)]
    pub capture_record_range: Option<RawCaptureRecordRange>,
    #[serde(default)]
    pub continuation_of: Option<String>,
    pub data_sha256: String,
    pub raw_replay_check: Option<ReplayCheckReport>,
    pub capture_config: Option<PathBuf>,
    pub capture_config_sha256: Option<String>,
    pub capture_report: Option<PathBuf>,
    pub capture_report_sha256: Option<String>,
    pub normalized_path: Option<PathBuf>,
    pub normalized_sha256: Option<String>,
    pub capture_analysis: Option<CaptureAnalysisReport>,
    pub capture_verification: Option<CaptureVerificationReport>,
    pub opening_account: Option<OpeningAccountProvenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpeningAccountProvenance {
    pub source_path: PathBuf,
    pub sha256: String,
    pub evidence_sha256: String,
    pub schema_version: u32,
    pub reap_version: String,
    pub executable_sha256: String,
    pub host_identity_sha256: String,
    pub live_config_sha256: String,
    pub live_config_fingerprint: String,
    pub environment: TradingEnvironment,
    pub account_id: String,
    pub account_identity_sha256: String,
    pub certification_finish_local_midpoint_ms: u64,
    pub certification_finish_server_ms: u64,
    pub capture_started_at_ms: u64,
    pub capture_gap_ms: u64,
    pub spot_valuation_symbols: BTreeMap<String, String>,
    pub portfolio: BacktestInitialPortfolioConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoldReport {
    pub id: String,
    pub train_dataset_ids: Vec<String>,
    pub test_dataset_ids: Vec<String>,
    pub selected_candidate_id: Option<String>,
    pub selection_score: Option<f64>,
    pub training: Vec<CandidateTrainingReport>,
    pub test_scenarios: Vec<TestScenarioReport>,
    pub evidence_complete: bool,
    pub passed: bool,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateTrainingReport {
    pub candidate_id: String,
    pub runs: Vec<ResearchRunReport>,
    pub aggregate: RunAggregate,
    pub eligible: bool,
    pub selection_score: Option<f64>,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestScenarioReport {
    pub scenario_id: String,
    pub kind: ResearchScenarioKind,
    pub runs: Vec<ResearchRunReport>,
    pub aggregate: RunAggregate,
    pub evidence_complete: bool,
    pub passed: bool,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchRunReport {
    pub candidate_id: String,
    pub dataset_id: String,
    pub scenario_id: String,
    pub report: Option<BacktestReport>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunAggregate {
    pub runs: usize,
    pub successful_runs: usize,
    pub input_events: u64,
    pub observed_duration_ns: u64,
    pub fills: usize,
    pub net_pnl_usd: f64,
    pub fee_cost_usd: f64,
    #[serde(default)]
    pub exact_fee_fills: u64,
    #[serde(default)]
    pub estimated_fee_fills: u64,
    pub funding_pnl_usd: f64,
    pub funding_settlements: u64,
    pub turnover_usd: f64,
    pub maximum_drawdown_usd: f64,
    pub maximum_abs_delta_usd: f64,
    pub maximum_final_abs_delta_usd: f64,
    pub maximum_abs_pending_delta_usd: f64,
    pub maximum_final_abs_pending_delta_usd: f64,
    pub maximum_gross_exposure_usd: f64,
    pub maximum_final_gross_exposure_usd: f64,
    pub maximum_active_orders: usize,
    pub maximum_active_order_notional_usd: f64,
    pub maximum_final_active_order_notional_usd: f64,
    pub average_abs_delta_usd: f64,
    pub inventory_open_duration_ns: u64,
    pub inventory_open_fraction: f64,
    pub clock_regressions: u64,
    #[serde(default)]
    pub strategy_halts: usize,
    pub pending_non_funding_actions: usize,
    pub maximum_terminal_pending_orders: usize,
    pub maximum_terminal_pending_cancel_requests: usize,
    pub accounting_complete: bool,
    pub final_valuation_complete: bool,
    pub execution_calibrated: bool,
    pub first_arrival_ns: Option<u64>,
    pub last_arrival_ns: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResearchAggregate {
    pub folds: usize,
    pub evidence_complete_folds: usize,
    pub passing_folds: usize,
    pub profitable_baseline_folds: usize,
    pub stress_scenarios: usize,
    pub passing_stress_scenarios: usize,
    pub passing_fold_fraction: f64,
    pub profitable_fold_fraction: f64,
    pub stress_pass_fraction: f64,
    pub total_baseline_test_pnl_usd: f64,
}

#[derive(Debug, Clone)]
struct LoadedCandidate {
    spec: ResearchCandidate,
    resolved_path: PathBuf,
    config: BacktestConfig,
    sha256: String,
    effective_strategy_sha256: String,
}

#[derive(Debug, Clone)]
struct LoadedDataset {
    spec: ResearchDataset,
    resolved_path: PathBuf,
    sha256: String,
    raw_replay_check: Option<ReplayCheckReport>,
    resolved_capture_config: Option<PathBuf>,
    capture_config_sha256: Option<String>,
    resolved_capture_report: Option<PathBuf>,
    capture_report_sha256: Option<String>,
    resolved_normalized_path: Option<PathBuf>,
    normalized_sha256: Option<String>,
    capture_analysis: Option<CaptureAnalysisReport>,
    capture_verification: Option<CaptureVerificationReport>,
    resolved_opening_account: Option<PathBuf>,
    opening_account: Option<OpeningAccountProvenance>,
}

#[derive(Debug, Clone)]
struct LoadedLatencyCalibration {
    provenance: LatencyCalibrationProvenance,
    resolved_path: PathBuf,
}

pub fn run_research_manifest_path(path: impl AsRef<Path>) -> Result<ResearchReport> {
    let path = path.as_ref();
    let manifest_bytes = std::fs::read(path)
        .with_context(|| format!("failed to read research manifest {}", path.display()))?;
    let manifest: ResearchManifest = toml::from_str(
        std::str::from_utf8(&manifest_bytes).context("research manifest is not UTF-8")?,
    )
    .with_context(|| format!("failed to parse research manifest {}", path.display()))?;
    manifest.validate()?;

    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let executable_path =
        std::env::current_exe().context("failed to resolve current executable")?;
    let executable_sha256 = sha256_path(&executable_path)?;
    let baseline = manifest
        .scenarios
        .iter()
        .find(|scenario| scenario.kind == ResearchScenarioKind::Baseline)
        .expect("manifest validation requires one baseline scenario");
    let latency_calibration = load_latency_calibration(
        manifest.latency_calibration.as_deref(),
        base,
        manifest.mode,
        baseline,
        &executable_sha256,
    )?;
    let candidates = load_candidates(&manifest.candidates, base)?;
    let uses_certified_opening_accounts = manifest
        .datasets
        .iter()
        .any(|dataset| dataset.opening_account.is_some());
    validate_candidate_initial_portfolios(
        manifest.mode,
        uses_certified_opening_accounts,
        &candidates,
    )?;
    validate_candidate_funding_evidence(
        manifest.mode,
        &manifest.gates,
        candidates.iter().map(|candidate| &candidate.config),
    )?;
    validate_scenario_currency_rates(&manifest.scenarios, &candidates)?;
    let datasets = load_datasets(
        &manifest.datasets,
        base,
        manifest.mode,
        &candidates,
        &executable_sha256,
        latency_calibration
            .as_ref()
            .map(|calibration| calibration.provenance.host_identity_sha256.as_str()),
        manifest.gates.maximum_opening_account_gap_ms,
    )?;
    let manifest_sha256 = sha256_bytes(&manifest_bytes);
    let mut cache = HashMap::new();
    let mut folds = Vec::with_capacity(manifest.folds.len());

    for fold in &manifest.folds {
        let mut training = Vec::with_capacity(candidates.len());
        for candidate in &candidates {
            let runs = run_sequence(
                &mut cache,
                candidate,
                &datasets,
                &fold.train,
                baseline,
                None,
            );
            let aggregate = RunAggregate::from_runs(&runs);
            let mut failures = training_failures(&runs, &aggregate, baseline, &manifest.gates);
            let selection_score = if failures.is_empty() {
                selection_score(&aggregate, manifest.selection_metric)
            } else {
                None
            };
            if failures.is_empty() && selection_score.is_none() {
                failures.push(format!(
                    "selection metric {:?} is undefined",
                    manifest.selection_metric
                ));
            }
            training.push(CandidateTrainingReport {
                candidate_id: candidate.spec.id.clone(),
                runs,
                aggregate,
                eligible: failures.is_empty(),
                selection_score,
                failures,
            });
        }

        let selected = select_training_candidate(&training);
        let mut failures = Vec::new();
        let mut test_scenarios = Vec::new();
        let (selected_candidate_id, selected_score) = if let Some(selected) = selected {
            let selected_candidate_id = selected.candidate_id.clone();
            let selected_score = selected.selection_score;
            let candidate = candidates
                .iter()
                .find(|candidate| candidate.spec.id == selected_candidate_id)
                .expect("selected candidate must be loaded");
            let selected_train = training
                .iter()
                .find(|candidate| candidate.candidate_id == selected_candidate_id)
                .expect("selected training report must exist");
            let test_initial_carry = fold
                .test
                .first()
                .and_then(|dataset_id| {
                    find_dataset(&datasets, dataset_id)
                        .spec
                        .continuation_of
                        .as_deref()
                })
                .filter(|parent_id| fold.train.last().is_some_and(|last| last == parent_id))
                .and_then(|_| selected_train.runs.last())
                .and_then(|run| run.report.as_ref())
                .and_then(|report| {
                    report
                        .settled_carry_state
                        .clone()
                        .map(|carry| (carry, report.execution.clone()))
                });
            for scenario in &manifest.scenarios {
                let runs = run_sequence(
                    &mut cache,
                    candidate,
                    &datasets,
                    &fold.test,
                    scenario,
                    test_initial_carry.clone(),
                );
                let aggregate = RunAggregate::from_runs(&runs);
                let (evidence_failures, performance_failures) =
                    test_failures(&runs, &aggregate, scenario, &manifest.gates);
                let evidence_complete = evidence_failures.is_empty();
                let mut scenario_failures = evidence_failures;
                scenario_failures.extend(performance_failures);
                test_scenarios.push(TestScenarioReport {
                    scenario_id: scenario.id.clone(),
                    kind: scenario.kind,
                    runs,
                    aggregate,
                    evidence_complete,
                    passed: scenario_failures.is_empty(),
                    failures: scenario_failures,
                });
            }

            let baseline_test = test_scenarios
                .iter()
                .find(|scenario| scenario.kind == ResearchScenarioKind::Baseline)
                .expect("baseline test scenario must exist");
            failures.extend(chronology_failures(
                &selected_train.runs,
                &baseline_test.runs,
            ));
            (Some(selected_candidate_id), selected_score)
        } else {
            failures.push("no candidate passed the training evidence gates".to_string());
            (None, None)
        };
        if let Some(failure) = deployment_selection_failure(
            manifest.deployment_candidate_id.as_deref(),
            selected_candidate_id.as_deref(),
        ) {
            failures.push(failure);
        }

        let evidence_complete = failures.is_empty()
            && test_scenarios
                .iter()
                .all(|scenario| scenario.evidence_complete);
        let passed = evidence_complete && test_scenarios.iter().all(|scenario| scenario.passed);
        folds.push(FoldReport {
            id: fold.id.clone(),
            train_dataset_ids: fold.train.clone(),
            test_dataset_ids: fold.test.clone(),
            selected_candidate_id,
            selection_score: selected_score,
            training,
            test_scenarios,
            evidence_complete,
            passed,
            failures,
        });
    }

    let aggregate = ResearchAggregate::from_folds(&folds);
    let mut failures = overall_failures(&manifest, &folds, &aggregate);
    failures.extend(cross_fold_chronology_failures(&folds));
    verify_input_hashes(
        path,
        &manifest_sha256,
        &executable_path,
        &executable_sha256,
        &candidates,
        &datasets,
        latency_calibration.as_ref(),
    )?;
    let passed = failures.is_empty();

    Ok(ResearchReport {
        schema_version: RESEARCH_SCHEMA_VERSION,
        mode: manifest.mode,
        selection_metric: manifest.selection_metric,
        deployment_candidate_id: manifest.deployment_candidate_id.clone(),
        gates: manifest.gates.clone(),
        manifest_sha256,
        executable_sha256,
        reap_version: env!("CARGO_PKG_VERSION").to_string(),
        java_reference_revision: manifest.java_reference_revision,
        latency_calibration: latency_calibration.map(|loaded| loaded.provenance),
        dataset_portfolio_semantics: if datasets
            .iter()
            .any(|dataset| dataset.spec.continuation_of.is_some())
        {
            DatasetPortfolioSemantics::SequentialSettledCarry
        } else if datasets
            .iter()
            .any(|dataset| dataset.opening_account.is_some())
        {
            DatasetPortfolioSemantics::IndependentCertifiedDatasetPortfolio
        } else if candidates
            .first()
            .is_some_and(|candidate| !candidate.config.initial_portfolio.is_empty())
        {
            DatasetPortfolioSemantics::IndependentConfiguredInitialPortfolio
        } else {
            DatasetPortfolioSemantics::IndependentZeroInitialPortfolio
        },
        candidates: candidates
            .iter()
            .map(|candidate| CandidateProvenance {
                id: candidate.spec.id.clone(),
                config: candidate.spec.config.clone(),
                config_sha256: candidate.sha256.clone(),
                effective_strategy_sha256: candidate.effective_strategy_sha256.clone(),
            })
            .collect(),
        datasets: datasets
            .iter()
            .map(|dataset| DatasetProvenance {
                id: dataset.spec.id.clone(),
                path: dataset.spec.path.clone(),
                format: dataset.spec.format,
                capture_record_range: dataset.spec.capture_record_range,
                continuation_of: dataset.spec.continuation_of.clone(),
                data_sha256: dataset.sha256.clone(),
                raw_replay_check: dataset.raw_replay_check.clone(),
                capture_config: dataset.spec.capture_config.clone(),
                capture_config_sha256: dataset.capture_config_sha256.clone(),
                capture_report: dataset.spec.capture_report.clone(),
                capture_report_sha256: dataset.capture_report_sha256.clone(),
                normalized_path: dataset.spec.normalized_path.clone(),
                normalized_sha256: dataset.normalized_sha256.clone(),
                capture_analysis: dataset.capture_analysis.clone(),
                capture_verification: dataset.capture_verification.clone(),
                opening_account: dataset.opening_account.clone(),
            })
            .collect(),
        scenarios: manifest.scenarios,
        folds,
        aggregate,
        passed,
        failures,
    })
}

impl ResearchManifest {
    pub fn validate(&self) -> Result<()> {
        let mut errors = Vec::new();
        if self.schema_version != RESEARCH_SCHEMA_VERSION {
            errors.push(format!(
                "schema_version must be {RESEARCH_SCHEMA_VERSION}, got {}",
                self.schema_version
            ));
        }
        if self.java_reference_revision != PINNED_JAVA_REVISION {
            errors.push(format!(
                "java_reference_revision must remain pinned to {PINNED_JAVA_REVISION}"
            ));
        }
        validate_named(
            "candidate",
            self.candidates.iter().map(|item| &item.id),
            &mut errors,
        );
        validate_named(
            "dataset",
            self.datasets.iter().map(|item| &item.id),
            &mut errors,
        );
        validate_named(
            "scenario",
            self.scenarios.iter().map(|item| &item.id),
            &mut errors,
        );
        validate_named("fold", self.folds.iter().map(|item| &item.id), &mut errors);
        if self.candidates.is_empty() {
            errors.push("at least one candidate is required".to_string());
        }
        match (self.mode, self.deployment_candidate_id.as_deref()) {
            (ResearchMode::ProductionCandidate, None) => errors.push(
                "production_candidate requires a predeclared deployment_candidate_id".to_string(),
            ),
            (ResearchMode::ProductionCandidate, Some(candidate_id)) => {
                if !self
                    .candidates
                    .iter()
                    .any(|candidate| candidate.id == candidate_id)
                {
                    errors.push(format!(
                        "deployment_candidate_id {candidate_id:?} does not name a candidate"
                    ));
                }
            }
            (ResearchMode::Smoke, Some(_)) => {
                errors.push("smoke research cannot declare a deployment_candidate_id".to_string())
            }
            (ResearchMode::Smoke, None) => {}
        }
        if self.datasets.is_empty() {
            errors.push("at least one dataset is required".to_string());
        }
        if self.folds.is_empty() {
            errors.push("at least one fold is required".to_string());
        }

        let baselines = self
            .scenarios
            .iter()
            .filter(|scenario| scenario.kind == ResearchScenarioKind::Baseline)
            .collect::<Vec<_>>();
        if baselines.len() != 1 {
            errors.push(format!(
                "exactly one baseline scenario is required, found {}",
                baselines.len()
            ));
        }
        for scenario in &self.scenarios {
            if let Err(error) = scenario.execution.validate() {
                errors.push(format!("scenario {}: {error}", scenario.id));
            }
        }
        if let Some(baseline) = baselines.first() {
            if self.mode == ResearchMode::ProductionCandidate && !baseline.execution.calibrated {
                errors.push(
                    "production_candidate requires a calibrated baseline scenario".to_string(),
                );
            }
            for stress in self
                .scenarios
                .iter()
                .filter(|scenario| scenario.kind == ResearchScenarioKind::Stress)
            {
                if !no_less_conservative(&stress.execution, &baseline.execution) {
                    errors.push(format!(
                        "stress scenario {} is less conservative than baseline {}",
                        stress.id, baseline.id
                    ));
                }
            }
        }

        let dataset_ids = self
            .datasets
            .iter()
            .map(|dataset| dataset.id.as_str())
            .collect::<HashSet<_>>();
        let datasets_by_id = self
            .datasets
            .iter()
            .map(|dataset| (dataset.id.as_str(), dataset))
            .collect::<HashMap<_, _>>();
        let mut continuation_children = HashMap::new();
        let mut chain_roots = 0_usize;
        let mut certified_chain_roots = 0_usize;
        for dataset in &self.datasets {
            if let Some(range) = dataset.capture_record_range
                && let Err(error) = range.validate()
            {
                errors.push(format!("dataset {}: {error}", dataset.id));
            }
            if dataset.format != ResearchDataFormat::RawCapture
                && (dataset.capture_record_range.is_some() || dataset.continuation_of.is_some())
            {
                errors.push(format!(
                    "dataset {}: capture_record_range and continuation_of are valid only for raw_capture datasets",
                    dataset.id
                ));
            }
            let Some(parent_id) = dataset.continuation_of.as_deref() else {
                chain_roots += 1;
                certified_chain_roots += usize::from(dataset.opening_account.is_some());
                if self.mode == ResearchMode::ProductionCandidate
                    && dataset
                        .capture_record_range
                        .is_some_and(|range| range.first != 1)
                {
                    errors.push(format!(
                        "production chain root {} must begin at capture record 1; later ranges require continuation_of",
                        dataset.id
                    ));
                }
                if self.mode == ResearchMode::ProductionCandidate
                    && dataset.opening_account.is_none()
                {
                    errors.push(format!(
                        "production chain root {} requires opening_account evidence",
                        dataset.id
                    ));
                }
                continue;
            };
            if dataset.opening_account.is_some() {
                errors.push(format!(
                    "continuation dataset {} must derive opening state and cannot declare opening_account",
                    dataset.id
                ));
            }
            let Some(parent) = datasets_by_id.get(parent_id).copied() else {
                errors.push(format!(
                    "dataset {} continues unknown dataset {}",
                    dataset.id, parent_id
                ));
                continue;
            };
            let (Some(parent_range), Some(range)) =
                (parent.capture_record_range, dataset.capture_record_range)
            else {
                errors.push(format!(
                    "continuation {} and parent {} must both declare capture_record_range",
                    dataset.id, parent.id
                ));
                continue;
            };
            if parent_range.last.checked_add(1) != Some(range.first) {
                errors.push(format!(
                    "continuation {} must begin at capture record {} immediately after parent {}",
                    dataset.id,
                    parent_range.last.saturating_add(1),
                    parent.id
                ));
            }
            if let Some(existing) = continuation_children.insert(parent_id, dataset.id.as_str()) {
                errors.push(format!(
                    "dataset {} has multiple continuation children {} and {}",
                    parent_id, existing, dataset.id
                ));
            }
        }
        if certified_chain_roots != 0 && certified_chain_roots != chain_roots {
            errors.push(
                "research chain roots must either all provide opening_account evidence or all omit it"
                    .to_string(),
            );
        }
        let mut referenced = HashSet::new();
        let mut test_owners = HashMap::new();
        for fold in &self.folds {
            if fold.train.is_empty() || fold.test.is_empty() {
                errors.push(format!(
                    "fold {} requires non-empty train and test sets",
                    fold.id
                ));
            }
            let train = fold
                .train
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>();
            let test = fold.test.iter().map(String::as_str).collect::<HashSet<_>>();
            if train.len() != fold.train.len() {
                errors.push(format!("fold {} repeats a training dataset", fold.id));
            }
            if test.len() != fold.test.len() {
                errors.push(format!("fold {} repeats a test dataset", fold.id));
            }
            for dataset_id in fold.train.iter().chain(&fold.test) {
                referenced.insert(dataset_id.as_str());
                if !dataset_ids.contains(dataset_id.as_str()) {
                    errors.push(format!(
                        "fold {} references unknown dataset {}",
                        fold.id, dataset_id
                    ));
                }
            }
            for dataset_id in &fold.test {
                if train.contains(dataset_id.as_str()) {
                    errors.push(format!(
                        "fold {} uses dataset {} for both train and test",
                        fold.id, dataset_id
                    ));
                }
                if let Some(owner) = test_owners.insert(dataset_id.as_str(), fold.id.as_str()) {
                    errors.push(format!(
                        "dataset {} is a test set in both folds {} and {}",
                        dataset_id, owner, fold.id
                    ));
                }
            }
            for (label, sequence) in [("train", &fold.train), ("test", &fold.test)] {
                for (index, dataset_id) in sequence.iter().enumerate() {
                    let Some(dataset) = datasets_by_id.get(dataset_id.as_str()).copied() else {
                        continue;
                    };
                    let Some(parent_id) = dataset.continuation_of.as_deref() else {
                        continue;
                    };
                    let expected_parent = if index > 0 {
                        sequence.get(index - 1).map(String::as_str)
                    } else if label == "test" {
                        fold.train.last().map(String::as_str)
                    } else {
                        None
                    };
                    if expected_parent != Some(parent_id) {
                        errors.push(format!(
                            "fold {} {} dataset {} must immediately follow continuation parent {}",
                            fold.id, label, dataset.id, parent_id
                        ));
                    }
                }
            }
        }
        for unused in dataset_ids.difference(&referenced) {
            errors.push(format!("dataset {unused} is not referenced by any fold"));
        }
        self.gates.validate(self.mode, &mut errors);
        let stress_count = self
            .scenarios
            .iter()
            .filter(|scenario| scenario.kind == ResearchScenarioKind::Stress)
            .count();
        if self.folds.len() < self.gates.minimum_folds {
            errors.push(format!(
                "manifest has {} folds but gates require {}",
                self.folds.len(),
                self.gates.minimum_folds
            ));
        }
        if stress_count < self.gates.minimum_stress_scenarios {
            errors.push(format!(
                "manifest has {stress_count} stress scenarios but gates require {}",
                self.gates.minimum_stress_scenarios
            ));
        }
        if self.mode == ResearchMode::ProductionCandidate {
            if self.latency_calibration.is_none() {
                errors.push(
                    "production_candidate requires a latency_calibration artifact".to_string(),
                );
            }
            if self.candidates.len() < 2 {
                errors.push(
                    "production_candidate requires at least two candidate configs".to_string(),
                );
            }
            if self
                .datasets
                .iter()
                .any(|dataset| dataset.format != ResearchDataFormat::RawCapture)
            {
                errors.push("production_candidate accepts only raw_capture datasets".to_string());
            }
            if self
                .datasets
                .iter()
                .any(|dataset| dataset.capture_config.is_none())
            {
                errors.push(
                    "production_candidate requires a capture_config for every dataset".to_string(),
                );
            }
            if self
                .datasets
                .iter()
                .any(|dataset| dataset.capture_report.is_none())
            {
                errors.push(
                    "production_candidate requires a capture_report for every dataset".to_string(),
                );
            }
        }
        for dataset in &self.datasets {
            if dataset.format != ResearchDataFormat::RawCapture
                && (dataset.capture_config.is_some()
                    || dataset.capture_report.is_some()
                    || dataset.normalized_path.is_some()
                    || dataset.opening_account.is_some()
                    || dataset.capture_record_range.is_some()
                    || dataset.continuation_of.is_some())
            {
                errors.push(format!(
                    "dataset {}: capture evidence, opening_account, record ranges, and continuations are valid only for raw_capture datasets",
                    dataset.id
                ));
            }
            if dataset.capture_report.is_some() && dataset.capture_config.is_none() {
                errors.push(format!(
                    "dataset {}: capture_report requires capture_config",
                    dataset.id
                ));
            }
            if dataset.normalized_path.is_some() && dataset.capture_report.is_none() {
                errors.push(format!(
                    "dataset {}: normalized_path requires capture_report",
                    dataset.id
                ));
            }
            if dataset.opening_account.is_some() && dataset.capture_report.is_none() {
                errors.push(format!(
                    "dataset {}: opening_account requires capture_report",
                    dataset.id
                ));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            bail!("invalid research manifest: {}", errors.join("; "))
        }
    }
}

impl ResearchGates {
    fn validate(&self, mode: ResearchMode, errors: &mut Vec<String>) {
        for (name, value) in [
            (
                "minimum_test_pnl_usd_per_fold",
                self.minimum_test_pnl_usd_per_fold,
            ),
            (
                "minimum_total_baseline_test_pnl_usd",
                self.minimum_total_baseline_test_pnl_usd,
            ),
        ] {
            if !value.is_finite() {
                errors.push(format!("gates.{name} must be finite"));
            }
        }
        for (name, value) in [
            ("maximum_test_drawdown_usd", self.maximum_test_drawdown_usd),
            (
                "maximum_test_abs_delta_usd",
                self.maximum_test_abs_delta_usd,
            ),
            (
                "maximum_test_final_abs_delta_usd",
                self.maximum_test_final_abs_delta_usd,
            ),
            (
                "maximum_test_abs_pending_delta_usd",
                self.maximum_test_abs_pending_delta_usd,
            ),
            (
                "maximum_test_final_abs_pending_delta_usd",
                self.maximum_test_final_abs_pending_delta_usd,
            ),
            (
                "maximum_test_gross_exposure_usd",
                self.maximum_test_gross_exposure_usd,
            ),
            (
                "maximum_test_final_gross_exposure_usd",
                self.maximum_test_final_gross_exposure_usd,
            ),
            (
                "maximum_test_active_order_notional_usd",
                self.maximum_test_active_order_notional_usd,
            ),
            (
                "maximum_test_final_active_order_notional_usd",
                self.maximum_test_final_active_order_notional_usd,
            ),
            (
                "maximum_test_average_abs_delta_usd",
                self.maximum_test_average_abs_delta_usd,
            ),
        ] {
            if !value.is_finite() || value < 0.0 {
                errors.push(format!("gates.{name} must be finite and non-negative"));
            }
        }
        for (name, value) in [
            (
                "maximum_inventory_open_fraction",
                self.maximum_inventory_open_fraction,
            ),
            (
                "minimum_profitable_fold_fraction",
                self.minimum_profitable_fold_fraction,
            ),
            (
                "minimum_stress_pass_fraction",
                self.minimum_stress_pass_fraction,
            ),
            (
                "minimum_passing_fold_fraction",
                self.minimum_passing_fold_fraction,
            ),
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                errors.push(format!("gates.{name} must be finite and within [0, 1]"));
            }
        }
        if self.minimum_folds == 0 {
            errors.push("gates.minimum_folds must be positive".to_string());
        }
        if self.maximum_opening_account_gap_ms == 0 {
            errors.push("gates.maximum_opening_account_gap_ms must be positive".to_string());
        }
        if mode == ResearchMode::ProductionCandidate {
            if self.maximum_opening_account_gap_ms > MAX_PRODUCTION_OPENING_ACCOUNT_GAP_MS {
                errors.push(format!(
                    "production_candidate maximum_opening_account_gap_ms must not exceed {MAX_PRODUCTION_OPENING_ACCOUNT_GAP_MS}"
                ));
            }
            if self.minimum_folds < 3 {
                errors.push("production_candidate requires at least three folds".to_string());
            }
            if self.minimum_stress_scenarios < 2 {
                errors.push(
                    "production_candidate requires at least two stress scenarios".to_string(),
                );
            }
            if self.minimum_train_input_events_per_fold == 0
                || self.minimum_test_input_events_per_fold == 0
                || self.minimum_train_fills_per_fold == 0
                || self.minimum_test_fills_per_fold == 0
                || self.minimum_test_duration_ns_per_fold == 0
            {
                errors.push(
                    "production_candidate requires non-zero event, fill, and duration evidence gates"
                        .to_string(),
                );
            }
            if !self.require_complete_accounting || !self.require_calibrated_execution {
                errors.push(
                    "production_candidate requires complete accounting and calibrated execution"
                        .to_string(),
                );
            }
            if self.minimum_total_baseline_test_pnl_usd <= 0.0
                || self.minimum_profitable_fold_fraction <= 0.0
                || self.minimum_stress_pass_fraction <= 0.0
                || self.minimum_passing_fold_fraction <= 0.0
            {
                errors.push(
                    "production_candidate requires positive total PnL and non-zero profitable, stress-pass, and passing-fold fractions"
                        .to_string(),
                );
            }
        }
    }
}

fn load_latency_calibration(
    spec: Option<&Path>,
    base: &Path,
    mode: ResearchMode,
    baseline: &ResearchScenario,
    executable_sha256: &str,
) -> Result<Option<LoadedLatencyCalibration>> {
    let Some(spec) = spec else {
        return Ok(None);
    };
    let resolved = resolve(base, spec);
    let canonical = resolved.canonicalize().with_context(|| {
        format!(
            "failed to resolve latency calibration {}",
            resolved.display()
        )
    })?;
    let artifact_size = std::fs::metadata(&canonical)
        .with_context(|| {
            format!(
                "failed to inspect latency calibration {}",
                canonical.display()
            )
        })?
        .len();
    if artifact_size > MAX_LATENCY_CALIBRATION_ARTIFACT_BYTES {
        bail!(
            "latency calibration is {artifact_size} bytes, maximum is {MAX_LATENCY_CALIBRATION_ARTIFACT_BYTES}"
        );
    }
    let bytes = std::fs::read(&canonical)
        .with_context(|| format!("failed to read latency calibration {}", canonical.display()))?;
    let sha256 = sha256_bytes(&bytes);
    let artifact: LatencyCalibrationArtifact =
        serde_json::from_slice(&bytes).with_context(|| {
            format!(
                "failed to parse latency calibration {}",
                canonical.display()
            )
        })?;
    artifact.validate_integrity().with_context(|| {
        format!(
            "latency calibration {} failed integrity validation",
            canonical.display()
        )
    })?;
    if artifact.reap_version != env!("CARGO_PKG_VERSION")
        || artifact.live_executable_sha256 != executable_sha256
    {
        bail!(
            "latency calibration was collected by a different Reap build than this research executable"
        );
    }
    if artifact.profile != baseline.execution.latency_profile {
        bail!("baseline latency profile does not exactly match the bound calibration artifact");
    }
    if mode == ResearchMode::ProductionCandidate && !baseline.execution.calibrated {
        bail!("production latency calibration requires a calibrated baseline execution");
    }
    let mut source_report_sha256s = artifact
        .source_reports
        .iter()
        .map(|source| source.sha256.clone())
        .collect::<Vec<_>>();
    source_report_sha256s.sort();
    source_report_sha256s.dedup();
    Ok(Some(LoadedLatencyCalibration {
        provenance: LatencyCalibrationProvenance {
            path: spec.to_path_buf(),
            sha256,
            schema_version: artifact.schema_version,
            reap_version: artifact.reap_version,
            live_executable_sha256: artifact.live_executable_sha256,
            host_identity_sha256: artifact.host_identity_sha256,
            account_identity_sha256s: artifact.account_identity_sha256s,
            live_config_sha256: artifact.live_config_sha256,
            live_config_fingerprint: artifact.live_config_fingerprint,
            live_config_evidence_fingerprint: artifact.live_config_evidence_fingerprint,
            minimum_samples_per_series: artifact.minimum_samples_per_series,
            matching_latency_is_upper_bound: artifact.matching_latency_is_upper_bound,
            source_report_sha256s,
            calibrated_series: artifact.series.len(),
        },
        resolved_path: canonical,
    }))
}

fn load_datasets(
    specs: &[ResearchDataset],
    base: &Path,
    mode: ResearchMode,
    candidates: &[LoadedCandidate],
    expected_executable_sha256: &str,
    expected_host_identity_sha256: Option<&str>,
    maximum_opening_account_gap_ms: u64,
) -> Result<Vec<LoadedDataset>> {
    let mut loaded = Vec::with_capacity(specs.len());
    let mut canonical_ranges = HashMap::<PathBuf, Vec<(String, RawCaptureRecordRange)>>::new();
    let mut whole_canonical_paths = HashSet::new();
    let mut hashes = HashMap::<String, PathBuf>::new();
    let mut opening_account_paths = HashSet::new();
    let mut opening_account_hashes = HashSet::new();
    let mut opening_account_evidence_hashes = HashSet::new();
    for spec in specs {
        let resolved = resolve(base, &spec.path);
        let canonical = resolved
            .canonicalize()
            .with_context(|| format!("failed to resolve dataset {}", resolved.display()))?;
        if let Some(existing) = canonical_ranges.get_mut(&canonical) {
            if whole_canonical_paths.contains(&canonical) {
                bail!(
                    "dataset path {} is referenced both as a whole file and a record range",
                    spec.path.display()
                );
            }
            let range = spec.capture_record_range.with_context(|| {
                format!(
                    "dataset path {} is referenced more than once without a capture_record_range",
                    spec.path.display()
                )
            })?;
            if let Some((other_id, other_range)) = existing
                .iter()
                .find(|(_, other)| range.first <= other.last && other.first <= range.last)
            {
                bail!(
                    "dataset {} range {}..={} overlaps dataset {} range {}..={} in one raw capture",
                    spec.id,
                    range.first,
                    range.last,
                    other_id,
                    other_range.first,
                    other_range.last
                );
            }
            existing.push((spec.id.clone(), range));
        } else {
            if spec.capture_record_range.is_none() {
                whole_canonical_paths.insert(canonical.clone());
            }
            let ranges = spec
                .capture_record_range
                .map(|range| vec![(spec.id.clone(), range)])
                .unwrap_or_default();
            canonical_ranges.insert(canonical.clone(), ranges);
        }
        let sha256 = sha256_path(&canonical)?;
        if let Some(existing_path) = hashes.get(&sha256) {
            if existing_path != &canonical {
                bail!("dataset {} duplicates another dataset's bytes", spec.id);
            }
        } else {
            hashes.insert(sha256.clone(), canonical.clone());
        }
        let raw_replay_check = if spec.format == ResearchDataFormat::RawCapture {
            let report = replay_check_path(&canonical)
                .with_context(|| format!("failed to check raw dataset {}", canonical.display()))?;
            if mode == ResearchMode::ProductionCandidate
                && (!report.is_healthy() || report.gaps > 0 || report.recoveries > 0)
            {
                bail!(
                    "production dataset {} failed zero-gap replay integrity: errors={}, gaps={}, recoveries={}, recovery_failures={}, unrecovered_streams={}",
                    spec.id,
                    report.errors.len(),
                    report.gaps,
                    report.recoveries,
                    report.recovery_failures,
                    report.unrecovered_streams
                );
            }
            Some(report)
        } else {
            None
        };
        if spec.capture_report.is_some() && spec.capture_config.is_none() {
            bail!("dataset {} capture_report requires capture_config", spec.id);
        }
        if spec.normalized_path.is_some() && spec.capture_report.is_none() {
            bail!(
                "dataset {} normalized_path requires capture_report",
                spec.id
            );
        }
        if mode == ResearchMode::ProductionCandidate
            && (spec.capture_config.is_none() || spec.capture_report.is_none())
        {
            bail!(
                "production dataset {} requires capture_config and capture_report evidence",
                spec.id
            );
        }
        let resolved_capture_report = spec
            .capture_report
            .as_ref()
            .map(|report_path| -> Result<PathBuf> {
                let resolved = resolve(base, report_path);
                resolved.canonicalize().with_context(|| {
                    format!("failed to resolve capture report {}", resolved.display())
                })
            })
            .transpose()?;
        let resolved_normalized_path = spec
            .normalized_path
            .as_ref()
            .map(|normalized_path| -> Result<PathBuf> {
                let resolved = resolve(base, normalized_path);
                resolved.canonicalize().with_context(|| {
                    format!(
                        "failed to resolve normalized capture {}",
                        resolved.display()
                    )
                })
            })
            .transpose()?;

        let mut resolved_capture_config = None;
        let mut capture_config_sha256 = None;
        let mut capture_report_sha256 = None;
        let mut normalized_sha256 = None;
        let mut capture_analysis = None;
        let mut capture_verification = None;
        if let Some(config_path) = &spec.capture_config {
            let resolved_config = resolve(base, config_path);
            let canonical_config = resolved_config.canonicalize().with_context(|| {
                format!(
                    "failed to resolve capture config {}",
                    resolved_config.display()
                )
            })?;
            let config_bytes = std::fs::read(&canonical_config).with_context(|| {
                format!(
                    "failed to read capture config {}",
                    canonical_config.display()
                )
            })?;
            let config_sha256 = sha256_bytes(&config_bytes);
            let config = CaptureConfig::from_toml(
                std::str::from_utf8(&config_bytes).context("capture config is not UTF-8")?,
            )
            .with_context(|| {
                format!(
                    "failed to parse capture config {}",
                    canonical_config.display()
                )
            })?;
            if mode == ResearchMode::ProductionCandidate {
                validate_production_capture_config(&spec.id, &config, candidates)?;
            }

            let analysis = if let Some(report_path) = &resolved_capture_report {
                let verification = verify_capture_paths(
                    &canonical_config,
                    report_path,
                    &canonical,
                    resolved_normalized_path.as_deref(),
                )
                .with_context(|| {
                    format!("failed to verify capture evidence for dataset {}", spec.id)
                })?;
                if verification.config.sha256 != config_sha256 {
                    bail!(
                        "capture config for dataset {} changed while evidence was being loaded",
                        spec.id
                    );
                }
                if !verification.passed {
                    bail!(
                        "dataset {} failed capture verification: {:?}",
                        spec.id,
                        verification.failures
                    );
                }
                if mode == ResearchMode::ProductionCandidate {
                    if verification.reap_version != env!("CARGO_PKG_VERSION")
                        || verification.java_reference_revision != PINNED_JAVA_REVISION
                        || verification.executable_sha256 != expected_executable_sha256
                    {
                        bail!(
                            "production dataset {} was captured by a different Reap build or Java reference than this research run",
                            spec.id
                        );
                    }
                    let expected_host_identity_sha256 = expected_host_identity_sha256.context(
                        "production capture evidence requires a latency-calibrated target host",
                    )?;
                    if verification.host_identity_sha256.as_deref()
                        != Some(expected_host_identity_sha256)
                    {
                        bail!(
                            "production dataset {} was captured on a different host than the latency calibration",
                            spec.id
                        );
                    }
                    if verification.host_periodic_checks == 0 {
                        bail!(
                            "production dataset {} has no completed periodic host check",
                            spec.id
                        );
                    }
                }
                capture_report_sha256 = Some(verification.run_report.sha256.clone());
                normalized_sha256 = verification
                    .normalized
                    .as_ref()
                    .map(|artifact| artifact.actual_sha256.clone());
                let analysis = verification.analysis.clone();
                capture_verification = Some(verification);
                analysis
            } else {
                analyze_capture_path(&canonical, &config)
                    .with_context(|| format!("failed to analyze research dataset {}", spec.id))?
            };
            if !analysis.integrity_healthy {
                bail!(
                    "dataset {} failed capture-analysis integrity: errors={}, gaps={}, recovery_failures={}, receive_timestamp_regressions={}, unrecovered_books={}",
                    spec.id,
                    analysis.error_count,
                    analysis.gaps,
                    analysis.recovery_failures,
                    analysis.receive_timestamp_regressions,
                    analysis.unrecovered_book_streams
                );
            }
            if analysis.sha256 != sha256 {
                bail!(
                    "dataset {} analysis hash does not match input hash",
                    spec.id
                );
            }
            resolved_capture_config = Some(canonical_config);
            capture_config_sha256 = Some(config_sha256);
            capture_analysis = Some(analysis);
        }
        let (resolved_opening_account, opening_account) = load_dataset_opening_account(
            spec,
            base,
            mode,
            candidates,
            expected_executable_sha256,
            expected_host_identity_sha256,
            maximum_opening_account_gap_ms,
            capture_verification.as_ref(),
        )?;
        if let (Some(path), Some(provenance)) = (&resolved_opening_account, &opening_account) {
            if !opening_account_paths.insert(path.clone()) {
                bail!(
                    "opening account certification {} is referenced by more than one dataset",
                    provenance.source_path.display()
                );
            }
            if !opening_account_hashes.insert(provenance.sha256.clone()) {
                bail!(
                    "dataset {} opening account certification duplicates another dataset's bytes",
                    spec.id
                );
            }
            if !opening_account_evidence_hashes.insert(provenance.evidence_sha256.clone()) {
                bail!(
                    "dataset {} reuses another dataset's opening account evidence",
                    spec.id
                );
            }
        }
        loaded.push(LoadedDataset {
            spec: spec.clone(),
            resolved_path: canonical,
            sha256,
            raw_replay_check,
            resolved_capture_config,
            capture_config_sha256,
            resolved_capture_report,
            capture_report_sha256,
            resolved_normalized_path,
            normalized_sha256,
            capture_analysis,
            capture_verification,
            resolved_opening_account,
            opening_account,
        });
    }
    let loaded_by_id = loaded
        .iter()
        .map(|dataset| (dataset.spec.id.as_str(), dataset))
        .collect::<HashMap<_, _>>();
    for dataset in &loaded {
        let Some(parent_id) = dataset.spec.continuation_of.as_deref() else {
            continue;
        };
        let parent = loaded_by_id.get(parent_id).copied().with_context(|| {
            format!(
                "dataset {} has no loaded parent {parent_id}",
                dataset.spec.id
            )
        })?;
        if dataset.resolved_path != parent.resolved_path
            || dataset.resolved_capture_config != parent.resolved_capture_config
            || dataset.resolved_capture_report != parent.resolved_capture_report
            || dataset.resolved_normalized_path != parent.resolved_normalized_path
        {
            bail!(
                "continuation dataset {} must use the exact raw/config/report/normalized sources of parent {}",
                dataset.spec.id,
                parent.spec.id
            );
        }
    }
    Ok(loaded)
}

#[allow(clippy::too_many_arguments)]
fn load_dataset_opening_account(
    dataset: &ResearchDataset,
    base: &Path,
    mode: ResearchMode,
    candidates: &[LoadedCandidate],
    expected_executable_sha256: &str,
    expected_host_identity_sha256: Option<&str>,
    maximum_gap_ms: u64,
    capture_verification: Option<&CaptureVerificationReport>,
) -> Result<(Option<PathBuf>, Option<OpeningAccountProvenance>)> {
    let Some(spec) = &dataset.opening_account else {
        return Ok((None, None));
    };
    let resolved = resolve(base, &spec.certification);
    let canonical = resolved.canonicalize().with_context(|| {
        format!(
            "failed to resolve opening account certification {}",
            resolved.display()
        )
    })?;
    let sha256 = sha256_path(&canonical)?;
    let artifact = verify_opening_account_certification_path(&canonical).with_context(|| {
        format!(
            "failed to reconstruct opening account certification for dataset {}",
            dataset.id
        )
    })?;
    let evidence_sha256 = opening_account_evidence_sha256(&artifact)?;
    if !artifact.summary.passed || !artifact.summary.evidence_complete {
        bail!(
            "dataset {} opening account certification did not pass complete cash-account policy",
            dataset.id
        );
    }
    let capture = capture_verification.context(format!(
        "dataset {} opening account requires verified capture timing",
        dataset.id
    ))?;
    if artifact.reap_version != capture.reap_version
        || artifact.java_reference_revision != capture.java_reference_revision
        || artifact.executable_sha256 != capture.executable_sha256
    {
        bail!(
            "dataset {} opening account and capture were produced by different Reap builds or Java references",
            dataset.id
        );
    }
    if capture.host_identity_sha256.as_deref() != Some(artifact.host_identity_sha256.as_str()) {
        bail!(
            "dataset {} opening account and capture do not identify one host",
            dataset.id
        );
    }
    if artifact.finish_clock.local_midpoint_ms > capture.session_started_at_ms {
        bail!(
            "dataset {} opening account certification finished at {}, after capture started at {}",
            dataset.id,
            artifact.finish_clock.local_midpoint_ms,
            capture.session_started_at_ms
        );
    }
    let capture_gap_ms = capture
        .session_started_at_ms
        .saturating_sub(artifact.finish_clock.local_midpoint_ms);
    if capture_gap_ms > maximum_gap_ms {
        bail!(
            "dataset {} opening account gap {} ms exceeds configured maximum {} ms",
            dataset.id,
            capture_gap_ms,
            maximum_gap_ms
        );
    }

    let live_config = LiveConfig::from_toml(&artifact.config.toml).with_context(|| {
        format!(
            "dataset {} opening account embeds an invalid live config",
            dataset.id
        )
    })?;
    if mode == ResearchMode::ProductionCandidate {
        let expected_host = expected_host_identity_sha256
            .context("production opening account requires a latency-calibrated target host")?;
        if artifact.schema_version != ACCOUNT_CERTIFICATION_SCHEMA_VERSION
            || artifact.java_reference_revision != PINNED_JAVA_REVISION
            || artifact.reap_version != env!("CARGO_PKG_VERSION")
            || artifact.executable_sha256 != expected_executable_sha256
        {
            bail!(
                "production dataset {} opening account was certified by a different Reap build or Java reference",
                dataset.id
            );
        }
        if artifact.host_identity_sha256 != expected_host {
            bail!(
                "production dataset {} opening account, capture, and latency calibration do not identify one host",
                dataset.id
            );
        }
        if artifact.summary.environment != TradingEnvironment::Production {
            bail!(
                "production dataset {} opening account is not from the production environment",
                dataset.id
            );
        }
    }

    let balance = parse_okx_account_balance_response_json(artifact.account_balance.body.as_bytes())
        .with_context(|| {
            format!(
                "failed to parse verified opening balances for dataset {}",
                dataset.id
            )
        })?;
    let positions =
        parse_okx_account_positions_response_json(artifact.account_positions.body.as_bytes())
            .with_context(|| {
                format!(
                    "failed to parse verified opening positions for dataset {}",
                    dataset.id
                )
            })?;
    let mut portfolio = None;
    for candidate in candidates {
        let derived = derive_certified_opening_portfolio(
            dataset,
            spec,
            candidate,
            &live_config,
            &artifact.summary.account_id,
            &balance,
            &positions,
        )?;
        if let Some(expected) = &portfolio {
            if expected != &derived {
                bail!(
                    "dataset {} certified opening portfolio differs for candidate {}; candidates must share one account and instrument universe",
                    dataset.id,
                    candidate.spec.id
                );
            }
        } else {
            portfolio = Some(derived);
        }
    }
    let portfolio = portfolio.context("research requires at least one candidate")?;
    if mode == ResearchMode::ProductionCandidate && !portfolio.has_positive_balance() {
        bail!(
            "production dataset {} certified opening account has no positive modeled balance",
            dataset.id
        );
    }
    Ok((
        Some(canonical),
        Some(OpeningAccountProvenance {
            source_path: spec.certification.clone(),
            sha256,
            evidence_sha256,
            schema_version: artifact.schema_version,
            reap_version: artifact.reap_version,
            executable_sha256: artifact.executable_sha256,
            host_identity_sha256: artifact.host_identity_sha256,
            live_config_sha256: artifact.config.sha256,
            live_config_fingerprint: artifact.config_fingerprint,
            environment: artifact.summary.environment,
            account_id: artifact.summary.account_id,
            account_identity_sha256: artifact.summary.account_identity_sha256,
            certification_finish_local_midpoint_ms: artifact.finish_clock.local_midpoint_ms,
            certification_finish_server_ms: artifact.finish_clock.server_ms,
            capture_started_at_ms: capture.session_started_at_ms,
            capture_gap_ms,
            spot_valuation_symbols: spec.spot_valuation_symbols.clone(),
            portfolio,
        }),
    ))
}

fn derive_certified_opening_portfolio(
    dataset: &ResearchDataset,
    opening: &ResearchOpeningAccount,
    candidate: &LoadedCandidate,
    live_config: &LiveConfig,
    account_id: &str,
    balance: &OkxAccountBalanceSnapshot,
    positions: &OkxAccountPositionsSnapshot,
) -> Result<BacktestInitialPortfolioConfig> {
    let candidate_account_ids = candidate
        .config
        .strategy
        .risk_groups
        .iter()
        .map(|group| group.account_id.as_deref())
        .collect::<BTreeSet<_>>();
    if candidate_account_ids != BTreeSet::from([Some(account_id)]) {
        bail!(
            "dataset {} candidate {} must bind every risk group to certified account {:?}",
            dataset.id,
            candidate.spec.id,
            account_id
        );
    }
    if let Some(group) = candidate.config.strategy.risk_groups.iter().find(|group| {
        group
            .coins
            .iter()
            .any(|coin| coin.borrow_limit_usd != 0.0 || coin.borrow_limit_coin != 0.0)
    }) {
        bail!(
            "dataset {} candidate {} risk group {} enables borrowing, which certified opening accounting does not model",
            dataset.id,
            candidate.spec.id,
            group.name
        );
    }
    validate_certified_instrument_scope(dataset, candidate, live_config, account_id)?;

    let mut required_currencies = BTreeSet::new();
    let mut spot_base_currencies = BTreeSet::new();
    for instrument in &candidate.config.strategy.instruments {
        if instrument.kind.is_spot() {
            required_currencies.insert(instrument.base_currency.clone());
            required_currencies.insert(instrument.quote_currency.clone());
            spot_base_currencies.insert(instrument.base_currency.clone());
        } else {
            required_currencies.insert(instrument.settle_currency.clone());
        }
    }
    let mapped_currencies = opening
        .spot_valuation_symbols
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if mapped_currencies != spot_base_currencies {
        bail!(
            "dataset {} opening spot valuation currencies {:?} do not exactly match candidate {} spot bases {:?}",
            dataset.id,
            mapped_currencies,
            candidate.spec.id,
            spot_base_currencies
        );
    }

    let mut details = BTreeMap::new();
    for detail in &balance.details {
        if details.insert(detail.currency.as_str(), detail).is_some() {
            bail!(
                "dataset {} opening account repeats balance currency {}",
                dataset.id,
                detail.currency
            );
        }
        if detail.forced_repayment_indicator.unwrap_or(0) != 0 {
            bail!(
                "dataset {} opening account currency {} has an active forced repayment indicator",
                dataset.id,
                detail.currency
            );
        }
        let has_unmodeled_value = [
            detail.cash_balance,
            detail.available_balance,
            detail.equity,
            detail.equity_usd,
            detail.discounted_equity_usd,
            detail.unrealized_pnl,
            detail.liability,
            detail.cross_liability,
            detail.isolated_liability,
            detail.unrealized_loss_liability,
            detail.accrued_interest,
            detail.borrow_frozen_usd,
        ]
        .into_iter()
        .flatten()
        .any(|value| value != 0.0);
        if !required_currencies.contains(&detail.currency) && has_unmodeled_value {
            bail!(
                "dataset {} opening account has nonzero unmodeled balance or equity in currency {}",
                dataset.id,
                detail.currency
            );
        }
    }

    let mut initial_balances = Vec::with_capacity(required_currencies.len());
    for currency in required_currencies {
        let detail = details.get(currency.as_str()).copied();
        let total = detail.and_then(|item| item.cash_balance).unwrap_or(0.0);
        let available = detail
            .map(|item| {
                item.available_balance.with_context(|| {
                    format!(
                        "dataset {} opening balance {} omits availBal",
                        dataset.id, currency
                    )
                })
            })
            .transpose()?
            .unwrap_or(0.0);
        let equity = detail
            .map(|item| {
                item.equity.with_context(|| {
                    format!(
                        "dataset {} opening balance {} omits eq",
                        dataset.id, currency
                    )
                })
            })
            .transpose()?
            .unwrap_or(0.0);
        initial_balances.push(BacktestInitialBalanceConfig {
            currency: currency.clone(),
            total,
            available: Some(available),
            equity: Some(equity),
            liability: Some(detail.and_then(|item| item.liability).unwrap_or(0.0)),
            max_loan: Some(detail.and_then(|item| item.max_loan).unwrap_or(0.0)),
            forced_repayment_indicator: detail.and_then(|item| item.forced_repayment_indicator),
            valuation_symbol: opening.spot_valuation_symbols.get(&currency).cloned(),
        });
    }

    let instruments = candidate
        .config
        .strategy
        .instruments
        .iter()
        .map(|instrument| (instrument.symbol.as_str(), instrument))
        .collect::<HashMap<_, _>>();
    let mut certified_positions = HashMap::new();
    for risk in &positions.positions {
        let position = &risk.position;
        if certified_positions
            .insert(position.symbol.as_str(), position)
            .is_some()
        {
            bail!(
                "dataset {} opening account repeats position {}",
                dataset.id,
                position.symbol
            );
        }
        if position.qty == 0.0 {
            continue;
        }
        let instrument = instruments.get(position.symbol.as_str()).with_context(|| {
            format!(
                "dataset {} opening account has nonzero unmodeled position {}",
                dataset.id, position.symbol
            )
        })?;
        if instrument.kind.is_spot() {
            bail!(
                "dataset {} opening account reported spot position {} instead of cash balance",
                dataset.id,
                position.symbol
            );
        }
    }
    let live_account = live_config
        .accounts
        .iter()
        .find(|account| account.id == account_id)
        .with_context(|| {
            format!(
                "dataset {} certified live config omits account {}",
                dataset.id, account_id
            )
        })?;
    let mut initial_positions = Vec::new();
    for instrument in candidate
        .config
        .strategy
        .instruments
        .iter()
        .filter(|instrument| instrument.kind.is_derivative())
    {
        let expected_margin_mode = match live_account.trade_modes.get(&instrument.symbol) {
            Some(OkxTradeModeConfig::Cross) => PositionMarginMode::Cross,
            Some(OkxTradeModeConfig::Isolated) => PositionMarginMode::Isolated,
            Some(OkxTradeModeConfig::Cash) | None => {
                bail!(
                    "dataset {} derivative {} has no supported configured margin mode",
                    dataset.id,
                    instrument.symbol
                )
            }
        };
        let certified = certified_positions.get(instrument.symbol.as_str()).copied();
        if certified.is_some_and(|position| {
            position.qty != 0.0 && position.margin_mode != Some(expected_margin_mode)
        }) {
            bail!(
                "dataset {} opening position {} margin mode differs from certified live config",
                dataset.id,
                instrument.symbol
            );
        }
        initial_positions.push(BacktestInitialPositionConfig {
            symbol: instrument.symbol.clone(),
            qty: certified.map_or(0.0, |position| position.qty),
            avg_price: certified
                .filter(|position| position.qty != 0.0)
                .map_or(0.0, |position| position.avg_price),
            margin_mode: Some(expected_margin_mode),
        });
    }
    initial_positions.sort_by(|left, right| left.symbol.cmp(&right.symbol));
    let initial = BacktestInitialPortfolioConfig {
        account_id: Some(account_id.to_string()),
        balances: initial_balances,
        positions: initial_positions,
        margin: BacktestInitialMarginConfig {
            ratio: None,
            exchange_ratio: balance.margin_ratio,
            adjusted_equity_usd: balance.adjusted_equity_usd,
            notional_usd: balance.notional_usd,
        },
    };
    initial
        .validate(
            &candidate.config.strategy.effective(),
            &candidate.config.backtest,
        )
        .with_context(|| {
            format!(
                "dataset {} certified opening state is incompatible with candidate {}",
                dataset.id, candidate.spec.id
            )
        })?;
    Ok(initial)
}

fn validate_certified_instrument_scope(
    dataset: &ResearchDataset,
    candidate: &LoadedCandidate,
    live_config: &LiveConfig,
    account_id: &str,
) -> Result<()> {
    let source_instruments = live_config
        .strategy
        .instruments
        .iter()
        .map(|instrument| (instrument.symbol.as_str(), instrument))
        .collect::<HashMap<_, _>>();
    let source_groups = live_config
        .strategy
        .risk_groups
        .iter()
        .map(|group| (group.name.as_str(), group))
        .collect::<HashMap<_, _>>();
    for instrument in &candidate.config.strategy.instruments {
        let source = source_instruments
            .get(instrument.symbol.as_str())
            .with_context(|| {
                format!(
                    "dataset {} certified live config does not contain candidate {} instrument {}",
                    dataset.id, candidate.spec.id, instrument.symbol
                )
            })?;
        if source.kind != instrument.kind
            || source.base_currency != instrument.base_currency
            || source.quote_currency != instrument.quote_currency
            || source.settle_currency != instrument.settle_currency
            || source.contract_value.to_bits() != instrument.contract_value.to_bits()
        {
            bail!(
                "dataset {} certified live instrument {} accounting contract differs from candidate {}",
                dataset.id,
                instrument.symbol,
                candidate.spec.id
            );
        }
        let source_group = source_groups
            .get(source.risk_group.as_str())
            .with_context(|| {
                format!(
                    "dataset {} certified live instrument {} references unknown risk group {}",
                    dataset.id, source.symbol, source.risk_group
                )
            })?;
        if source_group.account_id.as_deref() != Some(account_id) {
            bail!(
                "dataset {} certified live instrument {} is not routed to account {:?}",
                dataset.id,
                source.symbol,
                account_id
            );
        }
    }
    Ok(())
}

fn validate_production_capture_config(
    dataset_id: &str,
    config: &CaptureConfig,
    candidates: &[LoadedCandidate],
) -> Result<()> {
    if !config.host_guard.enabled {
        bail!("production dataset {dataset_id} requires an enabled capture host guard");
    }
    let host_guard_policy_errors = config.host_guard.production_policy_errors("host_guard");
    if !host_guard_policy_errors.is_empty() {
        bail!(
            "production dataset {dataset_id} capture host guard policy failed: {}",
            host_guard_policy_errors.join("; ")
        );
    }
    let connection_pacer_path = config
        .runtime
        .connection_attempt_pacer_path
        .as_ref()
        .context("production capture requires a process-shared connection pacer")?;
    if !connection_pacer_path.is_absolute() {
        bail!(
            "production dataset {dataset_id} requires an absolute process-shared connection pacer path"
        );
    }
    let streams = config
        .subscriptions
        .iter()
        .map(|subscription| {
            (
                subscription.channel.trim().to_string(),
                subscription.symbol.trim().to_string(),
            )
        })
        .collect::<HashSet<_>>();
    for subscription in &config.subscriptions {
        if subscription.connections < 2 {
            bail!(
                "production dataset {dataset_id} capture stream {}/{} requires at least two connections",
                subscription.channel,
                subscription.symbol
            );
        }
    }

    let has_stream =
        |channel: &str, symbol: &str| streams.contains(&(channel.to_string(), symbol.to_string()));
    let has_book = |symbol: &str| {
        ["books", "books-l2-tbt", "books50-l2-tbt"]
            .iter()
            .any(|channel| has_stream(channel, symbol))
    };
    for candidate in candidates {
        for route in &candidate.config.backtest.currency_rates {
            if !has_stream("index-tickers", &route.index_symbol) {
                bail!(
                    "production dataset {dataset_id} lacks index-tickers for candidate {} accounting currency {} via {}",
                    candidate.spec.id,
                    route.currency,
                    route.index_symbol
                );
            }
        }
        for instrument in &candidate.config.strategy.instruments {
            if !has_book(&instrument.symbol) {
                bail!(
                    "production dataset {dataset_id} lacks a book stream for candidate {} symbol {}",
                    candidate.spec.id,
                    instrument.symbol
                );
            }
            if !has_stream("trades", &instrument.symbol) {
                bail!(
                    "production dataset {dataset_id} lacks trades for candidate {} symbol {}",
                    candidate.spec.id,
                    instrument.symbol
                );
            }
            if instrument.kind.is_derivative() {
                for channel in ["mark-price", "price-limit"] {
                    if !has_stream(channel, &instrument.symbol) {
                        bail!(
                            "production dataset {dataset_id} lacks {channel} for candidate {} symbol {}",
                            candidate.spec.id,
                            instrument.symbol
                        );
                    }
                }
            }
            if instrument.kind.is_swap() && !has_stream("funding-rate", &instrument.symbol) {
                bail!(
                    "production dataset {dataset_id} lacks funding-rate for candidate {} symbol {}",
                    candidate.spec.id,
                    instrument.symbol
                );
            }
            if let Some(index_symbol) = &instrument.index_symbol
                && !has_stream("index-tickers", index_symbol)
            {
                bail!(
                    "production dataset {dataset_id} lacks index-tickers for candidate {} index symbol {}",
                    candidate.spec.id,
                    index_symbol
                );
            }
        }
    }
    Ok(())
}

fn training_failures(
    runs: &[ResearchRunReport],
    aggregate: &RunAggregate,
    scenario: &ResearchScenario,
    gates: &ResearchGates,
) -> Vec<String> {
    let mut failures = evidence_failures(runs, aggregate, scenario, gates);
    if aggregate.input_events < gates.minimum_train_input_events_per_fold {
        failures.push(format!(
            "training input events {} below {}",
            aggregate.input_events, gates.minimum_train_input_events_per_fold
        ));
    }
    if aggregate.fills < gates.minimum_train_fills_per_fold {
        failures.push(format!(
            "training fills {} below {}",
            aggregate.fills, gates.minimum_train_fills_per_fold
        ));
    }
    if aggregate.funding_settlements < gates.minimum_train_funding_settlements_per_fold {
        failures.push(format!(
            "training funding settlements {} below {}",
            aggregate.funding_settlements, gates.minimum_train_funding_settlements_per_fold
        ));
    }
    failures
}

fn test_failures(
    runs: &[ResearchRunReport],
    aggregate: &RunAggregate,
    scenario: &ResearchScenario,
    gates: &ResearchGates,
) -> (Vec<String>, Vec<String>) {
    let mut evidence = evidence_failures(runs, aggregate, scenario, gates);
    if aggregate.input_events < gates.minimum_test_input_events_per_fold {
        evidence.push(format!(
            "test input events {} below {}",
            aggregate.input_events, gates.minimum_test_input_events_per_fold
        ));
    }
    if aggregate.fills < gates.minimum_test_fills_per_fold {
        evidence.push(format!(
            "test fills {} below {}",
            aggregate.fills, gates.minimum_test_fills_per_fold
        ));
    }
    if aggregate.funding_settlements < gates.minimum_test_funding_settlements_per_fold {
        evidence.push(format!(
            "test funding settlements {} below {}",
            aggregate.funding_settlements, gates.minimum_test_funding_settlements_per_fold
        ));
    }
    if aggregate.observed_duration_ns < gates.minimum_test_duration_ns_per_fold {
        evidence.push(format!(
            "test duration {} ns below {} ns",
            aggregate.observed_duration_ns, gates.minimum_test_duration_ns_per_fold
        ));
    }

    let mut performance = Vec::new();
    if aggregate.net_pnl_usd < gates.minimum_test_pnl_usd_per_fold {
        performance.push(format!(
            "test PnL {} below {}",
            aggregate.net_pnl_usd, gates.minimum_test_pnl_usd_per_fold
        ));
    }
    if aggregate.maximum_drawdown_usd > gates.maximum_test_drawdown_usd {
        performance.push(format!(
            "test drawdown {} exceeds {}",
            aggregate.maximum_drawdown_usd, gates.maximum_test_drawdown_usd
        ));
    }
    if aggregate.maximum_abs_delta_usd > gates.maximum_test_abs_delta_usd {
        performance.push(format!(
            "test maximum absolute delta {} exceeds {}",
            aggregate.maximum_abs_delta_usd, gates.maximum_test_abs_delta_usd
        ));
    }
    if aggregate.maximum_final_abs_delta_usd > gates.maximum_test_final_abs_delta_usd {
        performance.push(format!(
            "test final absolute delta {} exceeds {}",
            aggregate.maximum_final_abs_delta_usd, gates.maximum_test_final_abs_delta_usd
        ));
    }
    if aggregate.maximum_abs_pending_delta_usd > gates.maximum_test_abs_pending_delta_usd {
        performance.push(format!(
            "test maximum absolute pending delta {} exceeds {}",
            aggregate.maximum_abs_pending_delta_usd, gates.maximum_test_abs_pending_delta_usd
        ));
    }
    if aggregate.maximum_final_abs_pending_delta_usd
        > gates.maximum_test_final_abs_pending_delta_usd
    {
        performance.push(format!(
            "test final absolute pending delta {} exceeds {}",
            aggregate.maximum_final_abs_pending_delta_usd,
            gates.maximum_test_final_abs_pending_delta_usd
        ));
    }
    if aggregate.maximum_gross_exposure_usd > gates.maximum_test_gross_exposure_usd {
        performance.push(format!(
            "test maximum gross exposure {} exceeds {}",
            aggregate.maximum_gross_exposure_usd, gates.maximum_test_gross_exposure_usd
        ));
    }
    if aggregate.maximum_final_gross_exposure_usd > gates.maximum_test_final_gross_exposure_usd {
        performance.push(format!(
            "test final gross exposure {} exceeds {}",
            aggregate.maximum_final_gross_exposure_usd, gates.maximum_test_final_gross_exposure_usd
        ));
    }
    if aggregate.maximum_active_orders > gates.maximum_test_active_orders {
        performance.push(format!(
            "test maximum active orders {} exceeds {}",
            aggregate.maximum_active_orders, gates.maximum_test_active_orders
        ));
    }
    if aggregate.maximum_active_order_notional_usd > gates.maximum_test_active_order_notional_usd {
        performance.push(format!(
            "test maximum active-order notional {} exceeds {}",
            aggregate.maximum_active_order_notional_usd,
            gates.maximum_test_active_order_notional_usd
        ));
    }
    if aggregate.maximum_final_active_order_notional_usd
        > gates.maximum_test_final_active_order_notional_usd
    {
        performance.push(format!(
            "test final active-order notional {} exceeds {}",
            aggregate.maximum_final_active_order_notional_usd,
            gates.maximum_test_final_active_order_notional_usd
        ));
    }
    if aggregate.average_abs_delta_usd > gates.maximum_test_average_abs_delta_usd {
        performance.push(format!(
            "test average absolute delta {} exceeds {}",
            aggregate.average_abs_delta_usd, gates.maximum_test_average_abs_delta_usd
        ));
    }
    if aggregate.inventory_open_fraction > gates.maximum_inventory_open_fraction {
        performance.push(format!(
            "test inventory-open fraction {} exceeds {}",
            aggregate.inventory_open_fraction, gates.maximum_inventory_open_fraction
        ));
    }
    (evidence, performance)
}

fn evidence_failures(
    runs: &[ResearchRunReport],
    aggregate: &RunAggregate,
    scenario: &ResearchScenario,
    gates: &ResearchGates,
) -> Vec<String> {
    let mut failures = runs
        .iter()
        .filter_map(|run| {
            run.error
                .as_ref()
                .map(|error| format!("dataset {} failed: {error}", run.dataset_id))
        })
        .collect::<Vec<_>>();
    if aggregate.successful_runs != aggregate.runs {
        failures.push(format!(
            "only {} of {} runs completed",
            aggregate.successful_runs, aggregate.runs
        ));
    }
    if gates.require_complete_accounting && !aggregate.accounting_complete {
        failures.push("accounting is incomplete".to_string());
    }
    if !aggregate.final_valuation_complete {
        failures.push("one or more final portfolio/order valuations are incomplete".to_string());
    }
    if aggregate.strategy_halts > 0 {
        failures.push(format!(
            "{} backtest runs ended with a terminal strategy safety halt",
            aggregate.strategy_halts
        ));
    }
    if gates.require_calibrated_execution
        && scenario.kind == ResearchScenarioKind::Baseline
        && (!scenario.execution.calibrated || !aggregate.execution_calibrated)
    {
        failures.push("execution assumptions are not declared calibrated".to_string());
    }
    if aggregate.pending_non_funding_actions > gates.maximum_pending_non_funding_actions_per_fold {
        failures.push(format!(
            "{} non-funding actions remain pending, limit {}",
            aggregate.pending_non_funding_actions,
            gates.maximum_pending_non_funding_actions_per_fold
        ));
    }
    if aggregate.maximum_terminal_pending_orders > gates.maximum_terminal_pending_orders_per_run {
        failures.push(format!(
            "up to {} exchange orders remain pending, limit {}",
            aggregate.maximum_terminal_pending_orders,
            gates.maximum_terminal_pending_orders_per_run
        ));
    }
    if aggregate.maximum_terminal_pending_cancel_requests
        > gates.maximum_terminal_pending_cancel_requests_per_run
    {
        failures.push(format!(
            "up to {} cancel requests remain pending, limit {}",
            aggregate.maximum_terminal_pending_cancel_requests,
            gates.maximum_terminal_pending_cancel_requests_per_run
        ));
    }
    for run in runs {
        let Some(report) = &run.report else {
            continue;
        };
        if report.input_clock_regressions > gates.maximum_clock_regressions_per_run {
            failures.push(format!(
                "dataset {} has {} clock regressions, limit {}",
                run.dataset_id,
                report.input_clock_regressions,
                gates.maximum_clock_regressions_per_run
            ));
        }
    }
    failures
}

fn overall_failures(
    manifest: &ResearchManifest,
    folds: &[FoldReport],
    aggregate: &ResearchAggregate,
) -> Vec<String> {
    let mut failures = Vec::new();
    if folds.len() < manifest.gates.minimum_folds {
        failures.push(format!(
            "fold count {} below {}",
            folds.len(),
            manifest.gates.minimum_folds
        ));
    }
    let stress_count = manifest
        .scenarios
        .iter()
        .filter(|scenario| scenario.kind == ResearchScenarioKind::Stress)
        .count();
    if stress_count < manifest.gates.minimum_stress_scenarios {
        failures.push(format!(
            "stress scenario count {} below {}",
            stress_count, manifest.gates.minimum_stress_scenarios
        ));
    }
    if let Some(expected) = manifest.deployment_candidate_id.as_deref() {
        let mismatched_folds = folds
            .iter()
            .filter(|fold| fold.selected_candidate_id.as_deref() != Some(expected))
            .map(|fold| fold.id.as_str())
            .collect::<Vec<_>>();
        if !mismatched_folds.is_empty() {
            failures.push(format!(
                "predeclared deployment candidate {expected} was not training-selected in folds: {}",
                mismatched_folds.join(", ")
            ));
        }
    }
    if folds.iter().any(|fold| !fold.evidence_complete) {
        failures.push("one or more folds have incomplete evidence".to_string());
    }
    if aggregate.passing_fold_fraction < manifest.gates.minimum_passing_fold_fraction {
        failures.push(format!(
            "passing fold fraction {} below {}",
            aggregate.passing_fold_fraction, manifest.gates.minimum_passing_fold_fraction
        ));
    }
    if aggregate.profitable_fold_fraction < manifest.gates.minimum_profitable_fold_fraction {
        failures.push(format!(
            "profitable baseline fold fraction {} below {}",
            aggregate.profitable_fold_fraction, manifest.gates.minimum_profitable_fold_fraction
        ));
    }
    if aggregate.stress_pass_fraction < manifest.gates.minimum_stress_pass_fraction {
        failures.push(format!(
            "stress pass fraction {} below {}",
            aggregate.stress_pass_fraction, manifest.gates.minimum_stress_pass_fraction
        ));
    }
    if aggregate.total_baseline_test_pnl_usd < manifest.gates.minimum_total_baseline_test_pnl_usd {
        failures.push(format!(
            "total baseline test PnL {} below {}",
            aggregate.total_baseline_test_pnl_usd,
            manifest.gates.minimum_total_baseline_test_pnl_usd
        ));
    }
    failures
}

#[cfg(test)]
mod tests {
    use reap_capture::{
        CAPTURE_RUN_REPORT_FORMAT_VERSION, CaptureBookHealth, CaptureConfigFileEvidence,
        CaptureFailureEvidence, CaptureOutputConfig, CapturePriority, CaptureRunReport,
        CaptureRuntimeConfig, CaptureStopReason, CaptureSubscriptionConfig, CaptureVenueConfig,
        HostGuardConfig, HostHealthSnapshot, analyze_capture,
    };
    use reap_strategy::{ChaosConfig, InstrumentConfig, InstrumentKindConfig, RiskGroupConfig};
    use tempfile::TempDir;

    use super::*;

    const RAW_CAPTURE_FIXTURE: &[u8] =
        include_bytes!("../../../fixtures/raw/okx/depth-reset.jsonl");

    #[test]
    fn opening_account_loader_preserves_bounded_file_errors() {
        let directory = TempDir::new().unwrap();
        let missing = directory.path().join("missing.json");
        assert!(
            verify_opening_account_certification_path(&missing)
                .unwrap_err()
                .to_string()
                .contains("invalid account-certification artifact path")
        );

        let malformed = directory.path().join("malformed.json");
        std::fs::write(&malformed, b"{").unwrap();
        let error = verify_opening_account_certification_path(&malformed)
            .unwrap_err()
            .to_string();
        assert!(error.contains("failed to parse account-certification artifact"));
        assert!(error.contains(&malformed.display().to_string()));

        let oversized = directory.path().join("oversized.json");
        std::fs::File::create(&oversized)
            .unwrap()
            .set_len(MAX_ACCOUNT_CERTIFICATION_ARTIFACT_BYTES + 1)
            .unwrap();
        let error = verify_opening_account_certification_path(&oversized)
            .unwrap_err()
            .to_string();
        assert!(error.contains("account-certification artifact is 50331649 bytes"));
        assert!(error.contains("limit is 50331648"));
    }

    struct ResearchCaptureFixture {
        _directory: TempDir,
        config_path: PathBuf,
        report_path: PathBuf,
        raw_path: PathBuf,
    }

    fn research_capture_fixture() -> ResearchCaptureFixture {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("capture.toml");
        let report_path = directory.path().join("capture-report.json");
        let raw_path = directory.path().join("capture.jsonl");
        std::fs::write(&raw_path, RAW_CAPTURE_FIXTURE).unwrap();

        let runtime = CaptureRuntimeConfig {
            connection_attempt_pacer_path: Some(
                directory.path().join("okx-connection-attempt.pacer"),
            ),
            ..CaptureRuntimeConfig::default()
        };
        let config = CaptureConfig {
            venue: CaptureVenueConfig::default(),
            runtime,
            output: CaptureOutputConfig::default(),
            host_guard: HostGuardConfig {
                enabled: true,
                check_interval_ms: 10,
                min_disk_available_bytes: 5 * 1024 * 1024 * 1024,
                min_memory_available_bytes: 1024 * 1024 * 1024,
                require_clock_synchronized: true,
            },
            subscriptions: vec![CaptureSubscriptionConfig {
                channel: "books".to_string(),
                symbol: "BTC-USDT".to_string(),
                connections: 2,
                priority: CapturePriority::Critical,
            }],
        };
        let config_bytes = toml::to_string(&config).unwrap().into_bytes();
        std::fs::write(&config_path, &config_bytes).unwrap();

        let recorded_raw_path = PathBuf::from("collector/original-capture.jsonl");
        let mut effective_config = config.clone();
        effective_config.output.raw_path = recorded_raw_path.clone();
        effective_config.output.normalized_path = None;
        let analysis = analyze_capture(RAW_CAPTURE_FIXTURE, &effective_config).unwrap();
        let expected_connections = 2;
        let report = CaptureRunReport {
            format_version: CAPTURE_RUN_REPORT_FORMAT_VERSION,
            reap_version: env!("CARGO_PKG_VERSION").to_string(),
            java_reference_revision: PINNED_JAVA_REVISION.to_string(),
            executable_sha256: "e".repeat(64),
            host_identity_sha256: Some("9".repeat(64)),
            host_preflight: Some(HostHealthSnapshot {
                checked_at_ms: 1,
                disk_available_bytes: 5 * 1024 * 1024 * 1024,
                memory_available_bytes: 1024 * 1024 * 1024,
                clock_synchronized: true,
            }),
            host_periodic_checks: 1,
            host_last_snapshot: Some(HostHealthSnapshot {
                checked_at_ms: 2,
                disk_available_bytes: 5 * 1024 * 1024 * 1024,
                memory_available_bytes: 1024 * 1024 * 1024,
                clock_synchronized: true,
            }),
            session_started_at_ms: 1,
            session_completed_at_ms: 3,
            capture_session_id: analysis.capture_sessions[0].clone(),
            config_fingerprint: effective_config.fingerprint().unwrap(),
            config_source: Some(CaptureConfigFileEvidence {
                source_path: PathBuf::from("collector/capture.toml"),
                bytes: config_bytes.len() as u64,
                sha256: sha256_bytes(&config_bytes),
            }),
            stop_reason: CaptureStopReason::DurationElapsed,
            elapsed_ms: 1_000,
            raw_path: recorded_raw_path,
            normalized_path: None,
            raw_records: analysis.lines,
            normalized_records: 0,
            raw_bytes: RAW_CAPTURE_FIXTURE.len() as u64,
            normalized_bytes: 0,
            raw_sha256: sha256_bytes(RAW_CAPTURE_FIXTURE),
            normalized_sha256: None,
            max_raw_queue_depth: 1,
            max_normalized_queue_depth: 0,
            parsed_events: analysis.parsed_events,
            accepted_events: analysis.accepted_events,
            duplicates: analysis.duplicate_events,
            gaps: analysis.gaps,
            recoveries: analysis.recoveries,
            recovery_failures: analysis.recovery_failures,
            sequence_resets: analysis.sequence_resets,
            same_sequence_updates: analysis.same_sequence_updates,
            recovery_requests: 0,
            missing_recovery_routes: 0,
            parse_errors: analysis.error_count,
            stale_book_events: 0,
            connection_disconnects: 0,
            expected_connections,
            ready_connections_at_stop: expected_connections,
            reached_all_connections_ready: true,
            books: analysis
                .books
                .iter()
                .map(|book| CaptureBookHealth {
                    symbol: book.symbol.clone(),
                    sequence_status: book.sequence_status.clone(),
                    book_status: book.book_status.clone(),
                    last_seq_id: book.last_seq_id,
                    buffered_updates: book.buffered_updates,
                    sequence_resets: book.sequence_resets,
                    same_sequence_updates: book.same_sequence_updates,
                    best_bid: book.best_bid,
                    best_ask: book.best_ask,
                })
                .collect(),
            failure: None,
            clean_capture: true,
        };
        write_capture_report(&report_path, &report);

        ResearchCaptureFixture {
            _directory: directory,
            config_path,
            report_path,
            raw_path,
        }
    }

    fn two_symbol_raw_capture(directory: &Path) -> PathBuf {
        let mut output = Vec::new();
        let mut capture_record_seq = 1_u64;
        for line in std::str::from_utf8(RAW_CAPTURE_FIXTURE).unwrap().lines() {
            let source = serde_json::from_str::<serde_json::Value>(line).unwrap();
            for (offset, symbol) in ["BTC-USDT", "BTC-PERP"].into_iter().enumerate() {
                let mut record = source.clone();
                record["capture_record_seq"] = capture_record_seq.into();
                record["recv_ts_ns"] =
                    (source["recv_ts_ns"].as_u64().unwrap() * 10 + offset as u64).into();
                record["symbol"] = symbol.into();
                record["payload"]["arg"]["instId"] = symbol.into();
                serde_json::to_writer(&mut output, &record).unwrap();
                output.push(b'\n');
                capture_record_seq += 1;
            }
        }
        let path = directory.join("two-symbol-capture.jsonl");
        std::fs::write(&path, output).unwrap();
        path
    }

    fn carry_candidate(directory: &Path) -> LoadedCandidate {
        let strategy = ChaosConfig {
            ref_symbol: "BTC-USDT".to_string(),
            risk_groups: vec![RiskGroupConfig {
                name: "main".to_string(),
                symbols: vec!["BTC-USDT".to_string(), "BTC-PERP".to_string()],
                ..RiskGroupConfig::default()
            }],
            instruments: vec![
                InstrumentConfig {
                    symbol: "BTC-USDT".to_string(),
                    risk_group: "main".to_string(),
                    base_currency: "BTC".to_string(),
                    quote_currency: "USD".to_string(),
                    quote_profit_margin: 1.0,
                    halted: true,
                    ..InstrumentConfig::default()
                },
                InstrumentConfig {
                    symbol: "BTC-PERP".to_string(),
                    risk_group: "main".to_string(),
                    kind: InstrumentKindConfig::Future,
                    base_currency: "BTC".to_string(),
                    quote_currency: "USD".to_string(),
                    settle_currency: "USD".to_string(),
                    quote_profit_margin: 1.0,
                    contract_value: 0.001,
                    min_trade_size: 1.0,
                    lot_size: 1.0,
                    halted: true,
                    ..InstrumentConfig::default()
                },
            ],
            ..ChaosConfig::default()
        };
        let config = BacktestConfig {
            strategy: strategy.clone(),
            backtest: BacktestExecutionConfig::default(),
            initial_portfolio: BacktestInitialPortfolioConfig {
                balances: vec![
                    BacktestInitialBalanceConfig {
                        currency: "BTC".to_string(),
                        total: 0.0,
                        valuation_symbol: Some("BTC-USDT".to_string()),
                        ..Default::default()
                    },
                    BacktestInitialBalanceConfig {
                        currency: "USD".to_string(),
                        total: 10_000.0,
                        ..Default::default()
                    },
                ],
                positions: vec![BacktestInitialPositionConfig {
                    symbol: "BTC-PERP".to_string(),
                    qty: 0.0,
                    avg_price: 0.0,
                    margin_mode: Some(PositionMarginMode::Cross),
                }],
                ..Default::default()
            },
        };
        LoadedCandidate {
            spec: ResearchCandidate {
                id: "carry-candidate".to_string(),
                config: PathBuf::from("candidate.toml"),
            },
            resolved_path: directory.join("candidate.toml"),
            sha256: "c".repeat(64),
            effective_strategy_sha256: effective_strategy_sha256(&strategy).unwrap(),
            config,
        }
    }

    fn loaded_raw_segment(
        id: &str,
        path: &Path,
        range: RawCaptureRecordRange,
        continuation_of: Option<&str>,
    ) -> LoadedDataset {
        LoadedDataset {
            spec: ResearchDataset {
                id: id.to_string(),
                path: path.to_path_buf(),
                format: ResearchDataFormat::RawCapture,
                capture_record_range: Some(range),
                continuation_of: continuation_of.map(str::to_string),
                capture_config: None,
                capture_report: None,
                normalized_path: None,
                opening_account: None,
            },
            resolved_path: path.to_path_buf(),
            sha256: sha256_path(path).unwrap(),
            raw_replay_check: None,
            resolved_capture_config: None,
            capture_config_sha256: None,
            resolved_capture_report: None,
            capture_report_sha256: None,
            resolved_normalized_path: None,
            normalized_sha256: None,
            capture_analysis: None,
            capture_verification: None,
            resolved_opening_account: None,
            opening_account: None,
        }
    }

    fn fixture_dataset(fixture: &ResearchCaptureFixture) -> ResearchDataset {
        ResearchDataset {
            id: "capture".to_string(),
            path: fixture.raw_path.clone(),
            format: ResearchDataFormat::RawCapture,
            capture_record_range: None,
            continuation_of: None,
            capture_config: Some(fixture.config_path.clone()),
            capture_report: Some(fixture.report_path.clone()),
            normalized_path: None,
            opening_account: None,
        }
    }

    fn load_test_production_datasets(
        datasets: &[ResearchDataset],
        base: &Path,
        candidates: &[LoadedCandidate],
    ) -> Result<Vec<LoadedDataset>> {
        let executable_sha256 = "e".repeat(64);
        let host_identity_sha256 = "9".repeat(64);
        load_datasets(
            datasets,
            base,
            ResearchMode::ProductionCandidate,
            candidates,
            &executable_sha256,
            Some(&host_identity_sha256),
            60_000,
        )
    }

    fn read_capture_report(path: &Path) -> CaptureRunReport {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn write_capture_report(path: &Path, report: &CaptureRunReport) {
        let mut bytes = serde_json::to_vec_pretty(report).unwrap();
        bytes.push(b'\n');
        std::fs::write(path, bytes).unwrap();
    }

    fn execution(
        calibrated: bool,
        latency_ms: u64,
        queue: f64,
        trade_fraction: f64,
        depth_fraction: f64,
    ) -> BacktestExecutionConfig {
        BacktestExecutionConfig {
            calibrated,
            market_data_latency_ms: latency_ms,
            order_entry_latency_ms: latency_ms,
            cancel_latency_ms: latency_ms,
            order_update_latency_ms: latency_ms,
            fill_account_latency_ms: latency_ms,
            latency_profile: Default::default(),
            currency_rates: Vec::new(),
            depth_fill_conservative_threshold: 0.0001,
            queue_ahead_multiplier: queue,
            historical_trade_fill_fraction: trade_fraction,
            displayed_depth_fill_fraction: depth_fraction,
            derivative_leverage: 1.0,
            exchange_cmr_multiplier: 50.0,
        }
    }

    #[test]
    fn research_scenarios_inherit_but_cannot_replace_candidate_currency_routes() {
        let route = crate::BacktestCurrencyRateConfig {
            currency: "USDT".to_string(),
            index_symbol: "USDT-USD".to_string(),
            max_age_ms: 75_000,
        };
        let mut candidate = BacktestExecutionConfig {
            currency_rates: vec![route.clone()],
            ..BacktestExecutionConfig::default()
        };
        let scenario = BacktestExecutionConfig {
            market_data_latency_ms: 10,
            ..BacktestExecutionConfig::default()
        };

        let inherited = effective_scenario_execution(&candidate, &scenario).unwrap();
        assert_eq!(inherited.currency_rates, vec![route.clone()]);
        assert_eq!(inherited.market_data_latency_ms, 10);

        let explicit_match = BacktestExecutionConfig {
            currency_rates: vec![route.clone()],
            ..scenario.clone()
        };
        assert!(effective_scenario_execution(&candidate, &explicit_match).is_ok());

        candidate.currency_rates[0].max_age_ms = 10_000;
        assert!(effective_scenario_execution(&candidate, &explicit_match).is_err());
    }

    fn gates() -> ResearchGates {
        ResearchGates {
            minimum_folds: 1,
            minimum_stress_scenarios: 1,
            minimum_train_input_events_per_fold: 1,
            minimum_train_fills_per_fold: 0,
            minimum_train_funding_settlements_per_fold: 0,
            minimum_test_input_events_per_fold: 1,
            minimum_test_fills_per_fold: 0,
            minimum_test_funding_settlements_per_fold: 0,
            minimum_test_duration_ns_per_fold: 1,
            minimum_test_pnl_usd_per_fold: -1_000_000.0,
            minimum_total_baseline_test_pnl_usd: -1_000_000.0,
            maximum_test_drawdown_usd: 1_000_000.0,
            maximum_test_abs_delta_usd: 1_000_000.0,
            maximum_test_final_abs_delta_usd: 1_000_000.0,
            maximum_test_abs_pending_delta_usd: 1_000_000.0,
            maximum_test_final_abs_pending_delta_usd: 1_000_000.0,
            maximum_test_gross_exposure_usd: 1_000_000.0,
            maximum_test_final_gross_exposure_usd: 1_000_000.0,
            maximum_test_active_orders: 1_000,
            maximum_test_active_order_notional_usd: 1_000_000.0,
            maximum_test_final_active_order_notional_usd: 1_000_000.0,
            maximum_test_average_abs_delta_usd: 1_000_000.0,
            maximum_inventory_open_fraction: 1.0,
            maximum_pending_non_funding_actions_per_fold: 10,
            maximum_terminal_pending_orders_per_run: 10,
            maximum_terminal_pending_cancel_requests_per_run: 10,
            maximum_clock_regressions_per_run: 0,
            maximum_opening_account_gap_ms: 60_000,
            minimum_profitable_fold_fraction: 0.0,
            minimum_stress_pass_fraction: 1.0,
            minimum_passing_fold_fraction: 1.0,
            require_complete_accounting: true,
            require_calibrated_execution: false,
        }
    }

    fn manifest() -> ResearchManifest {
        ResearchManifest {
            schema_version: RESEARCH_SCHEMA_VERSION,
            mode: ResearchMode::Smoke,
            java_reference_revision: PINNED_JAVA_REVISION.to_string(),
            latency_calibration: None,
            selection_metric: SelectionMetric::NetPnlUsd,
            deployment_candidate_id: None,
            candidates: vec![ResearchCandidate {
                id: "base".to_string(),
                config: "candidate.toml".into(),
            }],
            datasets: vec![
                ResearchDataset {
                    id: "train".to_string(),
                    path: "train.jsonl".into(),
                    format: ResearchDataFormat::NormalizedJsonl,
                    capture_record_range: None,
                    continuation_of: None,
                    capture_config: None,
                    capture_report: None,
                    normalized_path: None,
                    opening_account: None,
                },
                ResearchDataset {
                    id: "test".to_string(),
                    path: "test.jsonl".into(),
                    format: ResearchDataFormat::NormalizedJsonl,
                    capture_record_range: None,
                    continuation_of: None,
                    capture_config: None,
                    capture_report: None,
                    normalized_path: None,
                    opening_account: None,
                },
            ],
            scenarios: vec![
                ResearchScenario {
                    id: "baseline".to_string(),
                    kind: ResearchScenarioKind::Baseline,
                    execution: execution(false, 0, 1.0, 1.0, 1.0),
                },
                ResearchScenario {
                    id: "stress".to_string(),
                    kind: ResearchScenarioKind::Stress,
                    execution: execution(false, 10, 2.0, 0.25, 0.5),
                },
            ],
            folds: vec![ResearchFold {
                id: "fold-1".to_string(),
                train: vec!["train".to_string()],
                test: vec!["test".to_string()],
            }],
            gates: gates(),
        }
    }

    #[test]
    fn manifest_rejects_optimistic_stress_and_train_test_leakage() {
        let mut manifest = manifest();
        manifest.scenarios[1].execution.queue_ahead_multiplier = 0.5;
        manifest.folds[0].test = vec!["train".to_string()];

        let error = manifest.validate().unwrap_err().to_string();

        assert!(error.contains("less conservative"));
        assert!(error.contains("both train and test"));
    }

    #[test]
    fn manifest_rejects_capture_evidence_on_non_raw_dataset() {
        let mut manifest = manifest();
        manifest.datasets[0].capture_config = Some("capture.toml".into());
        manifest.datasets[0].capture_report = Some("capture-report.json".into());
        manifest.datasets[0].normalized_path = Some("normalized.jsonl".into());

        let error = manifest.validate().unwrap_err().to_string();

        assert!(error.contains("valid only for raw_capture datasets"));
    }

    #[test]
    fn manifest_accepts_only_explicit_adjacent_continuation_order() {
        let mut manifest = manifest();
        for dataset in &mut manifest.datasets {
            dataset.format = ResearchDataFormat::RawCapture;
            dataset.path = PathBuf::from("capture.jsonl");
        }
        manifest.datasets[0].capture_record_range =
            Some(RawCaptureRecordRange { first: 1, last: 5 });
        manifest.datasets[1].capture_record_range =
            Some(RawCaptureRecordRange { first: 6, last: 10 });
        manifest.datasets[1].continuation_of = Some("train".to_string());

        manifest.validate().unwrap();

        manifest.datasets[1].capture_record_range =
            Some(RawCaptureRecordRange { first: 7, last: 10 });
        let error = manifest.validate().unwrap_err().to_string();
        assert!(error.contains("must begin at capture record 6"));

        manifest.datasets[1].capture_record_range =
            Some(RawCaptureRecordRange { first: 6, last: 10 });
        manifest.folds[0].train = vec!["test".to_string()];
        manifest.folds[0].test = vec!["train".to_string()];
        let error = manifest.validate().unwrap_err().to_string();
        assert!(error.contains("must immediately follow continuation parent train"));
    }

    #[test]
    fn production_chain_root_cannot_start_mid_capture() {
        let mut manifest = manifest();
        manifest.mode = ResearchMode::ProductionCandidate;
        manifest.datasets[0].format = ResearchDataFormat::RawCapture;
        manifest.datasets[0].capture_record_range =
            Some(RawCaptureRecordRange { first: 2, last: 5 });

        let error = manifest.validate().unwrap_err().to_string();

        assert!(error.contains(
            "production chain root train must begin at capture record 1; later ranges require continuation_of"
        ));
    }

    #[test]
    fn raw_continuation_consumes_selected_training_carry() {
        let directory = tempfile::tempdir().unwrap();
        let raw_path = two_symbol_raw_capture(directory.path());
        let candidate = carry_candidate(directory.path());
        let datasets = vec![
            loaded_raw_segment(
                "train",
                &raw_path,
                RawCaptureRecordRange { first: 1, last: 12 },
                None,
            ),
            loaded_raw_segment(
                "test",
                &raw_path,
                RawCaptureRecordRange {
                    first: 13,
                    last: 14,
                },
                Some("train"),
            ),
        ];
        let scenario = ResearchScenario {
            id: "baseline".to_string(),
            kind: ResearchScenarioKind::Baseline,
            execution: BacktestExecutionConfig::default(),
        };
        let mut cache = HashMap::new();

        let training = run_sequence(
            &mut cache,
            &candidate,
            &datasets,
            &["train".to_string()],
            &scenario,
            None,
        );
        let training_report = training[0].report.as_ref().unwrap();
        let carry = training_report.settled_carry_state.clone().unwrap();
        assert_eq!(
            training_report
                .raw_replay_boundary
                .as_ref()
                .unwrap()
                .last_capture_record_seq,
            12
        );

        let testing = run_sequence(
            &mut cache,
            &candidate,
            &datasets,
            &["test".to_string()],
            &scenario,
            Some((carry, training_report.execution.clone())),
        );
        assert!(
            testing[0].report.is_some(),
            "{:?}",
            testing[0].error.as_deref()
        );
        let testing_report = testing[0].report.as_ref().unwrap();

        assert_eq!(
            testing_report.opening_equity_usd,
            Some(training_report.final_equity_usd)
        );
        assert_eq!(
            testing_report
                .raw_replay_boundary
                .as_ref()
                .unwrap()
                .first_capture_record_seq,
            13
        );
        assert!(
            testing_report.settled_carry_state.is_some(),
            "{:?}",
            testing_report.carry_state_failures
        );
        assert!(testing[0].error.is_none());
    }

    #[test]
    fn dataset_loader_allows_disjoint_ranges_and_rejects_overlap() {
        let fixture = research_capture_fixture();
        let mut root = fixture_dataset(&fixture);
        root.id = "root".to_string();
        root.capture_record_range = Some(RawCaptureRecordRange { first: 1, last: 6 });
        let mut continuation = fixture_dataset(&fixture);
        continuation.id = "continuation".to_string();
        continuation.capture_record_range = Some(RawCaptureRecordRange { first: 7, last: 7 });
        continuation.continuation_of = Some("root".to_string());

        let loaded = load_datasets(
            &[root.clone(), continuation.clone()],
            Path::new("."),
            ResearchMode::Smoke,
            &[],
            &"e".repeat(64),
            None,
            60_000,
        )
        .unwrap();
        assert_eq!(loaded.len(), 2);

        continuation.capture_record_range = Some(RawCaptureRecordRange { first: 6, last: 7 });
        let error = load_datasets(
            &[root, continuation],
            Path::new("."),
            ResearchMode::Smoke,
            &[],
            &"e".repeat(64),
            None,
            60_000,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("overlaps dataset root"));
    }

    #[test]
    fn manifest_rejects_mixed_dataset_opening_semantics() {
        let mut manifest = manifest();
        manifest.datasets[0].opening_account = Some(ResearchOpeningAccount {
            certification: PathBuf::from("account.json"),
            spot_valuation_symbols: BTreeMap::new(),
        });

        let error = manifest.validate().unwrap_err().to_string();

        assert!(error.contains("chain roots must either all provide opening_account"));
    }

    #[test]
    fn manifest_rejects_latency_stress_without_distribution_dominance() {
        let mut manifest = manifest();
        let rule = |samples_ms| crate::BacktestLatencyRule {
            class: crate::BacktestLatencyClass::MarketDepth,
            symbol: Some("BTC-USDT".to_string()),
            samples_ms,
        };
        manifest.scenarios[0].execution.latency_profile = crate::BacktestLatencyProfile {
            seed: 23,
            rules: vec![rule(vec![1, 3, 5])],
        };
        manifest.scenarios[1].execution.latency_profile = crate::BacktestLatencyProfile {
            seed: 23,
            rules: vec![rule(vec![0, 4, 6])],
        };

        let error = manifest.validate().unwrap_err().to_string();

        assert!(error.contains("less conservative"));
    }

    #[test]
    fn production_manifest_requires_strict_evidence_gates() {
        let mut manifest = manifest();
        manifest.mode = ResearchMode::ProductionCandidate;
        manifest.gates.maximum_opening_account_gap_ms = MAX_PRODUCTION_OPENING_ACCOUNT_GAP_MS + 1;

        let error = manifest.validate().unwrap_err().to_string();

        assert!(error.contains("at least three folds"));
        assert!(error.contains("at least two stress scenarios"));
        assert!(error.contains("calibrated execution"));
        assert!(error.contains("predeclared deployment_candidate_id"));
        assert!(error.contains("capture_config for every dataset"));
        assert!(error.contains("capture_report for every dataset"));
        assert!(error.contains("chain root train requires opening_account evidence"));
        assert!(error.contains("maximum_opening_account_gap_ms"));
    }

    #[test]
    fn manifest_binds_deployment_candidate_by_mode() {
        let mut production = manifest();
        production.mode = ResearchMode::ProductionCandidate;
        production.deployment_candidate_id = Some("missing".to_string());

        let error = production.validate().unwrap_err().to_string();
        assert!(error.contains("deployment_candidate_id \"missing\" does not name a candidate"));

        let mut smoke = manifest();
        smoke.deployment_candidate_id = Some("base".to_string());

        let error = smoke.validate().unwrap_err().to_string();
        assert!(error.contains("smoke research cannot declare a deployment_candidate_id"));
    }

    #[test]
    fn production_swap_candidates_require_funding_settlement_evidence() {
        let mut swap = BacktestConfig {
            strategy: Default::default(),
            backtest: Default::default(),
            initial_portfolio: Default::default(),
        };
        swap.strategy
            .instruments
            .push(reap_strategy::InstrumentConfig {
                kind: reap_strategy::InstrumentKindConfig::LinearSwap,
                ..Default::default()
            });
        let gates = gates();

        let error =
            validate_candidate_funding_evidence(ResearchMode::ProductionCandidate, &gates, [&swap])
                .unwrap_err()
                .to_string();

        assert!(error.contains("swap instruments"));
        assert!(error.contains("funding-settlement evidence gates"));
    }

    #[test]
    fn production_research_reserves_opening_capital_for_certified_datasets() {
        let candidate = |id: &str, total: Option<f64>| LoadedCandidate {
            spec: ResearchCandidate {
                id: id.to_string(),
                config: PathBuf::from(format!("{id}.toml")),
            },
            resolved_path: PathBuf::from(format!("/{id}.toml")),
            config: BacktestConfig {
                strategy: Default::default(),
                backtest: Default::default(),
                initial_portfolio: total.map_or_else(Default::default, |total| {
                    crate::BacktestInitialPortfolioConfig {
                        balances: vec![crate::BacktestInitialBalanceConfig {
                            currency: "USD".to_string(),
                            total,
                            valuation_symbol: None,
                            ..Default::default()
                        }],
                        ..Default::default()
                    }
                }),
            },
            sha256: id.repeat(64),
            effective_strategy_sha256: id.repeat(64),
        };

        let missing = candidate("missing", None);
        validate_candidate_initial_portfolios(ResearchMode::ProductionCandidate, true, &[missing])
            .unwrap();

        let first = candidate("first", Some(10_000.0));
        let different = candidate("different", Some(20_000.0));
        let error = validate_candidate_initial_portfolios(
            ResearchMode::Smoke,
            false,
            &[first.clone(), different],
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("identical opening capital and inventory"));

        let error = validate_candidate_initial_portfolios(
            ResearchMode::ProductionCandidate,
            true,
            &[first.clone(), candidate("same", Some(10_000.0))],
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("must omit initial_portfolio"));
    }

    #[test]
    fn certified_dataset_derives_exact_strategy_and_accounting_opening_state() {
        let live_config =
            LiveConfig::from_toml(include_str!("../../../examples/live-okx-demo.toml")).unwrap();
        let candidate = LoadedCandidate {
            spec: ResearchCandidate {
                id: "base".to_string(),
                config: PathBuf::from("candidate.toml"),
            },
            resolved_path: PathBuf::from("/candidate.toml"),
            config: BacktestConfig {
                strategy: live_config.strategy.clone(),
                backtest: BacktestExecutionConfig {
                    currency_rates: vec![crate::BacktestCurrencyRateConfig {
                        currency: "USDT".to_string(),
                        index_symbol: "USDT-USD".to_string(),
                        max_age_ms: 75_000,
                    }],
                    ..Default::default()
                },
                initial_portfolio: Default::default(),
            },
            sha256: "a".repeat(64),
            effective_strategy_sha256: "b".repeat(64),
        };
        let dataset = ResearchDataset {
            id: "capture".to_string(),
            path: PathBuf::from("capture.jsonl"),
            format: ResearchDataFormat::RawCapture,
            capture_record_range: None,
            continuation_of: None,
            capture_config: None,
            capture_report: None,
            normalized_path: None,
            opening_account: Some(ResearchOpeningAccount {
                certification: PathBuf::from("account.json"),
                spot_valuation_symbols: BTreeMap::from([(
                    "BTC".to_string(),
                    "BTC-USDT".to_string(),
                )]),
            }),
        };
        let balance = parse_okx_account_balance_response_json(
            br#"{"code":"0","msg":"","data":[{"uTime":"1000","totalEq":"10100","mgnRatio":"12.5","adjEq":"10000","borrowFroz":"0","notionalUsdForBorrow":"0","notionalUsd":"2000","details":[{"ccy":"BTC","uTime":"999","cashBal":"0.002","availBal":"0.0015","eq":"0.002","eqUsd":"100","disEq":"100","upl":"0","liab":"0","crossLiab":"0","isoLiab":"0","uplLiab":"0","interest":"0","borrowFroz":"0","maxLoan":"0","twap":"0"},{"ccy":"USDT","uTime":"999","cashBal":"9000","availBal":"8000","eq":"10000","eqUsd":"10000","disEq":"10000","upl":"1000","liab":"0","crossLiab":"0","isoLiab":"0","uplLiab":"0","interest":"0","borrowFroz":"0","maxLoan":"500","twap":"0"}]}]}"#,
        )
        .unwrap();
        let positions = parse_okx_account_positions_response_json(
            br#"{"code":"0","msg":"","data":[{"instType":"SWAP","instId":"BTC-USDT-SWAP","pos":"2","posSide":"net","mgnMode":"cross","avgPx":"50000","uTime":"1001","liab":"","interest":""}]}"#,
        )
        .unwrap();

        let initial = derive_certified_opening_portfolio(
            &dataset,
            dataset.opening_account.as_ref().unwrap(),
            &candidate,
            &live_config,
            "main",
            &balance,
            &positions,
        )
        .unwrap();

        assert_eq!(initial.account_id.as_deref(), Some("main"));
        assert_eq!(initial.balances.len(), 2);
        let btc = initial
            .balances
            .iter()
            .find(|balance| balance.currency == "BTC")
            .unwrap();
        assert_eq!(btc.total, 0.002);
        assert_eq!(btc.available, Some(0.0015));
        assert_eq!(btc.valuation_symbol.as_deref(), Some("BTC-USDT"));
        let usdt = initial
            .balances
            .iter()
            .find(|balance| balance.currency == "USDT")
            .unwrap();
        assert_eq!(usdt.total, 9_000.0);
        assert_eq!(usdt.available, Some(8_000.0));
        assert_eq!(usdt.equity, Some(10_000.0));
        assert_eq!(usdt.max_loan, Some(500.0));
        assert_eq!(initial.positions.len(), 1);
        assert_eq!(initial.positions[0].qty, 2.0);
        assert_eq!(initial.positions[0].avg_price, 50_000.0);
        assert_eq!(
            initial.positions[0].margin_mode,
            Some(reap_core::PositionMarginMode::Cross)
        );
        assert_eq!(initial.margin.exchange_ratio, Some(12.5));
        assert_eq!(initial.margin.adjusted_equity_usd, Some(10_000.0));
        assert_eq!(initial.margin.notional_usd, Some(2_000.0));

        let mut unsafe_balance = balance.clone();
        let mut unmodeled = unsafe_balance.details[0].clone();
        unmodeled.currency = "ETH".to_string();
        unmodeled.cash_balance = Some(0.0);
        unmodeled.available_balance = Some(0.0);
        unmodeled.equity = Some(0.0);
        unmodeled.equity_usd = Some(0.0);
        unmodeled.discounted_equity_usd = Some(0.0);
        unmodeled.unrealized_pnl = Some(0.0);
        unmodeled.liability = Some(0.0);
        unmodeled.forced_repayment_indicator = Some(1);
        unsafe_balance.details.push(unmodeled);
        let error = derive_certified_opening_portfolio(
            &dataset,
            dataset.opening_account.as_ref().unwrap(),
            &candidate,
            &live_config,
            "main",
            &unsafe_balance,
            &positions,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("active forced repayment indicator"));
    }

    #[test]
    fn production_non_swap_candidates_do_not_require_funding_settlement_evidence() {
        let spot = BacktestConfig {
            strategy: reap_strategy::ChaosConfig {
                instruments: vec![reap_strategy::InstrumentConfig::default()],
                ..Default::default()
            },
            backtest: Default::default(),
            initial_portfolio: Default::default(),
        };

        validate_candidate_funding_evidence(ResearchMode::ProductionCandidate, &gates(), [&spot])
            .unwrap();
    }

    #[test]
    fn production_swap_candidates_accept_positive_funding_settlement_evidence() {
        let mut swap = BacktestConfig {
            strategy: Default::default(),
            backtest: Default::default(),
            initial_portfolio: Default::default(),
        };
        swap.strategy
            .instruments
            .push(reap_strategy::InstrumentConfig {
                kind: reap_strategy::InstrumentKindConfig::InverseSwap,
                ..Default::default()
            });
        let mut gates = gates();
        gates.minimum_train_funding_settlements_per_fold = 1;
        gates.minimum_test_funding_settlements_per_fold = 1;

        validate_candidate_funding_evidence(ResearchMode::ProductionCandidate, &gates, [&swap])
            .unwrap();
    }

    #[test]
    fn production_dataset_rejects_a_recovered_sequence_gap() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let datasets = [ResearchDataset {
            id: "gap".to_string(),
            path: "fixtures/raw/okx/depth-gap.jsonl".into(),
            format: ResearchDataFormat::RawCapture,
            capture_record_range: None,
            continuation_of: None,
            capture_config: None,
            capture_report: None,
            normalized_path: None,
            opening_account: None,
        }];

        let error = load_test_production_datasets(&datasets, &base, &[])
            .unwrap_err()
            .to_string();

        assert!(error.contains("failed zero-gap replay integrity"));
    }

    #[test]
    fn production_dataset_loads_verified_schema_five_capture_evidence() {
        let fixture = research_capture_fixture();
        let datasets = [fixture_dataset(&fixture)];

        let loaded = load_test_production_datasets(&datasets, Path::new("."), &[]).unwrap();
        let dataset = &loaded[0];

        assert!(
            dataset
                .capture_verification
                .as_ref()
                .is_some_and(|verification| verification.passed)
        );
        assert_eq!(
            dataset.capture_report_sha256.as_deref(),
            dataset
                .capture_verification
                .as_ref()
                .map(|verification| verification.run_report.sha256.as_str())
        );
        assert_eq!(
            dataset
                .capture_analysis
                .as_ref()
                .map(|report| &report.sha256),
            Some(&dataset.sha256)
        );
        verify_input_hashes(
            &fixture.raw_path,
            &dataset.sha256,
            &fixture.raw_path,
            &dataset.sha256,
            &[],
            &loaded,
            None,
        )
        .unwrap();
    }

    #[test]
    fn production_dataset_rejects_capture_from_a_different_build() {
        let fixture = research_capture_fixture();
        let expected_host = "9".repeat(64);

        let error = load_datasets(
            &[fixture_dataset(&fixture)],
            Path::new("."),
            ResearchMode::ProductionCandidate,
            &[],
            &"d".repeat(64),
            Some(&expected_host),
            60_000,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("different Reap build or Java reference"));
    }

    #[test]
    fn production_dataset_rejects_capture_from_a_different_host() {
        let fixture = research_capture_fixture();
        let expected_executable = "e".repeat(64);
        let expected_host = "8".repeat(64);

        let error = load_datasets(
            &[fixture_dataset(&fixture)],
            Path::new("."),
            ResearchMode::ProductionCandidate,
            &[],
            &expected_executable,
            Some(&expected_host),
            60_000,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("different host than the latency calibration"));
    }

    #[test]
    fn production_dataset_requires_a_completed_periodic_host_check() {
        let fixture = research_capture_fixture();
        let mut report = read_capture_report(&fixture.report_path);
        report.host_periodic_checks = 0;
        report.host_last_snapshot = None;
        write_capture_report(&fixture.report_path, &report);

        let error =
            load_test_production_datasets(&[fixture_dataset(&fixture)], Path::new("."), &[])
                .unwrap_err()
                .to_string();

        assert!(error.contains("no completed periodic host check"));
    }

    #[test]
    fn research_hash_guard_detects_capture_report_mutation() {
        let fixture = research_capture_fixture();
        let loaded =
            load_test_production_datasets(&[fixture_dataset(&fixture)], Path::new("."), &[])
                .unwrap();
        let mut report_bytes = std::fs::read(&fixture.report_path).unwrap();
        report_bytes.extend_from_slice(b" \n");
        std::fs::write(&fixture.report_path, report_bytes).unwrap();

        let error = verify_input_hashes(
            &fixture.raw_path,
            &loaded[0].sha256,
            &fixture.raw_path,
            &loaded[0].sha256,
            &[],
            &loaded,
            None,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("capture report for dataset capture changed"));
    }

    #[test]
    fn production_dataset_rejects_legacy_capture_report() {
        let fixture = research_capture_fixture();
        let mut report = read_capture_report(&fixture.report_path);
        report.format_version = 2;
        report.config_source = None;
        write_capture_report(&fixture.report_path, &report);

        let error =
            load_test_production_datasets(&[fixture_dataset(&fixture)], Path::new("."), &[])
                .unwrap_err()
                .to_string();

        assert!(error.contains("failed capture verification"));
        assert!(error.contains("UnsupportedRunReportFormat"));
    }

    #[test]
    fn production_dataset_rejects_a_reported_capture_runtime_failure() {
        let fixture = research_capture_fixture();
        let mut report = read_capture_report(&fixture.report_path);
        report.stop_reason = CaptureStopReason::RuntimeFailure;
        report.failure = Some(CaptureFailureEvidence {
            code: "writer_backpressure".to_string(),
            message: "raw capture writer queue remained full for 1000ms".to_string(),
        });
        report.clean_capture = false;
        write_capture_report(&fixture.report_path, &report);

        let error =
            load_test_production_datasets(&[fixture_dataset(&fixture)], Path::new("."), &[])
                .unwrap_err()
                .to_string();

        assert!(error.contains("failed capture verification"));
        assert!(error.contains("RunReportedFailure"));
    }

    #[test]
    fn production_dataset_rejects_capture_config_byte_tampering() {
        let fixture = research_capture_fixture();
        let mut bytes = std::fs::read(&fixture.config_path).unwrap();
        bytes.extend_from_slice(b"\n# formatting-only tamper\n");
        std::fs::write(&fixture.config_path, bytes).unwrap();

        let error =
            load_test_production_datasets(&[fixture_dataset(&fixture)], Path::new("."), &[])
                .unwrap_err()
                .to_string();

        assert!(error.contains("failed capture verification"));
        assert!(error.contains("ConfigFileMismatch"));
    }

    #[test]
    fn production_dataset_requires_declared_normalized_capture() {
        let fixture = research_capture_fixture();
        let mut report = read_capture_report(&fixture.report_path);
        report.normalized_path = Some(PathBuf::from("collector/normalized.jsonl"));
        write_capture_report(&fixture.report_path, &report);

        let error =
            load_test_production_datasets(&[fixture_dataset(&fixture)], Path::new("."), &[])
                .unwrap_err()
                .to_string();

        assert!(error.contains("failed capture verification"));
        assert!(error.contains("NormalizedArtifactMissing"));
    }

    #[test]
    fn production_capture_config_requires_redundant_candidate_data_streams() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let candidates = load_candidates(
            &[ResearchCandidate {
                id: "base".to_string(),
                config: "examples/iarb2-okx-btc.toml".into(),
            }],
            &base,
        )
        .unwrap();
        let mut config =
            CaptureConfig::load(base.join("examples/capture-okx-public.toml")).unwrap();

        let error = validate_production_capture_config("capture", &config, &candidates)
            .unwrap_err()
            .to_string();
        assert!(error.contains("absolute process-shared connection pacer path"));
        config.runtime.connection_attempt_pacer_path =
            Some(base.join("var/reap/okx-connection-attempt.pacer"));

        validate_production_capture_config("capture", &config, &candidates).unwrap();

        let mut unguarded = config.clone();
        unguarded.host_guard.enabled = false;
        let error = validate_production_capture_config("capture", &unguarded, &candidates)
            .unwrap_err()
            .to_string();
        assert!(error.contains("requires an enabled capture host guard"));

        let mut weak_guard = config.clone();
        weak_guard.host_guard.check_interval_ms = 10_001;
        weak_guard.host_guard.min_disk_available_bytes = 5 * 1024 * 1024 * 1024 - 1;
        weak_guard.host_guard.min_memory_available_bytes = 1024 * 1024 * 1024 - 1;
        weak_guard.host_guard.require_clock_synchronized = false;
        let error = validate_production_capture_config("capture", &weak_guard, &candidates)
            .unwrap_err()
            .to_string();
        assert!(error.contains("capture host guard policy failed"));
        assert!(error.contains("must not exceed 10000"));
        assert!(error.contains("at least 5368709120"));
        assert!(error.contains("at least 1073741824"));
        assert!(error.contains("require_clock_synchronized must be true"));

        let mut single_source = config.clone();
        single_source.subscriptions[0].connections = 1;
        let error = validate_production_capture_config("capture", &single_source, &candidates)
            .unwrap_err()
            .to_string();
        assert!(error.contains("requires at least two connections"));

        let mut missing_currency_rate = config.clone();
        missing_currency_rate
            .subscriptions
            .retain(|stream| !(stream.channel == "index-tickers" && stream.symbol == "USDT-USD"));
        let error =
            validate_production_capture_config("capture", &missing_currency_rate, &candidates)
                .unwrap_err()
                .to_string();
        assert!(error.contains("accounting currency USDT via USDT-USD"));

        let mut missing_trades = config;
        missing_trades
            .subscriptions
            .retain(|stream| !(stream.channel == "trades" && stream.symbol == "BTC-USDT"));
        let error = validate_production_capture_config("capture", &missing_trades, &candidates)
            .unwrap_err()
            .to_string();
        assert!(error.contains("lacks trades"));
    }

    #[test]
    fn candidate_selection_is_deterministic_on_ties() {
        let aggregate = RunAggregate {
            net_pnl_usd: 10.0,
            ..RunAggregate::default()
        };
        let training = vec![
            CandidateTrainingReport {
                candidate_id: "zeta".to_string(),
                runs: Vec::new(),
                aggregate: aggregate.clone(),
                eligible: true,
                selection_score: Some(10.0),
                failures: Vec::new(),
            },
            CandidateTrainingReport {
                candidate_id: "alpha".to_string(),
                runs: Vec::new(),
                aggregate,
                eligible: true,
                selection_score: Some(10.0),
                failures: Vec::new(),
            },
        ];

        assert_eq!(
            select_training_candidate(&training).unwrap().candidate_id,
            "alpha"
        );
    }

    #[test]
    fn production_deployment_candidate_must_win_every_training_fold() {
        assert!(deployment_selection_failure(Some("base"), Some("base")).is_none());

        let failure = deployment_selection_failure(Some("base"), Some("alternative")).unwrap();
        assert!(failure.contains("selected candidate alternative"));
        assert!(failure.contains("predeclared deployment candidate base"));

        let failure = deployment_selection_failure(Some("base"), None).unwrap();
        assert!(failure.contains("selected candidate <none>"));

        let mut manifest = manifest();
        manifest.mode = ResearchMode::ProductionCandidate;
        manifest.deployment_candidate_id = Some("base".to_string());
        let folds = [FoldReport {
            id: "fold-1".to_string(),
            train_dataset_ids: vec!["train".to_string()],
            test_dataset_ids: vec!["test".to_string()],
            selected_candidate_id: Some("alternative".to_string()),
            selection_score: Some(1.0),
            training: Vec::new(),
            test_scenarios: Vec::new(),
            evidence_complete: true,
            passed: true,
            failures: Vec::new(),
        }];
        let failures = overall_failures(&manifest, &folds, &ResearchAggregate::default());
        assert!(failures.iter().any(|failure| {
            failure
                == "predeclared deployment candidate base was not training-selected in folds: fold-1"
        }));
    }

    #[test]
    fn candidate_identity_ignores_overridden_execution_but_tracks_strategy_changes() {
        let mut config: BacktestConfig =
            toml::from_str(include_str!("../../../examples/iarb2-basic.toml")).unwrap();
        let original = effective_strategy_sha256(&config.strategy).unwrap();

        config.backtest.order_entry_latency_ms = 999;
        assert_eq!(
            effective_strategy_sha256(&config.strategy).unwrap(),
            original
        );

        config.strategy.active_hedge_threshold_usd += 1.0;
        assert_ne!(
            effective_strategy_sha256(&config.strategy).unwrap(),
            original
        );
    }

    #[test]
    fn test_gates_cover_pending_transitions_and_active_order_exposure() {
        let mut gates = gates();
        gates.require_complete_accounting = false;
        gates.maximum_pending_non_funding_actions_per_fold = 0;
        gates.maximum_terminal_pending_orders_per_run = 0;
        gates.maximum_terminal_pending_cancel_requests_per_run = 0;
        gates.minimum_test_funding_settlements_per_fold = 1;
        gates.maximum_test_abs_pending_delta_usd = 10.0;
        gates.maximum_test_final_abs_pending_delta_usd = 10.0;
        gates.maximum_test_active_orders = 2;
        gates.maximum_test_active_order_notional_usd = 100.0;
        gates.maximum_test_final_active_order_notional_usd = 100.0;
        let aggregate = RunAggregate {
            maximum_abs_pending_delta_usd: 11.0,
            maximum_final_abs_pending_delta_usd: 12.0,
            maximum_active_orders: 3,
            maximum_active_order_notional_usd: 101.0,
            maximum_final_active_order_notional_usd: 102.0,
            pending_non_funding_actions: 1,
            maximum_terminal_pending_orders: 1,
            maximum_terminal_pending_cancel_requests: 1,
            final_valuation_complete: true,
            ..RunAggregate::default()
        };
        let manifest = manifest();
        let scenario = &manifest.scenarios[0];

        let (evidence, performance) = test_failures(&[], &aggregate, scenario, &gates);

        assert!(
            evidence
                .iter()
                .any(|failure| failure.contains("exchange orders remain pending"))
        );
        assert!(
            evidence
                .iter()
                .any(|failure| failure.contains("cancel requests remain pending"))
        );
        assert!(
            evidence
                .iter()
                .any(|failure| failure.contains("test funding settlements 0 below 1"))
        );
        assert!(
            performance
                .iter()
                .any(|failure| failure.contains("maximum absolute pending delta"))
        );
        assert!(
            performance
                .iter()
                .any(|failure| failure.contains("maximum active orders"))
        );
        assert!(
            performance
                .iter()
                .any(|failure| failure.contains("final active-order notional"))
        );
    }

    #[test]
    fn checked_in_smoke_manifest_runs_end_to_end() {
        let manifest =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/research-smoke.toml");

        let report = run_research_manifest_path(manifest).unwrap();

        assert!(report.passed, "{:?}", report.failures);
        assert_eq!(report.aggregate.folds, 1);
        assert_eq!(report.aggregate.stress_scenarios, 1);
        assert_eq!(report.selection_metric, SelectionMetric::NetPnlUsd);
        assert_eq!(report.deployment_candidate_id, None);
        assert_eq!(report.gates.minimum_folds, 1);
        assert_eq!(report.folds[0].train_dataset_ids, ["train-fixture"]);
        assert_eq!(report.folds[0].test_dataset_ids, ["test-fixture"]);
        assert_eq!(
            report.folds[0].selected_candidate_id.as_deref(),
            Some("base")
        );
        assert_eq!(report.java_reference_revision, PINNED_JAVA_REVISION);
        assert_eq!(report.manifest_sha256.len(), 64);
        assert_eq!(report.executable_sha256.len(), 64);
        assert_eq!(report.candidates[0].effective_strategy_sha256.len(), 64);
        assert!(
            report
                .datasets
                .iter()
                .all(|dataset| dataset.raw_replay_check.is_none())
        );
        assert!(
            report
                .datasets
                .iter()
                .all(|dataset| dataset.capture_analysis.is_none())
        );
        let selected = report.folds[0].selected_candidate_id.as_deref().unwrap();
        let train = &report.folds[0]
            .training
            .iter()
            .find(|candidate| candidate.candidate_id == selected)
            .unwrap()
            .runs;
        let test = &report.folds[0]
            .test_scenarios
            .iter()
            .find(|scenario| scenario.kind == ResearchScenarioKind::Baseline)
            .unwrap()
            .runs;
        assert!(chronology_failures(train, test).is_empty());
        assert!(
            chronology_failures(test, train)
                .iter()
                .any(|failure| failure.contains("training ends"))
        );
    }
}
