use std::collections::HashSet;

use reap_core::{NormalizedEvent, OrderIntent, SystemEvent, SystemEventKind, TimeMs};
use reap_risk::{RiskDecision, RiskGate};
use reap_strategy::Strategy;

#[derive(Debug, Default)]
pub struct EngineOutput {
    pub intents: Vec<OrderIntent>,
    pub rejected: Vec<RiskDecision>,
    pub system_events: Vec<SystemEvent>,
}

pub struct TradingEngine<S> {
    strategy: S,
    risk: RiskGate,
}

impl<S> TradingEngine<S>
where
    S: Strategy,
{
    pub fn new(strategy: S, risk: RiskGate) -> Self {
        Self { strategy, risk }
    }

    pub fn strategy(&self) -> &S {
        &self.strategy
    }

    pub fn risk(&self) -> &RiskGate {
        &self.risk
    }

    pub fn risk_mut(&mut self) -> &mut RiskGate {
        &mut self.risk
    }

    pub fn on_event(&mut self, event: NormalizedEvent) -> EngineOutput {
        let now_ms = event.ts_ms();
        let mut output = EngineOutput::default();
        let post_trade = self.risk.on_normalized_event(&event);
        output.system_events.extend(post_trade.events);
        output
            .system_events
            .extend(self.risk.check_staleness(now_ms));

        let input_requires_cancel = event_requires_cancel(&event);
        let halted_symbol = match &event {
            NormalizedEvent::System(system) if system.kind == SystemEventKind::SymbolHalted => {
                system.symbol.clone()
            }
            _ => None,
        };
        let strategy_event = event.into_strategy_event();
        let strategy_intents = self.strategy.on_owned_event(strategy_event);
        if !self.risk.is_killed()
            && let Some(reason) = self.strategy.safety_halt_reason()
        {
            output
                .system_events
                .extend(self.risk.on_strategy_halt(now_ms, reason).events);
        }
        self.apply_risk(now_ms, strategy_intents, &mut output);
        let fail_closed = self.risk.is_killed()
            || input_requires_cancel
            || output.system_events.iter().any(system_requires_cancel);
        if fail_closed {
            let existing_cancels = output
                .intents
                .iter()
                .filter_map(|intent| match intent {
                    OrderIntent::CancelOrder { order_id, .. } => Some(order_id.clone()),
                    OrderIntent::NewOrder(_) => None,
                })
                .collect::<HashSet<_>>();
            let order_ids = if self.risk.is_killed() {
                self.risk.live_order_ids().collect::<Vec<_>>()
            } else {
                match halted_symbol.as_deref() {
                    Some(symbol) => self.risk.live_order_ids_for(symbol).collect::<Vec<_>>(),
                    None => self.risk.live_order_ids().collect::<Vec<_>>(),
                }
            };
            let cancels = order_ids
                .into_iter()
                .filter(|order_id| !existing_cancels.contains(*order_id))
                .map(|order_id| OrderIntent::CancelOrder {
                    order_id: order_id.to_string(),
                    reason: "fail_closed".to_string(),
                })
                .collect::<Vec<_>>();
            self.apply_risk(now_ms, cancels, &mut output);
        }
        output
    }

    fn apply_risk(&self, now_ms: TimeMs, intents: Vec<OrderIntent>, output: &mut EngineOutput) {
        for intent in intents {
            match self.risk.pre_trade(now_ms, intent) {
                RiskDecision::Allowed(intent) => output.intents.push(intent),
                rejected @ RiskDecision::Rejected { .. } => output.rejected.push(rejected),
            }
        }
    }
}

fn event_requires_cancel(event: &NormalizedEvent) -> bool {
    matches!(event, NormalizedEvent::System(system) if system_requires_cancel(system))
}

