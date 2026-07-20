#[cfg(test)]
use std::cell::RefCell;
use std::collections::VecDeque;

use anyhow::{Context, Result, bail};
use reap_core::{FillLiquidity, OrderEvent, OrderIntent, OrderUpdate, StrategyEvent, Symbol};
#[cfg(test)]
use reap_strategy::ChaosExecutionPurpose;
use reap_strategy::{ChaosExecutionIntent, ChaosExecutionPurpose as ExecutionPurpose};

use crate::{
    BacktestLatencyClass, BacktestRunner, MatchingEngine, ScheduledAction, retime_order_update,
    time_ms,
};

enum QueuedIntent {
    Chaos(ChaosExecutionIntent),
    #[cfg(test)]
    Legacy(OrderIntent),
}

#[cfg(test)]
thread_local! {
    static CHAOS_INTENT_TRACE: RefCell<Option<Vec<(ChaosExecutionPurpose, OrderIntent)>>> =
        const { RefCell::new(None) };
}

#[cfg(test)]
pub(super) struct ChaosIntentTraceGuard;

#[cfg(test)]
impl Drop for ChaosIntentTraceGuard {
    fn drop(&mut self) {
        CHAOS_INTENT_TRACE.with(|trace| *trace.borrow_mut() = None);
    }
}

impl BacktestRunner {
    #[cfg(test)]
    pub(super) fn begin_chaos_intent_trace(&mut self) -> ChaosIntentTraceGuard {
        CHAOS_INTENT_TRACE.with(|trace| {
            assert!(
                trace.borrow().is_none(),
                "a backtest Chaos intent trace is already active on this test thread"
            );
            *trace.borrow_mut() = Some(Vec::new());
        });
        ChaosIntentTraceGuard
    }

    #[cfg(test)]
    pub(super) fn take_chaos_intent_trace(&mut self) -> Vec<(ChaosExecutionPurpose, OrderIntent)> {
        CHAOS_INTENT_TRACE.with(|trace| {
            let mut trace = trace.borrow_mut();
            std::mem::take(
                trace
                    .as_mut()
                    .expect("backtest Chaos intent trace must be active"),
            )
        })
    }

