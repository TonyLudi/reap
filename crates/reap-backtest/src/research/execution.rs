use std::collections::HashMap;

use anyhow::Result;

use crate::{BacktestCarryState, BacktestExecutionConfig, BacktestReport, BacktestRunner};

use super::configuration::effective_scenario_execution;
use super::{
    LoadedCandidate, LoadedDataset, ResearchDataFormat, ResearchRunReport, ResearchScenario,
};

pub(super) fn run_sequence(
    cache: &mut HashMap<(String, String, String), ResearchRunReport>,
    candidate: &LoadedCandidate,
    datasets: &[LoadedDataset],
    dataset_ids: &[String],
    scenario: &ResearchScenario,
    mut opening_carry: Option<(BacktestCarryState, BacktestExecutionConfig)>,
) -> Vec<ResearchRunReport> {
    let mut runs = Vec::with_capacity(dataset_ids.len());
    for (index, dataset_id) in dataset_ids.iter().enumerate() {
        let dataset = find_dataset(datasets, dataset_id);
        let continuation = dataset.spec.continuation_of.is_some();
        let carry = if continuation {
            if index == 0 {
                opening_carry.take()
            } else {
                runs.last()
                    .and_then(|run: &ResearchRunReport| run.report.as_ref())
                    .and_then(|report| {
                        report
                            .settled_carry_state
                            .clone()
                            .map(|carry| (carry, report.execution.clone()))
                    })
            }
        } else {
            None
        };
        if continuation && carry.is_none() {
            runs.push(ResearchRunReport {
                candidate_id: candidate.spec.id.clone(),
                dataset_id: dataset.spec.id.clone(),
                scenario_id: scenario.id.clone(),
                report: None,
                error: Some(format!(
                    "dataset {} requires settled carry from continuation parent {}",
                    dataset.spec.id,
                    dataset.spec.continuation_of.as_deref().unwrap_or_default()
                )),
            });
            continue;
        }
        runs.push(cached_run(cache, candidate, dataset, scenario, carry));
    }
    runs
}

fn cached_run(
    cache: &mut HashMap<(String, String, String), ResearchRunReport>,
    candidate: &LoadedCandidate,
    dataset: &LoadedDataset,
    scenario: &ResearchScenario,
    carry: Option<(BacktestCarryState, BacktestExecutionConfig)>,
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
    if let Some(opening_account) = &dataset.opening_account {
        config.initial_portfolio = opening_account.portfolio.clone();
    }
    config.backtest = effective_scenario_execution(&config.backtest, &scenario.execution)
        .expect("scenario currency rates must be validated before research runs");
    let result = (|| -> Result<BacktestReport> {
        let runner = if let Some((carry, source_execution)) = carry {
            let carry =
                carry.rebind_execution(&config.strategy, &source_execution, &config.backtest)?;
            BacktestRunner::from_config_with_carry(config, carry)?
        } else {
            BacktestRunner::from_config(config)?
        };
        match dataset.spec.format {
            ResearchDataFormat::Csv => runner.run_csv_path(&dataset.resolved_path),
            ResearchDataFormat::NormalizedJsonl => {
                runner.run_normalized_jsonl_path(&dataset.resolved_path)
            }
            ResearchDataFormat::RawCapture => {
                if let Some(range) = dataset.spec.capture_record_range {
                    runner.run_raw_capture_range_path(&dataset.resolved_path, range)
                } else {
                    runner.run_raw_capture_path(&dataset.resolved_path)
                }
            }
        }
    })();
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

pub(super) fn find_dataset<'a>(datasets: &'a [LoadedDataset], id: &str) -> &'a LoadedDataset {
    datasets
        .iter()
        .find(|dataset| dataset.spec.id == id)
        .expect("manifest validation requires every dataset id to exist")
}
