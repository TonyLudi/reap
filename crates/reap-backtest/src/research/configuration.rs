use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::{BacktestConfig, BacktestExecutionConfig};

use super::{LoadedCandidate, ResearchGates, ResearchMode, ResearchScenario};

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

pub(super) fn validate_named<'a>(
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
