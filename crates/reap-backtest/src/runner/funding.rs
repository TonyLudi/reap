use anyhow::{Result, bail};
use reap_core::{MarketEvent, NormalizedEvent, Symbol};

use crate::{BacktestRunner, FUNDING_LATE_TOLERANCE_NS, NS_PER_MS, ScheduledAction};

impl BacktestRunner {
    pub(super) fn register_funding_rate(&mut self, symbol: &str, rate: f64, funding_time_ms: u64) {
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
        if funding_time_ns.saturating_add(FUNDING_LATE_TOLERANCE_NS) < self.replay.now_ns {
            self.scheduled_funding.remove(&key);
            self.settled_funding.insert(key.clone());
            self.record_settled_funding(&key.0, key.1);
            self.missed_funding_settlements += 1;
            return;
        }
        let due_ns = if funding_time_ns < self.replay.now_ns {
            self.late_funding_rate_events += 1;
            self.replay.now_ns
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

    pub(super) fn preload_funding_settlement(&mut self, event: &NormalizedEvent) -> Result<()> {
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

    pub(super) fn settle_funding(&mut self, symbol: Symbol, funding_time_ms: u64) {
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
}
