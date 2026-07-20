use anyhow::{Result, bail};
use reap_core::{MarketEvent, StrategyEvent};

use crate::{
    ACCOUNT_REFRESH_INTERVAL_NS, BacktestRunner, MAX_ACTIONS_PER_DRAIN, NS_PER_MS, ScheduledAction,
    retime_order_update, retime_strategy_event, time_ms,
};

impl BacktestRunner {
    fn execute_action(&mut self, action: ScheduledAction) -> Result<()> {
        match action {
            ScheduledAction::ActivateOrder { symbol, order_id } => {
                let now_ms = time_ms(self.replay.now_ns);
                let updates = {
                    let matcher = self.matcher_mut(&symbol)?;
                    if !matcher.is_pending(&order_id) {
                        return Ok(());
                    }
                    matcher.activate(&order_id, now_ms)
                };
                self.orders.exchange_activations += 1;
                self.route_exchange_updates(updates)?;
            }
            ScheduledAction::CancelOrder {
                symbol,
                order_id,
                reason,
            } => {
                self.orders.pending_cancels.remove(&order_id);
                let now_ms = time_ms(self.replay.now_ns);
                let updates = self
                    .matcher_mut(&symbol)?
                    .cancel_at(&order_id, now_ms, &reason);
                self.route_exchange_updates(updates)?;
            }
            ScheduledAction::DeliverOrder(update) => {
                let update = retime_order_update(update, time_ms(self.replay.now_ns));
                let intents = self
                    .strategy
                    .on_execution_event(&StrategyEvent::Order(update));
                self.accept_chaos_intents(intents)?;
            }
            ScheduledAction::DeliverAccount(update) => {
                self.orders.pending_fill_account_updates =
                    self.orders.pending_fill_account_updates.saturating_sub(1);
                let event = retime_strategy_event(
                    StrategyEvent::Account(update),
                    time_ms(self.replay.now_ns),
                );
                let intents = self.strategy.on_execution_event(&event);
                self.orders.last_account_publish_ns = Some(self.replay.now_ns);
                self.accept_chaos_intents(intents)?;
            }
            ScheduledAction::DeliverStrategy(event) => {
                let currency_rate = match &event {
                    StrategyEvent::Market(MarketEvent::IndexPrice {
                        ts_ms,
                        symbol,
                        price,
                    }) => Some((symbol.clone(), *price, *ts_ms)),
                    _ => None,
                };
                let event = retime_strategy_event(event, time_ms(self.replay.now_ns));
                if let Some((symbol, price, source_ts_ms)) = currency_rate {
                    self.register_currency_rate(&symbol, price, source_ts_ms);
                }
                if matches!(event, StrategyEvent::Account(_)) {
                    self.orders.last_account_publish_ns = Some(self.replay.now_ns);
                }
                let intents = if matches!(event, StrategyEvent::Market(MarketEvent::Depth(_))) {
                    let intents = self.strategy.on_owned_execution_event_at(
                        event,
                        self.replay.now_ns,
                        self.replay.now_ns,
                        time_ms(self.replay.now_ns),
                        self.replay.trade_reprice_active,
                    );
                    self.schedule_new_trade_reprice_wake();
                    intents
                } else {
                    self.strategy.on_execution_event(&event)
                };
                self.accept_chaos_intents(intents)?;
            }
            ScheduledAction::DeliverTradeStrategy { event, arrival_ns } => {
                let event = retime_strategy_event(event, time_ms(self.replay.now_ns));
                let intents = self.strategy.on_owned_execution_event_at(
                    event,
                    arrival_ns,
                    self.replay.now_ns,
                    time_ms(self.replay.now_ns),
                    true,
                );
                self.replay.trade_reprice_active |= self.schedule_new_trade_reprice_wake();
                self.accept_chaos_intents(intents)?;
            }
            ScheduledAction::TradeRepriceWake { deadline_ns } => {
                debug_assert!(deadline_ns <= self.replay.now_ns);
                let intents = self.strategy.service_one_due_trade_reprice(
                    self.replay.now_ns,
                    time_ms(self.replay.now_ns),
                    self.replay.trade_reprice_active,
                );
                self.schedule_new_trade_reprice_wake();
                self.accept_chaos_intents(intents)?;
            }
            ScheduledAction::RefreshAccount => {
                let due = self.orders.last_account_publish_ns.is_some_and(|last| {
                    self.replay.now_ns.saturating_sub(last) >= ACCOUNT_REFRESH_INTERVAL_NS
                });
                if due && self.orders.pending_fill_account_updates == 0 {
                    let update = self.current_account_update(None);
                    let intents = self
                        .strategy
                        .on_execution_event(&StrategyEvent::Account(update));
                    self.orders.last_account_publish_ns = Some(self.replay.now_ns);
                    self.orders.periodic_account_refreshes =
                        self.orders.periodic_account_refreshes.saturating_add(1);
                    self.accept_chaos_intents(intents)?;
                }
                self.schedule_next_account_refresh();
            }
            ScheduledAction::SettleFunding {
                symbol,
                funding_time_ms,
            } => self.settle_funding(symbol, funding_time_ms),
        }
        Ok(())
    }

    pub(super) fn schedule_after(&mut self, delay_ms: u64, action: ScheduledAction) {
        let delay_ns = delay_ms.saturating_mul(NS_PER_MS);
        self.schedule_at(self.replay.now_ns.saturating_add(delay_ns), action);
    }

    pub(super) fn schedule_at(&mut self, due_ns: u64, action: ScheduledAction) {
        let seq = self.schedule.next_action_seq;
        self.schedule.next_action_seq = self.schedule.next_action_seq.saturating_add(1);
        self.schedule.scheduled.insert((due_ns, seq), action);
    }

    fn schedule_new_trade_reprice_wake(&mut self) -> bool {
        let mut inserted = false;
        while let Some(deadline_ns) = self.strategy.take_new_trade_reprice_wake_deadline_ns() {
            inserted = true;
            self.schedule_at(
                deadline_ns.max(self.replay.now_ns),
                ScheduledAction::TradeRepriceWake { deadline_ns },
            );
        }
        inserted
    }

    pub(super) fn drain_before(&mut self, cutoff_ns: u64) -> Result<()> {
        self.drain_scheduled(cutoff_ns, false)
    }

    pub(super) fn drain_through(&mut self, cutoff_ns: u64) -> Result<()> {
        self.drain_scheduled(cutoff_ns, true)
    }

    fn drain_scheduled(&mut self, cutoff_ns: u64, inclusive: bool) -> Result<()> {
        let mut processed = 0usize;
        while let Some((&(due_ns, _), _)) = self.schedule.scheduled.first_key_value() {
            if due_ns > cutoff_ns || (!inclusive && due_ns == cutoff_ns) {
                break;
            }
            let (_, action) = self
                .schedule
                .scheduled
                .pop_first()
                .expect("first scheduled action must still exist");
            let action_ns = self.replay.now_ns.max(due_ns);
            self.advance_metric_clock(action_ns);
            self.replay.now_ns = action_ns;
            self.execute_action(action)?;
            self.sample_risk_metrics();
            processed += 1;
            if processed > MAX_ACTIONS_PER_DRAIN {
                bail!(
                    "backtest exceeded {MAX_ACTIONS_PER_DRAIN} scheduled actions at {} ns",
                    self.replay.now_ns
                );
            }
        }
        Ok(())
    }
}
