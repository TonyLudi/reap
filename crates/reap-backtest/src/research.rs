use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use reap_capture::{
    CaptureAnalysisReport, CaptureConfig, CaptureVerificationReport, analyze_capture_path,
    verify_capture_paths,
};
use reap_feed::{ReplayCheckReport, replay_check_path};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    BacktestConfig, BacktestExecutionConfig, BacktestReport, BacktestRunner,
    LatencyCalibrationArtifact, MAX_LATENCY_CALIBRATION_ARTIFACT_BYTES,
};

pub const RESEARCH_SCHEMA_VERSION: u32 = 5;
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
    pub capture_config: Option<PathBuf>,
    #[serde(default)]
    pub capture_report: Option<PathBuf>,
    #[serde(default)]
    pub normalized_path: Option<PathBuf>,
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
    validate_candidate_funding_evidence(
        manifest.mode,
        &manifest.gates,
        candidates.iter().map(|candidate| &candidate.config),
    )?;
    validate_scenario_currency_rates(&manifest.scenarios, &candidates)?;
    let datasets = load_datasets(&manifest.datasets, base, manifest.mode, &candidates)?;
    let manifest_sha256 = sha256_bytes(&manifest_bytes);
    let mut cache = HashMap::new();
    let mut folds = Vec::with_capacity(manifest.folds.len());

    for fold in &manifest.folds {
        let mut training = Vec::with_capacity(candidates.len());
        for candidate in &candidates {
            let runs = fold
                .train
                .iter()
                .map(|dataset_id| {
                    let dataset = find_dataset(&datasets, dataset_id);
                    cached_run(&mut cache, candidate, dataset, baseline)
                })
                .collect::<Vec<_>>();
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
            for scenario in &manifest.scenarios {
                let runs = fold
                    .test
                    .iter()
                    .map(|dataset_id| {
                        let dataset = find_dataset(&datasets, dataset_id);
                        cached_run(&mut cache, candidate, dataset, scenario)
                    })
                    .collect::<Vec<_>>();
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

            let selected_train = training
                .iter()
                .find(|candidate| candidate.candidate_id == selected_candidate_id)
                .expect("selected training report must exist");
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
        dataset_portfolio_semantics: DatasetPortfolioSemantics::IndependentZeroInitialPortfolio,
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
                    || dataset.normalized_path.is_some())
            {
                errors.push(format!(
                    "dataset {}: capture_config, capture_report, and normalized_path are valid only for raw_capture datasets",
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
        if mode == ResearchMode::ProductionCandidate {
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

impl RunAggregate {
    fn from_runs(runs: &[ResearchRunReport]) -> Self {
        let mut aggregate = Self {
            runs: runs.len(),
            accounting_complete: true,
            final_valuation_complete: true,
            execution_calibrated: true,
            ..Self::default()
        };
        let mut abs_delta_integral = 0.0;
        for run in runs {
            let Some(report) = &run.report else {
                aggregate.accounting_complete = false;
                aggregate.final_valuation_complete = false;
                aggregate.execution_calibrated = false;
                continue;
            };
            aggregate.successful_runs += 1;
            aggregate.input_events = aggregate.input_events.saturating_add(report.input_events);
            aggregate.observed_duration_ns = aggregate
                .observed_duration_ns
                .saturating_add(report.observed_duration_ns);
            aggregate.fills = aggregate.fills.saturating_add(report.fills);
            aggregate.net_pnl_usd += report.final_equity_usd;
            aggregate.fee_cost_usd += report.fee_cost_usd;
            aggregate.exact_fee_fills = aggregate
                .exact_fee_fills
                .saturating_add(report.exact_fee_fills);
            aggregate.estimated_fee_fills = aggregate
                .estimated_fee_fills
                .saturating_add(report.estimated_fee_fills);
            aggregate.funding_pnl_usd += report.funding_pnl_usd;
            aggregate.funding_settlements = aggregate
                .funding_settlements
                .saturating_add(report.funding_settlements);
            aggregate.turnover_usd += report.turnover_usd;
            aggregate.maximum_drawdown_usd =
                aggregate.maximum_drawdown_usd.max(report.max_drawdown_usd);
            aggregate.maximum_abs_delta_usd = aggregate
                .maximum_abs_delta_usd
                .max(report.max_abs_delta_usd);
            aggregate.maximum_final_abs_delta_usd = aggregate
                .maximum_final_abs_delta_usd
                .max(report.final_delta_usd.abs());
            aggregate.maximum_abs_pending_delta_usd = aggregate
                .maximum_abs_pending_delta_usd
                .max(report.max_abs_pending_delta_usd);
            aggregate.maximum_final_abs_pending_delta_usd = aggregate
                .maximum_final_abs_pending_delta_usd
                .max(report.final_pending_delta_usd.abs());
            aggregate.maximum_gross_exposure_usd = aggregate
                .maximum_gross_exposure_usd
                .max(report.max_gross_exposure_usd);
            aggregate.maximum_final_gross_exposure_usd = aggregate
                .maximum_final_gross_exposure_usd
                .max(report.final_gross_exposure_usd);
            aggregate.maximum_active_orders = aggregate
                .maximum_active_orders
                .max(report.max_active_orders);
            aggregate.maximum_active_order_notional_usd = aggregate
                .maximum_active_order_notional_usd
                .max(report.max_active_order_notional_usd);
            aggregate.maximum_final_active_order_notional_usd = aggregate
                .maximum_final_active_order_notional_usd
                .max(report.final_active_order_notional_usd);
            abs_delta_integral += report.average_abs_delta_usd * report.observed_duration_ns as f64;
            aggregate.inventory_open_duration_ns = aggregate
                .inventory_open_duration_ns
                .saturating_add(report.inventory_open_duration_ns);
            aggregate.clock_regressions = aggregate
                .clock_regressions
                .saturating_add(report.input_clock_regressions);
            aggregate.pending_non_funding_actions = aggregate
                .pending_non_funding_actions
                .saturating_add(report.pending_activation_actions)
                .saturating_add(report.pending_cancel_actions)
                .saturating_add(report.pending_order_update_actions)
                .saturating_add(report.pending_strategy_event_actions);
            aggregate.maximum_terminal_pending_orders = aggregate
                .maximum_terminal_pending_orders
                .max(report.pending_orders);
            aggregate.maximum_terminal_pending_cancel_requests = aggregate
                .maximum_terminal_pending_cancel_requests
                .max(report.pending_cancel_requests);
            aggregate.accounting_complete &= report.accounting_complete;
            aggregate.final_valuation_complete &= report.final_valuation_complete;
            aggregate.execution_calibrated &= report.execution.calibrated;
            aggregate.first_arrival_ns =
                min_option(aggregate.first_arrival_ns, report.first_arrival_ns);
            aggregate.last_arrival_ns =
                max_option(aggregate.last_arrival_ns, report.last_arrival_ns);
        }
        if aggregate.observed_duration_ns > 0 {
            aggregate.average_abs_delta_usd =
                abs_delta_integral / aggregate.observed_duration_ns as f64;
            aggregate.inventory_open_fraction =
                aggregate.inventory_open_duration_ns as f64 / aggregate.observed_duration_ns as f64;
        }
        aggregate
    }
}

impl ResearchAggregate {
    fn from_folds(folds: &[FoldReport]) -> Self {
        let mut aggregate = Self {
            folds: folds.len(),
            ..Self::default()
        };
        for fold in folds {
            aggregate.evidence_complete_folds += usize::from(fold.evidence_complete);
            aggregate.passing_folds += usize::from(fold.passed);
            if let Some(baseline) = fold
                .test_scenarios
                .iter()
                .find(|scenario| scenario.kind == ResearchScenarioKind::Baseline)
            {
                aggregate.total_baseline_test_pnl_usd += baseline.aggregate.net_pnl_usd;
                aggregate.profitable_baseline_folds +=
                    usize::from(baseline.aggregate.net_pnl_usd > 0.0);
            }
            for stress in fold
                .test_scenarios
                .iter()
                .filter(|scenario| scenario.kind == ResearchScenarioKind::Stress)
            {
                aggregate.stress_scenarios += 1;
                aggregate.passing_stress_scenarios += usize::from(stress.passed);
            }
        }
        if aggregate.folds > 0 {
            aggregate.passing_fold_fraction =
                aggregate.passing_folds as f64 / aggregate.folds as f64;
            aggregate.profitable_fold_fraction =
                aggregate.profitable_baseline_folds as f64 / aggregate.folds as f64;
        }
        if aggregate.stress_scenarios > 0 {
            aggregate.stress_pass_fraction =
                aggregate.passing_stress_scenarios as f64 / aggregate.stress_scenarios as f64;
        }
        aggregate
    }
}

fn load_candidates(specs: &[ResearchCandidate], base: &Path) -> Result<Vec<LoadedCandidate>> {
    let mut loaded = Vec::with_capacity(specs.len());
    let mut canonical_paths = HashSet::new();
    let mut hashes = HashSet::new();
    let mut effective_strategy_hashes = HashSet::new();
    for spec in specs {
        let resolved = resolve(base, &spec.config);
        let canonical = resolved.canonicalize().with_context(|| {
            format!("failed to resolve candidate config {}", resolved.display())
        })?;
        if !canonical_paths.insert(canonical.clone()) {
            bail!(
                "candidate config {} is referenced more than once",
                spec.config.display()
            );
        }
        let bytes = std::fs::read(&canonical)
            .with_context(|| format!("failed to read candidate config {}", canonical.display()))?;
        let sha256 = sha256_bytes(&bytes);
        if !hashes.insert(sha256.clone()) {
            bail!(
                "candidate {} duplicates another candidate's config bytes",
                spec.id
            );
        }
        let config: BacktestConfig = toml::from_str(
            std::str::from_utf8(&bytes).context("candidate config is not UTF-8")?,
        )
        .with_context(|| format!("failed to parse candidate config {}", canonical.display()))?;
        config.backtest.validate()?;
        super::validate_currency_rate_coverage(&config.backtest, &config.strategy)?;
        let validation = config.strategy.effective().validate();
        if !validation.valid {
            bail!(
                "candidate {} has invalid strategy config: {}",
                spec.id,
                validation.errors.join("; ")
            );
        }
        let effective_strategy_sha256 = effective_strategy_sha256(&config)?;
        if !effective_strategy_hashes.insert(effective_strategy_sha256.clone()) {
            bail!(
                "candidate {} duplicates another candidate's effective strategy",
                spec.id
            );
        }
        loaded.push(LoadedCandidate {
            spec: spec.clone(),
            resolved_path: canonical,
            config,
            sha256,
            effective_strategy_sha256,
        });
    }
    Ok(loaded)
}

fn validate_candidate_funding_evidence<'a>(
    mode: ResearchMode,
    gates: &ResearchGates,
    candidates: impl IntoIterator<Item = &'a BacktestConfig>,
) -> Result<()> {
    let has_swap = candidates.into_iter().any(|candidate| {
        candidate
            .strategy
            .instruments
            .iter()
            .any(|instrument| instrument.kind.is_swap())
    });
    if mode == ResearchMode::ProductionCandidate
        && has_swap
        && (gates.minimum_train_funding_settlements_per_fold == 0
            || gates.minimum_test_funding_settlements_per_fold == 0)
    {
        bail!(
            "production_candidate with swap instruments requires non-zero training and test funding-settlement evidence gates"
        );
    }
    Ok(())
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
) -> Result<Vec<LoadedDataset>> {
    let mut loaded = Vec::with_capacity(specs.len());
    let mut canonical_paths = HashSet::new();
    let mut hashes = HashSet::new();
    for spec in specs {
        let resolved = resolve(base, &spec.path);
        let canonical = resolved
            .canonicalize()
            .with_context(|| format!("failed to resolve dataset {}", resolved.display()))?;
        if !canonical_paths.insert(canonical.clone()) {
            bail!(
                "dataset path {} is referenced more than once",
                spec.path.display()
            );
        }
        let sha256 = sha256_path(&canonical)?;
        if !hashes.insert(sha256.clone()) {
            bail!("dataset {} duplicates another dataset's bytes", spec.id);
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
        });
    }
    Ok(loaded)
}

fn validate_production_capture_config(
    dataset_id: &str,
    config: &CaptureConfig,
    candidates: &[LoadedCandidate],
) -> Result<()> {
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

fn verify_input_hashes(
    manifest_path: &Path,
    manifest_sha256: &str,
    executable_path: &Path,
    executable_sha256: &str,
    candidates: &[LoadedCandidate],
    datasets: &[LoadedDataset],
    latency_calibration: Option<&LoadedLatencyCalibration>,
) -> Result<()> {
    if sha256_path(manifest_path)? != manifest_sha256 {
        bail!("research manifest changed while research was running");
    }
    if sha256_path(executable_path)? != executable_sha256 {
        bail!("research executable changed while research was running");
    }
    for candidate in candidates {
        if sha256_path(&candidate.resolved_path)? != candidate.sha256 {
            bail!(
                "candidate config {} changed while research was running",
                candidate.spec.id
            );
        }
    }
    for dataset in datasets {
        let final_sha256 = sha256_path(&dataset.resolved_path)?;
        if final_sha256 != dataset.sha256 {
            bail!(
                "dataset {} changed while research was running",
                dataset.spec.id
            );
        }
        if let (Some(config_path), Some(expected_sha256)) = (
            &dataset.resolved_capture_config,
            &dataset.capture_config_sha256,
        ) {
            let final_config_sha256 = sha256_path(config_path)?;
            if &final_config_sha256 != expected_sha256 {
                bail!(
                    "capture config for dataset {} changed while research was running",
                    dataset.spec.id
                );
            }
        }
        if let (Some(report_path), Some(expected_sha256)) = (
            &dataset.resolved_capture_report,
            &dataset.capture_report_sha256,
        ) {
            let final_report_sha256 = sha256_path(report_path)?;
            if &final_report_sha256 != expected_sha256 {
                bail!(
                    "capture report for dataset {} changed while research was running",
                    dataset.spec.id
                );
            }
        }
        if let (Some(normalized_path), Some(expected_sha256)) = (
            &dataset.resolved_normalized_path,
            &dataset.normalized_sha256,
        ) {
            let final_normalized_sha256 = sha256_path(normalized_path)?;
            if &final_normalized_sha256 != expected_sha256 {
                bail!(
                    "normalized capture for dataset {} changed while research was running",
                    dataset.spec.id
                );
            }
        }
    }
    if let Some(calibration) = latency_calibration
        && sha256_path(&calibration.resolved_path)? != calibration.provenance.sha256
    {
        bail!("latency calibration changed while research was running");
    }
    Ok(())
}

fn cached_run(
    cache: &mut HashMap<(String, String, String), ResearchRunReport>,
    candidate: &LoadedCandidate,
    dataset: &LoadedDataset,
    scenario: &ResearchScenario,
) -> ResearchRunReport {
    let key = (
        candidate.spec.id.clone(),
        dataset.spec.id.clone(),
        scenario.id.clone(),
    );
    if let Some(run) = cache.get(&key) {
        return run.clone();
    }
    let mut config = candidate.config.clone();
    config.backtest = effective_scenario_execution(&config.backtest, &scenario.execution)
        .expect("scenario currency rates must be validated before research runs");
    let result = BacktestRunner::from_config(config).and_then(|runner| match dataset.spec.format {
        ResearchDataFormat::Csv => runner.run_csv_path(&dataset.resolved_path),
        ResearchDataFormat::NormalizedJsonl => {
            runner.run_normalized_jsonl_path(&dataset.resolved_path)
        }
        ResearchDataFormat::RawCapture => runner.run_raw_capture_path(&dataset.resolved_path),
    });
    let (report, error) = match result {
        Ok(report) => (Some(report), None),
        Err(error) => (None, Some(format!("{error:#}"))),
    };
    let run = ResearchRunReport {
        candidate_id: candidate.spec.id.clone(),
        dataset_id: dataset.spec.id.clone(),
        scenario_id: scenario.id.clone(),
        report,
        error,
    };
    cache.insert(key, run.clone());
    run
}

fn validate_scenario_currency_rates(
    scenarios: &[ResearchScenario],
    candidates: &[LoadedCandidate],
) -> Result<()> {
    for scenario in scenarios {
        for candidate in candidates {
            effective_scenario_execution(&candidate.config.backtest, &scenario.execution)
                .with_context(|| {
                    format!(
                        "scenario {} currency valuation conflicts with candidate {}",
                        scenario.id, candidate.spec.id
                    )
                })?;
        }
    }
    Ok(())
}

fn effective_scenario_execution(
    candidate: &BacktestExecutionConfig,
    scenario: &BacktestExecutionConfig,
) -> Result<BacktestExecutionConfig> {
    let mut effective = scenario.clone();
    if effective.currency_rates.is_empty() {
        effective.currency_rates = candidate.currency_rates.clone();
        return Ok(effective);
    }

    let mut candidate_rates = candidate.currency_rates.clone();
    let mut scenario_rates = effective.currency_rates.clone();
    candidate_rates.sort_by(|left, right| left.currency.cmp(&right.currency));
    scenario_rates.sort_by(|left, right| left.currency.cmp(&right.currency));
    if scenario_rates != candidate_rates {
        bail!(
            "scenario currency_rates must be empty to inherit the candidate or exactly match the candidate routes"
        );
    }
    effective.currency_rates = candidate_rates;
    Ok(effective)
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

fn chronology_failures(train: &[ResearchRunReport], test: &[ResearchRunReport]) -> Vec<String> {
    let mut failures = non_overlapping_failures("train", train);
    failures.extend(non_overlapping_failures("test", test));
    let train_last = train
        .iter()
        .filter_map(|run| run.report.as_ref()?.last_arrival_ns)
        .max();
    let test_first = test
        .iter()
        .filter_map(|run| run.report.as_ref()?.first_arrival_ns)
        .min();
    match (train_last, test_first) {
        (Some(train_last), Some(test_first)) if train_last < test_first => {}
        (Some(train_last), Some(test_first)) => failures.push(format!(
            "training ends at {train_last} ns but test begins at {test_first} ns"
        )),
        _ => failures.push("train/test arrival bounds are unavailable".to_string()),
    }
    failures
}

fn non_overlapping_failures(label: &str, runs: &[ResearchRunReport]) -> Vec<String> {
    let mut windows = runs
        .iter()
        .filter_map(|run| {
            let report = run.report.as_ref()?;
            Some((
                report.first_arrival_ns?,
                report.last_arrival_ns?,
                run.dataset_id.as_str(),
            ))
        })
        .collect::<Vec<_>>();
    windows.sort_by_key(|window| window.0);
    windows
        .windows(2)
        .filter(|pair| pair[0].1 >= pair[1].0)
        .map(|pair| {
            format!(
                "{label} datasets {} and {} overlap in event time",
                pair[0].2, pair[1].2
            )
        })
        .collect()
}

fn cross_fold_chronology_failures(folds: &[FoldReport]) -> Vec<String> {
    let mut previous: Option<(&str, u64)> = None;
    let mut failures = Vec::new();
    for fold in folds {
        let Some(baseline) = fold
            .test_scenarios
            .iter()
            .find(|scenario| scenario.kind == ResearchScenarioKind::Baseline)
        else {
            continue;
        };
        let first = baseline.aggregate.first_arrival_ns;
        let last = baseline.aggregate.last_arrival_ns;
        if let (Some(first), Some(last)) = (first, last) {
            if let Some((previous_id, previous_last)) = previous
                && previous_last >= first
            {
                failures.push(format!(
                    "test windows for folds {previous_id} and {} are not strictly chronological",
                    fold.id
                ));
            }
            previous = Some((&fold.id, last));
        }
    }
    failures
}

fn select_training_candidate(
    training: &[CandidateTrainingReport],
) -> Option<&CandidateTrainingReport> {
    let mut eligible = training
        .iter()
        .filter(|candidate| candidate.eligible && candidate.selection_score.is_some())
        .collect::<Vec<_>>();
    eligible.sort_by(|left, right| left.candidate_id.cmp(&right.candidate_id));
    eligible.into_iter().max_by(|left, right| {
        left.selection_score
            .expect("eligible score")
            .total_cmp(&right.selection_score.expect("eligible score"))
            .then_with(|| right.candidate_id.cmp(&left.candidate_id))
    })
}

fn selection_score(aggregate: &RunAggregate, metric: SelectionMetric) -> Option<f64> {
    let score = match metric {
        SelectionMetric::NetPnlUsd => aggregate.net_pnl_usd,
        SelectionMetric::PnlPerTurnoverBps => {
            if aggregate.turnover_usd <= 0.0 {
                return None;
            }
            aggregate.net_pnl_usd / aggregate.turnover_usd * 10_000.0
        }
    };
    score.is_finite().then_some(score)
}

fn no_less_conservative(
    stress: &BacktestExecutionConfig,
    baseline: &BacktestExecutionConfig,
) -> bool {
    stress.latency_is_no_less_conservative_than(baseline)
        && stress.depth_fill_conservative_threshold >= baseline.depth_fill_conservative_threshold
        && stress.queue_ahead_multiplier >= baseline.queue_ahead_multiplier
        && stress.historical_trade_fill_fraction <= baseline.historical_trade_fill_fraction
        && stress.displayed_depth_fill_fraction <= baseline.displayed_depth_fill_fraction
}

fn deployment_selection_failure(
    deployment_candidate_id: Option<&str>,
    selected_candidate_id: Option<&str>,
) -> Option<String> {
    let expected = deployment_candidate_id?;
    if selected_candidate_id == Some(expected) {
        None
    } else {
        Some(format!(
            "training selected candidate {} instead of predeclared deployment candidate {expected}",
            selected_candidate_id.unwrap_or("<none>")
        ))
    }
}

fn validate_named<'a>(
    kind: &str,
    values: impl Iterator<Item = &'a String>,
    errors: &mut Vec<String>,
) {
    let mut seen = HashSet::new();
    for value in values {
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            errors.push(format!(
                "{kind} id {value:?} must be 1-64 ASCII alphanumeric, '-' or '_' characters"
            ));
        }
        if !seen.insert(value) {
            errors.push(format!("duplicate {kind} id {value}"));
        }
    }
}

fn find_dataset<'a>(datasets: &'a [LoadedDataset], id: &str) -> &'a LoadedDataset {
    datasets
        .iter()
        .find(|dataset| dataset.spec.id == id)
        .expect("manifest validation requires every dataset id to exist")
}

fn resolve(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn min_option(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (left, right) => left.or(right),
    }
}

fn max_option(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left, right) => left.or(right),
    }
}

