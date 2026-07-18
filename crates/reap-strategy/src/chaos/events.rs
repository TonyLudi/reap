use reap_core::{MarketEvent, OrderBook, OrderIntent, StrategyEvent, TimeMs};

use crate::{ChaosExecutionIntent, Strategy};

use super::{ChaosStrategy, TimedPrice, should_accept_timestamp};

impl ChaosStrategy {
    pub(super) fn advance_time(&mut self, ts_ms: TimeMs) {
        self.now_ms = self.now_ms.max(ts_ms);
    }

    pub(super) fn on_depth(&mut self, book: &OrderBook) -> Vec<ChaosExecutionIntent> {
        self.advance_time(book.ts_ms);
        if let Some(entity) = self.entities.get_mut(&book.symbol) {
            entity.book = Some(book.clone());
        }
        self.refresh_quotes()
    }

    fn on_owned_depth(&mut self, book: OrderBook) -> Vec<ChaosExecutionIntent> {
        self.advance_time(book.ts_ms);
        if let Some(entity) = self.entities.get_mut(book.symbol.as_str()) {
            entity.book = Some(book);
        }
        self.refresh_quotes()
    }
}

impl ChaosStrategy {
    /// Processes one strategy event and returns in-memory typed execution purposes.
    ///
    /// Unlike serialized [`OrderIntent`] records, these values originate at the current Chaos
    /// decision sites and can be admitted to the live regular-execution policy.
    pub fn on_execution_event(&mut self, event: &StrategyEvent) -> Vec<ChaosExecutionIntent> {
        match event {
            StrategyEvent::Market(market) => self.on_market_event(market),
            StrategyEvent::Order(update) => self.on_order_update(update),
            StrategyEvent::Timer(timer) => {
                self.advance_time(timer.ts_ms);
                let mut intents = self.refresh_quotes();
                if self.halt_reason.is_none() && self.should_hedge_strategy_delta() {
                    intents.extend(self.hedge_delta(self.delta_to_hedge(), None, true));
                }
                intents
            }
            StrategyEvent::Account(update) => self.on_account_update(update),
            StrategyEvent::System(event) => self.on_system_event(event),
            StrategyEvent::Control(_) => Vec::new(),
        }
    }

    /// Owned-event counterpart of [`Self::on_execution_event`] for the single-writer engine.
    pub fn on_owned_execution_event(&mut self, event: StrategyEvent) -> Vec<ChaosExecutionIntent> {
        match event {
            StrategyEvent::Market(MarketEvent::Depth(book)) => self.on_owned_depth(book),
            event => self.on_execution_event(&event),
        }
    }

    pub(super) fn on_market_event(&mut self, event: &MarketEvent) -> Vec<ChaosExecutionIntent> {
        match event {
            MarketEvent::Depth(book) => self.on_depth(book),
            MarketEvent::Trade { ts_ms, .. } => {
                self.advance_time(*ts_ms);
                Vec::new()
            }
            MarketEvent::IndexPrice {
                ts_ms,
                symbol,
                price,
            } => {
                self.advance_time(*ts_ms);
                if !self.reference_health.index_symbols.contains(symbol) {
                    return Vec::new();
                }
                if price.is_finite() && *price > 0.0 {
                    self.reference_health
                        .index_prices
                        .entry(symbol.clone())
                        .and_modify(|value| {
                            if *ts_ms >= value.updated_ms {
                                value.price = *price;
                                value.updated_ms = *ts_ms;
                            }
                        })
                        .or_insert(TimedPrice {
                            price: *price,
                            updated_ms: *ts_ms,
                        });
                }
                self.refresh_quotes()
            }
            MarketEvent::FundingRate {
                ts_ms,
                symbol,
                rate,
                funding_time_ms,
                ..
            } => {
                self.advance_time(*ts_ms);
                if let Some(entity) = self.entities.get_mut(symbol)
                    && rate.is_finite()
                    && should_accept_timestamp(entity.funding_rate_updated_ms, *ts_ms)
                {
                    entity.funding_rate = *rate;
                    entity.funding_time_ms = *funding_time_ms;
                    entity.funding_rate_updated_ms = Some(*ts_ms);
                }
                self.refresh_quotes()
            }
            MarketEvent::BurstSignal {
                ts_ms,
                symbol,
                value,
            } => {
                self.advance_time(*ts_ms);
                let mut should_reprice = false;
                if *value == 0.0 {
                    if self.burst_symbol.as_deref() == Some(symbol) {
                        self.burst = 0.0;
                        self.burst_symbol = None;
                    }
                } else if self.burst == 0.0 {
                    self.burst = *value;
                    self.burst_symbol = Some(symbol.clone());
                    should_reprice = true;
                } else if self.burst * value < 0.0 {
                    self.burst = 0.0;
                    self.burst_symbol = None;
                } else if value.abs() > self.burst.abs() {
                    self.burst = *value;
                    self.burst_symbol = Some(symbol.clone());
                    should_reprice = true;
                }

                if should_reprice && self.config.act_on_burst {
                    self.refresh_quotes()
                } else {
                    Vec::new()
                }
            }
            MarketEvent::PriceLimits {
                ts_ms,
                symbol,
                mark_price,
                limit_down,
                limit_up,
            } => {
                self.advance_time(*ts_ms);
                if let Some(entity) = self.entities.get_mut(symbol) {
                    let valid_mark = mark_price.is_finite() && *mark_price > 0.0;
                    let valid_limits = limit_down.is_finite()
                        && *limit_down > 0.0
                        && limit_up.is_finite()
                        && *limit_up > 0.0;
                    if valid_mark && should_accept_timestamp(entity.mark_price_updated_ms, *ts_ms) {
                        entity.mark_price = Some(*mark_price);
                        entity.mark_price_updated_ms = Some(*ts_ms);
                    }
                    if should_accept_timestamp(entity.price_limits_updated_ms, *ts_ms) {
                        if limit_down.is_finite() && *limit_down > 0.0 {
                            entity.limit_down = Some(*limit_down);
                        }
                        if limit_up.is_finite() && *limit_up > 0.0 {
                            entity.limit_up = Some(*limit_up);
                        }
                        if valid_limits {
                            entity.price_limits_updated_ms = Some(*ts_ms);
                        }
                    }
                }
                self.refresh_quotes()
            }
        }
    }
}

impl Strategy for ChaosStrategy {
    fn on_event(&mut self, event: &StrategyEvent) -> Vec<OrderIntent> {
        self.on_execution_event(event)
            .into_iter()
            .map(ChaosExecutionIntent::into_order_intent)
            .collect()
    }

    fn on_owned_event(&mut self, event: StrategyEvent) -> Vec<OrderIntent> {
        self.on_owned_execution_event(event)
            .into_iter()
            .map(ChaosExecutionIntent::into_order_intent)
            .collect()
    }

    fn safety_halt_reason(&self) -> Option<&str> {
        ChaosStrategy::halt_reason(self)
    }
}
