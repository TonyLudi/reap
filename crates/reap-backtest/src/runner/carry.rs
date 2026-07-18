use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, bail};
use reap_core::Symbol;
use reap_strategy::ChaosConfig;

use crate::portfolio::Portfolio;
use crate::{
    BACKTEST_CARRY_STATE_SCHEMA_VERSION, BacktestCarryCurrencyRate, BacktestCarryState,
    BacktestExecutionConfig, BacktestPendingFundingCarry, BacktestRunner, NS_PER_MS,
    ScheduledAction,
};

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
    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_settled_carry_state(
        &self,
        marks: &HashMap<Symbol, f64>,
        currency_rates: &HashMap<String, f64>,
        terminal_equity_usd: Option<f64>,
        final_valuation_complete: bool,
        accounting_complete: bool,
    ) -> (Option<BacktestCarryState>, Vec<String>) {
        let mut failures = Vec::new();
        if self.accounting.initial_portfolio.is_empty() {
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
            let portfolio = self.accounting.portfolio.settled_initial_portfolio(
                &self.accounting.initial_portfolio,
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
                    .valuation
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
                .schedule
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
                        realized_rate: self.funding.realized_funding_rates.get(&key).copied(),
                    })
                })
                .collect::<Vec<_>>();
            pending_funding.sort_by(|left, right| {
                (&left.symbol, left.funding_time_ms).cmp(&(&right.symbol, right.funding_time_ms))
            });

            let state = BacktestCarryState {
                schema_version: BACKTEST_CARRY_STATE_SCHEMA_VERSION,
                settled_at_ns: self.replay.now_ns,
                terminal_equity_usd: terminal_equity_usd
                    .context("terminal equity disappeared while building settled carry")?,
                source_raw_boundary: self.replay.raw_replay_boundary.clone(),
                portfolio,
                terminal_depth_marks: self
                    .valuation
                    .depth_marks
                    .iter()
                    .filter(|(_, mark)| mark.is_finite() && **mark > 0.0)
                    .map(|(symbol, mark)| (symbol.clone(), *mark))
                    .collect(),
                terminal_exchange_marks: self
                    .valuation
                    .exchange_marks
                    .iter()
                    .filter(|(_, mark)| mark.is_finite() && **mark > 0.0)
                    .map(|(symbol, mark)| (symbol.clone(), *mark))
                    .collect(),
                currency_rates: carry_rates,
                pending_funding,
                last_settled_funding_time_ms: self.funding.last_settled_funding_time_ms.clone(),
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
}
