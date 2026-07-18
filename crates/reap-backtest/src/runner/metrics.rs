use crate::BacktestRunner;

impl BacktestRunner {
    pub(super) fn advance_metric_clock(&mut self, target_ns: u64) {
        if let Some(previous_ns) = self.metric_clock_ns {
            // Carry actions may run before the first input in the next replay segment.
            let observation_start_ns = self.replay.first_arrival_ns.unwrap_or(target_ns);
            let elapsed_ns = target_ns.saturating_sub(previous_ns.max(observation_start_ns));
            self.abs_delta_time_integral += self.current_abs_delta_usd * elapsed_ns as f64;
            if self.current_inventory_open {
                self.inventory_open_duration_ns =
                    self.inventory_open_duration_ns.saturating_add(elapsed_ns);
            }
        }
        self.metric_clock_ns = Some(target_ns);
    }

    pub(super) fn sample_risk_metrics(&mut self) {
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
            .orders
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
}
