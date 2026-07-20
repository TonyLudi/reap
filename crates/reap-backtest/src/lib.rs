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

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[cfg(test)]
use std::collections::HashMap;

use reap_core::{AccountUpdate, MarketEvent, OrderUpdate, StrategyEvent, Symbol};
#[cfg(test)]
use reap_core::{FillLiquidity, FundingSettlement, NormalizedEvent, OrderEvent, OrderIntent};
use reap_strategy::{ChaosConfig, ChaosStrategy};

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
#[path = "runner/metrics.rs"]
mod runner_metrics;
#[path = "runner/orders.rs"]
mod runner_orders;
#[path = "runner/report.rs"]
mod runner_report;
#[path = "runner/schedule.rs"]
mod runner_schedule;
#[path = "runner/state.rs"]
mod runner_state;
#[path = "runner/valuation.rs"]
mod runner_valuation;
use runner_construction::validate_currency_rate_coverage;
use runner_state::{
    AccountingState, FundingState, MetricState, OrderLifecycleState, ReplayState, ScheduleState,
    ValuationState,
};

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
    DeliverTradeStrategy {
        event: StrategyEvent,
        arrival_ns: u64,
    },
    TradeRepriceWake {
        deadline_ns: u64,
    },
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
    execution: BacktestExecutionConfig,
    latency_sampler: BacktestLatencySampler,
    replay: ReplayState,
    schedule: ScheduleState,
    orders: OrderLifecycleState,
    valuation: ValuationState,
    funding: FundingState,
    accounting: AccountingState,
    metrics: MetricState,
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
