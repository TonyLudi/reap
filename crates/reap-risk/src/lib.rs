use std::collections::{HashMap, HashSet};

use reap_core::{
    MarketEvent, NormalizedEvent, OrderIntent, OrderStatus, OrderUpdate, Symbol, SystemEvent,
    SystemEventKind, TimeMs, Venue,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RiskLimits {
    pub max_order_notional_usd: f64,
    pub max_abs_position_notional_usd: f64,
    pub max_live_order_notional_usd: f64,
    pub max_turnover_usd: f64,
    pub max_drawdown_usd: f64,
    pub max_feed_age_ms: TimeMs,
    pub max_private_age_ms: TimeMs,
    pub require_feed_health: bool,
    pub require_private_health: bool,
}

impl Default for RiskLimits {
    fn default() -> Self {
        Self {
            max_order_notional_usd: 25_000.0,
            max_abs_position_notional_usd: 100_000.0,
            max_live_order_notional_usd: 100_000.0,
            max_turnover_usd: 1_000_000.0,
            max_drawdown_usd: 10_000.0,
            max_feed_age_ms: 1_000,
            max_private_age_ms: 2_000,
            require_feed_health: true,
            require_private_health: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskRejectReason {
    KillSwitch(String),
    SymbolHalted(String),
    FeedNotReady,
    FeedStale,
    PrivateStreamNotReady,
    PrivateStreamStale,
    InvalidOrder,
    OrderNotional { value: f64, limit: f64 },
    PositionNotional { value: f64, limit: f64 },
    LiveOrderNotional { value: f64, limit: f64 },
    Turnover { value: f64, limit: f64 },
    Drawdown { value: f64, limit: f64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskDecision {
    Allowed(OrderIntent),
    Rejected {
        intent: OrderIntent,
        reason: RiskRejectReason,
    },
}

impl RiskDecision {
    pub fn allowed(self) -> Option<OrderIntent> {
        match self {
            Self::Allowed(intent) => Some(intent),
            Self::Rejected { .. } => None,
        }
    }
}

#[derive(Debug, Default)]
pub struct PostTradeOutcome {
    pub events: Vec<SystemEvent>,
}

#[derive(Debug)]
pub struct RiskGate {
    limits: RiskLimits,
    kill_switch: Option<String>,
    halted_symbols: HashMap<Symbol, String>,
    feed_health: HashMap<(Venue, Symbol), StreamHealth>,
    private_health: HashMap<Venue, StreamHealth>,
    marks: HashMap<Symbol, f64>,
    positions: HashMap<Symbol, f64>,
    live_orders: HashMap<String, LiveOrderRisk>,
    turnover_usd: f64,
    equity_usd: f64,
    peak_equity_usd: f64,
    seen_fills: HashSet<FillKey>,
}

impl RiskGate {
    pub fn new(limits: RiskLimits) -> Self {
        Self {
            limits,
            kill_switch: None,
            halted_symbols: HashMap::new(),
            feed_health: HashMap::new(),
            private_health: HashMap::new(),
            marks: HashMap::new(),
            positions: HashMap::new(),
            live_orders: HashMap::new(),
            turnover_usd: 0.0,
            equity_usd: 0.0,
            peak_equity_usd: 0.0,
            seen_fills: HashSet::new(),
        }
    }

    pub fn limits(&self) -> &RiskLimits {
        &self.limits
    }

    pub fn is_killed(&self) -> bool {
        self.kill_switch.is_some()
    }

    pub fn kill_reason(&self) -> Option<&str> {
        self.kill_switch.as_deref()
    }

    pub fn is_symbol_halted(&self, symbol: &str) -> bool {
        self.halted_symbols.contains_key(symbol)
    }

    pub fn turnover_usd(&self) -> f64 {
        self.turnover_usd
    }

    pub fn position(&self, symbol: &str) -> f64 {
        self.positions.get(symbol).copied().unwrap_or(0.0)
    }

    pub fn live_order_ids(&self) -> impl Iterator<Item = &str> {
        self.live_orders.keys().map(String::as_str)
    }

    pub fn live_order_ids_for<'a>(&'a self, symbol: &'a str) -> impl Iterator<Item = &'a str> {
        self.live_orders
            .iter()
            .filter(move |(_, order)| order.symbol == symbol)
            .map(|(order_id, _)| order_id.as_str())
    }

    pub fn on_market(&mut self, event: &MarketEvent) {
        if let MarketEvent::Depth(book) = event
            && let Some(mid) = book.mid()
        {
            self.marks.insert(book.symbol.clone(), mid);
        }
    }

    pub fn mark_feed_ready(&mut self, venue: Venue, symbol: impl Into<Symbol>, ts_ms: TimeMs) {
        self.feed_health.insert(
            (venue, symbol.into()),
            StreamHealth {
                last_ready_ms: ts_ms,
                stale: false,
            },
        );
    }

    pub fn mark_private_ready(&mut self, venue: Venue, ts_ms: TimeMs) {
        self.private_health.insert(
            venue,
            StreamHealth {
                last_ready_ms: ts_ms,
                stale: false,
            },
        );
    }

    pub fn pre_trade(&self, now_ms: TimeMs, intent: OrderIntent) -> RiskDecision {
        let OrderIntent::NewOrder(order) = &intent else {
            return RiskDecision::Allowed(intent);
        };
        let reject = self.pre_trade_reason(now_ms, order);
        match reject {
            Some(reason) => RiskDecision::Rejected { intent, reason },
            None => RiskDecision::Allowed(intent),
        }
    }

    pub fn on_order_update(&mut self, update: &OrderUpdate) -> PostTradeOutcome {
        if matches!(
            update.status,
            OrderStatus::Live | OrderStatus::PartiallyFilled | OrderStatus::PendingNew
        ) && update.open_qty > 0.0
        {
            self.live_orders.insert(
                update.order_id.clone(),
                LiveOrderRisk {
                    symbol: update.symbol.clone(),
                    notional_usd: update.open_qty * risk_price(update.price, update.avg_fill_price),
                    signed_qty: update.side.factor() * update.open_qty,
                },
            );
        } else {
            self.live_orders.remove(&update.order_id);
        }

        if update.has_fill() {
            let fill_key = FillKey {
                order_id: update.order_id.clone(),
                ts_ms: update.ts_ms,
                qty: update.last_fill_qty.to_bits(),
                price: update.last_fill_price.to_bits(),
            };
            if self.seen_fills.insert(fill_key) {
                if update.last_fill_price > 0.0 {
                    self.marks
                        .entry(update.symbol.clone())
                        .or_insert(update.last_fill_price);
                }
                *self.positions.entry(update.symbol.clone()).or_default() +=
                    update.side.factor() * update.last_fill_qty;
                self.turnover_usd += (update.last_fill_qty * update.last_fill_price).abs();
            }
        }
        self.evaluate_post_trade(update.ts_ms, Some(&update.symbol))
    }

    pub fn update_equity(&mut self, ts_ms: TimeMs, equity_usd: f64) -> PostTradeOutcome {
        self.equity_usd = equity_usd;
        self.peak_equity_usd = self.peak_equity_usd.max(equity_usd);
        self.evaluate_post_trade(ts_ms, None)
    }

    pub fn activate_kill_switch(
        &mut self,
        ts_ms: TimeMs,
        reason: impl Into<String>,
    ) -> SystemEvent {
        let reason = reason.into();
        self.kill_switch = Some(reason.clone());
        SystemEvent {
            ts_ms,
            kind: SystemEventKind::KillSwitchActivated,
            venue: None,
            symbol: None,
            reason,
        }
    }

    pub fn reset_kill_switch(&mut self, ts_ms: TimeMs, reason: impl Into<String>) -> SystemEvent {
        self.kill_switch = None;
        SystemEvent {
            ts_ms,
            kind: SystemEventKind::KillSwitchReset,
            venue: None,
            symbol: None,
            reason: reason.into(),
        }
    }

    pub fn halt_symbol(
        &mut self,
        ts_ms: TimeMs,
        symbol: impl Into<Symbol>,
        reason: impl Into<String>,
    ) -> SystemEvent {
        let symbol = symbol.into();
        let reason = reason.into();
        self.halted_symbols.insert(symbol.clone(), reason.clone());
        SystemEvent {
            ts_ms,
            kind: SystemEventKind::SymbolHalted,
            venue: None,
            symbol: Some(symbol),
            reason,
        }
    }

    pub fn resume_symbol(
        &mut self,
        ts_ms: TimeMs,
        symbol: impl Into<Symbol>,
        reason: impl Into<String>,
    ) -> SystemEvent {
        let symbol = symbol.into();
        self.halted_symbols.remove(&symbol);
        SystemEvent {
            ts_ms,
            kind: SystemEventKind::SymbolResumed,
            venue: None,
            symbol: Some(symbol),
            reason: reason.into(),
        }
    }

    pub fn apply_system_event(&mut self, event: &SystemEvent) {
        match event.kind {
            SystemEventKind::FeedStale
            | SystemEventKind::FeedGap
            | SystemEventKind::BookRecoveryStarted
            | SystemEventKind::BookRecoveryFailed => {
                if let (Some(venue), Some(symbol)) = (event.venue, event.symbol.clone()) {
                    self.feed_health.entry((venue, symbol)).or_default().stale = true;
                }
            }
            SystemEventKind::FeedHeartbeat | SystemEventKind::FeedRecovered => {
                if let (Some(venue), Some(symbol)) = (event.venue, event.symbol.clone()) {
                    self.mark_feed_ready(venue, symbol, event.ts_ms);
                }
            }
            SystemEventKind::PrivateStreamStale | SystemEventKind::ReconcileDrift => {
                if let Some(venue) = event.venue {
                    self.private_health.entry(venue).or_default().stale = true;
                } else {
                    for health in self.private_health.values_mut() {
                        health.stale = true;
                    }
                }
            }
            SystemEventKind::PrivateStreamHeartbeat | SystemEventKind::PrivateStreamRecovered => {
                if let Some(venue) = event.venue {
                    self.mark_private_ready(venue, event.ts_ms);
                }
            }
            SystemEventKind::KillSwitchActivated | SystemEventKind::RiskBreach => {
                self.kill_switch = Some(event.reason.clone());
            }
            SystemEventKind::KillSwitchReset => self.kill_switch = None,
            SystemEventKind::SymbolHalted => {
                if let Some(symbol) = &event.symbol {
                    self.halted_symbols
                        .insert(symbol.clone(), event.reason.clone());
                }
            }
            SystemEventKind::SymbolResumed => {
                if let Some(symbol) = &event.symbol {
                    self.halted_symbols.remove(symbol);
                }
            }
        }
    }

    pub fn check_staleness(&mut self, now_ms: TimeMs) -> Vec<SystemEvent> {
        let mut events = Vec::new();
        for ((venue, symbol), health) in &mut self.feed_health {
            if !health.stale
                && now_ms.saturating_sub(health.last_ready_ms) > self.limits.max_feed_age_ms
            {
                health.stale = true;
                events.push(SystemEvent {
                    ts_ms: now_ms,
                    kind: SystemEventKind::FeedStale,
                    venue: Some(*venue),
                    symbol: Some(symbol.clone()),
                    reason: format!("feed age exceeded {}ms", self.limits.max_feed_age_ms),
                });
            }
        }
        for (venue, health) in &mut self.private_health {
            if !health.stale
                && now_ms.saturating_sub(health.last_ready_ms) > self.limits.max_private_age_ms
            {
                health.stale = true;
                events.push(SystemEvent {
                    ts_ms: now_ms,
                    kind: SystemEventKind::PrivateStreamStale,
                    venue: Some(*venue),
                    symbol: None,
                    reason: format!(
                        "private stream age exceeded {}ms",
                        self.limits.max_private_age_ms
                    ),
                });
            }
        }
        events
    }

    pub fn on_normalized_event(&mut self, event: &NormalizedEvent) -> PostTradeOutcome {
        match event {
            NormalizedEvent::Market(market) => {
                self.on_market(market);
                PostTradeOutcome::default()
            }
            NormalizedEvent::Order(update) => self.on_order_update(update),
            NormalizedEvent::System(system) => {
                self.apply_system_event(system);
                PostTradeOutcome::default()
            }
            NormalizedEvent::Account(_)
            | NormalizedEvent::Timer(_)
            | NormalizedEvent::Control(_) => PostTradeOutcome::default(),
        }
    }

    fn pre_trade_reason(
        &self,
        now_ms: TimeMs,
        order: &reap_core::NewOrder,
    ) -> Option<RiskRejectReason> {
        if let Some(reason) = &self.kill_switch {
            return Some(RiskRejectReason::KillSwitch(reason.clone()));
        }
        if let Some(reason) = self.halted_symbols.get(&order.symbol) {
            return Some(RiskRejectReason::SymbolHalted(reason.clone()));
        }
        if !order.qty.is_finite()
            || !order.price.is_finite()
            || order.qty <= 0.0
            || order.price <= 0.0
        {
            return Some(RiskRejectReason::InvalidOrder);
        }
        if self.limits.require_feed_health {
            let feeds = self
                .feed_health
                .iter()
                .filter(|((_, symbol), _)| symbol == &order.symbol)
                .map(|(_, health)| health)
                .collect::<Vec<_>>();
            if feeds.is_empty() {
                return Some(RiskRejectReason::FeedNotReady);
            }
            if feeds.iter().all(|health| {
                health.stale
                    || now_ms.saturating_sub(health.last_ready_ms) > self.limits.max_feed_age_ms
            }) {
                return Some(RiskRejectReason::FeedStale);
            }
        }
        if self.limits.require_private_health {
            if self.private_health.is_empty() {
                return Some(RiskRejectReason::PrivateStreamNotReady);
            }
            if self.private_health.values().all(|health| {
                health.stale
                    || now_ms.saturating_sub(health.last_ready_ms) > self.limits.max_private_age_ms
            }) {
                return Some(RiskRejectReason::PrivateStreamStale);
            }
        }

        let notional = order.qty * order.price;
        if notional > self.limits.max_order_notional_usd {
            return Some(RiskRejectReason::OrderNotional {
                value: notional,
                limit: self.limits.max_order_notional_usd,
            });
        }
        let position = self.positions.get(&order.symbol).copied().unwrap_or(0.0);
        let pending_qty = self
            .live_orders
            .values()
            .filter(|live| live.symbol == order.symbol)
            .map(|live| live.signed_qty)
            .sum::<f64>();
        let projected =
            ((position + pending_qty + order.side.factor() * order.qty) * order.price).abs();
        if projected > self.limits.max_abs_position_notional_usd {
            return Some(RiskRejectReason::PositionNotional {
                value: projected,
                limit: self.limits.max_abs_position_notional_usd,
            });
        }
        let live_order_notional = self
            .live_orders
            .values()
            .map(|order| order.notional_usd)
            .sum::<f64>()
            + notional;
        if live_order_notional > self.limits.max_live_order_notional_usd {
            return Some(RiskRejectReason::LiveOrderNotional {
                value: live_order_notional,
                limit: self.limits.max_live_order_notional_usd,
            });
        }
        if self.turnover_usd + notional > self.limits.max_turnover_usd {
            return Some(RiskRejectReason::Turnover {
                value: self.turnover_usd + notional,
                limit: self.limits.max_turnover_usd,
            });
        }
        let drawdown = self.peak_equity_usd - self.equity_usd;
        if drawdown > self.limits.max_drawdown_usd {
            return Some(RiskRejectReason::Drawdown {
                value: drawdown,
                limit: self.limits.max_drawdown_usd,
            });
        }
        None
    }

    fn evaluate_post_trade(&mut self, ts_ms: TimeMs, symbol: Option<&str>) -> PostTradeOutcome {
        let breach = symbol.and_then(|symbol| {
            let mark = self.marks.get(symbol).copied().unwrap_or(0.0);
            let exposure = (self.position(symbol) * mark).abs();
            (exposure > self.limits.max_abs_position_notional_usd).then(|| {
                format!(
                    "position notional {exposure} exceeds {}",
                    self.limits.max_abs_position_notional_usd
                )
            })
        });
        let breach = breach.or_else(|| {
            let live_notional = self
                .live_orders
                .values()
                .map(|order| order.notional_usd)
                .sum::<f64>();
            (live_notional > self.limits.max_live_order_notional_usd).then(|| {
                format!(
                    "live order notional {live_notional} exceeds {}",
                    self.limits.max_live_order_notional_usd
                )
            })
        });
        let breach = breach.or_else(|| {
            (self.turnover_usd > self.limits.max_turnover_usd).then(|| {
                format!(
                    "turnover {} exceeds {}",
                    self.turnover_usd, self.limits.max_turnover_usd
                )
            })
        });
        let drawdown = self.peak_equity_usd - self.equity_usd;
        let breach = breach.or_else(|| {
            (drawdown > self.limits.max_drawdown_usd).then(|| {
                format!(
                    "drawdown {drawdown} exceeds {}",
                    self.limits.max_drawdown_usd
                )
            })
        });
        let Some(reason) = breach else {
            return PostTradeOutcome::default();
        };
        self.kill_switch = Some(reason.clone());
        PostTradeOutcome {
            events: vec![
                SystemEvent {
                    ts_ms,
                    kind: SystemEventKind::RiskBreach,
                    venue: None,
                    symbol: symbol.map(str::to_string),
                    reason: reason.clone(),
                },
                SystemEvent {
                    ts_ms,
                    kind: SystemEventKind::KillSwitchActivated,
                    venue: None,
                    symbol: symbol.map(str::to_string),
                    reason,
                },
            ],
        }
    }
}

#[derive(Debug, Clone, Default)]
struct StreamHealth {
    last_ready_ms: TimeMs,
    stale: bool,
}

#[derive(Debug)]
struct LiveOrderRisk {
    symbol: Symbol,
    notional_usd: f64,
    signed_qty: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FillKey {
    order_id: String,
    ts_ms: TimeMs,
    qty: u64,
    price: u64,
}

fn risk_price(order_price: f64, fill_price: f64) -> f64 {
    if order_price > 0.0 {
        order_price
    } else {
        fill_price
    }
}

#[cfg(test)]
mod tests {
    use reap_core::{FillLiquidity, NewOrder, OrderEvent, Side, TimeInForce};

    use super::*;

    fn order() -> OrderIntent {
        OrderIntent::NewOrder(NewOrder {
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            qty: 1.0,
            price: 100.0,
            time_in_force: TimeInForce::PostOnly,
            reason: "quote".to_string(),
        })
    }

    fn ready_gate() -> RiskGate {
        let mut gate = RiskGate::new(RiskLimits::default());
        gate.mark_feed_ready(Venue::Okx, "BTC-USDT", 100);
        gate.mark_private_ready(Venue::Okx, 100);
        gate
    }

    #[test]
    fn pre_trade_fails_closed_without_feed_or_private_health() {
        let gate = RiskGate::new(RiskLimits::default());
        assert!(matches!(
            gate.pre_trade(100, order()),
            RiskDecision::Rejected {
                reason: RiskRejectReason::FeedNotReady,
                ..
            }
        ));

        let gate = ready_gate();
        assert!(matches!(
            gate.pre_trade(100, order()),
            RiskDecision::Allowed(_)
        ));
    }

    #[test]
    fn stale_private_stream_blocks_new_orders_but_not_cancels() {
        let mut gate = ready_gate();
        let events = gate.check_staleness(2_101);
        assert!(
            events
                .iter()
                .any(|event| event.kind == SystemEventKind::PrivateStreamStale)
        );
        assert!(matches!(
            gate.pre_trade(2_101, order()),
            RiskDecision::Rejected { .. }
        ));
        assert!(matches!(
            gate.pre_trade(
                2_101,
                OrderIntent::CancelOrder {
                    order_id: "1".to_string(),
                    reason: "risk".to_string()
                }
            ),
            RiskDecision::Allowed(_)
        ));
    }

    #[test]
    fn accepted_heartbeats_refresh_stale_health() {
        let mut gate = ready_gate();
        let stale = gate.check_staleness(2_101);
        assert!(!stale.is_empty());
        gate.apply_system_event(&SystemEvent {
            ts_ms: 2_102,
            kind: SystemEventKind::FeedHeartbeat,
            venue: Some(Venue::Okx),
            symbol: Some("BTC-USDT".to_string()),
            reason: "book".to_string(),
        });
        gate.apply_system_event(&SystemEvent {
            ts_ms: 2_102,
            kind: SystemEventKind::PrivateStreamHeartbeat,
            venue: Some(Venue::Okx),
            symbol: None,
            reason: "orders".to_string(),
        });

        assert!(matches!(
            gate.pre_trade(2_102, order()),
            RiskDecision::Allowed(_)
        ));
    }

    #[test]
    fn kill_switch_and_symbol_halt_are_typed_events() {
        let mut gate = ready_gate();
        let halt = gate.halt_symbol(101, "BTC-USDT", "manual");
        assert_eq!(halt.kind, SystemEventKind::SymbolHalted);
        assert!(matches!(
            gate.pre_trade(101, order()),
            RiskDecision::Rejected {
                reason: RiskRejectReason::SymbolHalted(_),
                ..
            }
        ));
        gate.resume_symbol(102, "BTC-USDT", "operator");
        let kill = gate.activate_kill_switch(103, "operator");
        assert_eq!(kill.kind, SystemEventKind::KillSwitchActivated);
        assert!(gate.is_killed());
    }

    #[test]
    fn post_trade_position_breach_activates_kill_switch() {
        let limits = RiskLimits {
            max_abs_position_notional_usd: 50.0,
            ..RiskLimits::default()
        };
        let mut gate = RiskGate::new(limits);
        gate.marks.insert("BTC-USDT".to_string(), 100.0);
        let outcome = gate.on_order_update(&OrderUpdate {
            ts_ms: 10,
            order_id: "1".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            event: OrderEvent::FullyFilled,
            status: OrderStatus::Filled,
            price: 100.0,
            qty: 1.0,
            open_qty: 0.0,
            filled_qty: 1.0,
            avg_fill_price: 100.0,
            last_fill_qty: 1.0,
            last_fill_price: 100.0,
            last_fill_liquidity: Some(FillLiquidity::Taker),
            reason: "test".to_string(),
        });

        assert!(gate.is_killed());
        assert_eq!(outcome.events[0].kind, SystemEventKind::RiskBreach);
    }

    #[test]
    fn pre_trade_position_limit_includes_directional_live_orders() {
        let limits = RiskLimits {
            max_abs_position_notional_usd: 150.0,
            require_feed_health: false,
            require_private_health: false,
            ..RiskLimits::default()
        };
        let mut gate = RiskGate::new(limits);
        gate.on_order_update(&OrderUpdate {
            ts_ms: 1,
            order_id: "live".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            event: OrderEvent::New,
            status: OrderStatus::Live,
            price: 100.0,
            qty: 1.0,
            open_qty: 1.0,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            reason: "test".to_string(),
        });

        assert!(matches!(
            gate.pre_trade(1, order()),
            RiskDecision::Rejected {
                reason: RiskRejectReason::PositionNotional { .. },
                ..
            }
        ));
    }

    #[test]
    fn post_trade_live_order_breach_fails_closed() {
        let limits = RiskLimits {
            max_live_order_notional_usd: 50.0,
            ..RiskLimits::default()
        };
        let mut gate = RiskGate::new(limits);
        let outcome = gate.on_order_update(&OrderUpdate {
            ts_ms: 1,
            order_id: "live".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            event: OrderEvent::New,
            status: OrderStatus::Live,
            price: 100.0,
            qty: 1.0,
            open_qty: 1.0,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            reason: "test".to_string(),
        });

        assert!(gate.is_killed());
        assert_eq!(outcome.events[0].kind, SystemEventKind::RiskBreach);
    }
}
