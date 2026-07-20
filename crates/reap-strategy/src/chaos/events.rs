use reap_core::{MarketEvent, OrderBook, OrderIntent, StrategyEvent, TimeMs};

use crate::{ChaosExecutionIntent, Strategy};

use super::{ChaosStrategy, TimedPrice, should_accept_timestamp};

#[derive(Debug, Clone, Copy)]
struct LocalDeliveryClocks {
    receipt_ns: u64,
    processing_ns: u64,
    observed_now_ms: TimeMs,
}

impl ChaosStrategy {
    pub(super) fn advance_time(&mut self, ts_ms: TimeMs) {
        self.now_ms = self.now_ms.max(ts_ms);
    }

    pub(super) fn on_depth(&mut self, book: &OrderBook) -> Vec<ChaosExecutionIntent> {
        self.advance_time(book.ts_ms);
        if let Some(entity) = self.entities.get_mut(&book.symbol) {
            entity.book = Some(book.clone());
            entity.implied_depth.on_depth(book.ts_ms);
        }
        self.refresh_quotes()
    }

    fn on_owned_depth_at(
        &mut self,
        book: OrderBook,
        arrival_ns: u64,
        observed_now_ms: TimeMs,
        strategy_is_live: bool,
        refresh_when_inactive: bool,
        mut worker_clock: impl FnMut() -> TimeMs,
    ) -> Vec<ChaosExecutionIntent> {
        self.advance_time(book.ts_ms);
        if let Some(entity) = self.entities.get_mut(book.symbol.as_str()) {
            entity.implied_depth.on_depth(observed_now_ms);
            entity.book = Some(book);
        }
        if strategy_is_live {
            if self.trigger_live_depth_pricing_worker(arrival_ns, observed_now_ms) {
                let work_start_ms = worker_clock();
                self.start_immediate_live_pricing_worker(work_start_ms);
                self.advance_time(work_start_ms);
                let intents = self.refresh_quotes();
                self.finish_live_pricing_worker(worker_clock());
                intents
            } else {
                Vec::new()
            }
        } else {
            if refresh_when_inactive {
                self.observe_compatibility_depth_worker(arrival_ns, observed_now_ms);
                self.refresh_quotes()
            } else {
                Vec::new()
            }
        }
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
        let observed_now_ms = strategy_event_ts_ms(&event);
        // This compatibility-only seam has no captured local arrival. Give
        // its private worker shadow a causal deterministic timeline rather
        // than pinning every depth to nanosecond zero, which would retain
        // already-due Java timers indefinitely.
        let Some(processing_ns) = observed_now_ms.checked_mul(1_000_000) else {
            self.halt_reason =
                Some("compatibility strategy timestamp exceeds nanosecond range".to_string());
            return Vec::new();
        };
        self.on_owned_execution_event_at(
            event,
            processing_ns,
            processing_ns,
            observed_now_ms,
            false,
        )
    }

    /// Receipt/processing-aware counterpart used only by deterministic replay
    /// and the existing live single writer. Neither local clock is written
    /// into the public event or strategy time.
    pub fn on_owned_execution_event_at(
        &mut self,
        event: StrategyEvent,
        receipt_ns: u64,
        processing_ns: u64,
        observed_now_ms: TimeMs,
        strategy_is_live: bool,
    ) -> Vec<ChaosExecutionIntent> {
        self.on_owned_execution_event_at_with_finish_clock(
            event,
            receipt_ns,
            processing_ns,
            observed_now_ms,
            strategy_is_live,
            || observed_now_ms,
        )
    }

    /// Live-clock counterpart that samples pricing completion after the
    /// synchronous worker finishes. Replay callers use the deterministic
    /// zero-duration wrapper above.
    pub fn on_owned_execution_event_at_with_finish_clock(
        &mut self,
        event: StrategyEvent,
        receipt_ns: u64,
        processing_ns: u64,
        observed_now_ms: TimeMs,
        strategy_is_live: bool,
        worker_clock: impl FnMut() -> TimeMs,
    ) -> Vec<ChaosExecutionIntent> {
        self.on_owned_execution_event_at_inner(
            event,
            LocalDeliveryClocks {
                receipt_ns,
                processing_ns,
                observed_now_ms,
            },
            strategy_is_live,
            true,
            worker_clock,
        )
    }

    /// Live-runtime counterpart. A reached depth still updates entity state
    /// while a closed Live gate suppresses Java's pricing-worker invocation.
    pub fn on_owned_live_execution_event_at_with_finish_clock(
        &mut self,
        event: StrategyEvent,
        receipt_ns: u64,
        processing_ns: u64,
        observed_now_ms: TimeMs,
        strategy_is_live: bool,
        worker_clock: impl FnMut() -> TimeMs,
    ) -> Vec<ChaosExecutionIntent> {
        self.on_owned_execution_event_at_inner(
            event,
            LocalDeliveryClocks {
                receipt_ns,
                processing_ns,
                observed_now_ms,
            },
            strategy_is_live,
            false,
            worker_clock,
        )
    }

