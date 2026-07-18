use std::collections::VecDeque;

use anyhow::{Context, Result, bail};
use reap_core::{FillLiquidity, OrderEvent, OrderIntent, OrderUpdate, StrategyEvent, Symbol};
use reap_strategy::Strategy;

use crate::{
    BacktestLatencyClass, BacktestRunner, MatchingEngine, ScheduledAction, retime_order_update,
    time_ms,
};

impl BacktestRunner {
    pub(super) fn route_exchange_updates(&mut self, updates: Vec<OrderUpdate>) -> Result<()> {
        for update in updates {
            if update.event == OrderEvent::Cancelled {
                self.cancelled_orders += 1;
            } else if update.event == OrderEvent::Rejected {
                self.rejected_orders += 1;
            }

            let account_update = if update.has_fill() {
                if self.opening_equity_usd.is_none() {
                    bail!(
                        "fill for {} arrived before the configured opening portfolio could be valued",
                        update.symbol
                    );
                }
                self.fills += 1;
                match update.last_fill_liquidity {
                    Some(FillLiquidity::Maker) => self.maker_fills += 1,
                    Some(FillLiquidity::Taker) => self.taker_fills += 1,
                    None => {}
                }
                let currency_rates = self.fresh_currency_rates();
                self.portfolio.apply_fill(&update, &currency_rates);
                self.sample_risk_metrics();
                Some(self.current_account_update(Some(&update.symbol)))
            } else {
                None
            };

            let order_update_delay_ms = self
                .latency_sampler
                .sample(BacktestLatencyClass::OrderUpdate, &update.symbol);
            let fill_symbol = update.symbol.clone();
            self.schedule_after(order_update_delay_ms, ScheduledAction::DeliverOrder(update));
            if let Some(account_update) = account_update {
                let fill_account_delay_ms = self
                    .latency_sampler
                    .sample(BacktestLatencyClass::OrderFill, &fill_symbol);
                self.pending_fill_account_updates =
                    self.pending_fill_account_updates.saturating_add(1);
                self.schedule_after(
                    fill_account_delay_ms,
                    ScheduledAction::DeliverAccount(account_update),
                );
            }
        }
        Ok(())
    }

    pub(super) fn accept_intents(&mut self, commands: Vec<OrderIntent>) -> Result<()> {
        let mut queue = VecDeque::from(commands);
        while let Some(command) = queue.pop_front() {
            match command {
                OrderIntent::NewOrder(order) => {
                    self.observe_order_entry_readiness();
                    if !self.order_entry_ready() {
                        self.new_orders_blocked_not_ready =
                            self.new_orders_blocked_not_ready.saturating_add(1);
                        continue;
                    }
                    self.orders_sent += 1;
                    let symbol = order.symbol.clone();
                    let now_ms = time_ms(self.now_ns);
                    let (order_id, pending) =
                        self.matcher_mut(&symbol)?.prepare_submit(order, now_ms);
                    let order_entry_delay_ms = self
                        .latency_sampler
                        .sample(BacktestLatencyClass::MatchingNew, &symbol);
                    self.schedule_after(
                        order_entry_delay_ms,
                        ScheduledAction::ActivateOrder {
                            symbol,
                            order_id: order_id.clone(),
                        },
                    );

                    let pending = retime_order_update(pending, now_ms);
                    queue.extend(self.strategy.on_event(&StrategyEvent::Order(pending)));
                }
                OrderIntent::CancelOrder { order_id, reason } => {
                    self.cancel_requests += 1;
                    let Some(symbol) = self.open_order_symbol(&order_id) else {
                        self.ignored_cancel_requests += 1;
                        continue;
                    };
                    if !self.pending_cancels.insert(order_id.clone()) {
                        self.deduplicated_cancel_requests += 1;
                        continue;
                    }
                    let cancel_delay_ms = self
                        .latency_sampler
                        .sample(BacktestLatencyClass::MatchingCancel, &symbol);
                    self.schedule_after(
                        cancel_delay_ms,
                        ScheduledAction::CancelOrder {
                            symbol,
                            order_id,
                            reason,
                        },
                    );
                }
            }
        }
        Ok(())
    }

    pub(super) fn matcher_mut(&mut self, symbol: &str) -> Result<&mut MatchingEngine> {
        self.matchers
            .get_mut(symbol)
            .with_context(|| format!("no matcher configured for symbol {symbol}"))
    }

    fn open_order_symbol(&self, order_id: &str) -> Option<Symbol> {
        self.matchers
            .iter()
            .find(|(_, matcher)| matcher.is_open_order(order_id))
            .map(|(symbol, _)| symbol.clone())
    }
}
