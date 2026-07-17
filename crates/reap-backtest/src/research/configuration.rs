use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::{BacktestConfig, BacktestExecutionConfig};

use super::{
    LoadedCandidate, MAX_PRODUCTION_OPENING_ACCOUNT_GAP_MS, PINNED_JAVA_REVISION,
    RESEARCH_SCHEMA_VERSION, ResearchDataFormat, ResearchGates, ResearchManifest, ResearchMode,
    ResearchScenario, ResearchScenarioKind,
};

pub(super) fn validate_candidate_initial_portfolios(
    mode: ResearchMode,
    uses_certified_opening_accounts: bool,
    candidates: &[LoadedCandidate],
) -> Result<()> {
    if uses_certified_opening_accounts || mode == ResearchMode::ProductionCandidate {
        if let Some(candidate) = candidates
            .iter()
            .find(|candidate| !candidate.config.initial_portfolio.is_empty())
        {
            bail!(
                "candidate {} must omit initial_portfolio when datasets derive opening state from account certification",
                candidate.spec.id
            );
        }
        return Ok(());
    }
    let Some(first) = candidates.first() else {
        return Ok(());
    };
    for candidate in &candidates[1..] {
        if candidate.config.initial_portfolio != first.config.initial_portfolio {
            bail!(
                "candidate {} initial_portfolio differs from candidate {}; research candidates must use identical opening capital and inventory",
                candidate.spec.id,
                first.spec.id
            );
        }
    }
    Ok(())
}

pub(super) fn validate_candidate_funding_evidence<'a>(
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

pub(super) fn validate_scenario_currency_rates(
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

pub(super) fn effective_scenario_execution(
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

fn no_less_conservative(
    stress: &BacktestExecutionConfig,
    baseline: &BacktestExecutionConfig,
) -> bool {
    stress.latency_is_no_less_conservative_than(baseline)
        && stress.depth_fill_conservative_threshold >= baseline.depth_fill_conservative_threshold
        && stress.queue_ahead_multiplier >= baseline.queue_ahead_multiplier
        && stress.historical_trade_fill_fraction <= baseline.historical_trade_fill_fraction
        && stress.displayed_depth_fill_fraction <= baseline.displayed_depth_fill_fraction
        && stress.derivative_leverage <= baseline.derivative_leverage
        && stress.exchange_cmr_multiplier <= baseline.exchange_cmr_multiplier
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

pub(super) fn resolve(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
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
