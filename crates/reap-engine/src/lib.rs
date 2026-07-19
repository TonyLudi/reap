use std::collections::HashSet;

use reap_core::{NormalizedEvent, OrderIntent, SystemEvent, SystemEventKind, TimeMs};
use reap_risk::{RiskDecision, RiskGate};
use reap_strategy::{ChaosExecutionIntent, ChaosStrategy, Strategy};

#[derive(Debug, Default)]
pub struct EngineOutput {
    pub intents: Vec<OrderIntent>,
    pub rejected: Vec<RiskDecision>,
    pub system_events: Vec<SystemEvent>,
}

/// Typed output used by the live Chaos composition.
///
/// Strategy-created purposes remain opaque instead of being lowered to a
/// serializable `OrderIntent`. Risk-created fail-closed cancellation
/// candidates are kept separate because ownership is proven later by the live
/// regular-execution policy.
#[derive(Debug, Default)]
pub struct ChaosEngineOutput {
    pub intents: Vec<ChaosExecutionIntent>,
    pub safety_cancel_candidates: Vec<SafetyCancelCandidate>,
    pub rejected: Vec<RiskDecision>,
    pub system_events: Vec<SystemEvent>,
}

/// A risk-generated request to cancel an order if live policy proves that the
/// canonical identity is an owned regular order.
#[derive(Debug)]
pub struct SafetyCancelCandidate {
    order_id: String,
    reason: String,
}