fn sha256_path(path: &Path) -> Result<String> {
    let file = File::open(path)
        .with_context(|| format!("failed to open {} for SHA-256", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .with_context(|| format!("failed to hash {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn effective_strategy_sha256(config: &BacktestConfig) -> Result<String> {
    let bytes = serde_json::to_vec(&config.strategy.effective())
        .context("failed to serialize effective candidate strategy")?;
    Ok(sha256_bytes(&bytes))
}

#[cfg(test)]
mod tests {
    use reap_capture::{
        CAPTURE_RUN_REPORT_FORMAT_VERSION, CaptureBookHealth, CaptureConfigFileEvidence,
        CaptureOutputConfig, CapturePriority, CaptureRunReport, CaptureRuntimeConfig,
        CaptureStopReason, CaptureSubscriptionConfig, CaptureVenueConfig, analyze_capture,
    };
    use tempfile::TempDir;

    use super::*;

    const RAW_CAPTURE_FIXTURE: &[u8] =
        include_bytes!("../../../fixtures/raw/okx/depth-reset.jsonl");

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

        let config = CaptureConfig {
            venue: CaptureVenueConfig::default(),
            runtime: CaptureRuntimeConfig::default(),
            output: CaptureOutputConfig::default(),
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

    fn fixture_dataset(fixture: &ResearchCaptureFixture) -> ResearchDataset {
        ResearchDataset {
            id: "capture".to_string(),
            path: fixture.raw_path.clone(),
            format: ResearchDataFormat::RawCapture,
            capture_config: Some(fixture.config_path.clone()),
            capture_report: Some(fixture.report_path.clone()),
            normalized_path: None,
        }
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
                    capture_config: None,
                    capture_report: None,
                    normalized_path: None,
                },
                ResearchDataset {
                    id: "test".to_string(),
                    path: "test.jsonl".into(),
                    format: ResearchDataFormat::NormalizedJsonl,
                    capture_config: None,
                    capture_report: None,
                    normalized_path: None,
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

        let error = manifest.validate().unwrap_err().to_string();

        assert!(error.contains("at least three folds"));
        assert!(error.contains("at least two stress scenarios"));
        assert!(error.contains("calibrated execution"));
        assert!(error.contains("predeclared deployment_candidate_id"));
        assert!(error.contains("capture_config for every dataset"));
        assert!(error.contains("capture_report for every dataset"));
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
    fn production_non_swap_candidates_do_not_require_funding_settlement_evidence() {
        let spot = BacktestConfig {
            strategy: reap_strategy::ChaosConfig {
                instruments: vec![reap_strategy::InstrumentConfig::default()],
                ..Default::default()
            },
            backtest: Default::default(),
        };

        validate_candidate_funding_evidence(ResearchMode::ProductionCandidate, &gates(), [&spot])
            .unwrap();
    }

    #[test]
    fn production_swap_candidates_accept_positive_funding_settlement_evidence() {
        let mut swap = BacktestConfig {
            strategy: Default::default(),
            backtest: Default::default(),
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
            capture_config: None,
            capture_report: None,
            normalized_path: None,
        }];

        let error = load_datasets(&datasets, &base, ResearchMode::ProductionCandidate, &[])
            .unwrap_err()
            .to_string();

        assert!(error.contains("failed zero-gap replay integrity"));
    }

    #[test]
    fn production_dataset_loads_verified_schema_three_capture_evidence() {
        let fixture = research_capture_fixture();
        let datasets = [fixture_dataset(&fixture)];

        let loaded = load_datasets(
            &datasets,
            Path::new("."),
            ResearchMode::ProductionCandidate,
            &[],
        )
        .unwrap();
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
    fn research_hash_guard_detects_capture_report_mutation() {
        let fixture = research_capture_fixture();
        let loaded = load_datasets(
            &[fixture_dataset(&fixture)],
            Path::new("."),
            ResearchMode::ProductionCandidate,
            &[],
        )
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

        let error = load_datasets(
            &[fixture_dataset(&fixture)],
            Path::new("."),
            ResearchMode::ProductionCandidate,
            &[],
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("failed capture verification"));
        assert!(error.contains("UnsupportedRunReportFormat"));
    }

    #[test]
    fn production_dataset_rejects_capture_config_byte_tampering() {
        let fixture = research_capture_fixture();
        let mut bytes = std::fs::read(&fixture.config_path).unwrap();
        bytes.extend_from_slice(b"\n# formatting-only tamper\n");
        std::fs::write(&fixture.config_path, bytes).unwrap();

        let error = load_datasets(
            &[fixture_dataset(&fixture)],
            Path::new("."),
            ResearchMode::ProductionCandidate,
            &[],
        )
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

        let error = load_datasets(
            &[fixture_dataset(&fixture)],
            Path::new("."),
            ResearchMode::ProductionCandidate,
            &[],
        )
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
        let config = CaptureConfig::load(base.join("examples/capture-okx-public.toml")).unwrap();

        validate_production_capture_config("capture", &config, &candidates).unwrap();

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
        let original = effective_strategy_sha256(&config).unwrap();

        config.backtest.order_entry_latency_ms = 999;
        assert_eq!(effective_strategy_sha256(&config).unwrap(), original);

        config.strategy.active_hedge_threshold_usd += 1.0;
        assert_ne!(effective_strategy_sha256(&config).unwrap(), original);
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
