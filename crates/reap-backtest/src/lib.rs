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

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::portfolio::{Portfolio, required_accounting_currencies};
#[cfg(test)]
use reap_core::FundingSettlement;
use reap_core::{
    AccountUpdate, Balance, FillLiquidity, MarginSnapshot, MarketEvent, NormalizedEvent,
    OrderEvent, OrderIntent, OrderUpdate, Position, StrategyEvent, Symbol,
};
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

impl BacktestCarryState {
    pub fn validate_for(
        &self,
        config: &ChaosConfig,
        execution: &BacktestExecutionConfig,
    ) -> Result<()> {
        if self.schema_version != BACKTEST_CARRY_STATE_SCHEMA_VERSION {
            bail!(
                "unsupported backtest carry-state schema {}, expected {}",
                self.schema_version,
                BACKTEST_CARRY_STATE_SCHEMA_VERSION
            );
        }
        if !self.terminal_equity_usd.is_finite() || self.terminal_equity_usd < 0.0 {
            bail!("backtest carry terminal_equity_usd must be finite and non-negative");
        }
        execution.validate()?;
        self.portfolio
            .validate(&config.effective(), execution)
            .context("invalid settled carry portfolio")?;
        if self.portfolio.is_empty() {
            bail!("settled carry requires a non-empty account portfolio");
        }

        let instruments = config
            .instruments
            .iter()
            .map(|instrument| (instrument.symbol.as_str(), instrument))
            .collect::<HashMap<_, _>>();
        for marks in [&self.terminal_depth_marks, &self.terminal_exchange_marks] {
            for (symbol, mark) in marks {
                if !instruments.contains_key(symbol.as_str()) {
                    bail!("settled carry contains a mark for unknown symbol {symbol}");
                }
                if !mark.is_finite() || *mark <= 0.0 {
                    bail!("settled carry mark for {symbol} must be finite and positive");
                }
            }
        }

        let expected_rates = execution
            .currency_rates
            .iter()
            .map(|route| (route.currency.as_str(), route.index_symbol.as_str()))
            .collect::<HashMap<_, _>>();
        if self.currency_rates.len() != expected_rates.len() {
            bail!(
                "settled carry has {} currency rates, expected {}",
                self.currency_rates.len(),
                expected_rates.len()
            );
        }
        let mut rates = HashMap::new();
        for rate in &self.currency_rates {
            if expected_rates.get(rate.currency.as_str()).copied()
                != Some(rate.index_symbol.as_str())
            {
                bail!(
                    "settled carry currency route {}/{} does not match execution config",
                    rate.currency,
                    rate.index_symbol
                );
            }
            if !rate.usd_per_unit.is_finite() || rate.usd_per_unit <= 0.0 {
                bail!(
                    "settled carry currency rate for {} must be finite and positive",
                    rate.currency
                );
            }
            if rate.effective_at_ns > self.settled_at_ns {
                bail!(
                    "settled carry currency rate for {} is effective after settlement",
                    rate.currency
                );
            }
            if rates
                .insert(rate.currency.clone(), rate.usd_per_unit)
                .is_some()
            {
                bail!("settled carry repeats currency rate {}", rate.currency);
            }
        }

        let mut marks = self
            .terminal_depth_marks
            .iter()
            .map(|(symbol, mark)| (symbol.clone(), *mark))
            .collect::<HashMap<_, _>>();
        marks.extend(
            self.terminal_exchange_marks
                .iter()
                .map(|(symbol, mark)| (symbol.clone(), *mark)),
        );
        let reconstructed = Portfolio::with_initial(&config.instruments, &self.portfolio);
        let reconstructed_equity = reconstructed
            .equity_usd_checked(&marks, &rates)
            .context("settled carry portfolio cannot be independently valued")?;
        require_close(
            "settled carry terminal equity",
            reconstructed_equity,
            self.terminal_equity_usd,
        )?;
        let derivative_notional = reconstructed
            .derivative_notional_usd_checked(&marks, &rates)
            .context("settled carry derivative notional cannot be independently valued")?;
        require_optional_close(
            "settled carry adjusted equity",
            self.portfolio.margin.adjusted_equity_usd,
            Some(self.terminal_equity_usd),
        )?;
        require_optional_close(
            "settled carry derivative notional",
            self.portfolio.margin.notional_usd,
            Some(derivative_notional),
        )?;
        let expected_ratio =
            (derivative_notional > 0.0).then_some(self.terminal_equity_usd / derivative_notional);
        require_optional_close(
            "settled carry margin ratio",
            self.portfolio.margin.ratio,
            expected_ratio,
        )?;
        require_optional_close(
            "settled carry exchange margin ratio",
            self.portfolio.margin.exchange_ratio,
            expected_ratio.map(|ratio| {
                ratio * execution.derivative_leverage * execution.exchange_cmr_multiplier
            }),
        )?;

        for balance in &self.portfolio.balances {
            require_close(
                &format!("settled carry available balance for {}", balance.currency),
                balance.available(),
                balance.total,
            )?;
            require_close(
                &format!("settled carry equity for {}", balance.currency),
                balance.equity(),
                balance.total,
            )?;
            if balance.liability() != 0.0 {
                bail!(
                    "settled carry liability for {} must be zero",
                    balance.currency
                );
            }
        }
        for position in &self.portfolio.positions {
            if position.qty == 0.0 {
                if position.avg_price != 0.0 {
                    bail!(
                        "settled carry flat position {} must have zero average price",
                        position.symbol
                    );
                }
                continue;
            }
            let mark = marks.get(&position.symbol).with_context(|| {
                format!(
                    "settled carry nonzero position {} has no terminal mark",
                    position.symbol
                )
            })?;
            require_close(
                &format!("settled carry average price for {}", position.symbol),
                position.avg_price,
                *mark,
            )?;
        }

        let mut pending_keys = HashSet::new();
        for pending in &self.pending_funding {
            let instrument = instruments.get(pending.symbol.as_str()).with_context(|| {
                format!(
                    "settled carry funding uses unknown symbol {}",
                    pending.symbol
                )
            })?;
            if !instrument.kind.is_swap() {
                bail!(
                    "settled carry funding symbol {} is not a swap",
                    pending.symbol
                );
            }
            let expected_due = pending
                .funding_time_ms
                .checked_mul(NS_PER_MS)
                .context("settled carry funding timestamp overflows nanoseconds")?;
            if pending.funding_time_ms == 0
                || pending.due_at_ns != expected_due
                || pending.due_at_ns <= self.settled_at_ns
            {
                bail!(
                    "settled carry pending funding {}/{} has an invalid due time",
                    pending.symbol,
                    pending.funding_time_ms
                );
            }
            if pending.realized_rate.is_some_and(|rate| !rate.is_finite()) {
                bail!(
                    "settled carry pending funding {}/{} has a non-finite realized rate",
                    pending.symbol,
                    pending.funding_time_ms
                );
            }
            if self
                .last_settled_funding_time_ms
                .get(&pending.symbol)
                .is_some_and(|settled| *settled >= pending.funding_time_ms)
            {
                bail!(
                    "settled carry pending funding {}/{} overlaps its settlement watermark",
                    pending.symbol,
                    pending.funding_time_ms
                );
            }
            if !pending_keys.insert((pending.symbol.as_str(), pending.funding_time_ms)) {
                bail!(
                    "settled carry repeats pending funding {}/{}",
                    pending.symbol,
                    pending.funding_time_ms
                );
            }
        }
        for (symbol, funding_time_ms) in &self.last_settled_funding_time_ms {
            let instrument = instruments.get(symbol.as_str()).with_context(|| {
                format!("settled carry funding watermark uses unknown symbol {symbol}")
            })?;
            let funding_time_ns = funding_time_ms
                .checked_mul(NS_PER_MS)
                .context("settled carry funding watermark overflows nanoseconds")?;
            if !instrument.kind.is_swap()
                || *funding_time_ms == 0
                || funding_time_ns > self.settled_at_ns
            {
                bail!("settled carry funding watermark for {symbol} is invalid");
            }
        }

        if let Some(boundary) = &self.source_raw_boundary
            && (boundary.validate().is_err() || self.settled_at_ns < boundary.maximum_recv_ts_ns)
        {
            bail!("settled carry source raw boundary is invalid");
        }
        Ok(())
    }

    pub fn rebind_execution(
        mut self,
        config: &ChaosConfig,
        source: &BacktestExecutionConfig,
        target: &BacktestExecutionConfig,
    ) -> Result<Self> {
        self.validate_for(config, source)
            .context("source carry state is invalid")?;
        target.validate()?;
        let ratio = self.portfolio.margin.ratio;
        self.portfolio.margin.exchange_ratio =
            ratio.map(|ratio| ratio * target.derivative_leverage * target.exchange_cmr_multiplier);
        self.validate_for(config, target)
            .context("carry state is incompatible with target execution")?;
        Ok(self)
    }
}

fn require_close(name: &str, actual: f64, expected: f64) -> Result<()> {
    let tolerance = 1.0e-9 * actual.abs().max(expected.abs()).max(1.0);
    if !actual.is_finite() || !expected.is_finite() || (actual - expected).abs() > tolerance {
        bail!("{name} mismatch: actual={actual}, expected={expected}");
    }
    Ok(())
}

fn require_optional_close(name: &str, actual: Option<f64>, expected: Option<f64>) -> Result<()> {
    match (actual, expected) {
        (Some(actual), Some(expected)) => require_close(name, actual, expected),
        (None, None) => Ok(()),
        _ => bail!("{name} presence mismatch: actual={actual:?}, expected={expected:?}"),
    }
}

impl BacktestRunner {
    pub fn new(config: ChaosConfig) -> Result<Self> {
        Self::with_execution_config(config, BacktestExecutionConfig::default())
    }

    pub fn from_config(config: BacktestConfig) -> Result<Self> {
        Self::with_initial_portfolio_config(
            config.strategy,
            config.backtest,
            config.initial_portfolio,
        )
    }

    pub fn from_config_with_carry(
        config: BacktestConfig,
        carry: BacktestCarryState,
    ) -> Result<Self> {
        Self::with_carry_state(config.strategy, config.backtest, carry)
    }

    pub fn with_execution_config(
        config: ChaosConfig,
        execution: BacktestExecutionConfig,
    ) -> Result<Self> {
        Self::with_initial_portfolio_config(
            config,
            execution,
            BacktestInitialPortfolioConfig::default(),
        )
    }