impl SafetyCancelCandidate {
    pub fn order_id(&self) -> &str {
        &self.order_id
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    pub fn to_order_intent(&self) -> OrderIntent {
        OrderIntent::CancelOrder {
            order_id: self.order_id.clone(),
            reason: self.reason.clone(),
        }
    }
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
            let mut order_ids = if self.risk.is_killed() {
                self.risk.live_order_ids().collect::<Vec<_>>()
            } else {
                match halted_symbol.as_deref() {
                    Some(symbol) => self.risk.live_order_ids_for(symbol).collect::<Vec<_>>(),
                    None => self.risk.live_order_ids().collect::<Vec<_>>(),
                }
            };
            retain_and_sort_synthesized_cancel_ids(&mut order_ids, &existing_cancels);
            let cancels = order_ids
                .into_iter()
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

impl TradingEngine<ChaosStrategy> {
    /// Processes one event without erasing the authority provenance of Chaos
    /// Quote, Hedge, and CancelOwned purposes.
    pub fn on_chaos_event(&mut self, event: NormalizedEvent) -> ChaosEngineOutput {
        let now_ms = event.ts_ms();
        let mut output = ChaosEngineOutput::default();
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
        let strategy_intents = self.strategy.on_owned_execution_event(strategy_event);
        if !self.risk.is_killed()
            && let Some(reason) = self.strategy.safety_halt_reason()
        {
            output
                .system_events
                .extend(self.risk.on_strategy_halt(now_ms, reason).events);
        }
        self.apply_chaos_risk(now_ms, strategy_intents, &mut output);
        let fail_closed = self.risk.is_killed()
            || input_requires_cancel
            || output.system_events.iter().any(system_requires_cancel);
        if fail_closed {
            let existing_cancels = output
                .intents
                .iter()
                .filter_map(|intent| {
                    intent
                        .as_cancel_owned()
                        .map(|cancel| cancel.order_id().to_string())
                })
                .collect::<HashSet<_>>();
            let mut order_ids = if self.risk.is_killed() {
                self.risk.live_order_ids().collect::<Vec<_>>()
            } else {
                match halted_symbol.as_deref() {
                    Some(symbol) => self.risk.live_order_ids_for(symbol).collect::<Vec<_>>(),
                    None => self.risk.live_order_ids().collect::<Vec<_>>(),
                }
            };
            retain_and_sort_synthesized_cancel_ids(&mut order_ids, &existing_cancels);
            for order_id in order_ids {
                let intent = OrderIntent::CancelOrder {
                    order_id: order_id.to_string(),
                    reason: "fail_closed".to_string(),
                };
                match self.risk.pre_trade(now_ms, intent) {
                    RiskDecision::Allowed(OrderIntent::CancelOrder { order_id, reason }) => {
                        output
                            .safety_cancel_candidates
                            .push(SafetyCancelCandidate { order_id, reason });
                    }
                    RiskDecision::Allowed(OrderIntent::NewOrder(_)) => {
                        unreachable!("risk cannot change a cancellation into a new order")
                    }
                    rejected @ RiskDecision::Rejected { .. } => output.rejected.push(rejected),
                }
            }
        }
        output
    }

    fn apply_chaos_risk(
        &self,
        now_ms: TimeMs,
        intents: Vec<ChaosExecutionIntent>,
        output: &mut ChaosEngineOutput,
    ) {
        for intent in intents {
            match self.risk.pre_trade(now_ms, intent.to_order_intent()) {
                RiskDecision::Allowed(_) => output.intents.push(intent),
                rejected @ RiskDecision::Rejected { .. } => output.rejected.push(rejected),
            }
        }
    }
}

fn retain_and_sort_synthesized_cancel_ids(
    order_ids: &mut Vec<&str>,
    existing_cancels: &HashSet<String>,
) {
    order_ids.retain(|order_id| !existing_cancels.contains(*order_id));
    order_ids.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    order_ids.dedup();
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
    use std::collections::BTreeSet;
    use std::process::Command;

    use reap_core::{
        AccountUpdate, FillLiquidity, Level, MarketEvent, NewOrder, NormalizedEvent, OrderBook,
        OrderEvent, OrderStatus, OrderUpdate, Position, Side, StrategyEvent, SystemEvent,
        TimeInForce, TimerEvent, Venue,
    };
    use reap_risk::RiskLimits;
    use reap_strategy::ChaosConfig;

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
            last_fill_fee: None,
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
            last_fill_fee: None,
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
                last_fill_fee: None,
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

    #[test]
    fn chaos_typed_loop_matches_generic_intents_risk_and_fail_closed_ordering() {
        let config: ChaosConfig =
            toml::from_str(include_str!("../../../examples/iarb2-basic.toml")).unwrap();
        let limits = RiskLimits {
            require_feed_health: false,
            require_private_health: false,
            ..RiskLimits::default()
        };
        let mut generic = TradingEngine::new(
            ChaosStrategy::new(config.clone()).unwrap(),
            RiskGate::new(limits.clone()),
        );
        let mut typed =
            TradingEngine::new(ChaosStrategy::new(config).unwrap(), RiskGate::new(limits));
        let fixture = vec![
            NormalizedEvent::Market(MarketEvent::Depth(OrderBook::one_level(
                "BTC-USDT",
                1,
                Level::new(50_000.0, 2.0),
                Level::new(50_001.0, 2.0),
            ))),
            NormalizedEvent::Market(MarketEvent::Depth(OrderBook::one_level(
                "BTC-PERP",
                1,
                Level::new(50_003.0, 10_000.0),
                Level::new(50_004.0, 10_000.0),
            ))),
            NormalizedEvent::Order(OrderUpdate {
                ts_ms: 2,
                order_id: "fixture-quote-fill".to_string(),
                symbol: "BTC-USDT".to_string(),
                side: Side::Buy,
                event: OrderEvent::FullyFilled,
                status: OrderStatus::Filled,
                price: 50_000.0,
                time_in_force: None,
                qty: 0.1,
                open_qty: 0.0,
                filled_qty: 0.1,
                avg_fill_price: 50_000.0,
                last_fill_qty: 0.1,
                last_fill_price: 50_000.0,
                last_fill_liquidity: Some(FillLiquidity::Maker),
                last_fill_fee: None,
                reason: "quote".to_string(),
            }),
            NormalizedEvent::Account(AccountUpdate {
                ts_ms: 2,
                balances: Vec::new(),
                positions: vec![Position {
                    symbol: "BTC-USDT".to_string(),
                    qty: 0.1,
                    avg_price: 50_000.0,
                    margin_mode: None,
                }],
                margins: Vec::new(),
            }),
        ];

        for event in fixture {
            assert_chaos_outputs_match(
                generic.on_event(event.clone()),
                typed.on_chaos_event(event),
            );
        }

        let live_order = NormalizedEvent::Order(OrderUpdate {
            ts_ms: 5,
            order_id: "owned-live-1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            event: OrderEvent::New,
            status: OrderStatus::Live,
            price: 100.0,
            time_in_force: Some(TimeInForce::PostOnly),
            qty: 0.1,
            open_qty: 0.1,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            last_fill_fee: None,
            reason: "quote".to_string(),
        });
        assert_chaos_outputs_match(
            generic.on_event(live_order.clone()),
            typed.on_chaos_event(live_order),
        );
        let kill = NormalizedEvent::System(SystemEvent {
            ts_ms: 6,
            kind: SystemEventKind::KillSwitchActivated,
            venue: None,
            account_id: None,
            symbol: None,
            reason: "differential fail-closed check".to_string(),
        });
        assert_chaos_outputs_match(generic.on_event(kill.clone()), typed.on_chaos_event(kill));
    }

    #[test]
    fn symbol_fail_closed_keeps_strategy_cancel_order_then_sorts_synthesized_ids() {
        let config: ChaosConfig =
            toml::from_str(include_str!("../../../examples/iarb2-basic.toml")).unwrap();
        let limits = RiskLimits {
            require_feed_health: false,
            require_private_health: false,
            ..RiskLimits::default()
        };
        let mut generic = TradingEngine::new(
            ChaosStrategy::new(config.clone()).unwrap(),
            RiskGate::new(limits.clone()),
        );
        let mut typed =
            TradingEngine::new(ChaosStrategy::new(config).unwrap(), RiskGate::new(limits));
        let registrations = [
            ("synth-2", "BTC-USDT", "external", 97.0),
            ("strategy-a", "BTC-USDT", "quote:1", 99.0),
            ("perp-only", "BTC-PERP", "external", 96.0),
            ("synth-10", "BTC-USDT", "external", 98.0),
            ("strategy-z", "BTC-USDT", "quote", 100.0),
            ("synth-02", "BTC-USDT", "external", 95.0),
        ];
        for (order_id, symbol, reason, price) in registrations {
            let update = NormalizedEvent::Order(OrderUpdate {
                ts_ms: 1,
                order_id: order_id.to_string(),
                symbol: symbol.to_string(),
                side: Side::Buy,
                event: OrderEvent::New,
                status: OrderStatus::Live,
                price,
                time_in_force: Some(TimeInForce::PostOnly),
                qty: 1.0,
                open_qty: 1.0,
                filled_qty: 0.0,
                avg_fill_price: 0.0,
                last_fill_qty: 0.0,
                last_fill_price: 0.0,
                last_fill_liquidity: None,
                last_fill_fee: None,
                reason: reason.to_string(),
            });
            assert_chaos_outputs_match(
                generic.on_event(update.clone()),
                typed.on_chaos_event(update),
            );
        }

        let halt = NormalizedEvent::System(SystemEvent {
            ts_ms: 2,
            kind: SystemEventKind::SymbolHalted,
            venue: None,
            account_id: None,
            symbol: Some("BTC-USDT".to_string()),
            reason: "Goal D exact symbol scope".to_string(),
        });
        let generic_output = generic.on_event(halt.clone());
        let typed_output = typed.on_chaos_event(halt);

        let generic_projection = generic_output
            .intents
            .iter()
            .filter_map(project_cancel)
            .collect::<Vec<_>>();
        assert_eq!(
            generic_projection,
            [
                ("strategy-z", "quote_disabled"),
                ("strategy-a", "quote_disabled"),
                ("synth-02", "fail_closed"),
                ("synth-10", "fail_closed"),
                ("synth-2", "fail_closed"),
            ]
        );
        let strategy_projection = typed_output
            .intents
            .iter()
            .filter_map(|intent| {
                intent
                    .as_cancel_owned()
                    .map(|cancel| (cancel.order_id(), cancel.reason()))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            strategy_projection,
            [
                ("strategy-z", "quote_disabled"),
                ("strategy-a", "quote_disabled"),
            ]
        );
        let synthesized_projection = typed_output
            .safety_cancel_candidates
            .iter()
            .map(|candidate| (candidate.order_id(), candidate.reason()))
            .collect::<Vec<_>>();
        assert_eq!(
            synthesized_projection,
            [
                ("synth-02", "fail_closed"),
                ("synth-10", "fail_closed"),
                ("synth-2", "fail_closed"),
            ]
        );
        assert_chaos_outputs_match(generic_output, typed_output);
    }

    fn project_cancel(intent: &OrderIntent) -> Option<(&str, &str)> {
        match intent {
            OrderIntent::CancelOrder { order_id, reason } => Some((order_id, reason)),
            OrderIntent::NewOrder(_) => None,
        }
    }

    #[test]
    fn goal_d_fail_closed_process_order_probe() {
        const CHILD_ENV: &str = "REAP_GOAL_D_CANCEL_PROBE_CHILD";
        const MARKER: &str = "REAP_GOAL_D_CANCEL_PROBE=";
        const EXPECTED: &str = concat!(
            "{\"cases\":[",
            "{\"insertion\":\"forward\",\"generic\":[\"reap-02\",\"reap-10\",\"reap-2\",",
            "\"reap-a1\",\"reap-perp\",\"reap-z0\"],\"typed\":[\"reap-02\",\"reap-10\",",
            "\"reap-2\",\"reap-a1\",\"reap-perp\",\"reap-z0\"]},",
            "{\"insertion\":\"reverse\",\"generic\":[\"reap-02\",\"reap-10\",\"reap-2\",",
            "\"reap-a1\",\"reap-perp\",\"reap-z0\"],\"typed\":[\"reap-02\",\"reap-10\",",
            "\"reap-2\",\"reap-a1\",\"reap-perp\",\"reap-z0\"]},",
            "{\"insertion\":\"interleaved\",\"generic\":[\"reap-02\",\"reap-10\",\"reap-2\",",
            "\"reap-a1\",\"reap-perp\",\"reap-z0\"],\"typed\":[\"reap-02\",\"reap-10\",",
            "\"reap-2\",\"reap-a1\",\"reap-perp\",\"reap-z0\"]},",
            "{\"insertion\":\"rotated\",\"generic\":[\"reap-02\",\"reap-10\",\"reap-2\",",
            "\"reap-a1\",\"reap-perp\",\"reap-z0\"],\"typed\":[\"reap-02\",\"reap-10\",",
            "\"reap-2\",\"reap-a1\",\"reap-perp\",\"reap-z0\"]}",
            "]}"
        );

        if std::env::var_os(CHILD_ENV).is_some() {
            println!("{MARKER}{}", fail_closed_process_projection());
            return;
        }

        let executable = std::env::current_exe().expect("current test executable");
        let mut projections = BTreeSet::new();
        for _ in 0..24 {
            let output = Command::new(&executable)
                .args([
                    "--exact",
                    "tests::goal_d_fail_closed_process_order_probe",
                    "--nocapture",
                ])
                .env(CHILD_ENV, "1")
                .output()
                .expect("spawn independent fail-closed probe process");
            assert!(
                output.status.success(),
                "child probe failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let stdout = String::from_utf8(output.stdout).expect("probe output is UTF-8");
            let projection = stdout
                .lines()
                .find_map(|line| line.strip_prefix(MARKER))
                .expect("child probe marker")
                .to_string();
            projections.insert(projection);
        }

        eprintln!(
            "Goal D independent-process fail-closed projections ({} distinct):",
            projections.len()
        );
        for projection in &projections {
            eprintln!("{projection}");
        }
        assert_eq!(projections, BTreeSet::from([EXPECTED.to_string()]));
    }

    fn fail_closed_process_projection() -> String {
        let insertion_cases: [(&str, &[usize]); 4] = [
            ("forward", &[0, 1, 2, 3, 4, 5]),
            ("reverse", &[5, 4, 3, 2, 1, 0]),
            ("interleaved", &[2, 5, 0, 4, 1, 3]),
            ("rotated", &[3, 4, 5, 0, 1, 2]),
        ];
        let cases = insertion_cases
            .into_iter()
            .map(|(name, insertion)| {
                let (generic_ids, typed_ids) = fail_closed_insertion_projection(insertion);
                format!(
                    "{{\"insertion\":\"{name}\",\"generic\":[{}],\"typed\":[{}]}}",
                    quoted_ids(&generic_ids),
                    quoted_ids(&typed_ids)
                )
            })
            .collect::<Vec<_>>();
        format!("{{\"cases\":[{}]}}", cases.join(","))
    }

    fn fail_closed_insertion_projection(insertion: &[usize]) -> (Vec<String>, Vec<String>) {
        let config: ChaosConfig =
            toml::from_str(include_str!("../../../examples/iarb2-basic.toml")).unwrap();
        let limits = RiskLimits {
            require_feed_health: false,
            require_private_health: false,
            ..RiskLimits::default()
        };
        let mut generic = TradingEngine::new(
            ChaosStrategy::new(config.clone()).unwrap(),
            RiskGate::new(limits.clone()),
        );
        let mut typed =
            TradingEngine::new(ChaosStrategy::new(config).unwrap(), RiskGate::new(limits));
        let registrations = [
            ("reap-z0", "BTC-USDT"),
            ("reap-a1", "BTC-USDT"),
            ("reap-02", "BTC-USDT"),
            ("reap-10", "BTC-USDT"),
            ("reap-2", "BTC-USDT"),
            ("reap-perp", "BTC-PERP"),
        ];
        let expected = [
            "reap-02",
            "reap-10",
            "reap-2",
            "reap-a1",
            "reap-perp",
            "reap-z0",
        ];
        assert_eq!(insertion.len(), registrations.len());
        let mut seen = BTreeSet::new();
        for &index in insertion {
            assert!(seen.insert(index), "insertion index must be unique");
            let (order_id, symbol) = registrations[index];
            let event = NormalizedEvent::Order(OrderUpdate {
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
                last_fill_fee: None,
                reason: "external baseline observation".to_string(),
            });
            let generic_output = generic.on_event(event.clone());
            let typed_output = typed.on_chaos_event(event);
            assert!(generic_output.intents.is_empty());
            assert!(typed_output.intents.is_empty());
            assert!(typed_output.safety_cancel_candidates.is_empty());
        }
        let kill = NormalizedEvent::System(SystemEvent {
            ts_ms: 2,
            kind: SystemEventKind::KillSwitchActivated,
            venue: None,
            account_id: None,
            symbol: None,
            reason: "Goal D Phase 0 ordering probe".to_string(),
        });
        let generic_output = generic.on_event(kill.clone());
        let typed_output = typed.on_chaos_event(kill);
        assert!(typed_output.intents.is_empty());
        let generic_ids = generic_output
            .intents
            .iter()
            .filter_map(|intent| match intent {
                OrderIntent::CancelOrder { order_id, reason } => {
                    assert_eq!(reason, "fail_closed");
                    Some(order_id.clone())
                }
                OrderIntent::NewOrder(_) => None,
            })
            .collect::<Vec<_>>();
        let typed_ids = typed_output
            .safety_cancel_candidates
            .iter()
            .map(|candidate| {
                assert_eq!(candidate.reason(), "fail_closed");
                candidate.order_id().to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(generic_ids, expected);
        assert_eq!(typed_ids, expected);
        (generic_ids, typed_ids)
    }

    fn quoted_ids(ids: &[String]) -> String {
        ids.iter()
            .map(|id| format!("\"{id}\""))
            .collect::<Vec<_>>()
            .join(",")
    }

    fn assert_chaos_outputs_match(generic: EngineOutput, typed: ChaosEngineOutput) {
        let typed_intents = typed
            .intents
            .iter()
            .map(ChaosExecutionIntent::to_order_intent)
            .chain(
                typed
                    .safety_cancel_candidates
                    .iter()
                    .map(SafetyCancelCandidate::to_order_intent),
            )
            .collect::<Vec<_>>();
        assert_eq!(
            format!("{:?}", generic.intents),
            format!("{typed_intents:?}")
        );
        assert_eq!(
            format!("{:?}", generic.rejected),
            format!("{:?}", typed.rejected)
        );
        assert_eq!(
            format!("{:?}", generic.system_events),
            format!("{:?}", typed.system_events)
        );
    }
}
