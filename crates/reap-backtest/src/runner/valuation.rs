use std::collections::HashMap;

use reap_core::Symbol;

use crate::{BacktestCurrencyRateReport, BacktestRunner, CurrencyRateObservation, NS_PER_MS};

impl BacktestRunner {
    pub(super) fn register_currency_rate(
        &mut self,
        index_symbol: &str,
        usd_per_unit: f64,
        source_ts_ms: u64,
    ) {
        let Some(currency) = self
            .valuation
            .currency_by_index_symbol
            .get(index_symbol)
            .cloned()
        else {
            return;
        };
        self.valuation.currency_rate_events = self.valuation.currency_rate_events.saturating_add(1);
        if !usd_per_unit.is_finite() || usd_per_unit <= 0.0 {
            self.valuation.invalid_currency_rate_events = self
                .valuation
                .invalid_currency_rate_events
                .saturating_add(1);
            return;
        }
        self.valuation.currency_rate_observations.insert(
            currency,
            CurrencyRateObservation {
                usd_per_unit,
                source_ts_ms,
                effective_at_ns: self.replay.now_ns,
            },
        );
    }

    pub(super) fn fresh_currency_rates(&self) -> HashMap<String, f64> {
        let mut rates = HashMap::from([("USD".to_string(), 1.0)]);
        for route in &self.execution.currency_rates {
            let Some(observation) = self
                .valuation
                .currency_rate_observations
                .get(&route.currency)
            else {
                continue;
            };
            let maximum_age_ns = route.max_age_ms.saturating_mul(NS_PER_MS);
            let source_ns = observation.source_ts_ms.saturating_mul(NS_PER_MS);
            if self.replay.now_ns.saturating_sub(source_ns) <= maximum_age_ns {
                rates.insert(route.currency.clone(), observation.usd_per_unit);
            }
        }
        rates
    }

    pub(super) fn fallback_currency_rates(&self) -> HashMap<String, f64> {
        let mut rates = HashMap::from([("USD".to_string(), 1.0)]);
        for route in &self.execution.currency_rates {
            let rate = self
                .valuation
                .currency_rate_observations
                .get(&route.currency)
                .map(|observation| observation.usd_per_unit)
                .unwrap_or(1.0);
            rates.insert(route.currency.clone(), rate);
        }
        rates
    }

    pub(super) fn currency_rate_reports(&self) -> Vec<BacktestCurrencyRateReport> {
        let mut reports = self
            .execution
            .currency_rates
            .iter()
            .map(|route| {
                let observation = self
                    .valuation
                    .currency_rate_observations
                    .get(&route.currency);
                let age_ns = observation.map(|observation| {
                    self.replay
                        .now_ns
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

    pub(super) fn active_order_notional_usd_checked(
        &self,
        currency_rates: &HashMap<String, f64>,
    ) -> Option<f64> {
        self.orders
            .matchers
            .iter()
            .try_fold(0.0, |total, (symbol, matcher)| {
                let notional = matcher.active_order_notional_checked()?;
                if notional == 0.0 {
                    return Some(total);
                }
                let rate = self
                    .accounting
                    .portfolio
                    .notional_currency_rate_usd_checked(symbol, currency_rates)?;
                let value = notional * rate;
                value.is_finite().then_some(total + value)
            })
            .filter(|notional| notional.is_finite())
    }

    pub(super) fn active_order_notional_usd(&self, currency_rates: &HashMap<String, f64>) -> f64 {
        self.orders
            .matchers
            .iter()
            .filter_map(|(symbol, matcher)| {
                matcher.active_order_notional_checked().map(|notional| {
                    notional
                        * self
                            .accounting
                            .portfolio
                            .notional_currency_rate_usd(symbol, currency_rates)
                })
            })
            .sum()
    }

    pub(super) fn order_entry_ready(&self) -> bool {
        self.valuation_inputs_ready() && self.valuation.opening_equity_usd.is_some()
    }

    fn valuation_inputs_ready(&self) -> bool {
        self.orders
            .matchers
            .values()
            .all(|matcher| matcher.depth().is_some())
            && self.execution.currency_rates.iter().all(|route| {
                let Some(observation) = self
                    .valuation
                    .currency_rate_observations
                    .get(&route.currency)
                else {
                    return false;
                };
                observation.usd_per_unit.is_finite()
                    && observation.usd_per_unit > 0.0
                    && self
                        .replay
                        .now_ns
                        .saturating_sub(observation.source_ts_ms.saturating_mul(NS_PER_MS))
                        <= route.max_age_ms.saturating_mul(NS_PER_MS)
            })
    }

    pub(super) fn observe_order_entry_readiness(&mut self) {
        if self.valuation.opening_equity_usd.is_none() && self.valuation_inputs_ready() {
            let marks = self.valuation_marks();
            let currency_rates = self.fresh_currency_rates();
            if let Some(opening_equity_usd) = self
                .accounting
                .portfolio
                .equity_usd_checked(&marks, &currency_rates)
            {
                self.valuation.opening_equity_usd = Some(opening_equity_usd);
                self.valuation.opening_valuation_at_ns = Some(self.replay.now_ns);
                self.peak_equity_usd = opening_equity_usd;
            }
        }
        if self.orders.order_entry_ready_at_ns.is_none() && self.order_entry_ready() {
            self.orders.order_entry_ready_at_ns = Some(self.replay.now_ns);
        }
    }

    pub(super) fn valuation_marks(&self) -> HashMap<Symbol, f64> {
        let mut marks = self
            .orders
            .matchers
            .iter()
            .filter_map(|(symbol, matcher)| Some((symbol.clone(), matcher.depth()?.mid()?)))
            .collect::<HashMap<_, _>>();
        marks.extend(self.valuation.depth_marks.clone());
        marks.extend(self.valuation.exchange_marks.clone());
        marks
    }
}