fn system_requires_cancel(event: &SystemEvent) -> bool {
    matches!(
        event.kind,
        SystemEventKind::FeedStale
            | SystemEventKind::FeedGap
            | SystemEventKind::BookRecoveryStarted
            | SystemEventKind::BookRecoveryFailed
            | SystemEventKind::PrivateStreamStale
            | SystemEventKind::ReconcileDrift
            | SystemEventKind::RiskBreach
            | SystemEventKind::KillSwitchActivated
            | SystemEventKind::SymbolHalted
    )
}

#[cfg(test)]
mod tests {
    use reap_core::{
        FillLiquidity, NewOrder, OrderEvent, OrderStatus, OrderUpdate, Side, StrategyEvent,
        SystemEvent, TimeInForce, TimerEvent, Venue,
    };
    use reap_risk::RiskLimits;

    use super::*;

    struct TestStrategy;

    impl Strategy for TestStrategy {
        fn on_event(&mut self, event: &StrategyEvent) -> Vec<OrderIntent> {
            if matches!(event, StrategyEvent::Timer(_)) {
                vec![OrderIntent::NewOrder(NewOrder {
                    symbol: "BTC-USDT".to_string(),
                    side: Side::Buy,
                    qty: 1.0,
                    price: 100.0,
                    time_in_force: TimeInForce::PostOnly,
                    reduce_only: false,
                    self_trade_prevention: None,
                    reason: "test".to_string(),
                })]
            } else {
                Vec::new()
            }
        }
    }

    #[derive(Default)]
    struct HaltingStrategy {
        halted: bool,
    }

    impl Strategy for HaltingStrategy {
        fn on_event(&mut self, event: &StrategyEvent) -> Vec<OrderIntent> {
            if matches!(event, StrategyEvent::Timer(_)) {
                self.halted = true;
                vec![OrderIntent::NewOrder(NewOrder {
                    symbol: "BTC-USDT".to_string(),
                    side: Side::Buy,
                    qty: 1.0,
                    price: 100.0,
                    time_in_force: TimeInForce::PostOnly,
                    reduce_only: false,
                    self_trade_prevention: None,
                    reason: "must be rejected".to_string(),
                })]
            } else {
                Vec::new()
            }
        }

        fn safety_halt_reason(&self) -> Option<&str> {
            self.halted.then_some("test strategy safety limit")
        }
    }

    fn event(kind: SystemEventKind, ts_ms: u64) -> NormalizedEvent {
        NormalizedEvent::System(SystemEvent {
            ts_ms,
            kind,
            venue: Some(Venue::Okx),
            account_id: None,
            symbol: (kind == SystemEventKind::FeedRecovered).then(|| "BTC-USDT".to_string()),
            reason: "test".to_string(),
        })
    }

    #[test]
    fn event_loop_enforces_fail_closed_risk_before_gateway() {
        let mut engine = TradingEngine::new(TestStrategy, RiskGate::new(RiskLimits::default()));
        let blocked = engine.on_event(NormalizedEvent::Timer(TimerEvent {
            ts_ms: 1,
            name: "quote".to_string(),
        }));
        assert_eq!(blocked.rejected.len(), 1);

        engine.on_event(event(SystemEventKind::FeedRecovered, 2));
        engine.on_event(event(SystemEventKind::PrivateStreamRecovered, 2));
        let allowed = engine.on_event(NormalizedEvent::Timer(TimerEvent {
            ts_ms: 2,
            name: "quote".to_string(),
        }));
        assert_eq!(allowed.intents.len(), 1);
    }

    #[test]
    fn kill_switch_event_emits_cancels_for_live_orders() {
        let limits = RiskLimits {
            require_feed_health: false,
            require_private_health: false,
            ..RiskLimits::default()
        };
        let mut engine = TradingEngine::new(TestStrategy, RiskGate::new(limits));
        engine.on_event(NormalizedEvent::Order(OrderUpdate {
            ts_ms: 1,
            order_id: "live-1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            event: OrderEvent::New,
            status: OrderStatus::Live,
            price: 100.0,
            time_in_force: Some(TimeInForce::PostOnly),
            qty: 1.0,
            open_qty: 1.0,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: Some(FillLiquidity::Maker),
            reason: "test".to_string(),
        }));
        let output = engine.on_event(NormalizedEvent::System(SystemEvent {
            ts_ms: 2,
            kind: SystemEventKind::KillSwitchActivated,
            venue: None,
            account_id: None,
            symbol: None,
            reason: "operator".to_string(),
        }));

        assert!(matches!(
            output.intents.as_slice(),
            [OrderIntent::CancelOrder { order_id, .. }] if order_id == "live-1"
        ));
    }