    pub fn with_carry_state(
        config: ChaosConfig,
        execution: BacktestExecutionConfig,
        carry: BacktestCarryState,
    ) -> Result<Self> {
        carry.validate_for(&config, &execution)?;
        let mut runner =
            Self::with_initial_portfolio_config(config, execution, carry.portfolio.clone())?;
        runner.now_ns = carry.settled_at_ns;
        runner.opening_equity_usd = Some(carry.terminal_equity_usd);
        runner.opening_valuation_at_ns = Some(carry.settled_at_ns);
        runner.peak_equity_usd = carry.terminal_equity_usd;
        runner.depth_marks = carry.terminal_depth_marks.into_iter().collect();
        runner.exchange_marks = carry.terminal_exchange_marks.into_iter().collect();
        runner.currency_rate_observations = carry
            .currency_rates
            .into_iter()
            .map(|rate| {
                (
                    rate.currency,
                    CurrencyRateObservation {
                        usd_per_unit: rate.usd_per_unit,
                        source_ts_ms: rate.source_ts_ms,
                        effective_at_ns: rate.effective_at_ns,
                    },
                )
            })
            .collect();
        for pending in carry.pending_funding {
            let key = (pending.symbol.clone(), pending.funding_time_ms);
            if let Some(rate) = pending.realized_rate {
                runner.realized_funding_rates.insert(key.clone(), rate);
            }
            runner.scheduled_funding.insert(key);
            runner.schedule_at(
                pending.due_at_ns,
                ScheduledAction::SettleFunding {
                    symbol: pending.symbol,
                    funding_time_ms: pending.funding_time_ms,
                },
            );
        }
        runner.last_settled_funding_time_ms = carry.last_settled_funding_time_ms;
        runner.carry_source_boundary = carry.source_raw_boundary;
        runner.deliver_initial_account_snapshot()?;
        Ok(runner)
    }

    pub fn with_initial_portfolio_config(
        config: ChaosConfig,
        execution: BacktestExecutionConfig,
        initial_portfolio: BacktestInitialPortfolioConfig,
    ) -> Result<Self> {
        execution.validate()?;
        validate_latency_profile_symbols(&execution, &config)?;
        validate_currency_rate_coverage(&execution, &config)?;
        initial_portfolio.validate(&config.effective(), &execution)?;
        let matching_assumptions = MatchingAssumptions {
            depth_fill_conservative_threshold: execution.depth_fill_conservative_threshold,
            queue_ahead_multiplier: execution.queue_ahead_multiplier,
            historical_trade_fill_fraction: execution.historical_trade_fill_fraction,
            displayed_depth_fill_fraction: execution.displayed_depth_fill_fraction,
        };
        let matchers = config
            .instruments
            .iter()
            .map(|inst| {
                (
                    inst.symbol.clone(),
                    MatchingEngine::with_assumptions(inst.clone(), matching_assumptions),
                )
            })
            .collect();
        let latency_sampler = BacktestLatencySampler::new(&execution);
        let currency_by_index_symbol = execution
            .currency_rates
            .iter()
            .map(|route| (route.index_symbol.clone(), route.currency.clone()))
            .collect();
        let portfolio = Portfolio::with_initial(&config.instruments, &initial_portfolio);
        let initial_inventory_open = portfolio
            .positions()
            .values()
            .any(|quantity| *quantity != 0.0);
        let opening_equity_usd = initial_portfolio.is_empty().then_some(0.0);
        let opening_valuation_at_ns = initial_portfolio.is_empty().then_some(0);
        let strategy =
            ChaosStrategy::new(config.clone()).context("invalid chaos/iarb2 configuration")?;
        Ok(Self {
            strategy_config: config,
            portfolio,
            initial_account_snapshot_delivered: initial_portfolio.is_empty(),
            initial_portfolio,
            strategy,
            matchers,
            execution,
            latency_sampler,
            time_basis: BacktestTimeBasis::EventTimestampMs,
            raw_replay_boundary: None,
            carry_source_boundary: None,
            scheduled: BTreeMap::new(),
            next_action_seq: 1,
            pending_cancels: HashSet::new(),
            pending_fill_account_updates: 0,
            last_account_publish_ns: None,
            periodic_account_refreshes: 0,
            depth_marks: HashMap::new(),
            exchange_marks: HashMap::new(),
            currency_by_index_symbol,
            currency_rate_observations: HashMap::new(),
            realized_funding_rates: HashMap::new(),
            scheduled_funding: HashSet::new(),
            settled_funding: HashSet::new(),
            last_settled_funding_time_ms: BTreeMap::new(),
            now_ns: 0,
            first_arrival_ns: None,
            last_arrival_ns: None,
            input_events: 0,
            input_clock_regressions: 0,
            max_input_clock_regression_ns: 0,
            order_entry_ready_at_ns: None,
            new_orders_blocked_not_ready: 0,
            orders_sent: 0,
            cancel_requests: 0,
            deduplicated_cancel_requests: 0,
            ignored_cancel_requests: 0,
            exchange_activations: 0,
            cancelled_orders: 0,
            rejected_orders: 0,
            fills: 0,
            maker_fills: 0,
            taker_fills: 0,
            funding_rate_events: 0,
            funding_settlements: 0,
            late_funding_rate_events: 0,
            invalid_funding_rate_events: 0,
            missed_funding_settlements: 0,
            funding_settlement_failures: 0,
            currency_rate_events: 0,
            invalid_currency_rate_events: 0,
            opening_equity_usd,
            opening_valuation_at_ns,
            peak_equity_usd: 0.0,
            max_drawdown_usd: 0.0,
            max_abs_delta_usd: 0.0,
            max_abs_pending_delta_usd: 0.0,
            max_gross_exposure_usd: 0.0,
            max_active_orders: 0,
            max_active_order_notional_usd: 0.0,
            abs_delta_time_integral: 0.0,
            inventory_open_duration_ns: 0,
            metric_clock_ns: None,
            current_abs_delta_usd: 0.0,
            current_inventory_open: initial_inventory_open,
            risk_metric_samples: 0,
            invalid_risk_metric_samples: 0,
        })
    }

    pub fn run_csv_path(mut self, path: impl AsRef<Path>) -> Result<BacktestReport> {
        let events = load_events_from_path(path.as_ref()).with_context(|| {
            format!(
                "failed to load replay events from {}",
                path.as_ref().display()
            )
        })?;
        self.run(events)
    }

    pub fn run_normalized_jsonl_path(mut self, path: impl AsRef<Path>) -> Result<BacktestReport> {
        let events = load_normalized_jsonl_from_path(path.as_ref()).with_context(|| {
            format!(
                "failed to load normalized replay events from {}",
                path.as_ref().display()
            )
        })?;
        self.run(events)
    }

    pub fn run_raw_capture_path(mut self, path: impl AsRef<Path>) -> Result<BacktestReport> {
        let path = path.as_ref();
        self.time_basis = BacktestTimeBasis::CaptureReceiveTimestampNs;
        let preload_boundary = replay_raw_capture_timed_path_with_boundary(path, |timed| {
            self.preload_funding_settlement(&timed.event)
        })
        .with_context(|| format!("failed to preload realized funding from {}", path.display()))?;
        if let Some(boundary) = &preload_boundary {
            self.validate_carry_handoff(boundary)?;
        } else if self.carry_source_boundary.is_some() {
            bail!("settled raw carry requires sequenced raw replay input");
        }
        let boundary = replay_raw_capture_timed_path_with_boundary(path, |timed| {
            self.process_replay_event_at(timed.event, timed.recv_ts_ns)
        })
        .with_context(|| format!("failed to replay raw capture from {}", path.display()))?;
        if preload_boundary != boundary {
            bail!("raw capture boundary changed between preload and replay passes");
        }
        if let Some(boundary) = &boundary {
            self.advance_raw_horizon(boundary.maximum_recv_ts_ns)?;
        }
        self.raw_replay_boundary = boundary;
        self.require_all_configured_books()?;
        self.finish_report()
    }

    pub fn run_raw_capture_range_path(
        mut self,
        path: impl AsRef<Path>,
        range: RawCaptureRecordRange,
    ) -> Result<BacktestReport> {
        let path = path.as_ref();
        self.time_basis = BacktestTimeBasis::CaptureReceiveTimestampNs;
        let preload_boundary = replay_raw_capture_timed_range_path(path, range, |timed| {
            self.preload_funding_settlement(&timed.event)
        })
        .with_context(|| {
            format!(
                "failed to preload realized funding from raw capture range {}..={} in {}",
                range.first,
                range.last,
                path.display()
            )
        })?;
        self.validate_carry_handoff(&preload_boundary)?;
        let replay_boundary = replay_raw_capture_timed_range_path(path, range, |timed| {
            self.process_replay_event_at(timed.event, timed.recv_ts_ns)
        })
        .with_context(|| {
            format!(
                "failed to replay raw capture range {}..={} from {}",
                range.first,
                range.last,
                path.display()
            )
        })?;
        if preload_boundary != replay_boundary {
            bail!("raw capture range boundary changed between preload and replay passes");
        }
        self.advance_raw_horizon(replay_boundary.maximum_recv_ts_ns)?;
        self.raw_replay_boundary = Some(replay_boundary);
        self.require_all_configured_books()?;
        self.finish_report()
    }

    pub fn run<I>(&mut self, events: I) -> Result<BacktestReport>
    where
        I: IntoIterator<Item = NormalizedEvent>,
    {
        self.time_basis = BacktestTimeBasis::EventTimestampMs;
        let events = events.into_iter().collect::<Vec<_>>();
        for event in &events {
            self.preload_funding_settlement(event)?;
        }
        for event in events {
            let arrival_ns = event.ts_ms().saturating_mul(NS_PER_MS);
            self.process_replay_event_at(event, arrival_ns)?;
        }

        self.finish_report()
    }

    #[cfg(test)]
    fn process_replay_event(&mut self, event: NormalizedEvent) -> Result<()> {
        let arrival_ns = event.ts_ms().saturating_mul(NS_PER_MS);
        self.process_replay_event_at(event, arrival_ns)
    }

