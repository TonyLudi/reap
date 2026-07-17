use crate::BacktestExecutionConfig;

use super::{
    CandidateTrainingReport, FoldReport, ResearchAggregate, ResearchRunReport,
    ResearchScenarioKind, RunAggregate, SelectionMetric,
};

impl RunAggregate {
    pub(super) fn from_runs(runs: &[ResearchRunReport]) -> Self {
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
            if let Some(net_pnl_usd) = report.net_pnl_usd {
                aggregate.net_pnl_usd += net_pnl_usd;
            } else {
                aggregate.accounting_complete = false;
            }
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
            aggregate.strategy_halts = aggregate
                .strategy_halts
                .saturating_add(usize::from(report.strategy_halt_reason.is_some()));
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
    pub(super) fn from_folds(folds: &[FoldReport]) -> Self {
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

pub(super) fn chronology_failures(
    train: &[ResearchRunReport],
    test: &[ResearchRunReport],
) -> Vec<String> {
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

pub(super) fn cross_fold_chronology_failures(folds: &[FoldReport]) -> Vec<String> {
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

pub(super) fn select_training_candidate(
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

pub(super) fn selection_score(aggregate: &RunAggregate, metric: SelectionMetric) -> Option<f64> {
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

pub(super) fn no_less_conservative(
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

pub(super) fn deployment_selection_failure(
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
