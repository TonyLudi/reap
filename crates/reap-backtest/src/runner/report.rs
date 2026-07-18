use anyhow::{Result, bail};

use crate::{BacktestReport, BacktestRunner, MatchingEngine, ScheduledAction};

impl BacktestRunner {
    pub(super) fn finish_report(&mut self) -> Result<BacktestReport> {
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