    fn on_owned_execution_event_at_inner(
        &mut self,
        event: StrategyEvent,
        clocks: LocalDeliveryClocks,
        strategy_is_live: bool,
        refresh_when_inactive: bool,
        worker_clock: impl FnMut() -> TimeMs,
    ) -> Vec<ChaosExecutionIntent> {
        match event {
            StrategyEvent::Market(MarketEvent::Depth(book)) => self.on_owned_depth_at(
                book,
                clocks.processing_ns,
                clocks.observed_now_ms,
                strategy_is_live,
                refresh_when_inactive,
                worker_clock,
            ),
            StrategyEvent::Market(MarketEvent::Trade {
                ts_ms,
                symbol,
                price,
                qty,
                taker_side,
            }) => {
                self.advance_time(ts_ms);
                let crossed = self.entities.get_mut(&symbol).is_some_and(|entity| {
                    entity.implied_depth.on_public_trade(
                        entity.book.as_ref(),
                        price,
                        qty,
                        taker_side,
                    )
                });
                if crossed && strategy_is_live {
                    self.schedule_trade_reprice(clocks.receipt_ns, clocks.processing_ns);
                }
                Vec::new()
            }
            event => self.on_execution_event(&event),
        }
    }

    /// Borrowed counterpart of [`Self::on_owned_execution_event_at`].
    pub fn on_execution_event_at(
        &mut self,
        event: &StrategyEvent,
        receipt_ns: u64,
        processing_ns: u64,
        observed_now_ms: TimeMs,
        strategy_is_live: bool,
    ) -> Vec<ChaosExecutionIntent> {
        match event {
            StrategyEvent::Market(MarketEvent::Depth(book)) => {
                self.advance_time(book.ts_ms);
                if let Some(entity) = self.entities.get_mut(&book.symbol) {
                    entity.implied_depth.on_depth(observed_now_ms);
                    entity.book = Some(book.clone());
                }
                if strategy_is_live {
                    if self.trigger_live_depth_pricing_worker(processing_ns, observed_now_ms) {
                        self.start_immediate_live_pricing_worker(observed_now_ms);
                        self.advance_time(observed_now_ms);
                        let intents = self.refresh_quotes();
                        self.finish_live_pricing_worker(observed_now_ms);
                        intents
                    } else {
                        Vec::new()
                    }
                } else {
                    self.observe_compatibility_depth_worker(processing_ns, observed_now_ms);
                    self.refresh_quotes()
                }
            }
            StrategyEvent::Market(MarketEvent::Trade {
                ts_ms,
                symbol,
                price,
                qty,
                taker_side,
            }) => {
                self.advance_time(*ts_ms);
                let crossed = self.entities.get_mut(symbol).is_some_and(|entity| {
                    entity.implied_depth.on_public_trade(
                        entity.book.as_ref(),
                        *price,
                        *qty,
                        *taker_side,
                    )
                });
                if crossed && strategy_is_live {
                    self.schedule_trade_reprice(receipt_ns, processing_ns);
                }
                Vec::new()
            }
            event => self.on_execution_event(event),
        }
    }

    pub(super) fn on_market_event(&mut self, event: &MarketEvent) -> Vec<ChaosExecutionIntent> {
        match event {
            MarketEvent::Depth(book) => self.on_depth(book),
            MarketEvent::Trade {
                ts_ms,
                symbol,
                price,
                qty,
                taker_side,
            } => {
                self.advance_time(*ts_ms);
                if let Some(entity) = self.entities.get_mut(symbol) {
                    entity.implied_depth.on_public_trade(
                        entity.book.as_ref(),
                        *price,
                        *qty,
                        *taker_side,
                    );
                }
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
                    if self.pricing.burst_symbol.as_deref() == Some(symbol) {
                        self.pricing.burst = 0.0;
                        self.pricing.burst_symbol = None;
                    }
                } else if self.pricing.burst == 0.0 {
                    self.pricing.burst = *value;
                    self.pricing.burst_symbol = Some(symbol.clone());
                    should_reprice = true;
                } else if self.pricing.burst * value < 0.0 {
                    self.pricing.burst = 0.0;
                    self.pricing.burst_symbol = None;
                } else if value.abs() > self.pricing.burst.abs() {
                    self.pricing.burst = *value;
                    self.pricing.burst_symbol = Some(symbol.clone());
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

fn strategy_event_ts_ms(event: &StrategyEvent) -> TimeMs {
    match event {
        StrategyEvent::Market(event) => event.ts_ms(),
        StrategyEvent::Order(update) => update.ts_ms,
        StrategyEvent::Account(update) => update.ts_ms,
        StrategyEvent::Timer(event) => event.ts_ms,
        StrategyEvent::Control(event) => event.ts_ms,
        StrategyEvent::System(event) => event.ts_ms,
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
