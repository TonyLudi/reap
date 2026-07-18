mod calibration;
mod execution;
mod matching;
mod portfolio;
mod replay;
mod research;
mod research_verification;

pub use calibration::{
    LATENCY_CALIBRATION_SCHEMA_VERSION, LatencyCalibrationArtifact, LatencyCalibrationSeries,
    LatencySourceReport, MAX_LATENCY_CALIBRATION_ARTIFACT_BYTES,
    MAX_LATENCY_CALIBRATION_RETAINED_INPUT_SAMPLES, MAX_LATENCY_CALIBRATION_SOURCE_REPORTS,
};
use execution::BacktestLatencySampler;
pub use execution::{
    BacktestConfig, BacktestCurrencyRateConfig, BacktestExecutionConfig,
    BacktestInitialBalanceConfig, BacktestInitialMarginConfig, BacktestInitialPortfolioConfig,
    BacktestInitialPositionConfig, BacktestLatencyClass, BacktestLatencyProfile,
    BacktestLatencyRule, BacktestLatencyUsage, BacktestTimeBasis,
};
use matching::MatchingAssumptions;
pub use matching::MatchingEngine;
pub use replay::{
    RawCaptureRecordRange, RawReplayBoundary, ReplayRow, TimedReplayEvent, load_events_from_path,
    load_normalized_jsonl, load_normalized_jsonl_from_path, replay_raw_capture,
    replay_raw_capture_path, replay_raw_capture_timed, replay_raw_capture_timed_path,
    replay_raw_capture_timed_path_with_boundary, replay_raw_capture_timed_range,
    replay_raw_capture_timed_range_path,
};
pub use research::{
    CandidateProvenance, CandidateTrainingReport, DatasetPortfolioSemantics, DatasetProvenance,
    FoldReport, LatencyCalibrationProvenance, OpeningAccountProvenance, PINNED_JAVA_REVISION,
    RESEARCH_SCHEMA_VERSION, ResearchAggregate, ResearchCandidate, ResearchDataFormat,
    ResearchDataset, ResearchFold, ResearchGates, ResearchManifest, ResearchMode,
    ResearchOpeningAccount, ResearchReport, ResearchRunReport, ResearchScenario,
    ResearchScenarioKind, RunAggregate, SelectionMetric, TestScenarioReport,
    effective_strategy_sha256, run_research_manifest_path,
};
pub use research_verification::{
    MAX_RESEARCH_MANIFEST_BYTES, MAX_RESEARCH_REPORT_BYTES, RESEARCH_VERIFICATION_FORMAT_VERSION,
    ResearchFileEvidence, ResearchOpeningAccountEvidence, ResearchVerificationFailure,
    ResearchVerificationReport, verify_research_paths,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::portfolio::Portfolio;
use reap_core::{AccountUpdate, MarketEvent, OrderUpdate, StrategyEvent, Symbol};
#[cfg(test)]
use reap_core::{FillLiquidity, FundingSettlement, NormalizedEvent, OrderEvent, OrderIntent};
use reap_strategy::{ChaosConfig, ChaosStrategy, Strategy};

const MAX_ACTIONS_PER_DRAIN: usize = 1_000_000;
const NS_PER_MS: u64 = 1_000_000;
const FUNDING_LATE_TOLERANCE_NS: u64 = 60_000 * NS_PER_MS;
const ACCOUNT_REFRESH_INTERVAL_NS: u64 = 10_000 * NS_PER_MS;
pub const BACKTEST_CARRY_STATE_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestCurrencyRateReport {
    pub currency: String,
    pub index_symbol: Symbol,
    pub usd_per_unit: Option<f64>,
    pub source_ts_ms: Option<u64>,
    pub effective_at_ns: Option<u64>,
    pub age_ms: Option<u64>,
    pub max_age_ms: u64,
    pub usable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BacktestCarryCurrencyRate {
    pub currency: String,
    pub index_symbol: Symbol,
    pub usd_per_unit: f64,
    pub source_ts_ms: u64,
    pub effective_at_ns: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BacktestPendingFundingCarry {
    pub symbol: Symbol,
    pub funding_time_ms: u64,
    pub due_at_ns: u64,
    pub realized_rate: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BacktestCarryState {
    pub schema_version: u16,
    pub settled_at_ns: u64,
    pub terminal_equity_usd: f64,
    pub source_raw_boundary: Option<RawReplayBoundary>,
    pub portfolio: BacktestInitialPortfolioConfig,
    pub terminal_depth_marks: BTreeMap<Symbol, f64>,
    pub terminal_exchange_marks: BTreeMap<Symbol, f64>,
    pub currency_rates: Vec<BacktestCarryCurrencyRate>,
    pub pending_funding: Vec<BacktestPendingFundingCarry>,
    pub last_settled_funding_time_ms: BTreeMap<Symbol, u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestReport {
    pub execution: BacktestExecutionConfig,
    #[serde(default)]
    pub initial_portfolio: BacktestInitialPortfolioConfig,
    pub latency_usage: Vec<BacktestLatencyUsage>,
    pub time_basis: BacktestTimeBasis,
    #[serde(default)]
    pub raw_replay_boundary: Option<RawReplayBoundary>,
    pub input_events: u64,
    pub first_arrival_ns: Option<u64>,
    pub last_arrival_ns: Option<u64>,
    pub input_clock_regressions: u64,
    pub max_input_clock_regression_ns: u64,
    #[serde(default)]
    pub order_entry_ready_at_ns: Option<u64>,
    #[serde(default)]
    pub order_entry_ready_at_end: bool,
    #[serde(default)]
    pub new_orders_blocked_not_ready: usize,
    #[serde(default)]
    pub strategy_halt_reason: Option<String>,
    pub orders_sent: usize,
    pub cancel_requests: usize,
    pub deduplicated_cancel_requests: usize,
    pub ignored_cancel_requests: usize,
    pub exchange_activations: usize,
    pub cancelled_orders: usize,
    pub rejected_orders: usize,
    pub fills: usize,
    pub maker_fills: usize,
    pub taker_fills: usize,
    pub pending_scheduled_actions: usize,
    pub pending_activation_actions: usize,
    pub pending_cancel_actions: usize,
    pub pending_order_update_actions: usize,
    pub pending_strategy_event_actions: usize,
    pub pending_funding_actions: usize,
    pub periodic_account_refreshes: u64,
    pub pending_orders: usize,
    pub live_orders: usize,
    pub pending_cancel_requests: usize,
    pub final_delta_usd: f64,
    pub final_pending_delta_usd: f64,
    pub final_active_order_notional_usd: f64,
    #[serde(default)]
    pub opening_equity_usd: Option<f64>,
    #[serde(default)]
    pub opening_valuation_at_ns: Option<u64>,
    #[serde(default)]
    pub opening_valuation_complete: bool,
    pub final_equity_usd: f64,
    #[serde(default)]
    pub net_pnl_usd: Option<f64>,
    pub final_valuation_complete: bool,
    pub final_gross_exposure_usd: f64,
    pub cash_usd: f64,
    #[serde(default)]
    pub cash_by_currency: BTreeMap<String, f64>,
    #[serde(default)]
    pub inverse_cash_coin_by_symbol: BTreeMap<Symbol, f64>,
    #[serde(default)]
    pub account_balances: BTreeMap<String, f64>,
    pub fee_cost_usd: f64,
    #[serde(default)]
    pub exact_fee_fills: u64,
    #[serde(default)]
    pub estimated_fee_fills: u64,
    pub funding_pnl_usd: f64,
    pub turnover_usd: f64,
    #[serde(default)]
    pub currency_rate_events: u64,
    #[serde(default)]
    pub invalid_currency_rate_events: u64,
    #[serde(default)]
    pub currency_conversion_failures: u64,
    #[serde(default)]
    pub invalid_accounting_events: u64,
    #[serde(default)]
    pub currency_rate_coverage_complete: bool,
    #[serde(default)]
    pub missing_currency_rates: Vec<String>,
    #[serde(default)]
    pub currency_rates: Vec<BacktestCurrencyRateReport>,
    pub observed_duration_ns: u64,
    pub max_drawdown_usd: f64,
    pub max_abs_delta_usd: f64,
    pub max_abs_pending_delta_usd: f64,
    pub max_gross_exposure_usd: f64,
    pub max_active_orders: usize,
    pub max_active_order_notional_usd: f64,
    pub average_abs_delta_usd: f64,
    pub inventory_open_duration_ns: u64,
    pub inventory_open_fraction: f64,
    pub risk_metric_samples: u64,
    pub invalid_risk_metric_samples: u64,
    pub funding_rate_events: u64,
    #[serde(default)]
    pub funding_settlement_observations: u64,
    pub funding_settlements: u64,
    pub late_funding_rate_events: u64,
    pub invalid_funding_rate_events: u64,
    pub missed_funding_settlements: u64,
    pub funding_settlement_failures: u64,
    pub accounting_complete: bool,
    #[serde(default)]
    pub settled_carry_state: Option<BacktestCarryState>,
    #[serde(default)]
    pub carry_state_failures: Vec<String>,
    pub positions: BTreeMap<Symbol, f64>,
    #[serde(default)]
    pub position_avg_prices: BTreeMap<Symbol, f64>,
}

#[path = "runner/accounting.rs"]
mod runner_accounting;
#[path = "runner/carry.rs"]
mod runner_carry;
#[path = "runner/construction.rs"]
mod runner_construction;
#[path = "runner/funding.rs"]
mod runner_funding;
#[path = "runner/input.rs"]
mod runner_input;
#[path = "runner/orders.rs"]
mod runner_orders;
use runner_construction::validate_currency_rate_coverage;

#[derive(Debug)]
enum ScheduledAction {
    ActivateOrder {
        symbol: Symbol,
        order_id: String,
    },
    CancelOrder {
        symbol: Symbol,
        order_id: String,
        reason: String,
    },
    DeliverOrder(OrderUpdate),
    DeliverAccount(AccountUpdate),
    DeliverStrategy(StrategyEvent),
    RefreshAccount,
    SettleFunding {
        symbol: Symbol,
        funding_time_ms: u64,
    },
}

#[derive(Debug, Clone, Copy)]
struct CurrencyRateObservation {
    usd_per_unit: f64,
    source_ts_ms: u64,
    effective_at_ns: u64,
}

pub struct BacktestRunner {
    strategy_config: ChaosConfig,
    strategy: ChaosStrategy,
    matchers: BTreeMap<Symbol, MatchingEngine>,
    portfolio: Portfolio,
    initial_portfolio: BacktestInitialPortfolioConfig,
    initial_account_snapshot_delivered: bool,
    execution: BacktestExecutionConfig,
    latency_sampler: BacktestLatencySampler,
    time_basis: BacktestTimeBasis,
    raw_replay_boundary: Option<RawReplayBoundary>,
    carry_source_boundary: Option<RawReplayBoundary>,
    scheduled: BTreeMap<(u64, u64), ScheduledAction>,
    next_action_seq: u64,
    pending_cancels: HashSet<String>,
    pending_fill_account_updates: usize,
    last_account_publish_ns: Option<u64>,
    periodic_account_refreshes: u64,
    depth_marks: HashMap<Symbol, f64>,
    exchange_marks: HashMap<Symbol, f64>,
    currency_by_index_symbol: HashMap<Symbol, String>,
    currency_rate_observations: HashMap<String, CurrencyRateObservation>,
    realized_funding_rates: HashMap<(Symbol, u64), f64>,
    scheduled_funding: HashSet<(Symbol, u64)>,
    settled_funding: HashSet<(Symbol, u64)>,
    last_settled_funding_time_ms: BTreeMap<Symbol, u64>,
    now_ns: u64,
    first_arrival_ns: Option<u64>,
    last_arrival_ns: Option<u64>,
    input_events: u64,
    input_clock_regressions: u64,
    max_input_clock_regression_ns: u64,
    order_entry_ready_at_ns: Option<u64>,
    new_orders_blocked_not_ready: usize,
    orders_sent: usize,
    cancel_requests: usize,
    deduplicated_cancel_requests: usize,
    ignored_cancel_requests: usize,
    exchange_activations: usize,
    cancelled_orders: usize,
    rejected_orders: usize,
    fills: usize,
    maker_fills: usize,
    taker_fills: usize,
    funding_rate_events: u64,
    funding_settlements: u64,
    late_funding_rate_events: u64,
    invalid_funding_rate_events: u64,
    missed_funding_settlements: u64,
    funding_settlement_failures: u64,
    currency_rate_events: u64,
    invalid_currency_rate_events: u64,
    opening_equity_usd: Option<f64>,
    opening_valuation_at_ns: Option<u64>,
    peak_equity_usd: f64,
    max_drawdown_usd: f64,
    max_abs_delta_usd: f64,
    max_abs_pending_delta_usd: f64,
    max_gross_exposure_usd: f64,
    max_active_orders: usize,
    max_active_order_notional_usd: f64,
    abs_delta_time_integral: f64,
    inventory_open_duration_ns: u64,
    metric_clock_ns: Option<u64>,
    current_abs_delta_usd: f64,
    current_inventory_open: bool,
    risk_metric_samples: u64,
    invalid_risk_metric_samples: u64,
}

impl BacktestRunner {
    fn execute_action(&mut self, action: ScheduledAction) -> Result<()> {
        match action {
            ScheduledAction::ActivateOrder { symbol, order_id } => {
                let now_ms = time_ms(self.now_ns);
                let updates = {
                    let matcher = self.matcher_mut(&symbol)?;
                    if !matcher.is_pending(&order_id) {
                        return Ok(());
                    }
                    matcher.activate(&order_id, now_ms)
                };
                self.exchange_activations += 1;
                self.route_exchange_updates(updates)?;
            }
            ScheduledAction::CancelOrder {
                symbol,
                order_id,
                reason,
            } => {
                self.pending_cancels.remove(&order_id);
                let now_ms = time_ms(self.now_ns);
                let updates = self
                    .matcher_mut(&symbol)?
                    .cancel_at(&order_id, now_ms, &reason);
                self.route_exchange_updates(updates)?;
            }
            ScheduledAction::DeliverOrder(update) => {
                let update = retime_order_update(update, time_ms(self.now_ns));
                let commands = self.strategy.on_event(&StrategyEvent::Order(update));
                self.accept_intents(commands)?;
            }
            ScheduledAction::DeliverAccount(update) => {
                self.pending_fill_account_updates =
                    self.pending_fill_account_updates.saturating_sub(1);
                let event =
                    retime_strategy_event(StrategyEvent::Account(update), time_ms(self.now_ns));
                let commands = self.strategy.on_event(&event);
                self.last_account_publish_ns = Some(self.now_ns);
                self.accept_intents(commands)?;
            }
            ScheduledAction::DeliverStrategy(event) => {
                let currency_rate = match &event {
                    StrategyEvent::Market(MarketEvent::IndexPrice {
                        ts_ms,
                        symbol,
                        price,
                    }) => Some((symbol.clone(), *price, *ts_ms)),
                    _ => None,
                };
                let event = retime_strategy_event(event, time_ms(self.now_ns));
                if let Some((symbol, price, source_ts_ms)) = currency_rate {
                    self.register_currency_rate(&symbol, price, source_ts_ms);
                }
                if matches!(event, StrategyEvent::Account(_)) {
                    self.last_account_publish_ns = Some(self.now_ns);
                }
                let commands = self.strategy.on_event(&event);
                self.accept_intents(commands)?;
            }
            ScheduledAction::RefreshAccount => {
                let due = self.last_account_publish_ns.is_some_and(|last| {
                    self.now_ns.saturating_sub(last) >= ACCOUNT_REFRESH_INTERVAL_NS
                });
                if due && self.pending_fill_account_updates == 0 {
                    let update = self.current_account_update(None);
                    let commands = self.strategy.on_event(&StrategyEvent::Account(update));
                    self.last_account_publish_ns = Some(self.now_ns);
                    self.periodic_account_refreshes =
                        self.periodic_account_refreshes.saturating_add(1);
                    self.accept_intents(commands)?;
                }
                self.schedule_next_account_refresh();
            }
            ScheduledAction::SettleFunding {
                symbol,
                funding_time_ms,
            } => self.settle_funding(symbol, funding_time_ms),
        }
        Ok(())
    }

    fn schedule_after(&mut self, delay_ms: u64, action: ScheduledAction) {
        let delay_ns = delay_ms.saturating_mul(NS_PER_MS);
        self.schedule_at(self.now_ns.saturating_add(delay_ns), action);
    }

    fn schedule_at(&mut self, due_ns: u64, action: ScheduledAction) {
        let seq = self.next_action_seq;
        self.next_action_seq = self.next_action_seq.saturating_add(1);
        self.scheduled.insert((due_ns, seq), action);
    }

    fn drain_before(&mut self, cutoff_ns: u64) -> Result<()> {
        self.drain_scheduled(cutoff_ns, false)
    }

    fn drain_through(&mut self, cutoff_ns: u64) -> Result<()> {
        self.drain_scheduled(cutoff_ns, true)
    }

    fn drain_scheduled(&mut self, cutoff_ns: u64, inclusive: bool) -> Result<()> {
        let mut processed = 0usize;
        while let Some((&(due_ns, _), _)) = self.scheduled.first_key_value() {
            if due_ns > cutoff_ns || (!inclusive && due_ns == cutoff_ns) {
                break;
            }
            let (_, action) = self
                .scheduled
                .pop_first()
                .expect("first scheduled action must still exist");
            let action_ns = self.now_ns.max(due_ns);
            self.advance_metric_clock(action_ns);
            self.now_ns = action_ns;
            self.execute_action(action)?;
            self.sample_risk_metrics();
            processed += 1;
            if processed > MAX_ACTIONS_PER_DRAIN {
                bail!(
                    "backtest exceeded {MAX_ACTIONS_PER_DRAIN} scheduled actions at {} ns",
                    self.now_ns
                );
            }
        }
        Ok(())
    }

    fn advance_metric_clock(&mut self, target_ns: u64) {
        if let Some(previous_ns) = self.metric_clock_ns {
            // Carry actions may run before the first input in the next replay segment.
            let observation_start_ns = self.first_arrival_ns.unwrap_or(target_ns);
            let elapsed_ns = target_ns.saturating_sub(previous_ns.max(observation_start_ns));
            self.abs_delta_time_integral += self.current_abs_delta_usd * elapsed_ns as f64;
            if self.current_inventory_open {
                self.inventory_open_duration_ns =
                    self.inventory_open_duration_ns.saturating_add(elapsed_ns);
            }
        }
        self.metric_clock_ns = Some(target_ns);
    }

    fn register_currency_rate(&mut self, index_symbol: &str, usd_per_unit: f64, source_ts_ms: u64) {
        let Some(currency) = self.currency_by_index_symbol.get(index_symbol).cloned() else {
            return;
        };
        self.currency_rate_events = self.currency_rate_events.saturating_add(1);
        if !usd_per_unit.is_finite() || usd_per_unit <= 0.0 {
            self.invalid_currency_rate_events = self.invalid_currency_rate_events.saturating_add(1);
            return;
        }
        self.currency_rate_observations.insert(
            currency,
            CurrencyRateObservation {
                usd_per_unit,
                source_ts_ms,
                effective_at_ns: self.now_ns,
            },
        );
    }

    fn fresh_currency_rates(&self) -> HashMap<String, f64> {
        let mut rates = HashMap::from([("USD".to_string(), 1.0)]);
        for route in &self.execution.currency_rates {
            let Some(observation) = self.currency_rate_observations.get(&route.currency) else {
                continue;
            };
            let maximum_age_ns = route.max_age_ms.saturating_mul(NS_PER_MS);
            let source_ns = observation.source_ts_ms.saturating_mul(NS_PER_MS);
            if self.now_ns.saturating_sub(source_ns) <= maximum_age_ns {
                rates.insert(route.currency.clone(), observation.usd_per_unit);
            }
        }
        rates
    }

    fn fallback_currency_rates(&self) -> HashMap<String, f64> {
        let mut rates = HashMap::from([("USD".to_string(), 1.0)]);
        for route in &self.execution.currency_rates {
            let rate = self
                .currency_rate_observations
                .get(&route.currency)
                .map(|observation| observation.usd_per_unit)
                .unwrap_or(1.0);
            rates.insert(route.currency.clone(), rate);
        }
        rates
    }

    fn currency_rate_reports(&self) -> Vec<BacktestCurrencyRateReport> {
        let mut reports = self
            .execution
            .currency_rates
            .iter()
            .map(|route| {
                let observation = self.currency_rate_observations.get(&route.currency);
                let age_ns = observation.map(|observation| {
                    self.now_ns
                        .saturating_sub(observation.source_ts_ms.saturating_mul(NS_PER_MS))
                });
                let usable = observation.is_some_and(|observation| {
                    observation.usd_per_unit.is_finite()
                        && observation.usd_per_unit > 0.0
                        && age_ns.unwrap_or(u64::MAX) <= route.max_age_ms.saturating_mul(NS_PER_MS)
                });
                BacktestCurrencyRateReport {
                    currency: route.currency.clone(),
                    index_symbol: route.index_symbol.clone(),
                    usd_per_unit: observation.map(|observation| observation.usd_per_unit),
                    source_ts_ms: observation.map(|observation| observation.source_ts_ms),
                    effective_at_ns: observation.map(|observation| observation.effective_at_ns),
                    age_ms: age_ns.map(|age_ns| age_ns.div_ceil(NS_PER_MS)),
                    max_age_ms: route.max_age_ms,
                    usable,
                }
            })
            .collect::<Vec<_>>();
        reports.sort_by(|left, right| left.currency.cmp(&right.currency));
        reports
    }

    fn active_order_notional_usd_checked(
        &self,
        currency_rates: &HashMap<String, f64>,
    ) -> Option<f64> {
        self.matchers
            .iter()
            .try_fold(0.0, |total, (symbol, matcher)| {
                let notional = matcher.active_order_notional_checked()?;
                if notional == 0.0 {
                    return Some(total);
                }
                let rate = self
                    .portfolio
                    .notional_currency_rate_usd_checked(symbol, currency_rates)?;
                let value = notional * rate;
                value.is_finite().then_some(total + value)
            })
            .filter(|notional| notional.is_finite())
    }

    fn active_order_notional_usd(&self, currency_rates: &HashMap<String, f64>) -> f64 {
        self.matchers
            .iter()
            .filter_map(|(symbol, matcher)| {
                matcher.active_order_notional_checked().map(|notional| {
                    notional
                        * self
                            .portfolio
                            .notional_currency_rate_usd(symbol, currency_rates)
                })
            })
            .sum()
    }

    fn sample_risk_metrics(&mut self) {
        self.current_inventory_open = self
            .portfolio
            .positions()
            .values()
            .any(|quantity| *quantity != 0.0);
        if self.opening_equity_usd.is_none() {
            return;
        }
        self.risk_metric_samples = self.risk_metric_samples.saturating_add(1);
        let mut valid = true;
        let marks = self.valuation_marks();
        let currency_rates = self.fresh_currency_rates();
        if let Some(equity_usd) = self.portfolio.equity_usd_checked(&marks, &currency_rates) {
            self.peak_equity_usd = self.peak_equity_usd.max(equity_usd);
            self.max_drawdown_usd = self.max_drawdown_usd.max(self.peak_equity_usd - equity_usd);
        } else {
            valid = false;
        }
        if let Some(gross_exposure_usd) = self
            .portfolio
            .gross_exposure_usd_checked(&marks, &currency_rates)
        {
            self.max_gross_exposure_usd = self.max_gross_exposure_usd.max(gross_exposure_usd);
        } else {
            valid = false;
        }

        let abs_delta_usd = self.strategy.delta_usd().abs();
        if abs_delta_usd.is_finite() {
            self.current_abs_delta_usd = abs_delta_usd;
            self.max_abs_delta_usd = self.max_abs_delta_usd.max(abs_delta_usd);
        } else {
            valid = false;
        }
        let abs_pending_delta_usd = self.strategy.pending_delta_usd().abs();
        if abs_pending_delta_usd.is_finite() {
            self.max_abs_pending_delta_usd =
                self.max_abs_pending_delta_usd.max(abs_pending_delta_usd);
        } else {
            valid = false;
        }
        let active_orders = self
            .matchers
            .values()
            .map(|matcher| matcher.pending_order_count() + matcher.live_order_count())
            .sum();
        self.max_active_orders = self.max_active_orders.max(active_orders);
        if let Some(active_order_notional_usd) =
            self.active_order_notional_usd_checked(&currency_rates)
        {
            self.max_active_order_notional_usd = self
                .max_active_order_notional_usd
                .max(active_order_notional_usd);
        } else {
            valid = false;
        }
        if !valid {
            self.invalid_risk_metric_samples = self.invalid_risk_metric_samples.saturating_add(1);
        }
    }

    fn order_entry_ready(&self) -> bool {
        self.valuation_inputs_ready() && self.opening_equity_usd.is_some()
    }

    fn valuation_inputs_ready(&self) -> bool {
        self.matchers
            .values()
            .all(|matcher| matcher.depth().is_some())
            && self.execution.currency_rates.iter().all(|route| {
                let Some(observation) = self.currency_rate_observations.get(&route.currency) else {
                    return false;
                };
                observation.usd_per_unit.is_finite()
                    && observation.usd_per_unit > 0.0
                    && self
                        .now_ns
                        .saturating_sub(observation.source_ts_ms.saturating_mul(NS_PER_MS))
                        <= route.max_age_ms.saturating_mul(NS_PER_MS)
            })
    }

    fn observe_order_entry_readiness(&mut self) {
        if self.opening_equity_usd.is_none() && self.valuation_inputs_ready() {
            let marks = self.valuation_marks();
            let currency_rates = self.fresh_currency_rates();
            if let Some(opening_equity_usd) =
                self.portfolio.equity_usd_checked(&marks, &currency_rates)
            {
                self.opening_equity_usd = Some(opening_equity_usd);
                self.opening_valuation_at_ns = Some(self.now_ns);
                self.peak_equity_usd = opening_equity_usd;
            }
        }
        if self.order_entry_ready_at_ns.is_none() && self.order_entry_ready() {
            self.order_entry_ready_at_ns = Some(self.now_ns);
        }
    }

    fn valuation_marks(&self) -> HashMap<Symbol, f64> {
        let mut marks = self
            .matchers
            .iter()
            .filter_map(|(symbol, matcher)| Some((symbol.clone(), matcher.depth()?.mid()?)))
            .collect::<HashMap<_, _>>();
        marks.extend(self.depth_marks.clone());
        marks.extend(self.exchange_marks.clone());
        marks
    }

    #[allow(clippy::too_many_arguments)]
    fn build_settled_carry_state(
        &self,
        marks: &HashMap<Symbol, f64>,
        currency_rates: &HashMap<String, f64>,
        terminal_equity_usd: Option<f64>,
        final_valuation_complete: bool,
        accounting_complete: bool,
    ) -> (Option<BacktestCarryState>, Vec<String>) {
        let mut failures = Vec::new();
        if self.initial_portfolio.is_empty() {
            failures.push("settled carry requires a non-empty opening portfolio".to_string());
        }
        if !final_valuation_complete || terminal_equity_usd.is_none() {
            failures.push("settled carry requires complete terminal valuation".to_string());
        }
        if !accounting_complete {
            failures.push("settled carry requires complete accounting".to_string());
        }
        if let Some(reason) = self.strategy.halt_reason() {
            failures.push(format!(
                "settled carry is unavailable after terminal strategy halt: {reason}"
            ));
        }
        if !failures.is_empty() {
            return (None, failures);
        }

        let build = (|| -> Result<BacktestCarryState> {
            let portfolio = self.portfolio.settled_initial_portfolio(
                &self.initial_portfolio,
                marks,
                currency_rates,
                self.execution.derivative_leverage,
                self.execution.exchange_cmr_multiplier,
            )?;
            portfolio
                .validate(&self.strategy_config.effective(), &self.execution)
                .context("terminal settled portfolio failed opening-state validation")?;
            let mut carry_rates = Vec::with_capacity(self.execution.currency_rates.len());
            for route in &self.execution.currency_rates {
                let observation = self
                    .currency_rate_observations
                    .get(&route.currency)
                    .with_context(|| {
                        format!(
                            "terminal settled carry has no observation for currency {}",
                            route.currency
                        )
                    })?;
                if currency_rates.get(&route.currency).copied() != Some(observation.usd_per_unit) {
                    bail!(
                        "terminal settled carry currency observation for {} is stale",
                        route.currency
                    );
                }
                carry_rates.push(BacktestCarryCurrencyRate {
                    currency: route.currency.clone(),
                    index_symbol: route.index_symbol.clone(),
                    usd_per_unit: observation.usd_per_unit,
                    source_ts_ms: observation.source_ts_ms,
                    effective_at_ns: observation.effective_at_ns,
                });
            }
            carry_rates.sort_by(|left, right| left.currency.cmp(&right.currency));

            let mut pending_funding = self
                .scheduled
                .iter()
                .filter_map(|(&(due_at_ns, _), action)| {
                    let ScheduledAction::SettleFunding {
                        symbol,
                        funding_time_ms,
                    } = action
                    else {
                        return None;
                    };
                    let key = (symbol.clone(), *funding_time_ms);
                    Some(BacktestPendingFundingCarry {
                        symbol: symbol.clone(),
                        funding_time_ms: *funding_time_ms,
                        due_at_ns,
                        realized_rate: self.realized_funding_rates.get(&key).copied(),
                    })
                })
                .collect::<Vec<_>>();
            pending_funding.sort_by(|left, right| {
                (&left.symbol, left.funding_time_ms).cmp(&(&right.symbol, right.funding_time_ms))
            });

            let state = BacktestCarryState {
                schema_version: BACKTEST_CARRY_STATE_SCHEMA_VERSION,
                settled_at_ns: self.now_ns,
                terminal_equity_usd: terminal_equity_usd
                    .context("terminal equity disappeared while building settled carry")?,
                source_raw_boundary: self.raw_replay_boundary.clone(),
                portfolio,
                terminal_depth_marks: self
                    .depth_marks
                    .iter()
                    .filter(|(_, mark)| mark.is_finite() && **mark > 0.0)
                    .map(|(symbol, mark)| (symbol.clone(), *mark))
                    .collect(),
                terminal_exchange_marks: self
                    .exchange_marks
                    .iter()
                    .filter(|(_, mark)| mark.is_finite() && **mark > 0.0)
                    .map(|(symbol, mark)| (symbol.clone(), *mark))
                    .collect(),
                currency_rates: carry_rates,
                pending_funding,
                last_settled_funding_time_ms: self.last_settled_funding_time_ms.clone(),
            };
            state.validate_for(&self.strategy_config, &self.execution)?;
            Ok(state)
        })();
        match build {
            Ok(state) => (Some(state), failures),
            Err(error) => {
                failures.push(format!("failed to build settled carry: {error:#}"));
                (None, failures)
            }
        }
    }

    fn finish_report(&mut self) -> Result<BacktestReport> {
        let now_ns = self.now_ns;
        self.drain_through(now_ns)?;
        self.observe_order_entry_readiness();
        self.advance_metric_clock(now_ns);
        self.sample_risk_metrics();
        let marks = self.valuation_marks();
        let currency_rates = self.fresh_currency_rates();
        let fallback_currency_rates = self.fallback_currency_rates();
        let currency_rate_reports = self.currency_rate_reports();
        let currency_rate_coverage_complete =
            currency_rate_reports.iter().all(|report| report.usable);
        let missing_currency_rates = currency_rate_reports
            .iter()
            .filter(|report| !report.usable)
            .map(|report| report.currency.clone())
            .collect::<Vec<_>>();
        let checked_final_equity = self.portfolio.equity_usd_checked(&marks, &currency_rates);
        let checked_final_active_order_notional =
            self.active_order_notional_usd_checked(&currency_rates);
        let checked_final_gross_exposure = self
            .portfolio
            .gross_exposure_usd_checked(&marks, &currency_rates);
        let final_delta_usd = self.strategy.delta_usd();
        let final_pending_delta_usd = self.strategy.pending_delta_usd();
        let final_valuation_complete = checked_final_equity.is_some()
            && checked_final_active_order_notional.is_some()
            && checked_final_gross_exposure.is_some()
            && currency_rate_coverage_complete
            && final_delta_usd.is_finite()
            && final_pending_delta_usd.is_finite();
        let final_equity_usd = checked_final_equity
            .unwrap_or_else(|| self.portfolio.equity_usd(&marks, &fallback_currency_rates));
        let opening_valuation_complete = self.opening_equity_usd.is_some();
        let net_pnl_usd = if final_valuation_complete {
            self.opening_equity_usd
                .zip(checked_final_equity)
                .map(|(opening, final_equity)| final_equity - opening)
        } else {
            None
        };
        let final_active_order_notional_usd = checked_final_active_order_notional
            .unwrap_or_else(|| self.active_order_notional_usd(&fallback_currency_rates));
        let final_gross_exposure_usd = checked_final_gross_exposure.unwrap_or_else(|| {
            self.portfolio
                .gross_exposure_usd_checked(&marks, &fallback_currency_rates)
                .unwrap_or(0.0)
        });
        let pending_orders = self
            .matchers
            .values()
            .map(MatchingEngine::pending_order_count)
            .sum();
        let live_orders = self
            .matchers
            .values()
            .map(MatchingEngine::live_order_count)
            .sum();
        let mut pending_activation_actions = 0;
        let mut pending_cancel_actions = 0;
        let mut pending_order_update_actions = 0;
        let mut pending_strategy_event_actions = 0;
        let mut pending_funding_actions = 0;
        for action in self.scheduled.values() {
            match action {
                ScheduledAction::ActivateOrder { .. } => pending_activation_actions += 1,
                ScheduledAction::CancelOrder { .. } => pending_cancel_actions += 1,
                ScheduledAction::DeliverOrder(_) => pending_order_update_actions += 1,
                ScheduledAction::DeliverAccount(_)
                | ScheduledAction::DeliverStrategy(_)
                | ScheduledAction::RefreshAccount => pending_strategy_event_actions += 1,
                ScheduledAction::SettleFunding { .. } => pending_funding_actions += 1,
            }
        }
        let accounting_complete = self.late_funding_rate_events == 0
            && self.invalid_funding_rate_events == 0
            && self.missed_funding_settlements == 0
            && self.funding_settlement_failures == 0
            && self.invalid_currency_rate_events == 0
            && self.portfolio.currency_conversion_failures() == 0
            && self.portfolio.invalid_accounting_events() == 0
            && self.invalid_risk_metric_samples == 0
            && opening_valuation_complete
            && net_pnl_usd.is_some()
            && final_valuation_complete;
        let (settled_carry_state, carry_state_failures) = self.build_settled_carry_state(
            &marks,
            &currency_rates,
            checked_final_equity,
            final_valuation_complete,
            accounting_complete,
        );
        let order_entry_ready_at_end = self.order_entry_ready();
        let observed_duration_ns = self
            .first_arrival_ns
            .zip(self.metric_clock_ns)
            .map(|(first, metric_horizon)| metric_horizon.saturating_sub(first))
            .unwrap_or(0);
        if self.inventory_open_duration_ns > observed_duration_ns {
            bail!(
                "inventory-open duration {}ns exceeds observed metric horizon {}ns",
                self.inventory_open_duration_ns,
                observed_duration_ns
            );
        }
        let average_abs_delta_usd = if observed_duration_ns == 0 {
            0.0
        } else {
            self.abs_delta_time_integral / observed_duration_ns as f64
        };
        let inventory_open_fraction = if observed_duration_ns == 0 {
            0.0
        } else {
            self.inventory_open_duration_ns as f64 / observed_duration_ns as f64
        };

        Ok(BacktestReport {
            execution: self.execution.clone(),
            initial_portfolio: self.initial_portfolio.clone(),
            latency_usage: self.latency_sampler.usage(),
            time_basis: self.time_basis,
            raw_replay_boundary: self.raw_replay_boundary.clone(),
            input_events: self.input_events,
            first_arrival_ns: self.first_arrival_ns,
            last_arrival_ns: self.last_arrival_ns,
            input_clock_regressions: self.input_clock_regressions,
            max_input_clock_regression_ns: self.max_input_clock_regression_ns,
            order_entry_ready_at_ns: self.order_entry_ready_at_ns,
            order_entry_ready_at_end,
            new_orders_blocked_not_ready: self.new_orders_blocked_not_ready,
            strategy_halt_reason: self.strategy.halt_reason().map(str::to_string),
            orders_sent: self.orders_sent,
            cancel_requests: self.cancel_requests,
            deduplicated_cancel_requests: self.deduplicated_cancel_requests,
            ignored_cancel_requests: self.ignored_cancel_requests,
            exchange_activations: self.exchange_activations,
            cancelled_orders: self.cancelled_orders,
            rejected_orders: self.rejected_orders,
            fills: self.fills,
            maker_fills: self.maker_fills,
            taker_fills: self.taker_fills,
            pending_scheduled_actions: self.scheduled.len(),
            pending_activation_actions,
            pending_cancel_actions,
            pending_order_update_actions,
            pending_strategy_event_actions,
            pending_funding_actions,
            periodic_account_refreshes: self.periodic_account_refreshes,
            pending_orders,
            live_orders,
            pending_cancel_requests: self.pending_cancels.len(),
            final_delta_usd,
            final_pending_delta_usd,
            final_active_order_notional_usd,
            opening_equity_usd: self.opening_equity_usd,
            opening_valuation_at_ns: self.opening_valuation_at_ns,
            opening_valuation_complete,
            final_equity_usd,
            net_pnl_usd,
            final_valuation_complete,
            final_gross_exposure_usd,
            cash_usd: self.portfolio.cash_usd(&fallback_currency_rates),
            cash_by_currency: self.portfolio.cash_by_currency(),
            inverse_cash_coin_by_symbol: self.portfolio.inverse_cash_coin_by_symbol(),
            account_balances: self
                .initial_portfolio
                .balances
                .iter()
                .map(|balance| {
                    (
                        balance.currency.clone(),
                        self.portfolio.account_balance(&balance.currency),
                    )
                })
                .collect(),
            fee_cost_usd: self.portfolio.fee_cost_usd(),
            exact_fee_fills: self.portfolio.exact_fee_fills(),
            estimated_fee_fills: self.portfolio.estimated_fee_fills(),
            funding_pnl_usd: self.portfolio.funding_pnl_usd(),
            turnover_usd: self.portfolio.turnover_usd(),
            currency_rate_events: self.currency_rate_events,
            invalid_currency_rate_events: self.invalid_currency_rate_events,
            currency_conversion_failures: self.portfolio.currency_conversion_failures(),
            invalid_accounting_events: self.portfolio.invalid_accounting_events(),
            currency_rate_coverage_complete,
            missing_currency_rates,
            currency_rates: currency_rate_reports,
            observed_duration_ns,
            max_drawdown_usd: self.max_drawdown_usd,
            max_abs_delta_usd: self.max_abs_delta_usd,
            max_abs_pending_delta_usd: self.max_abs_pending_delta_usd,
            max_gross_exposure_usd: self.max_gross_exposure_usd,
            max_active_orders: self.max_active_orders,
            max_active_order_notional_usd: self.max_active_order_notional_usd,
            average_abs_delta_usd,
            inventory_open_duration_ns: self.inventory_open_duration_ns,
            inventory_open_fraction,
            risk_metric_samples: self.risk_metric_samples,
            invalid_risk_metric_samples: self.invalid_risk_metric_samples,
            funding_rate_events: self.funding_rate_events,
            funding_settlement_observations: self.realized_funding_rates.len() as u64,
            funding_settlements: self.funding_settlements,
            late_funding_rate_events: self.late_funding_rate_events,
            invalid_funding_rate_events: self.invalid_funding_rate_events,
            missed_funding_settlements: self.missed_funding_settlements,
            funding_settlement_failures: self.funding_settlement_failures,
            accounting_complete,
            settled_carry_state,
            carry_state_failures,
            positions: self
                .portfolio
                .positions()
                .iter()
                .map(|(symbol, quantity)| (symbol.clone(), *quantity))
                .collect(),
            position_avg_prices: self
                .portfolio
                .positions()
                .keys()
                .map(|symbol| (symbol.clone(), self.portfolio.position_avg_price(symbol)))
                .collect(),
        })
    }
}

fn time_ms(time_ns: u64) -> u64 {
    time_ns / NS_PER_MS
}

fn retime_order_update(mut update: OrderUpdate, ts_ms: u64) -> OrderUpdate {
    update.ts_ms = ts_ms;
    update
}

fn retime_strategy_event(mut event: StrategyEvent, ts_ms: u64) -> StrategyEvent {
    match &mut event {
        StrategyEvent::Market(MarketEvent::Depth(book)) => book.ts_ms = ts_ms,
        StrategyEvent::Market(
            MarketEvent::Trade {
                ts_ms: event_ts, ..
            }
            | MarketEvent::IndexPrice {
                ts_ms: event_ts, ..
            }
            | MarketEvent::FundingRate {
                ts_ms: event_ts, ..
            }
            | MarketEvent::BurstSignal {
                ts_ms: event_ts, ..
            }
            | MarketEvent::PriceLimits {
                ts_ms: event_ts, ..
            },
        ) => *event_ts = ts_ms,
        StrategyEvent::Order(update) => update.ts_ms = ts_ms,
        StrategyEvent::Account(update) => update.ts_ms = ts_ms,
        StrategyEvent::Timer(event) => event.ts_ms = ts_ms,
        StrategyEvent::Control(event) => event.ts_ms = ts_ms,
        StrategyEvent::System(event) => event.ts_ms = ts_ms,
    }
    event
}

#[cfg(test)]
#[path = "../tests/runner_unit/mod.rs"]
mod tests;
