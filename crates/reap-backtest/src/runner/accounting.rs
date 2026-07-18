use std::collections::HashMap;

use anyhow::{Result, bail};
use reap_core::{AccountUpdate, Balance, MarginSnapshot, Position, StrategyEvent, Symbol};
use reap_strategy::Strategy;

use crate::{ACCOUNT_REFRESH_INTERVAL_NS, BacktestRunner, ScheduledAction, time_ms};

impl BacktestRunner {
    pub(super) fn deliver_initial_account_snapshot(&mut self) -> Result<()> {
        if self.initial_account_snapshot_delivered {
            return Ok(());
        }
        let commands = self.strategy.on_event(&StrategyEvent::Account(
            self.initial_portfolio
                .account_update(time_ms(self.replay.now_ns)),
        ));
        if !commands.is_empty() {
            bail!("initial portfolio unexpectedly produced strategy order intents");
        }
        self.initial_account_snapshot_delivered = true;
        self.last_account_publish_ns = Some(self.replay.now_ns);
        self.schedule_next_account_refresh();
        Ok(())
    }

    pub(super) fn current_account_update(&self, source_symbol: Option<&str>) -> AccountUpdate {
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
            ts_ms: time_ms(self.replay.now_ns),
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

    pub(super) fn schedule_next_account_refresh(&mut self) {
        if self.initial_portfolio.is_empty() {
            return;
        }
        let due_ns = self
            .replay
            .now_ns
            .saturating_add(ACCOUNT_REFRESH_INTERVAL_NS);
        if due_ns > self.replay.now_ns {
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
}