    pub(super) fn route_exchange_updates(&mut self, updates: Vec<OrderUpdate>) -> Result<()> {
        for update in updates {
            if update.event == OrderEvent::Cancelled {
                self.orders.cancelled_orders += 1;
            } else if update.event == OrderEvent::Rejected {
                self.orders.rejected_orders += 1;
            }

            let account_update = if update.has_fill() {
                if self.valuation.opening_equity_usd.is_none() {
                    bail!(
                        "fill for {} arrived before the configured opening portfolio could be valued",
                        update.symbol
                    );
                }
                self.orders.fills += 1;
                match update.last_fill_liquidity {
                    Some(FillLiquidity::Maker) => self.orders.maker_fills += 1,
                    Some(FillLiquidity::Taker) => self.orders.taker_fills += 1,
                    None => {}
                }
                let currency_rates = self.fresh_currency_rates();
                self.accounting
                    .portfolio
                    .apply_fill(&update, &currency_rates);
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
                self.orders.pending_fill_account_updates =
                    self.orders.pending_fill_account_updates.saturating_add(1);
                self.schedule_after(
                    fill_account_delay_ms,
                    ScheduledAction::DeliverAccount(account_update),
                );
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn accept_intents(&mut self, commands: Vec<OrderIntent>) -> Result<()> {
        self.accept_queued_intents(commands.into_iter().map(QueuedIntent::Legacy))
    }

    pub(super) fn accept_chaos_intents(
        &mut self,
        intents: Vec<ChaosExecutionIntent>,
    ) -> Result<()> {
        #[cfg(test)]
        CHAOS_INTENT_TRACE.with(|trace| {
            if let Some(trace) = trace.borrow_mut().as_mut() {
                trace.extend(
                    intents
                        .iter()
                        .map(|intent| (intent.purpose(), intent.to_order_intent())),
                );
            }
        });
        self.accept_queued_intents(intents.into_iter().map(QueuedIntent::Chaos))
    }

    fn accept_queued_intents(
        &mut self,
        intents: impl IntoIterator<Item = QueuedIntent>,
    ) -> Result<()> {
        let mut queue = VecDeque::from_iter(intents);
        while let Some(queued) = queue.pop_front() {
            match queued {
                QueuedIntent::Chaos(intent)
                    if matches!(
                        intent.purpose(),
                        ExecutionPurpose::Quote | ExecutionPurpose::Hedge
                    ) =>
                {
                    self.observe_order_entry_readiness();
                    if !self.order_entry_ready() {
                        self.orders.new_orders_blocked_not_ready =
                            self.orders.new_orders_blocked_not_ready.saturating_add(1);
                        continue;
                    }
                    self.orders.orders_sent += 1;
                    let symbol = intent
                        .as_quote()
                        .map(|quote| quote.symbol())
                        .or_else(|| intent.as_hedge().map(|hedge| hedge.symbol()))
                        .expect("quote or hedge purpose must expose a submit symbol")
                        .to_string();
                    let now_ms = time_ms(self.replay.now_ns);
                    let (order_id, pending) = {
                        let matcher = self.orders.matchers.get_mut(&symbol).with_context(|| {
                            format!("no matcher configured for symbol {symbol}")
                        })?;
                        self.strategy.with_locally_sent_intent(
                            intent,
                            || now_ms,
                            |intent| match intent.into_order_intent() {
                                OrderIntent::NewOrder(order) => {
                                    Ok::<_, anyhow::Error>(matcher.prepare_submit(order, now_ms))
                                }
                                OrderIntent::CancelOrder { .. } => {
                                    unreachable!("quote or hedge must lower to a new order")
                                }
                            },
                        )?
                    };
                    self.finish_accepted_new_order(symbol, order_id, pending, &mut queue);
                }
                QueuedIntent::Chaos(intent) => match intent.into_order_intent() {
                    OrderIntent::CancelOrder { order_id, reason } => {
                        self.accept_cancel_order(order_id, reason);
                    }
                    OrderIntent::NewOrder(_) => {
                        unreachable!("only CancelOwned reaches the non-submit Chaos branch")
                    }
                },
                #[cfg(test)]
                QueuedIntent::Legacy(OrderIntent::NewOrder(order)) => {
                    self.observe_order_entry_readiness();
                    if !self.order_entry_ready() {
                        self.orders.new_orders_blocked_not_ready =
                            self.orders.new_orders_blocked_not_ready.saturating_add(1);
                        continue;
                    }
                    self.orders.orders_sent += 1;
                    let symbol = order.symbol.clone();
                    let now_ms = time_ms(self.replay.now_ns);
                    let (order_id, pending) =
                        self.matcher_mut(&symbol)?.prepare_submit(order, now_ms);
                    self.finish_accepted_new_order(symbol, order_id, pending, &mut queue);
                }
                #[cfg(test)]
                QueuedIntent::Legacy(OrderIntent::CancelOrder { order_id, reason }) => {
                    self.accept_cancel_order(order_id, reason);
                }
            }
        }
        Ok(())
    }

    fn finish_accepted_new_order(
        &mut self,
        symbol: Symbol,
        order_id: String,
        pending: OrderUpdate,
        queue: &mut VecDeque<QueuedIntent>,
    ) {
        let now_ms = time_ms(self.replay.now_ns);
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
        queue.extend(
            self.strategy
                .on_execution_event(&StrategyEvent::Order(pending))
                .into_iter()
                .map(QueuedIntent::Chaos),
        );
    }

    fn accept_cancel_order(&mut self, order_id: String, reason: String) {
        self.orders.cancel_requests += 1;
        let Some(symbol) = self.open_order_symbol(&order_id) else {
            self.orders.ignored_cancel_requests += 1;
            return;
        };
        if !self.orders.pending_cancels.insert(order_id.clone()) {
            self.orders.deduplicated_cancel_requests += 1;
            return;
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

    pub(super) fn matcher_mut(&mut self, symbol: &str) -> Result<&mut MatchingEngine> {
        self.orders
            .matchers
            .get_mut(symbol)
            .with_context(|| format!("no matcher configured for symbol {symbol}"))
    }

    fn open_order_symbol(&self, order_id: &str) -> Option<Symbol> {
        self.orders
            .matchers
            .iter()
            .find(|(_, matcher)| matcher.is_open_order(order_id))
            .map(|(symbol, _)| symbol.clone())
    }
}