    fn process_replay_event_at(
        &mut self,
        event: NormalizedEvent,
        candidate_arrival_ns: u64,
    ) -> Result<()> {
        let arrival_ns = self.register_input_arrival(candidate_arrival_ns);
        self.drain_before(arrival_ns)?;
        self.advance_metric_clock(arrival_ns);
        self.now_ns = arrival_ns;
        self.deliver_initial_account_snapshot()?;

        match &event {
            NormalizedEvent::Market(MarketEvent::Depth(book)) => {
                let now_ns = self.now_ns;
                let latency_ms = self
                    .latency_sampler
                    .sample(BacktestLatencyClass::MarketDepth, &book.symbol);
                if let Some(mid) = book.mid().filter(|mid| mid.is_finite() && *mid > 0.0) {
                    self.depth_marks.insert(book.symbol.clone(), mid);
                }
                let updates = self
                    .matcher_mut(&book.symbol)?
                    .on_depth_at(book.clone(), time_ms(now_ns));
                self.route_exchange_updates(updates)?;
                self.drain_through(now_ns)?;
                self.schedule_after(
                    latency_ms,
                    ScheduledAction::DeliverStrategy(event.into_strategy_event()),
                );
                self.drain_through(now_ns)?;
            }
            NormalizedEvent::Market(MarketEvent::Trade {
                symbol,
                price,
                qty,
                taker_side,
                ..
            }) => {
                let now_ns = self.now_ns;
                let latency_ms = self
                    .latency_sampler
                    .sample(BacktestLatencyClass::HistoricalTrade, symbol);
                let updates = self.matcher_mut(symbol)?.on_trade_at(
                    *price,
                    *qty,
                    *taker_side,
                    time_ms(now_ns),
                );
                self.route_exchange_updates(updates)?;
                self.drain_through(now_ns)?;
                self.schedule_after(
                    latency_ms,
                    ScheduledAction::DeliverStrategy(event.into_strategy_event()),
                );
                self.drain_through(now_ns)?;
            }
            NormalizedEvent::Market(
                MarketEvent::IndexPrice { symbol, .. } | MarketEvent::BurstSignal { symbol, .. },
            ) => {
                let now_ns = self.now_ns;
                let latency_ms = self
                    .latency_sampler
                    .sample(BacktestLatencyClass::ReferenceData, symbol);
                self.drain_through(now_ns)?;
                self.schedule_after(
                    latency_ms,
                    ScheduledAction::DeliverStrategy(event.into_strategy_event()),
                );
                self.drain_through(now_ns)?;
            }
            NormalizedEvent::Market(MarketEvent::FundingRate {
                symbol,
                rate,
                funding_time_ms,
                ..
            }) => {
                let now_ns = self.now_ns;
                let latency_ms = self
                    .latency_sampler
                    .sample(BacktestLatencyClass::ReferenceData, symbol);
                self.register_funding_rate(symbol, *rate, *funding_time_ms);
                self.drain_through(now_ns)?;
                self.schedule_after(
                    latency_ms,
                    ScheduledAction::DeliverStrategy(event.into_strategy_event()),
                );
                self.drain_through(now_ns)?;
            }
            NormalizedEvent::Market(MarketEvent::PriceLimits {
                symbol, mark_price, ..
            }) => {
                let now_ns = self.now_ns;
                let latency_ms = self
                    .latency_sampler
                    .sample(BacktestLatencyClass::ReferenceData, symbol);
                if mark_price.is_finite() && *mark_price > 0.0 {
                    self.exchange_marks.insert(symbol.clone(), *mark_price);
                }
                self.drain_through(now_ns)?;
                self.schedule_after(
                    latency_ms,
                    ScheduledAction::DeliverStrategy(event.into_strategy_event()),
                );
                self.drain_through(now_ns)?;
            }
            NormalizedEvent::Order(update) => {
                let now_ns = self.now_ns;
                self.route_exchange_updates(vec![update.clone()])?;
                self.drain_through(now_ns)?;
            }
            NormalizedEvent::Account(_)
            | NormalizedEvent::Timer(_)
            | NormalizedEvent::Control(_)
            | NormalizedEvent::System(_) => {
                let now_ns = self.now_ns;
                self.schedule_at(
                    now_ns,
                    ScheduledAction::DeliverStrategy(event.into_strategy_event()),
                );
                self.drain_through(now_ns)?;
            }
        }
        self.observe_order_entry_readiness();
        self.sample_risk_metrics();
        Ok(())
    }

    fn deliver_initial_account_snapshot(&mut self) -> Result<()> {
        if self.initial_account_snapshot_delivered {
            return Ok(());
        }
        let commands = self.strategy.on_event(&StrategyEvent::Account(
            self.initial_portfolio.account_update(time_ms(self.now_ns)),
        ));
        if !commands.is_empty() {
            bail!("initial portfolio unexpectedly produced strategy order intents");
        }
        self.initial_account_snapshot_delivered = true;
        self.last_account_publish_ns = Some(self.now_ns);
        self.schedule_next_account_refresh();
        Ok(())
    }

    fn register_input_arrival(&mut self, candidate_ns: u64) -> u64 {
        self.input_events += 1;
        let arrival_ns = match self.last_arrival_ns {
            Some(last_ns) if candidate_ns < last_ns => {
                let regression_ns = last_ns - candidate_ns;
                self.input_clock_regressions += 1;
                self.max_input_clock_regression_ns =
                    self.max_input_clock_regression_ns.max(regression_ns);
                last_ns
            }
            _ => candidate_ns,
        };
        self.first_arrival_ns.get_or_insert(arrival_ns);
        self.last_arrival_ns = Some(arrival_ns);
        arrival_ns
    }

    fn advance_raw_horizon(&mut self, horizon_ns: u64) -> Result<()> {
        if horizon_ns <= self.now_ns {
            return Ok(());
        }
        self.drain_through(horizon_ns)?;
        self.advance_metric_clock(horizon_ns);
        self.now_ns = horizon_ns;
        self.observe_order_entry_readiness();
        self.sample_risk_metrics();
        Ok(())
    }

    fn validate_carry_handoff(&self, current: &RawReplayBoundary) -> Result<()> {
        current.validate()?;
        let Some(previous) = &self.carry_source_boundary else {
            return Ok(());
        };
        previous.validate()?;
        if previous.capture_session_id != current.capture_session_id {
            bail!(
                "settled carry crosses capture sessions: previous={}, current={}",
                previous.capture_session_id,
                current.capture_session_id
            );
        }
        let expected = previous
            .last_capture_record_seq
            .checked_add(1)
            .context("previous capture record sequence exhausted")?;
        if current.first_capture_record_seq != expected {
            bail!(
                "settled carry requires the next capture record sequence {expected}, received {}",
                current.first_capture_record_seq
            );
        }
        if current.first_recv_ts_ns < self.now_ns {
            bail!(
                "settled carry receive time regresses: settled_at_ns={}, current_first_recv_ts_ns={}",
                self.now_ns,
                current.first_recv_ts_ns
            );
        }
        Ok(())
    }

    fn register_funding_rate(&mut self, symbol: &str, rate: f64, funding_time_ms: u64) {
        self.funding_rate_events += 1;
        if !self.portfolio.supports_funding(symbol) || !rate.is_finite() || funding_time_ms == 0 {
            self.invalid_funding_rate_events += 1;
            return;
        }

        let key = (symbol.to_string(), funding_time_ms);
        if self.settled_funding.contains(&key)
            || self
                .last_settled_funding_time_ms
                .get(symbol)
                .is_some_and(|settled| *settled >= funding_time_ms)
        {
            return;
        }
        if !self.scheduled_funding.insert(key.clone()) {
            return;
        }

        let funding_time_ns = funding_time_ms.saturating_mul(NS_PER_MS);
        if funding_time_ns.saturating_add(FUNDING_LATE_TOLERANCE_NS) < self.now_ns {
            self.scheduled_funding.remove(&key);
            self.settled_funding.insert(key.clone());
            self.record_settled_funding(&key.0, key.1);
            self.missed_funding_settlements += 1;
            return;
        }
        let due_ns = if funding_time_ns < self.now_ns {
            self.late_funding_rate_events += 1;
            self.now_ns
        } else {
            funding_time_ns
        };
        self.schedule_at(
            due_ns,
            ScheduledAction::SettleFunding {
                symbol: symbol.to_string(),
                funding_time_ms,
            },
        );
    }

    fn preload_funding_settlement(&mut self, event: &NormalizedEvent) -> Result<()> {
        let NormalizedEvent::Market(MarketEvent::FundingRate {
            symbol,
            settlement: Some(settlement),
            ..
        }) = event
        else {
            return Ok(());
        };
        if settlement.funding_time_ms == 0 || !settlement.rate.is_finite() {
            bail!(
                "invalid realized funding settlement for {symbol} at {}: {}",
                settlement.funding_time_ms,
                settlement.rate
            );
        }
        let key = (symbol.clone(), settlement.funding_time_ms);
        if let Some(previous) = self.realized_funding_rates.get(&key) {
            if *previous != settlement.rate {
                bail!(
                    "conflicting realized funding rates for {} at {}: {} and {}",
                    key.0,
                    key.1,
                    previous,
                    settlement.rate
                );
            }
        } else {
            self.realized_funding_rates.insert(key, settlement.rate);
        }
        Ok(())
    }

    fn settle_funding(&mut self, symbol: Symbol, funding_time_ms: u64) {
        let key = (symbol.clone(), funding_time_ms);
        self.scheduled_funding.remove(&key);
        if !self.settled_funding.insert(key.clone()) {
            return;
        }
        self.record_settled_funding(&symbol, funding_time_ms);
        let Some(rate) = self.realized_funding_rates.get(&key).copied() else {
            self.funding_settlement_failures += 1;
            return;
        };
        if self.opening_equity_usd.is_none() {
            self.funding_settlement_failures += 1;
            return;
        }
        let mark = self
            .exchange_marks
            .get(&symbol)
            .or_else(|| self.depth_marks.get(&symbol))
            .copied()
            .unwrap_or(f64::NAN);
        let currency_rates = self.fresh_currency_rates();
        if self
            .portfolio
            .apply_funding(&symbol, rate, mark, &currency_rates)
            .is_some()
        {
            self.funding_settlements += 1;
        } else {
            self.funding_settlement_failures += 1;
        }
    }

    fn record_settled_funding(&mut self, symbol: &str, funding_time_ms: u64) {
        self.last_settled_funding_time_ms
            .entry(symbol.to_string())
            .and_modify(|current| *current = (*current).max(funding_time_ms))
            .or_insert(funding_time_ms);
    }

