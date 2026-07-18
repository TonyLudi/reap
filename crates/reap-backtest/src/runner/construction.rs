use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Context, Result, bail};
use reap_strategy::{ChaosConfig, ChaosStrategy};

use crate::execution::BacktestLatencySampler;
use crate::portfolio::{Portfolio, required_accounting_currencies};
use crate::{
    AccountingState, BacktestCarryState, BacktestConfig, BacktestExecutionConfig,
    BacktestInitialPortfolioConfig, BacktestRunner, BacktestTimeBasis, CurrencyRateObservation,
    FundingState, MatchingAssumptions, MatchingEngine, OrderLifecycleState, ReplayState,
    ScheduleState, ScheduledAction, ValuationState,
};

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
        runner.replay.now_ns = carry.settled_at_ns;
        runner.valuation.opening_equity_usd = Some(carry.terminal_equity_usd);
        runner.valuation.opening_valuation_at_ns = Some(carry.settled_at_ns);
        runner.peak_equity_usd = carry.terminal_equity_usd;
        runner.valuation.depth_marks = carry.terminal_depth_marks.into_iter().collect();
        runner.valuation.exchange_marks = carry.terminal_exchange_marks.into_iter().collect();
        runner.valuation.currency_rate_observations = carry
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
                runner
                    .funding
                    .realized_funding_rates
                    .insert(key.clone(), rate);
            }
            runner.funding.scheduled_funding.insert(key);
            runner.schedule_at(
                pending.due_at_ns,
                ScheduledAction::SettleFunding {
                    symbol: pending.symbol,
                    funding_time_ms: pending.funding_time_ms,
                },
            );
        }
        runner.funding.last_settled_funding_time_ms = carry.last_settled_funding_time_ms;
        runner.replay.carry_source_boundary = carry.source_raw_boundary;
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
        let initial_account_snapshot_delivered = initial_portfolio.is_empty();
        let strategy =
            ChaosStrategy::new(config.clone()).context("invalid chaos/iarb2 configuration")?;
        Ok(Self {
            strategy_config: config,
            strategy,
            execution,
            latency_sampler,
            replay: ReplayState {
                time_basis: BacktestTimeBasis::EventTimestampMs,
                raw_replay_boundary: None,
                carry_source_boundary: None,
                now_ns: 0,
                first_arrival_ns: None,
                last_arrival_ns: None,
                input_events: 0,
                input_clock_regressions: 0,
                max_input_clock_regression_ns: 0,
            },
            schedule: ScheduleState {
                scheduled: BTreeMap::new(),
                next_action_seq: 1,
            },
            orders: OrderLifecycleState {
                matchers,
                initial_account_snapshot_delivered,
                pending_cancels: HashSet::new(),
                pending_fill_account_updates: 0,
                last_account_publish_ns: None,
                periodic_account_refreshes: 0,
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
            },
            valuation: ValuationState {
                depth_marks: HashMap::new(),
                exchange_marks: HashMap::new(),
                currency_by_index_symbol,
                currency_rate_observations: HashMap::new(),
                currency_rate_events: 0,
                invalid_currency_rate_events: 0,
                opening_equity_usd,
                opening_valuation_at_ns,
            },
            funding: FundingState {
                realized_funding_rates: HashMap::new(),
                scheduled_funding: HashSet::new(),
                settled_funding: HashSet::new(),
                last_settled_funding_time_ms: BTreeMap::new(),
            },
            accounting: AccountingState {
                portfolio,
                initial_portfolio,
                funding_rate_events: 0,
                funding_settlements: 0,
                late_funding_rate_events: 0,
                invalid_funding_rate_events: 0,
                missed_funding_settlements: 0,
                funding_settlement_failures: 0,
            },
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

pub(super) fn validate_currency_rate_coverage(
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
