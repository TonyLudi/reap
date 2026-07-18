use anyhow::{Result, bail};
use reap_core::{MarketEvent, StrategyEvent};
use reap_strategy::Strategy;

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
                self.exchange_activations += 1;
                self.route_exchange_updates(updates)?;
            }
            ScheduledAction::CancelOrder {
                symbol,
                order_id,
                reason,
            } => {
                self.pending_cancels.remove(&order_id);
                let now_ms = time_ms(self.replay.now_ns);
                let updates = self
                    .matcher_mut(&symbol)?
                    .cancel_at(&order_id, now_ms, &reason);
                self.route_exchange_updates(updates)?;
            }
            ScheduledAction::DeliverOrder(update) => {
                let update = retime_order_update(update, time_ms(self.replay.now_ns));
                let commands = self.strategy.on_event(&StrategyEvent::Order(update));
                self.accept_intents(commands)?;
            }
            ScheduledAction::DeliverAccount(update) => {
                self.pending_fill_account_updates =
                    self.pending_fill_account_updates.saturating_sub(1);
                let event = retime_strategy_event(
                    StrategyEvent::Account(update),
                    time_ms(self.replay.now_ns),
                );
                let commands = self.strategy.on_event(&event);
                self.last_account_publish_ns = Some(self.replay.now_ns);
                self.accept_intents(commands)?;
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
                    self.last_account_publish_ns = Some(self.replay.now_ns);
                }
                let commands = self.strategy.on_event(&event);
                self.accept_intents(commands)?;
            }
            ScheduledAction::RefreshAccount => {
                let due = self.last_account_publish_ns.is_some_and(|last| {
                    self.replay.now_ns.saturating_sub(last) >= ACCOUNT_REFRESH_INTERVAL_NS
                });
                if due && self.pending_fill_account_updates == 0 {
                    let update = self.current_account_update(None);
                    let commands = self.strategy.on_event(&StrategyEvent::Account(update));
                    self.last_account_publish_ns = Some(self.replay.now_ns);
                    self.periodic_account_refreshes =
                        self.periodic_account_refreshes.saturating_add(1);
                    self.accept_intents(commands)?;
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