    fn route_exchange_updates(&mut self, updates: Vec<OrderUpdate>) -> Result<()> {
        for update in updates {
            if update.event == OrderEvent::Cancelled {
                self.cancelled_orders += 1;
            } else if update.event == OrderEvent::Rejected {
                self.rejected_orders += 1;
            }

            let account_update = if update.has_fill() {
                if self.opening_equity_usd.is_none() {
                    bail!(
                        "fill for {} arrived before the configured opening portfolio could be valued",
                        update.symbol
                    );
                }
                self.fills += 1;
                match update.last_fill_liquidity {
                    Some(FillLiquidity::Maker) => self.maker_fills += 1,
                    Some(FillLiquidity::Taker) => self.taker_fills += 1,
                    None => {}
                }
                let currency_rates = self.fresh_currency_rates();
                self.portfolio.apply_fill(&update, &currency_rates);
                self.sample_risk_metrics();
                Some(self.current_account_update(Some(&update.symbol)))
            } else {
                None
            };

            let order_update_delay_ms = self
                .latency_sampler
                .sample(BacktestLatencyClass::OrderUpdate, &update.symbol);
            let fill_symbol = update.symbol.clone();
            self.schedule_after(order_update_delay_ms, ScheduledAction::DeliverOrder(update));
            if let Some(account_update) = account_update {
                let fill_account_delay_ms = self
                    .latency_sampler
                    .sample(BacktestLatencyClass::OrderFill, &fill_symbol);
                self.pending_fill_account_updates =
                    self.pending_fill_account_updates.saturating_add(1);
                self.schedule_after(
                    fill_account_delay_ms,
                    ScheduledAction::DeliverAccount(account_update),
                );
            }
        }
        Ok(())
    }

    fn current_account_update(&self, source_symbol: Option<&str>) -> AccountUpdate {
        let marks = self.valuation_marks();
        let mut position_symbols = self
            .strategy_config
            .instruments
            .iter()
            .filter(|instrument| instrument.kind.is_derivative())
            .map(|instrument| instrument.symbol.clone())
            .collect::<Vec<_>>();
        if let Some(source_symbol) = source_symbol
            && !position_symbols
                .iter()
                .any(|symbol| symbol == source_symbol)
        {
            position_symbols.push(source_symbol.to_string());
        }
        if let Some(source_symbol) = source_symbol
            && let Some(index) = position_symbols
                .iter()
                .position(|symbol| symbol == source_symbol)
        {
            let source = position_symbols.remove(index);
            position_symbols.push(source);
        }
        AccountUpdate {
            ts_ms: time_ms(self.now_ns),
            balances: if self.initial_portfolio.is_empty() {
                Vec::new()
            } else {
                self.initial_portfolio
                    .balances
                    .iter()
                    .map(|balance| {
                        let total = self.portfolio.account_balance(&balance.currency);
                        let change = total - balance.total;
                        Balance {
                            account_id: self.initial_portfolio.account_id.clone(),
                            currency: balance.currency.clone(),
                            total,
                            available: balance.available() + change,
                            equity: self
                                .portfolio
                                .account_equity(&balance.currency, &marks)
                                .unwrap_or_else(|| balance.equity() + change),
                            liability: balance.liability(),
                            max_loan: balance.max_loan(),
                            forced_repayment_indicator: balance.forced_repayment_indicator,
                        }
                    })
                    .collect()
            },
            positions: position_symbols
                .into_iter()
                .map(|symbol| Position {
                    qty: self
                        .portfolio
                        .positions()
                        .get(&symbol)
                        .copied()
                        .unwrap_or(0.0),
                    avg_price: self.portfolio.position_avg_price(&symbol),
                    margin_mode: self
                        .initial_portfolio
                        .positions
                        .iter()
                        .find(|position| position.symbol == symbol)
                        .and_then(|position| position.margin_mode),
                    symbol,
                })
                .collect(),
            margins: self.current_margin_snapshots(&marks),
        }
    }

    fn schedule_next_account_refresh(&mut self) {
        if self.initial_portfolio.is_empty() {
            return;
        }
        let due_ns = self.now_ns.saturating_add(ACCOUNT_REFRESH_INTERVAL_NS);
        if due_ns > self.now_ns {
            self.schedule_at(due_ns, ScheduledAction::RefreshAccount);
        }
    }

    fn current_margin_snapshots(&self, marks: &HashMap<Symbol, f64>) -> Vec<MarginSnapshot> {
        if self.initial_portfolio.is_empty() {
            return Vec::new();
        }
        let currency_rates = self.fresh_currency_rates();
        let Some(adjusted_equity_usd) = self.portfolio.equity_usd_checked(marks, &currency_rates)
        else {
            return Vec::new();
        };
        let Some(notional_usd) = self
            .portfolio
            .derivative_notional_usd_checked(marks, &currency_rates)
        else {
            return Vec::new();
        };
        let ratio = (notional_usd > 0.0).then_some(adjusted_equity_usd / notional_usd);
        vec![MarginSnapshot {
            account_id: self.initial_portfolio.account_id.clone(),
            ratio,
            exchange_ratio: ratio.map(|ratio| {
                ratio * self.execution.derivative_leverage * self.execution.exchange_cmr_multiplier
            }),
            adjusted_equity_usd: Some(adjusted_equity_usd),
            notional_usd: Some(notional_usd),
        }]
    }

    fn accept_intents(&mut self, commands: Vec<OrderIntent>) -> Result<()> {
        let mut queue = VecDeque::from(commands);
        while let Some(command) = queue.pop_front() {
            match command {
                OrderIntent::NewOrder(order) => {
                    self.observe_order_entry_readiness();
                    if !self.order_entry_ready() {
                        self.new_orders_blocked_not_ready =
                            self.new_orders_blocked_not_ready.saturating_add(1);
                        continue;
                    }
                    self.orders_sent += 1;
                    let symbol = order.symbol.clone();
                    let now_ms = time_ms(self.now_ns);
                    let (order_id, pending) =
                        self.matcher_mut(&symbol)?.prepare_submit(order, now_ms);
                    let order_entry_delay_ms = self
                        .latency_sampler
                        .sample(BacktestLatencyClass::MatchingNew, &symbol);
                    self.schedule_after(
                        order_entry_delay_ms,
                        ScheduledAction::ActivateOrder {
                            symbol,
                            order_id: order_id.clone(),
                        },
                    );

                    let pending = retime_order_update(pending, now_ms);
                    queue.extend(self.strategy.on_event(&StrategyEvent::Order(pending)));
                }
                OrderIntent::CancelOrder { order_id, reason } => {
                    self.cancel_requests += 1;
                    let Some(symbol) = self.open_order_symbol(&order_id) else {
                        self.ignored_cancel_requests += 1;
                        continue;
                    };
                    if !self.pending_cancels.insert(order_id.clone()) {
                        self.deduplicated_cancel_requests += 1;
                        continue;
                    }
                    let cancel_delay_ms = self
                        .latency_sampler
                        .sample(BacktestLatencyClass::MatchingCancel, &symbol);
                    self.schedule_after(
                        cancel_delay_ms,
                        ScheduledAction::CancelOrder {
                            symbol,
                            order_id,
                            reason,
                        },
                    );
                }
            }
        }
        Ok(())
    }

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

    fn require_all_configured_books(&self) -> Result<()> {
        let mut missing = self
            .matchers
            .iter()
            .filter(|(_, matcher)| matcher.depth().is_none())
            .map(|(symbol, _)| symbol.clone())
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return Ok(());
        }
        missing.sort();
        bail!(
            "raw capture did not produce a valid book for configured symbols: {}",
            missing.join(", ")
        )
    }

    fn matcher_mut(&mut self, symbol: &str) -> Result<&mut MatchingEngine> {
        self.matchers
            .get_mut(symbol)
            .with_context(|| format!("no matcher configured for symbol {symbol}"))
    }

    fn open_order_symbol(&self, order_id: &str) -> Option<Symbol> {
        self.matchers
            .iter()
            .find(|(_, matcher)| matcher.is_open_order(order_id))
            .map(|(symbol, _)| symbol.clone())
    }
}

fn time_ms(time_ns: u64) -> u64 {
    time_ns / NS_PER_MS
}

fn validate_latency_profile_symbols(
    execution: &BacktestExecutionConfig,
    config: &ChaosConfig,
) -> Result<()> {
    let mut known_symbols = config
        .instruments
        .iter()
        .map(|instrument| instrument.symbol.as_str())
        .collect::<HashSet<_>>();
    known_symbols.insert(&config.ref_symbol);
    known_symbols.extend(
        config
            .instruments
            .iter()
            .filter_map(|instrument| instrument.index_symbol.as_deref()),
    );
    known_symbols.extend(
        execution
            .currency_rates
            .iter()
            .map(|route| route.index_symbol.as_str()),
    );
    let mut unknown = execution
        .latency_profile
        .rules
        .iter()
        .filter_map(|rule| rule.symbol.as_deref())
        .filter(|symbol| !known_symbols.contains(symbol))
        .collect::<Vec<_>>();
    unknown.sort_unstable();
    unknown.dedup();
    if !unknown.is_empty() {
        bail!(
            "backtest latency profile references symbols outside the strategy instrument/reference/index universe (including accounting indexes): {}",
            unknown.join(", ")
        );
    }
    Ok(())
}