    #[test]
    fn strategy_safety_halt_latches_risk_rejects_new_orders_and_cancels_live_orders() {
        let limits = RiskLimits {
            require_feed_health: false,
            require_private_health: false,
            ..RiskLimits::default()
        };
        let mut engine = TradingEngine::new(HaltingStrategy::default(), RiskGate::new(limits));
        engine.on_event(NormalizedEvent::Order(OrderUpdate {
            ts_ms: 1,
            order_id: "live-1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Sell,
            event: OrderEvent::New,
            status: OrderStatus::Live,
            price: 101.0,
            time_in_force: Some(TimeInForce::PostOnly),
            qty: 1.0,
            open_qty: 1.0,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            reason: "quote".to_string(),
        }));

        let output = engine.on_event(NormalizedEvent::Timer(TimerEvent {
            ts_ms: 2,
            name: "risk".to_string(),
        }));

        assert!(engine.risk().is_killed());
        assert!(output.system_events.iter().any(|event| {
            event.kind == SystemEventKind::RiskBreach
                && event
                    .reason
                    .contains("strategy halted: test strategy safety limit")
        }));
        assert_eq!(output.rejected.len(), 1);
        assert!(matches!(
            output.intents.as_slice(),
            [OrderIntent::CancelOrder { order_id, .. }] if order_id == "live-1"
        ));

        let reset = engine.on_event(event(SystemEventKind::KillSwitchReset, 3));
        assert!(engine.risk().is_killed());
        assert!(
            reset
                .system_events
                .iter()
                .any(|event| event.kind == SystemEventKind::RiskBreach)
        );
    }

    #[test]
    fn global_kill_takes_precedence_over_symbol_only_cancel_scope() {
        let limits = RiskLimits {
            require_feed_health: false,
            require_private_health: false,
            ..RiskLimits::default()
        };
        let mut engine = TradingEngine::new(TestStrategy, RiskGate::new(limits));
        for (order_id, symbol) in [("btc-live", "BTC-USDT"), ("eth-live", "ETH-USDT")] {
            engine.on_event(NormalizedEvent::Order(OrderUpdate {
                ts_ms: 1,
                order_id: order_id.to_string(),
                symbol: symbol.to_string(),
                side: Side::Buy,
                event: OrderEvent::New,
                status: OrderStatus::Live,
                price: 100.0,
                time_in_force: Some(TimeInForce::PostOnly),
                qty: 1.0,
                open_qty: 1.0,
                filled_qty: 0.0,
                avg_fill_price: 0.0,
                last_fill_qty: 0.0,
                last_fill_price: 0.0,
                last_fill_liquidity: None,
                reason: "quote".to_string(),
            }));
        }
        engine
            .risk_mut()
            .activate_kill_switch(2, "test global kill");

        let output = engine.on_event(NormalizedEvent::System(SystemEvent {
            ts_ms: 3,
            kind: SystemEventKind::SymbolHalted,
            venue: None,
            account_id: None,
            symbol: Some("BTC-USDT".to_string()),
            reason: "test symbol halt".to_string(),
        }));

        assert_eq!(output.intents.len(), 2);
        assert!(output.intents.iter().any(|intent| matches!(
            intent,
            OrderIntent::CancelOrder { order_id, .. } if order_id == "btc-live"
        )));
        assert!(output.intents.iter().any(|intent| matches!(
            intent,
            OrderIntent::CancelOrder { order_id, .. } if order_id == "eth-live"
        )));
    }
}