fn validate_currency_rate_coverage(
    execution: &BacktestExecutionConfig,
    config: &ChaosConfig,
) -> Result<()> {
    let configured = execution
        .currency_rates
        .iter()
        .map(|route| route.currency.as_str())
        .collect::<HashSet<_>>();
    let mut missing = required_accounting_currencies(&config.instruments)
        .into_iter()
        .filter(|currency| !configured.contains(currency.as_str()))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    missing.sort();
    bail!(
        "backtest.currency_rates lacks direct USD valuation routes for instrument accounting currencies: {}",
        missing.join(", ")
    )
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
mod tests {
    use reap_core::{Level, NewOrder, OrderBook, OrderStatus, Side, TimeInForce, TimerEvent};
    use reap_strategy::{InstrumentConfig, InstrumentKindConfig, RiskGroupConfig};

    use super::*;

    fn config() -> ChaosConfig {
        ChaosConfig {
            ref_symbol: "BTC-USDT".to_string(),
            active_hedge_threshold_usd: 500.0,
            min_hedge_interval_ms: 0,
            risk_groups: vec![RiskGroupConfig {
                name: "main".to_string(),
                symbols: vec!["BTC-USDT".to_string(), "BTC-PERP".to_string()],
                soft_delta_limit_usd: 50_000.0,
                hard_delta_limit_usd: 75_000.0,
                delta_stop_limit_usd: 100_000.0,
                live_order_limit_usd: 100_000.0,
                ..RiskGroupConfig::default()
            }],
            instruments: vec![
                InstrumentConfig {
                    symbol: "BTC-USDT".to_string(),
                    risk_group: "main".to_string(),
                    kind: InstrumentKindConfig::Spot,
                    max_order_size_usd: 5_000.0,
                    min_order_size_usd: 100.0,
                    max_order_size: 1.0,
                    tick_size: 0.1,
                    lot_size: 0.0001,
                    ..InstrumentConfig::default()
                },
                InstrumentConfig {
                    symbol: "BTC-PERP".to_string(),
                    risk_group: "main".to_string(),
                    kind: InstrumentKindConfig::Future,
                    contract_value: 0.001,
                    max_order_size_usd: 10_000.0,
                    min_order_size_usd: 100.0,
                    max_order_size: 10_000.0,
                    min_trade_size: 1.0,
                    lot_size: 1.0,
                    min_position: -100_000.0,
                    max_position: 100_000.0,
                    ..InstrumentConfig::default()
                },
            ],
            ..ChaosConfig::default()
        }
    }

    fn initial_books() -> Vec<NormalizedEvent> {
        vec![
            NormalizedEvent::from(MarketEvent::Depth(OrderBook::one_level(
                "BTC-USDT",
                1,
                Level::new(50_000.0, 2.0),
                Level::new(50_001.0, 2.0),
            ))),
            NormalizedEvent::from(MarketEvent::Depth(OrderBook::one_level(
                "BTC-PERP",
                1,
                Level::new(50_003.0, 10_000.0),
                Level::new(50_004.0, 10_000.0),
            ))),
        ]
    }

    fn usdt_config() -> ChaosConfig {
        let mut config = config();
        for instrument in &mut config.instruments {
            instrument.base_currency = "BTC".to_string();
            instrument.quote_currency = "USDT".to_string();
            instrument.taker_fee = 0.0;
            if instrument.kind.is_derivative() {
                instrument.settle_currency = "USDT".to_string();
            }
        }
        config
    }

    fn usdt_execution(market_data_latency_ms: u64, max_age_ms: u64) -> BacktestExecutionConfig {
        BacktestExecutionConfig {
            market_data_latency_ms,
            currency_rates: vec![BacktestCurrencyRateConfig {
                currency: "USDT".to_string(),
                index_symbol: "USDT-USD".to_string(),
                max_age_ms,
            }],
            ..BacktestExecutionConfig::default()
        }
    }

    fn external_spot_fill(ts_ms: u64, price: f64) -> NormalizedEvent {
        NormalizedEvent::Order(OrderUpdate {
            ts_ms,
            order_id: "external-fill".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            event: OrderEvent::FullyFilled,
            status: OrderStatus::Filled,
            price,
            time_in_force: Some(TimeInForce::Ioc),
            qty: 1.0,
            open_qty: 0.0,
            filled_qty: 1.0,
            avg_fill_price: price,
            last_fill_qty: 1.0,
            last_fill_price: price,
            last_fill_liquidity: Some(FillLiquidity::Taker),
            last_fill_fee: None,
            reason: "external-test-fill".to_string(),
        })
    }

    fn seed_perp_matcher(runner: &mut BacktestRunner, ts_ms: u64) {
        runner.matcher_mut("BTC-PERP").unwrap().on_depth_at(
            OrderBook::one_level(
                "BTC-PERP",
                ts_ms,
                Level::new(100.0, 10_000.0),
                Level::new(101.0, 10_000.0),
            ),
            ts_ms,
        );
    }

    #[test]
    fn replayed_quote_fill_triggers_hedge_order() {
        let mut runner = BacktestRunner::new(config()).unwrap();
        let mut events = initial_books();
        events.push(NormalizedEvent::from(MarketEvent::Trade {
            ts_ms: 2,
            symbol: "BTC-USDT".to_string(),
            price: 49_000.0,
            qty: 1.0,
            taker_side: Side::Sell,
        }));

        let report = runner.run(events).unwrap();
        assert!(report.orders_sent >= 3);
        assert!(report.fills >= 1);
        assert!(report.taker_fills >= 1);
        assert!(report.final_delta_usd.abs() < 5_000.0);
        assert_eq!(report.execution, BacktestExecutionConfig::default());
    }

    #[test]
    fn normalized_fixture_replays_quote_and_hedge_path() {
        let events = load_normalized_jsonl(
            include_str!("../../../fixtures/normalized/chaos_quote_hedge.jsonl").as_bytes(),
        )
        .unwrap();
        let mut runner = BacktestRunner::new(config()).unwrap();

        let report = runner.run(events).unwrap();

        assert!(report.orders_sent >= 1);
        assert_eq!(report.fills, 2);
        assert_eq!(report.maker_fills, 1);
        assert_eq!(report.taker_fills, 1);
        assert!(report.final_delta_usd.abs() < 1_000.0);
    }

    #[test]
    fn delayed_entry_is_reported_as_pending_at_end_of_data() {
        let execution = BacktestExecutionConfig {
            order_entry_latency_ms: 10,
            ..BacktestExecutionConfig::default()
        };
        let mut runner = BacktestRunner::with_execution_config(config(), execution).unwrap();

        let report = runner.run(initial_books()).unwrap();

        assert!(report.orders_sent > 0);
        assert_eq!(report.exchange_activations, 0);
        assert_eq!(report.pending_orders, report.orders_sent);
        assert_eq!(report.pending_activation_actions, report.pending_orders);
        assert!(report.pending_scheduled_actions >= report.pending_orders);
    }

    #[test]
    fn delayed_market_data_is_not_delivered_past_end_of_data() {
        let execution = BacktestExecutionConfig {
            market_data_latency_ms: 10,
            ..BacktestExecutionConfig::default()
        };
        let mut runner = BacktestRunner::with_execution_config(config(), execution).unwrap();

        let report = runner.run(initial_books()).unwrap();

        assert_eq!(report.orders_sent, 0);
        assert_eq!(report.pending_scheduled_actions, 2);
        assert_eq!(report.pending_strategy_event_actions, 2);
        assert_eq!(report.pending_orders, 0);
        assert_eq!(report.latency_usage.len(), 2);
        assert!(report.latency_usage.iter().all(|usage| {
            usage.class == BacktestLatencyClass::MarketDepth
                && usage.samples == 1
                && usage.minimum_latency_ms == 10
                && usage.maximum_latency_ms == 10
        }));
    }

    #[test]
    fn symbol_latency_rule_overrides_class_rule_in_the_scheduler() {
        let execution = BacktestExecutionConfig {
            market_data_latency_ms: 99,
            latency_profile: BacktestLatencyProfile {
                seed: 17,
                rules: vec![
                    BacktestLatencyRule {
                        class: BacktestLatencyClass::MarketDepth,
                        symbol: None,
                        samples_ms: vec![0],
                    },
                    BacktestLatencyRule {
                        class: BacktestLatencyClass::MarketDepth,
                        symbol: Some("BTC-PERP".to_string()),
                        samples_ms: vec![10],
                    },
                ],
            },
            ..BacktestExecutionConfig::default()
        };
        let mut runner = BacktestRunner::with_execution_config(config(), execution).unwrap();

        let report = runner.run(initial_books()).unwrap();

        assert_eq!(report.orders_sent, 0);
        assert_eq!(report.pending_strategy_event_actions, 1);
        assert_eq!(report.latency_usage.len(), 2);
        assert_eq!(
            report
                .latency_usage
                .iter()
                .find(|usage| usage.symbol == "BTC-USDT")
                .unwrap()
                .maximum_latency_ms,
            0
        );
        assert_eq!(
            report
                .latency_usage
                .iter()
                .find(|usage| usage.symbol == "BTC-PERP")
                .unwrap()
                .maximum_latency_ms,
            10
        );
    }

    #[test]
    fn runner_rejects_unknown_symbol_latency_rule() {
        let execution = BacktestExecutionConfig {
            latency_profile: BacktestLatencyProfile {
                seed: 1,
                rules: vec![BacktestLatencyRule {
                    class: BacktestLatencyClass::MarketDepth,
                    symbol: Some("ETH-USDT".to_string()),
                    samples_ms: vec![1],
                }],
            },
            ..BacktestExecutionConfig::default()
        };

        let error = BacktestRunner::with_execution_config(config(), execution)
            .err()
            .unwrap()
            .to_string();

        assert!(error.contains("outside the strategy instrument/reference/index universe"));
        assert!(error.contains("ETH-USDT"));
    }

    #[test]
    fn runner_requires_explicit_rates_for_non_usd_accounting_currencies() {
        let error = BacktestRunner::new(usdt_config())
            .err()
            .unwrap()
            .to_string();

        assert!(error.contains("lacks direct USD valuation routes"));
        assert!(error.contains("USDT"));
    }

    #[test]
    fn delivered_currency_index_values_portfolio_and_report_evidence() {
        let mut runner =
            BacktestRunner::with_execution_config(usdt_config(), usdt_execution(0, 1_000)).unwrap();
        runner.depth_marks.insert("BTC-USDT".to_string(), 110.0);
        let events = vec![
            NormalizedEvent::Market(MarketEvent::IndexPrice {
                ts_ms: 1,
                symbol: "USDT-USD".to_string(),
                price: 0.95,
            }),
            external_spot_fill(2, 100.0),
            NormalizedEvent::Timer(TimerEvent {
                ts_ms: 3,
                name: "finish".to_string(),
            }),
        ];

        let report = runner.run(events).unwrap();

        assert!((report.final_equity_usd - 9.5).abs() < 1e-12);
        assert!((report.cash_usd + 95.0).abs() < 1e-12);
        assert_eq!(report.cash_by_currency.get("USDT"), Some(&-100.0));
        assert_eq!(report.currency_rate_events, 1);
        assert_eq!(report.currency_conversion_failures, 0);
        assert!(report.currency_rate_coverage_complete);
        assert!(report.missing_currency_rates.is_empty());
        assert_eq!(report.currency_rates.len(), 1);
        assert_eq!(report.currency_rates[0].usd_per_unit, Some(0.95));
        assert_eq!(report.currency_rates[0].source_ts_ms, Some(1));
        assert_eq!(report.currency_rates[0].effective_at_ns, Some(NS_PER_MS));
        assert_eq!(report.currency_rates[0].age_ms, Some(2));
        assert!(report.currency_rates[0].usable);
        assert!(report.final_valuation_complete);
        assert!(report.accounting_complete);
    }

    #[test]
    fn order_entry_waits_for_books_and_fresh_accounting_rates() {
        let mut runner =
            BacktestRunner::with_execution_config(usdt_config(), usdt_execution(0, 1_000)).unwrap();
        let mut events = initial_books();
        events.push(NormalizedEvent::Market(MarketEvent::IndexPrice {
            ts_ms: 2,
            symbol: "USDT-USD".to_string(),
            price: 1.0,
        }));
        events.push(NormalizedEvent::from(MarketEvent::Depth(
            OrderBook::one_level(
                "BTC-USDT",
                3,
                Level::new(50_001.0, 2.0),
                Level::new(50_002.0, 2.0),
            ),
        )));

        let report = runner.run(events).unwrap();

        assert!(report.new_orders_blocked_not_ready > 0);
        assert_eq!(report.order_entry_ready_at_ns, Some(2 * NS_PER_MS));
        assert!(report.order_entry_ready_at_end);
        assert!(report.orders_sent > 0);
        assert_eq!(report.invalid_risk_metric_samples, 0);
        assert!(report.accounting_complete);
    }

    #[test]
    fn configured_opening_portfolio_reports_true_net_pnl_and_strategy_balances() {
        let initial = BacktestInitialPortfolioConfig {
            balances: vec![
                BacktestInitialBalanceConfig {
                    currency: "BTC".to_string(),
                    total: 0.002,
                    valuation_symbol: Some("BTC-USDT".to_string()),
                    ..Default::default()
                },
                BacktestInitialBalanceConfig {
                    currency: "USDT".to_string(),
                    total: 1_000.0,
                    valuation_symbol: None,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mut runner = BacktestRunner::with_initial_portfolio_config(
            usdt_config(),
            usdt_execution(0, 1_000),
            initial.clone(),
        )
        .unwrap();
        let mut events = initial_books();
        events.push(NormalizedEvent::Market(MarketEvent::IndexPrice {
            ts_ms: 2,
            symbol: "USDT-USD".to_string(),
            price: 1.0,
        }));
        events.push(NormalizedEvent::from(MarketEvent::Depth(
            OrderBook::one_level(
                "BTC-USDT",
                3,
                Level::new(50_000.0, 2.0),
                Level::new(50_001.0, 2.0),
            ),
        )));

        let report = runner.run(events).unwrap();

        let expected_opening = 1_000.0 + 0.002 * 50_000.5;
        assert_eq!(report.initial_portfolio, initial);
        assert_eq!(report.opening_valuation_at_ns, Some(2 * NS_PER_MS));
        assert!(report.opening_valuation_complete);
        assert!((report.opening_equity_usd.unwrap() - expected_opening).abs() < 1e-9);
        assert!((report.final_equity_usd - expected_opening).abs() < 1e-9);
        assert!(report.net_pnl_usd.unwrap().abs() < 1e-9);
        assert_eq!(report.account_balances.get("BTC"), Some(&0.002));
        assert_eq!(report.account_balances.get("USDT"), Some(&1_000.0));
        assert_eq!(report.positions.get("BTC-USDT"), Some(&0.002));
        assert!(report.orders_sent > 0);
        assert!(report.accounting_complete);
    }

    #[test]
    fn periodic_account_refreshes_do_not_bypass_pending_fill_latency() {
        let mut strategy = usdt_config();
        for instrument in &mut strategy.instruments {
            instrument.quote_profit_margin = 1.0;
            instrument.halted = true;
        }
        let initial = BacktestInitialPortfolioConfig {
            balances: vec![
                BacktestInitialBalanceConfig {
                    currency: "BTC".to_string(),
                    total: 0.0,
                    valuation_symbol: Some("BTC-USDT".to_string()),
                    ..Default::default()
                },
                BacktestInitialBalanceConfig {
                    currency: "USDT".to_string(),
                    total: 100_000.0,
                    ..Default::default()
                },
            ],
            positions: vec![BacktestInitialPositionConfig {
                symbol: "BTC-PERP".to_string(),
                qty: 0.0,
                avg_price: 0.0,
                margin_mode: Some(reap_core::PositionMarginMode::Cross),
            }],
            ..Default::default()
        };
        let mut execution = usdt_execution(0, 30_000);
        let mut refresh_runner = BacktestRunner::with_initial_portfolio_config(
            strategy.clone(),
            execution.clone(),
            initial.clone(),
        )
        .unwrap();
        let mut refresh_events = initial_books();
        refresh_events.push(NormalizedEvent::Market(MarketEvent::IndexPrice {
            ts_ms: 2,
            symbol: "USDT-USD".to_string(),
            price: 1.0,
        }));
        refresh_events.push(NormalizedEvent::Timer(TimerEvent {
            ts_ms: 10_002,
            name: "refresh".to_string(),
        }));
        let refreshed = refresh_runner.run(refresh_events).unwrap();
        assert_eq!(refreshed.periodic_account_refreshes, 1);

        execution.fill_account_latency_ms = 20_000;
        let mut delayed_runner =
            BacktestRunner::with_initial_portfolio_config(strategy, execution, initial).unwrap();
        let mut delayed_events = initial_books();
        delayed_events.push(NormalizedEvent::Market(MarketEvent::IndexPrice {
            ts_ms: 2,
            symbol: "USDT-USD".to_string(),
            price: 1.0,
        }));
        delayed_events.push(external_spot_fill(3, 50_000.0));
        delayed_events.push(NormalizedEvent::Timer(TimerEvent {
            ts_ms: 12_000,
            name: "refresh".to_string(),
        }));
        let delayed = delayed_runner.run(delayed_events).unwrap();
        assert_eq!(delayed.periodic_account_refreshes, 0);
        assert!(delayed.pending_strategy_event_actions >= 2);
    }

    #[test]
    fn settled_carry_round_trips_portfolio_margin_and_raw_handoff() {
        let mut strategy = usdt_config();
        for instrument in &mut strategy.instruments {
            instrument.quote_profit_margin = 1.0;
            instrument.halted = true;
        }
        let execution = usdt_execution(0, 1_000);
        let initial = BacktestInitialPortfolioConfig {
            account_id: None,
            balances: vec![
                BacktestInitialBalanceConfig {
                    currency: "BTC".to_string(),
                    total: 0.002,
                    valuation_symbol: Some("BTC-USDT".to_string()),
                    ..Default::default()
                },
                BacktestInitialBalanceConfig {
                    currency: "USDT".to_string(),
                    total: 1_000.0,
                    valuation_symbol: None,
                    ..Default::default()
                },
            ],
            positions: vec![BacktestInitialPositionConfig {
                symbol: "BTC-PERP".to_string(),
                qty: 2.0,
                avg_price: 49_000.0,
                margin_mode: Some(reap_core::PositionMarginMode::Cross),
            }],
            margin: BacktestInitialMarginConfig::default(),
        };
        let mut runner = BacktestRunner::with_initial_portfolio_config(
            strategy.clone(),
            execution.clone(),
            initial,
        )
        .unwrap();
        let mut events = initial_books();
        events.push(NormalizedEvent::Market(MarketEvent::IndexPrice {
            ts_ms: 2,
            symbol: "USDT-USD".to_string(),
            price: 1.0,
        }));
        events.push(NormalizedEvent::Timer(TimerEvent {
            ts_ms: 3,
            name: "finish".to_string(),
        }));

        let report = runner.run(events).unwrap();
        assert!(report.carry_state_failures.is_empty());
        let mut carry = report.settled_carry_state.unwrap();
        assert_eq!(carry.settled_at_ns, 3 * NS_PER_MS);
        assert_eq!(carry.portfolio.balances[0].available, Some(0.002));
        assert_eq!(carry.portfolio.positions[0].qty, 2.0);
        assert_eq!(carry.portfolio.positions[0].avg_price, 50_003.5);
        assert_eq!(
            carry.portfolio.positions[0].margin_mode,
            Some(reap_core::PositionMarginMode::Cross)
        );
        assert!(carry.terminal_exchange_marks.is_empty());
        assert_eq!(carry.terminal_depth_marks.get("BTC-PERP"), Some(&50_003.5));

        let mut bad_balance = carry.clone();
        bad_balance.portfolio.balances[1].total += 1.0;
        assert!(
            bad_balance
                .validate_for(&strategy, &execution)
                .unwrap_err()
                .to_string()
                .contains("settled carry")
        );
        let mut bad_average = carry.clone();
        bad_average.portfolio.positions[0].avg_price += 1.0;
        assert!(bad_average.validate_for(&strategy, &execution).is_err());
        let mut bad_margin = carry.clone();
        bad_margin.portfolio.margin.exchange_ratio = bad_margin
            .portfolio
            .margin
            .exchange_ratio
            .map(|ratio| ratio + 1.0);
        assert!(bad_margin.validate_for(&strategy, &execution).is_err());

        carry.source_raw_boundary = Some(RawReplayBoundary {
            capture_session_id: "session-a".to_string(),
            first_capture_record_seq: 1,
            last_capture_record_seq: 10,
            raw_records: 10,
            first_recv_ts_ns: 1,
            last_recv_ts_ns: 3 * NS_PER_MS,
            maximum_recv_ts_ns: 3 * NS_PER_MS,
        });
        let carried =
            BacktestRunner::with_carry_state(strategy.clone(), execution.clone(), carry).unwrap();
        assert_eq!(carried.opening_equity_usd, report.opening_equity_usd);
        carried
            .validate_carry_handoff(&RawReplayBoundary {
                capture_session_id: "session-a".to_string(),
                first_capture_record_seq: 11,
                last_capture_record_seq: 20,
                raw_records: 10,
                first_recv_ts_ns: 3 * NS_PER_MS + 1,
                last_recv_ts_ns: 4 * NS_PER_MS,
                maximum_recv_ts_ns: 4 * NS_PER_MS,
            })
            .unwrap();
        let error = carried
            .validate_carry_handoff(&RawReplayBoundary {
                capture_session_id: "session-a".to_string(),
                first_capture_record_seq: 12,
                last_capture_record_seq: 20,
                raw_records: 9,
                first_recv_ts_ns: 3 * NS_PER_MS + 1,
                last_recv_ts_ns: 4 * NS_PER_MS,
                maximum_recv_ts_ns: 4 * NS_PER_MS,
            })
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("next capture record sequence 11")
        );
        let error = carried
            .validate_carry_handoff(&RawReplayBoundary {
                capture_session_id: "session-a".to_string(),
                first_capture_record_seq: 11,
                last_capture_record_seq: 20,
                raw_records: 10,
                first_recv_ts_ns: 3 * NS_PER_MS - 1,
                last_recv_ts_ns: 4 * NS_PER_MS,
                maximum_recv_ts_ns: 4 * NS_PER_MS,
            })
            .unwrap_err();
        assert!(error.to_string().contains("receive time regresses"));
    }

    #[test]
    fn settled_carry_preserves_pending_funding_and_settlement_watermark() {
        let mut strategy = config();
        strategy.instruments[1].kind = InstrumentKindConfig::LinearSwap;
        for instrument in &mut strategy.instruments {
            instrument.base_currency = "BTC".to_string();
            instrument.quote_currency = "USD".to_string();
            if instrument.kind.is_derivative() {
                instrument.settle_currency = "USD".to_string();
            }
            instrument.quote_profit_margin = 1.0;
            instrument.halted = true;
        }
        let execution = BacktestExecutionConfig::default();
        let initial = BacktestInitialPortfolioConfig {
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
                qty: 10.0,
                avg_price: 50_000.0,
                margin_mode: Some(reap_core::PositionMarginMode::Cross),
            }],
            ..Default::default()
        };
        let mut first = BacktestRunner::with_initial_portfolio_config(
            strategy.clone(),
            execution.clone(),
            initial,
        )
        .unwrap();
        let mut first_events = initial_books();
        first_events.push(NormalizedEvent::from(MarketEvent::FundingRate {
            ts_ms: 2,
            symbol: "BTC-PERP".to_string(),
            rate: 0.001,
            funding_time_ms: 100,
            settlement: None,
        }));
        let first_report = first.run(first_events).unwrap();
        let carry = first_report.settled_carry_state.unwrap();
        assert_eq!(carry.pending_funding.len(), 1);
        assert_eq!(carry.pending_funding[0].funding_time_ms, 100);
        assert_eq!(carry.pending_funding[0].realized_rate, None);
        let mut overlapping = carry.clone();
        overlapping
            .last_settled_funding_time_ms
            .insert("BTC-PERP".to_string(), 100);
        assert!(
            overlapping
                .validate_for(&strategy, &execution)
                .unwrap_err()
                .to_string()
                .contains("overlaps its settlement watermark")
        );

        let carry_settled_at_ns = carry.settled_at_ns;
        let mut second = BacktestRunner::with_carry_state(strategy, execution, carry).unwrap();
        assert!(second.initial_account_snapshot_delivered);
        assert_eq!(second.last_account_publish_ns, Some(carry_settled_at_ns));
        let second_report = second
            .run([NormalizedEvent::from(MarketEvent::FundingRate {
                ts_ms: 101,
                symbol: "BTC-PERP".to_string(),
                rate: 0.002,
                funding_time_ms: 200,
                settlement: Some(FundingSettlement {
                    funding_time_ms: 100,
                    rate: 0.001,
                }),
            })])
            .unwrap();

        assert_eq!(second_report.funding_settlements, 1);
        assert!((second_report.funding_pnl_usd + 0.500_035).abs() < 1e-9);
        assert_eq!(second_report.observed_duration_ns, 0);
        assert_eq!(second_report.inventory_open_duration_ns, 0);
        assert_eq!(second_report.inventory_open_fraction, 0.0);
        assert!(
            second_report.settled_carry_state.is_some(),
            "{:?}",
            second_report.carry_state_failures
        );
        let second_carry = second_report.settled_carry_state.unwrap();
        assert_eq!(
            second_carry.last_settled_funding_time_ms.get("BTC-PERP"),
            Some(&100)
        );
        assert_eq!(second_carry.pending_funding.len(), 1);
        assert_eq!(second_carry.pending_funding[0].funding_time_ms, 200);
    }

    #[test]
    fn configured_opening_portfolio_keeps_order_entry_blocked_without_valuation() {
        let initial = BacktestInitialPortfolioConfig {
            balances: vec![
                BacktestInitialBalanceConfig {
                    currency: "BTC".to_string(),
                    total: 0.01,
                    valuation_symbol: Some("BTC-USDT".to_string()),
                    ..Default::default()
                },
                BacktestInitialBalanceConfig {
                    currency: "USDT".to_string(),
                    total: 1_000.0,
                    valuation_symbol: None,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mut runner = BacktestRunner::with_initial_portfolio_config(
            usdt_config(),
            usdt_execution(0, 1_000),
            initial,
        )
        .unwrap();

        let report = runner.run(initial_books()).unwrap();

        assert_eq!(report.opening_equity_usd, None);
        assert_eq!(report.net_pnl_usd, None);
        assert!(!report.opening_valuation_complete);
        assert!(!report.order_entry_ready_at_end);
        assert_eq!(report.orders_sent, 0);
        assert!(!report.accounting_complete);
    }

    #[test]
    fn stale_currency_index_makes_final_accounting_incomplete() {
        let mut runner =
            BacktestRunner::with_execution_config(usdt_config(), usdt_execution(0, 1)).unwrap();
        runner.depth_marks.insert("BTC-USDT".to_string(), 110.0);
        let events = vec![
            NormalizedEvent::Market(MarketEvent::IndexPrice {
                ts_ms: 1,
                symbol: "USDT-USD".to_string(),
                price: 0.95,
            }),
            external_spot_fill(2, 100.0),
            NormalizedEvent::Timer(TimerEvent {
                ts_ms: 4,
                name: "stale".to_string(),
            }),
        ];

        let report = runner.run(events).unwrap();

        assert!(!report.currency_rate_coverage_complete);
        assert_eq!(report.missing_currency_rates, vec!["USDT".to_string()]);
        assert!((report.final_equity_usd - 9.5).abs() < 1e-12);
        assert!((report.final_gross_exposure_usd - 104.5).abs() < 1e-12);
        assert!(!report.final_valuation_complete);
        assert!(!report.accounting_complete);
        assert!(report.invalid_risk_metric_samples > 0);
    }

    #[test]
    fn fill_before_delayed_currency_index_records_conversion_failure() {
        let mut runner =
            BacktestRunner::with_execution_config(usdt_config(), usdt_execution(10, 1_000))
                .unwrap();
        runner.depth_marks.insert("BTC-USDT".to_string(), 110.0);
        let events = vec![
            NormalizedEvent::Market(MarketEvent::IndexPrice {
                ts_ms: 1,
                symbol: "USDT-USD".to_string(),
                price: 0.95,
            }),
            external_spot_fill(2, 100.0),
            NormalizedEvent::Timer(TimerEvent {
                ts_ms: 11,
                name: "deliver-reference".to_string(),
            }),
        ];

        let report = runner.run(events).unwrap();

        assert_eq!(report.currency_rate_events, 1);
        assert!(report.currency_rate_coverage_complete);
        assert_eq!(report.currency_conversion_failures, 1);
        assert_eq!(report.currency_rates[0].source_ts_ms, Some(1));
        assert_eq!(
            report.currency_rates[0].effective_at_ns,
            Some(11 * NS_PER_MS)
        );
        assert_eq!(report.currency_rates[0].age_ms, Some(10));
        assert_eq!(report.turnover_usd, 100.0);
        assert!(!report.accounting_complete);
    }

    #[test]
    fn source_age_can_make_a_currency_index_stale_at_delivery() {
        let mut runner =
            BacktestRunner::with_execution_config(usdt_config(), usdt_execution(10, 5)).unwrap();
        let events = vec![
            NormalizedEvent::Market(MarketEvent::IndexPrice {
                ts_ms: 1,
                symbol: "USDT-USD".to_string(),
                price: 0.95,
            }),
            NormalizedEvent::Timer(TimerEvent {
                ts_ms: 11,
                name: "deliver-stale-reference".to_string(),
            }),
        ];

        let report = runner.run(events).unwrap();

        assert_eq!(report.currency_rate_events, 1);
        assert_eq!(
            report.currency_rates[0].effective_at_ns,
            Some(11 * NS_PER_MS)
        );
        assert_eq!(report.currency_rates[0].age_ms, Some(10));
        assert!(!report.currency_rates[0].usable);
        assert!(!report.currency_rate_coverage_complete);
        assert!(!report.accounting_complete);
    }

    #[test]
    fn input_clock_regressions_are_clamped_and_reported() {
        let mut runner = BacktestRunner::new(config()).unwrap();
        let mut events = initial_books();
        events[0] = NormalizedEvent::from(MarketEvent::Depth(OrderBook::one_level(
            "BTC-USDT",
            10,
            Level::new(50_000.0, 2.0),
            Level::new(50_001.0, 2.0),
        )));

        let report = runner.run(events).unwrap();

        assert_eq!(report.input_clock_regressions, 1);
        assert_eq!(report.max_input_clock_regression_ns, 9 * NS_PER_MS);
        assert_eq!(report.last_arrival_ns, Some(10 * NS_PER_MS));
    }

    #[test]
    fn order_remains_fillable_until_delayed_cancel_is_effective() {
        let execution = BacktestExecutionConfig {
            cancel_latency_ms: 10,
            ..BacktestExecutionConfig::default()
        };
        let mut runner = BacktestRunner::with_execution_config(config(), execution).unwrap();
        runner.now_ns = NS_PER_MS;
        runner.matcher_mut("BTC-USDT").unwrap().on_depth_at(
            OrderBook::one_level(
                "BTC-USDT",
                1,
                Level::new(100.0, 1.0),
                Level::new(101.0, 1.0),
            ),
            1,
        );
        seed_perp_matcher(&mut runner, 1);
        runner
            .accept_intents(vec![OrderIntent::NewOrder(NewOrder {
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                qty: 0.5,
                price: 100.0,
                time_in_force: TimeInForce::PostOnly,
                reduce_only: false,
                self_trade_prevention: None,
                reason: "manual_test".to_string(),
            })])
            .unwrap();
        runner.drain_through(NS_PER_MS).unwrap();
        runner
            .accept_intents(vec![OrderIntent::CancelOrder {
                order_id: "BTC-USDT-1".to_string(),
                reason: "manual_cancel".to_string(),
            }])
            .unwrap();

        runner
            .process_replay_event_at(
                NormalizedEvent::from(MarketEvent::Trade {
                    ts_ms: 5,
                    symbol: "BTC-USDT".to_string(),
                    price: 100.0,
                    qty: 1.5,
                    taker_side: Side::Sell,
                }),
                5 * NS_PER_MS,
            )
            .unwrap();
        runner
            .process_replay_event_at(
                NormalizedEvent::Timer(TimerEvent {
                    ts_ms: 11,
                    name: "advance".to_string(),
                }),
                11 * NS_PER_MS,
            )
            .unwrap();
        let report = runner.finish_report().unwrap();

        assert_eq!(report.fills, 1);
        assert_eq!(report.maker_fills, 1);
        assert_eq!(report.cancel_requests, 1);
        assert_eq!(report.cancelled_orders, 0);
        assert_eq!(report.pending_cancel_requests, 0);
    }

    #[test]
    fn nanosecond_arrival_clock_preserves_cancel_before_next_market_event() {
        let execution = BacktestExecutionConfig {
            cancel_latency_ms: 1,
            ..BacktestExecutionConfig::default()
        };
        let mut runner = BacktestRunner::with_execution_config(config(), execution).unwrap();
        runner.now_ns = 100_100_000;
        runner.matcher_mut("BTC-USDT").unwrap().on_depth_at(
            OrderBook::one_level(
                "BTC-USDT",
                100,
                Level::new(100.0, 1.0),
                Level::new(101.0, 1.0),
            ),
            100,
        );
        seed_perp_matcher(&mut runner, 100);
        runner
            .accept_intents(vec![OrderIntent::NewOrder(NewOrder {
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                qty: 0.5,
                price: 100.0,
                time_in_force: TimeInForce::PostOnly,
                reduce_only: false,
                self_trade_prevention: None,
                reason: "nanosecond_test".to_string(),
            })])
            .unwrap();
        runner.drain_through(100_100_000).unwrap();
        runner
            .accept_intents(vec![OrderIntent::CancelOrder {
                order_id: "BTC-USDT-1".to_string(),
                reason: "cancel_before_trade".to_string(),
            }])
            .unwrap();

        runner
            .process_replay_event_at(
                NormalizedEvent::from(MarketEvent::Trade {
                    ts_ms: 101,
                    symbol: "BTC-USDT".to_string(),
                    price: 100.0,
                    qty: 1.5,
                    taker_side: Side::Sell,
                }),
                101_200_000,
            )
            .unwrap();
        let report = runner.finish_report().unwrap();

        assert_eq!(report.cancelled_orders, 1);
        assert_eq!(report.fills, 0);
        assert_eq!(report.last_arrival_ns, Some(101_200_000));
    }

    #[test]
    fn realized_funding_rate_settles_signed_linear_swap_position() {
        let mut cfg = config();
        cfg.instruments[1].kind = InstrumentKindConfig::LinearSwap;
        cfg.instruments[1].taker_fee = 0.0;
        let mut runner = BacktestRunner::new(cfg).unwrap();
        runner.depth_marks.insert("BTC-PERP".to_string(), 50_000.0);
        runner.portfolio.apply_fill(
            &OrderUpdate {
                ts_ms: 0,
                order_id: "initial-fill".to_string(),
                symbol: "BTC-PERP".to_string(),
                side: Side::Buy,
                event: OrderEvent::FullyFilled,
                status: OrderStatus::Filled,
                price: 50_000.0,
                time_in_force: Some(TimeInForce::Ioc),
                qty: 100.0,
                open_qty: 0.0,
                filled_qty: 100.0,
                avg_fill_price: 50_000.0,
                last_fill_qty: 100.0,
                last_fill_price: 50_000.0,
                last_fill_liquidity: Some(FillLiquidity::Taker),
                last_fill_fee: None,
                reason: "initial".to_string(),
            },
            &HashMap::new(),
        );
        let events = vec![
            NormalizedEvent::from(MarketEvent::FundingRate {
                ts_ms: 1,
                symbol: "BTC-PERP".to_string(),
                rate: 0.001,
                funding_time_ms: 10,
                settlement: None,
            }),
            NormalizedEvent::from(MarketEvent::FundingRate {
                ts_ms: 5,
                symbol: "BTC-PERP".to_string(),
                rate: 0.002,
                funding_time_ms: 10,
                settlement: None,
            }),
            NormalizedEvent::from(MarketEvent::FundingRate {
                ts_ms: 11,
                symbol: "BTC-PERP".to_string(),
                rate: 0.003,
                funding_time_ms: 20,
                settlement: Some(FundingSettlement {
                    funding_time_ms: 10,
                    rate: 0.0015,
                }),
            }),
            NormalizedEvent::Timer(TimerEvent {
                ts_ms: 12,
                name: "funding".to_string(),
            }),
        ];

        let report = runner.run(events).unwrap();

        assert_eq!(report.funding_rate_events, 3);
        assert_eq!(report.funding_settlement_observations, 1);
        assert_eq!(report.funding_settlements, 1);
        assert_eq!(report.pending_funding_actions, 1);
        assert!((report.funding_pnl_usd + 7.5).abs() < 1e-9);
        assert!((report.final_equity_usd + 7.5).abs() < 1e-9);
        assert!(report.accounting_complete);
    }

    #[test]
    fn funding_beyond_the_data_horizon_remains_explicitly_pending() {
        let mut cfg = config();
        cfg.instruments[1].kind = InstrumentKindConfig::LinearSwap;
        let mut runner = BacktestRunner::new(cfg).unwrap();

        let report = runner
            .run([NormalizedEvent::from(MarketEvent::FundingRate {
                ts_ms: 1,
                symbol: "BTC-PERP".to_string(),
                rate: 0.001,
                funding_time_ms: 100,
                settlement: None,
            })])
            .unwrap();

        assert_eq!(report.funding_settlements, 0);
        assert_eq!(report.pending_funding_actions, 1);
        assert_eq!(report.pending_scheduled_actions, 1);
        assert!(report.accounting_complete);
    }

    #[test]
    fn due_funding_without_a_realized_rate_marks_accounting_incomplete() {
        let mut cfg = config();
        cfg.instruments[1].kind = InstrumentKindConfig::LinearSwap;
        let mut runner = BacktestRunner::new(cfg).unwrap();

        let report = runner
            .run([
                NormalizedEvent::from(MarketEvent::FundingRate {
                    ts_ms: 1,
                    symbol: "BTC-PERP".to_string(),
                    rate: 0.001,
                    funding_time_ms: 10,
                    settlement: None,
                }),
                NormalizedEvent::Timer(TimerEvent {
                    ts_ms: 10,
                    name: "funding".to_string(),
                }),
            ])
            .unwrap();

        assert_eq!(report.funding_settlements, 0);
        assert_eq!(report.funding_settlement_failures, 1);
        assert!(!report.accounting_complete);
        assert!(
            report
                .carry_state_failures
                .iter()
                .any(|failure| failure.contains("requires complete accounting"))
        );
    }

    #[test]
    fn conflicting_realized_funding_rates_are_rejected() {
        let mut cfg = config();
        cfg.instruments[1].kind = InstrumentKindConfig::LinearSwap;
        let mut runner = BacktestRunner::new(cfg).unwrap();
        let event = |ts_ms, settled_rate| {
            NormalizedEvent::from(MarketEvent::FundingRate {
                ts_ms,
                symbol: "BTC-PERP".to_string(),
                rate: 0.001,
                funding_time_ms: 20,
                settlement: Some(FundingSettlement {
                    funding_time_ms: 10,
                    rate: settled_rate,
                }),
            })
        };

        let error = runner
            .run([event(11, 0.001), event(12, 0.002)])
            .unwrap_err()
            .to_string();

        assert!(error.contains("conflicting realized funding rates"));
    }

    #[test]
    fn stale_first_funding_forecast_marks_accounting_incomplete() {
        let mut cfg = config();
        cfg.instruments[1].kind = InstrumentKindConfig::LinearSwap;
        let mut runner = BacktestRunner::new(cfg).unwrap();

        let report = runner
            .run([NormalizedEvent::from(MarketEvent::FundingRate {
                ts_ms: 100_000,
                symbol: "BTC-PERP".to_string(),
                rate: 0.001,
                funding_time_ms: 1,
                settlement: None,
            })])
            .unwrap();

        assert_eq!(report.missed_funding_settlements, 1);
        assert!(!report.accounting_complete);
    }

    #[test]
    fn report_tracks_drawdown_delta_and_inventory_duration_on_the_event_clock() {
        let mut cfg = config();
        cfg.active_hedge_threshold_usd = 1_000_000_000.0;
        for instrument in &mut cfg.instruments {
            instrument.maker_fee = 0.0;
            instrument.taker_fee = 0.0;
            instrument.quote_profit_margin = 0.5;
            instrument.hedge_profit_margin = 0.5;
        }
        let mut runner = BacktestRunner::new(cfg).unwrap();
        let events = vec![
            NormalizedEvent::from(MarketEvent::Depth(OrderBook::one_level(
                "BTC-USDT",
                1,
                Level::new(49_999.0, 2.0),
                Level::new(50_001.0, 2.0),
            ))),
            NormalizedEvent::from(MarketEvent::Depth(OrderBook::one_level(
                "BTC-PERP",
                1,
                Level::new(49_999.0, 10_000.0),
                Level::new(50_001.0, 10_000.0),
            ))),
            NormalizedEvent::Order(OrderUpdate {
                ts_ms: 2,
                order_id: "external-fill".to_string(),
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                event: OrderEvent::FullyFilled,
                status: OrderStatus::Filled,
                price: 50_000.0,
                time_in_force: Some(TimeInForce::Ioc),
                qty: 1.0,
                open_qty: 0.0,
                filled_qty: 1.0,
                avg_fill_price: 50_000.0,
                last_fill_qty: 1.0,
                last_fill_price: 50_000.0,
                last_fill_liquidity: Some(FillLiquidity::Taker),
                last_fill_fee: None,
                reason: "fixture".to_string(),
            }),
            NormalizedEvent::from(MarketEvent::Depth(OrderBook::one_level(
                "BTC-USDT",
                12,
                Level::new(44_999.0, 2.0),
                Level::new(45_001.0, 2.0),
            ))),
        ];

        let report = runner.run(events).unwrap();

        assert_eq!(report.observed_duration_ns, 11_000_000);
        assert_eq!(report.final_equity_usd, -5_000.0);
        assert_eq!(report.max_drawdown_usd, 5_000.0);
        assert_eq!(report.max_abs_delta_usd, 50_000.0);
        assert_eq!(report.inventory_open_duration_ns, 10_000_000);
        assert!((report.inventory_open_fraction - 10.0 / 11.0).abs() < 1e-12);
        assert!(report.final_valuation_complete);
        assert_eq!(report.invalid_risk_metric_samples, 0);
        assert!(report.accounting_complete);
    }

    #[test]
    fn raw_horizon_extends_metric_duration_past_the_last_normalized_event() {
        let mut runner = BacktestRunner::new(config()).unwrap();
        for event in initial_books() {
            runner.process_replay_event(event).unwrap();
        }
        runner
            .process_replay_event(external_spot_fill(2, 50_000.0))
            .unwrap();
        runner.advance_raw_horizon(3 * NS_PER_MS).unwrap();

        let report = runner.finish_report().unwrap();

        assert_eq!(report.first_arrival_ns, Some(NS_PER_MS));
        assert_eq!(report.last_arrival_ns, Some(2 * NS_PER_MS));
        assert_eq!(report.observed_duration_ns, 2 * NS_PER_MS);
        assert_eq!(report.inventory_open_duration_ns, NS_PER_MS);
        assert_eq!(report.inventory_open_fraction, 0.5);
    }

    #[test]
    fn raw_capture_requires_every_strategy_book() {
        let mut runner = BacktestRunner::new(config()).unwrap();
        replay_raw_capture(
            include_str!("../../../fixtures/raw/okx/depth-gap.jsonl").as_bytes(),
            |event| runner.process_replay_event(event),
        )
        .unwrap();

        let error = runner.require_all_configured_books().unwrap_err();

        assert!(error.to_string().contains("BTC-PERP"));
    }
}
