use std::collections::{HashMap, HashSet, VecDeque};

use reap_core::{
    AccountUpdate, MarketEvent, NormalizedEvent, OrderIntent, OrderStatus, OrderUpdate, Symbol,
    SystemEvent, SystemEventKind, TimeInForce, TimeMs, Venue,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StablecoinGuardConfig {
    pub symbol: Symbol,
    pub max_downside_deviation: f64,
}

impl Default for StablecoinGuardConfig {
    fn default() -> Self {
        Self {
            symbol: String::new(),
            max_downside_deviation: 0.01,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RiskLimits {
    pub max_order_notional_usd: f64,
    pub max_abs_position_notional_usd: f64,
    pub max_live_order_notional_usd: f64,
    pub max_live_order_count: usize,
    pub max_live_order_count_per_symbol: usize,
    pub order_reject_count_limit: usize,
    pub order_reject_count_per_symbol_limit: usize,
    pub order_reject_window_ms: TimeMs,
    pub unfilled_ioc_cancel_count_per_symbol_limit: usize,
    pub unfilled_ioc_cancel_window_ms: TimeMs,
    pub max_turnover_usd: f64,
    pub max_drawdown_usd: f64,
    pub max_feed_age_ms: TimeMs,
    pub max_private_age_ms: TimeMs,
    pub require_feed_health: bool,
    pub require_private_health: bool,
    pub stablecoin_guards: Vec<StablecoinGuardConfig>,
    pub stablecoin_max_age_ms: TimeMs,
    pub stablecoin_breach_debounce_ms: TimeMs,
    pub forced_repayment_indicator_limit: u8,
}

impl Default for RiskLimits {
    fn default() -> Self {
        Self {
            max_order_notional_usd: 25_000.0,
            max_abs_position_notional_usd: 100_000.0,
            max_live_order_notional_usd: 100_000.0,
            max_live_order_count: 256,
            max_live_order_count_per_symbol: 64,
            order_reject_count_limit: 10,
            order_reject_count_per_symbol_limit: 5,
            order_reject_window_ms: 60_000,
            unfilled_ioc_cancel_count_per_symbol_limit: 10,
            unfilled_ioc_cancel_window_ms: 60_000,
            max_turnover_usd: 1_000_000.0,
            max_drawdown_usd: 10_000.0,
            max_feed_age_ms: 1_000,
            max_private_age_ms: 2_000,
            require_feed_health: true,
            require_private_health: true,
            stablecoin_guards: Vec::new(),
            stablecoin_max_age_ms: 75_000,
            stablecoin_breach_debounce_ms: 5_000,
            forced_repayment_indicator_limit: 1,
        }
    }
}

impl RiskLimits {
    pub fn validation_error(&self) -> Option<String> {
        for (name, value) in [
            ("max_order_notional_usd", self.max_order_notional_usd),
            (
                "max_abs_position_notional_usd",
                self.max_abs_position_notional_usd,
            ),
            (
                "max_live_order_notional_usd",
                self.max_live_order_notional_usd,
            ),
            ("max_turnover_usd", self.max_turnover_usd),
            ("max_drawdown_usd", self.max_drawdown_usd),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Some(format!("{name} must be finite and non-negative"));
            }
        }
        if self.max_feed_age_ms == 0 {
            return Some("max_feed_age_ms must be positive".to_string());
        }
        if self.max_private_age_ms == 0 {
            return Some("max_private_age_ms must be positive".to_string());
        }
        if self.stablecoin_max_age_ms == 0 {
            return Some("stablecoin_max_age_ms must be positive".to_string());
        }
        if self.stablecoin_breach_debounce_ms == 0 {
            return Some("stablecoin_breach_debounce_ms must be positive".to_string());
        }
        if self.max_live_order_count == 0 {
            return Some("max_live_order_count must be positive".to_string());
        }
        if self.max_live_order_count_per_symbol == 0 {
            return Some("max_live_order_count_per_symbol must be positive".to_string());
        }
        if self.max_live_order_count_per_symbol > self.max_live_order_count {
            return Some(
                "max_live_order_count_per_symbol must not exceed max_live_order_count".to_string(),
            );
        }
        if self.order_reject_count_limit == 0 {
            return Some("order_reject_count_limit must be positive".to_string());
        }
        if self.order_reject_count_per_symbol_limit == 0 {
            return Some("order_reject_count_per_symbol_limit must be positive".to_string());
        }
        if self.order_reject_count_per_symbol_limit > self.order_reject_count_limit {
            return Some(
                "order_reject_count_per_symbol_limit must not exceed order_reject_count_limit"
                    .to_string(),
            );
        }
        if self.order_reject_window_ms == 0 {
            return Some("order_reject_window_ms must be positive".to_string());
        }
        if self.unfilled_ioc_cancel_count_per_symbol_limit == 0 {
            return Some("unfilled_ioc_cancel_count_per_symbol_limit must be positive".to_string());
        }
        if self.unfilled_ioc_cancel_window_ms == 0 {
            return Some("unfilled_ioc_cancel_window_ms must be positive".to_string());
        }
        if !(1..=5).contains(&self.forced_repayment_indicator_limit) {
            return Some("forced_repayment_indicator_limit must be between 1 and 5".to_string());
        }
        let mut symbols = HashSet::new();
        for guard in &self.stablecoin_guards {
            if guard.symbol.trim().is_empty() {
                return Some("stablecoin guard symbol must not be empty".to_string());
            }
            if !symbols.insert(guard.symbol.as_str()) {
                return Some(format!(
                    "duplicate stablecoin guard symbol {}",
                    guard.symbol
                ));
            }
            if !guard.max_downside_deviation.is_finite()
                || guard.max_downside_deviation <= 0.0
                || guard.max_downside_deviation >= 1.0
            {
                return Some(format!(
                    "stablecoin guard {} max_downside_deviation must be finite and between 0 and 1",
                    guard.symbol
                ));
            }
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum InstrumentRiskModel {
    Spot,
    LinearDerivative { contract_value: f64 },
    InverseDerivative { contract_value: f64 },
}

impl InstrumentRiskModel {
    pub fn is_valid(self) -> bool {
        match self {
            Self::Spot => true,
            Self::LinearDerivative { contract_value }
            | Self::InverseDerivative { contract_value } => {
                contract_value.is_finite() && contract_value > 0.0
            }
        }
    }

    fn notional_usd(self, qty: f64, price: f64) -> f64 {
        match self {
            Self::Spot => qty * price,
            Self::LinearDerivative { contract_value } => qty * contract_value * price,
            Self::InverseDerivative { contract_value } => qty * contract_value,
        }
        .abs()
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
    StablecoinReferenceNotReady {
        symbol: Symbol,
    },
    StablecoinReferenceStale {
        symbol: Symbol,
        age_ms: TimeMs,
        limit_ms: TimeMs,
    },
    StablecoinReferenceConflict {
        symbol: Symbol,
        ts_ms: TimeMs,
    },
    StablecoinReferenceInvalid {
        symbol: Symbol,
    },
    StablecoinDepeg {
        symbol: Symbol,
        price: f64,
        minimum_price: f64,
    },
    InvalidOrder,
    OrderNotional {
        value: f64,
        limit: f64,
    },
    PositionNotional {
        value: f64,
        limit: f64,
    },
    LiveOrderNotional {
        value: f64,
        limit: f64,
    },
    LiveOrderCount {
        value: usize,
        limit: usize,
    },
    SymbolLiveOrderCount {
        symbol: Symbol,
        value: usize,
        limit: usize,
    },
    Turnover {
        value: f64,
        limit: f64,
    },
    Drawdown {
        value: f64,
        limit: f64,
    },
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StablecoinGuardHealth {
    pub symbol: Symbol,
    pub healthy: bool,
    pub reason: String,
}

#[derive(Debug)]
pub struct RiskGate {
    limits: RiskLimits,
    kill_switch: Option<String>,
    halted_symbols: HashMap<Symbol, String>,
    feed_health: HashMap<(Venue, Symbol), StreamHealth>,
    private_health: HashMap<(Venue, Option<String>), StreamHealth>,
    marks: HashMap<Symbol, f64>,
    instrument_models: HashMap<Symbol, InstrumentRiskModel>,
    positions: HashMap<Symbol, f64>,
    live_orders: HashMap<String, LiveOrderRisk>,
    order_rejections: VecDeque<OrderRejection>,
    rejected_order_ids: HashSet<String>,
    last_order_rejection_ms: TimeMs,
    unfilled_ioc_cancellations: VecDeque<UnfilledIocCancellation>,
    unfilled_ioc_cancelled_order_ids: HashSet<String>,
    last_unfilled_ioc_cancel_ms: TimeMs,
    turnover_usd: f64,
    equity_usd: f64,
    equity_by_account: HashMap<Option<String>, f64>,
    peak_equity_usd: f64,
    seen_fills: HashSet<FillKey>,
    stablecoin_rates: HashMap<Symbol, StablecoinRateState>,
    stablecoin_breach_since_ms: HashMap<Symbol, TimeMs>,
}

impl RiskGate {
    pub fn new(limits: RiskLimits) -> Self {
        let kill_switch = limits
            .validation_error()
            .map(|error| format!("invalid risk limits: {error}"));
        Self {
            limits,
            kill_switch,
            halted_symbols: HashMap::new(),
            feed_health: HashMap::new(),
            private_health: HashMap::new(),
            marks: HashMap::new(),
            instrument_models: HashMap::new(),
            positions: HashMap::new(),
            live_orders: HashMap::new(),
            order_rejections: VecDeque::new(),
            rejected_order_ids: HashSet::new(),
            last_order_rejection_ms: 0,
            unfilled_ioc_cancellations: VecDeque::new(),
            unfilled_ioc_cancelled_order_ids: HashSet::new(),
            last_unfilled_ioc_cancel_ms: 0,
            turnover_usd: 0.0,
            equity_usd: 0.0,
            equity_by_account: HashMap::new(),
            peak_equity_usd: 0.0,
            seen_fills: HashSet::new(),
            stablecoin_rates: HashMap::new(),
            stablecoin_breach_since_ms: HashMap::new(),
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

    pub fn stablecoin_guard_health(&self, now_ms: TimeMs) -> Vec<StablecoinGuardHealth> {
        self.limits
            .stablecoin_guards
            .iter()
            .map(|guard| match self.stablecoin_reject_reason(guard, now_ms) {
                Some(reason) => StablecoinGuardHealth {
                    symbol: guard.symbol.clone(),
                    healthy: false,
                    reason: stablecoin_reason(&reason),
                },
                None => StablecoinGuardHealth {
                    symbol: guard.symbol.clone(),
                    healthy: true,
                    reason: "stablecoin reference is fresh and within limit".to_string(),
                },
            })
            .collect()
    }

    pub fn position(&self, symbol: &str) -> f64 {
        self.positions.get(symbol).copied().unwrap_or(0.0)
    }

    pub fn set_instrument_model(
        &mut self,
        symbol: impl Into<Symbol>,
        model: InstrumentRiskModel,
    ) -> bool {
        if !model.is_valid() {
            return false;
        }
        self.instrument_models.insert(symbol.into(), model);
        true
    }

    fn instrument_model(&self, symbol: &str) -> InstrumentRiskModel {
        self.instrument_models
            .get(symbol)
            .copied()
            .unwrap_or(InstrumentRiskModel::Spot)
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
        let MarketEvent::IndexPrice {
            ts_ms,
            symbol,
            price,
        } = event
        else {
            return;
        };
        if !self
            .limits
            .stablecoin_guards
            .iter()
            .any(|guard| guard.symbol == *symbol)
        {
            return;
        }
        if let Some(state) = self.stablecoin_rates.get_mut(symbol) {
            if *ts_ms < state.ts_ms {
                return;
            }
            if *ts_ms == state.ts_ms {
                if price.to_bits() != state.price.to_bits() {
                    state.conflict = true;
                }
                return;
            }
            *state = StablecoinRateState {
                ts_ms: *ts_ms,
                price: *price,
                conflict: false,
            };
        } else {
            self.stablecoin_rates.insert(
                symbol.clone(),
                StablecoinRateState {
                    ts_ms: *ts_ms,
                    price: *price,
                    conflict: false,
                },
            );
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
        self.mark_private_account_ready(venue, None, ts_ms);
    }

    pub fn mark_private_account_ready(
        &mut self,
        venue: Venue,
        account_id: Option<String>,
        ts_ms: TimeMs,
    ) {
        self.private_health.insert(
            (venue, account_id),
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
            let risk_price = risk_price(update.price, update.avg_fill_price);
            self.live_orders.insert(
                update.order_id.clone(),
                LiveOrderRisk {
                    symbol: update.symbol.clone(),
                    notional_usd: self
                        .instrument_model(&update.symbol)
                        .notional_usd(update.open_qty, risk_price),
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
                self.turnover_usd += self
                    .instrument_model(&update.symbol)
                    .notional_usd(update.last_fill_qty, update.last_fill_price);
            }
        }
        if self.kill_switch.is_none() {
            if update.status == OrderStatus::Rejected
                && let Some((symbol, reason)) = self.observe_order_rejection(update)
            {
                return self.activate_risk_breach(update.ts_ms, symbol, reason);
            }
            if update.status == OrderStatus::Cancelled
                && update.time_in_force == Some(TimeInForce::Ioc)
                && update.filled_qty == 0.0
                && let Some((symbol, reason)) = self.observe_unfilled_ioc_cancel(update)
            {
                return self.activate_risk_breach(update.ts_ms, symbol, reason);
            }
        }
        self.evaluate_post_trade(update.ts_ms, Some(&update.symbol))
    }

    fn observe_order_rejection(
        &mut self,
        update: &OrderUpdate,
    ) -> Option<(Option<Symbol>, String)> {
        let now_ms = self.last_order_rejection_ms.max(update.ts_ms);
        self.last_order_rejection_ms = now_ms;
        while self.order_rejections.front().is_some_and(|rejection| {
            now_ms.saturating_sub(rejection.ts_ms) > self.limits.order_reject_window_ms
        }) {
            let expired = self
                .order_rejections
                .pop_front()
                .expect("front rejection was present");
            self.rejected_order_ids.remove(&expired.order_id);
        }
        if !self.rejected_order_ids.insert(update.order_id.clone()) {
            return None;
        }
        self.order_rejections.push_back(OrderRejection {
            ts_ms: now_ms,
            order_id: update.order_id.clone(),
            symbol: update.symbol.clone(),
        });

        let symbol_count = self
            .order_rejections
            .iter()
            .filter(|rejection| rejection.symbol == update.symbol)
            .count();
        if symbol_count >= self.limits.order_reject_count_per_symbol_limit {
            return Some((
                Some(update.symbol.clone()),
                format!(
                    "symbol {} order rejection count {} reached limit {} in {}ms; latest_order={}",
                    update.symbol,
                    symbol_count,
                    self.limits.order_reject_count_per_symbol_limit,
                    self.limits.order_reject_window_ms,
                    update.order_id
                ),
            ));
        }
        let total_count = self.order_rejections.len();
        (total_count >= self.limits.order_reject_count_limit).then(|| {
            (
                None,
                format!(
                    "order rejection count {} reached limit {} in {}ms; latest_order={} symbol={}",
                    total_count,
                    self.limits.order_reject_count_limit,
                    self.limits.order_reject_window_ms,
                    update.order_id,
                    update.symbol
                ),
            )
        })
    }

    fn observe_unfilled_ioc_cancel(
        &mut self,
        update: &OrderUpdate,
    ) -> Option<(Option<Symbol>, String)> {
        let now_ms = self.last_unfilled_ioc_cancel_ms.max(update.ts_ms);
        self.last_unfilled_ioc_cancel_ms = now_ms;
        while self
            .unfilled_ioc_cancellations
            .front()
            .is_some_and(|cancellation| {
                now_ms.saturating_sub(cancellation.ts_ms)
                    > self.limits.unfilled_ioc_cancel_window_ms
            })
        {
            let expired = self
                .unfilled_ioc_cancellations
                .pop_front()
                .expect("front IOC cancellation was present");
            self.unfilled_ioc_cancelled_order_ids
                .remove(&expired.order_id);
        }
        if !self
            .unfilled_ioc_cancelled_order_ids
            .insert(update.order_id.clone())
        {
            return None;
        }
        self.unfilled_ioc_cancellations
            .push_back(UnfilledIocCancellation {
                ts_ms: now_ms,
                order_id: update.order_id.clone(),
                symbol: update.symbol.clone(),
            });

        let symbol_count = self
            .unfilled_ioc_cancellations
            .iter()
            .filter(|cancellation| cancellation.symbol == update.symbol)
            .count();
        (symbol_count >= self.limits.unfilled_ioc_cancel_count_per_symbol_limit).then(|| {
            (
                Some(update.symbol.clone()),
                format!(
                    "symbol {} unfilled IOC cancellation count {} reached limit {} in {}ms; latest_order={}",
                    update.symbol,
                    symbol_count,
                    self.limits.unfilled_ioc_cancel_count_per_symbol_limit,
                    self.limits.unfilled_ioc_cancel_window_ms,
                    update.order_id
                ),
            )
        })
    }

    pub fn update_equity(&mut self, ts_ms: TimeMs, equity_usd: f64) -> PostTradeOutcome {
        self.equity_usd = equity_usd;
        self.peak_equity_usd = self.peak_equity_usd.max(equity_usd);
        self.evaluate_post_trade(ts_ms, None)
    }

    pub fn on_account_update(&mut self, update: &AccountUpdate) -> PostTradeOutcome {
        if let Some(balance) = update.balances.iter().find(|balance| {
            balance
                .forced_repayment_indicator
                .is_some_and(|indicator| indicator >= self.limits.forced_repayment_indicator_limit)
        }) {
            let indicator = balance
                .forced_repayment_indicator
                .expect("forced repayment predicate requires an indicator");
            return self.activate_risk_breach(
                update.ts_ms,
                None,
                format!(
                    "currency {} forced repayment indicator {} reached limit {}",
                    balance.currency, indicator, self.limits.forced_repayment_indicator_limit
                ),
            );
        }
        for position in &update.positions {
            self.positions.insert(position.symbol.clone(), position.qty);
        }
        for margin in &update.margins {
            if let Some(equity) = margin.adjusted_equity_usd {
                self.equity_by_account
                    .insert(margin.account_id.clone(), equity);
            }
        }
        if !self.equity_by_account.is_empty() {
            self.equity_usd = self.equity_by_account.values().sum();
            self.peak_equity_usd = self.peak_equity_usd.max(self.equity_usd);
        }
        self.evaluate_post_trade(update.ts_ms, None)
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
            account_id: None,
            symbol: None,
            reason,
        }
    }

    pub fn on_strategy_halt(
        &mut self,
        ts_ms: TimeMs,
        reason: impl Into<String>,
    ) -> PostTradeOutcome {
        if self.kill_switch.is_some() {
            return PostTradeOutcome::default();
        }
        self.activate_risk_breach(ts_ms, None, format!("strategy halted: {}", reason.into()))
    }

    pub fn reset_kill_switch(&mut self, ts_ms: TimeMs, reason: impl Into<String>) -> SystemEvent {
        self.kill_switch = None;
        SystemEvent {
            ts_ms,
            kind: SystemEventKind::KillSwitchReset,
            venue: None,
            account_id: None,
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
            account_id: None,
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
            account_id: None,
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
                    if event.account_id.is_some() {
                        self.private_health
                            .entry((venue, event.account_id.clone()))
                            .or_default()
                            .stale = true;
                    } else {
                        let mut matched = false;
                        for ((health_venue, _), health) in &mut self.private_health {
                            if *health_venue == venue {
                                health.stale = true;
                                matched = true;
                            }
                        }
                        if !matched {
                            self.private_health.entry((venue, None)).or_default().stale = true;
                        }
                    }
                } else {
                    for health in self.private_health.values_mut() {
                        health.stale = true;
                    }
                }
            }
            SystemEventKind::PrivateStreamHeartbeat | SystemEventKind::PrivateStreamRecovered => {
                if let Some(venue) = event.venue {
                    self.mark_private_account_ready(venue, event.account_id.clone(), event.ts_ms);
                }
            }
            SystemEventKind::OrderTransportStale
            | SystemEventKind::OrderTransportHeartbeat
            | SystemEventKind::OrderTransportRecovered => {}
            SystemEventKind::KillSwitchActivated | SystemEventKind::RiskBreach => {
                self.kill_switch = Some(event.reason.clone());
            }
            SystemEventKind::KillSwitchReset => {
                self.kill_switch = None;
                self.order_rejections.clear();
                self.rejected_order_ids.clear();
                self.last_order_rejection_ms = 0;
                self.unfilled_ioc_cancellations.clear();
                self.unfilled_ioc_cancelled_order_ids.clear();
                self.last_unfilled_ioc_cancel_ms = 0;
            }
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
            SystemEventKind::AccountHalted => {}
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
                    account_id: None,
                    symbol: Some(symbol.clone()),
                    reason: format!("feed age exceeded {}ms", self.limits.max_feed_age_ms),
                });
            }
        }
        for ((venue, account_id), health) in &mut self.private_health {
            if !health.stale
                && now_ms.saturating_sub(health.last_ready_ms) > self.limits.max_private_age_ms
            {
                health.stale = true;
                events.push(SystemEvent {
                    ts_ms: now_ms,
                    kind: SystemEventKind::PrivateStreamStale,
                    venue: Some(*venue),
                    account_id: account_id.clone(),
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
        let evaluate_stablecoins = self.event_updates_stablecoin_state(event);
        let mut outcome = match event {
            NormalizedEvent::Market(market) => {
                self.on_market(market);
                PostTradeOutcome::default()
            }
            NormalizedEvent::Order(update) => self.on_order_update(update),
            NormalizedEvent::System(system) => {
                self.apply_system_event(system);
                PostTradeOutcome::default()
            }
            NormalizedEvent::Account(update) => self.on_account_update(update),
            NormalizedEvent::Timer(_) | NormalizedEvent::Control(_) => PostTradeOutcome::default(),
        };
        if evaluate_stablecoins {
            outcome
                .events
                .extend(self.evaluate_stablecoin_guards(event.ts_ms()).events);
        }
        outcome
    }

    fn event_updates_stablecoin_state(&self, event: &NormalizedEvent) -> bool {
        match event {
            NormalizedEvent::Timer(_) => !self.limits.stablecoin_guards.is_empty(),
            NormalizedEvent::Market(MarketEvent::IndexPrice { symbol, .. }) => self
                .limits
                .stablecoin_guards
                .iter()
                .any(|guard| guard.symbol == *symbol),
            NormalizedEvent::System(system) => {
                system.kind == SystemEventKind::KillSwitchReset
                    && !self.limits.stablecoin_guards.is_empty()
            }
            NormalizedEvent::Order(_)
            | NormalizedEvent::Account(_)
            | NormalizedEvent::Control(_)
            | NormalizedEvent::Market(_) => false,
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
        if let Some(reason) = self
            .limits
            .stablecoin_guards
            .iter()
            .find_map(|guard| self.stablecoin_reject_reason(guard, now_ms))
        {
            return Some(reason);
        }
        if self.limits.require_feed_health {
            let mut matching_feed = false;
            let mut fresh_feed = false;
            for ((_, symbol), health) in &self.feed_health {
                if symbol != &order.symbol {
                    continue;
                }
                matching_feed = true;
                if !health.stale
                    && now_ms.saturating_sub(health.last_ready_ms) <= self.limits.max_feed_age_ms
                {
                    fresh_feed = true;
                    break;
                }
            }
            if !matching_feed {
                return Some(RiskRejectReason::FeedNotReady);
            }
            if !fresh_feed {
                return Some(RiskRejectReason::FeedStale);
            }
        }
        if self.limits.require_private_health {
            if self.private_health.is_empty() {
                return Some(RiskRejectReason::PrivateStreamNotReady);
            }
            if self.private_health.values().any(|health| {
                health.stale
                    || now_ms.saturating_sub(health.last_ready_ms) > self.limits.max_private_age_ms
            }) {
                return Some(RiskRejectReason::PrivateStreamStale);
            }
        }

        let live_order_count = self.live_orders.len().saturating_add(1);
        if live_order_count > self.limits.max_live_order_count {
            return Some(RiskRejectReason::LiveOrderCount {
                value: live_order_count,
                limit: self.limits.max_live_order_count,
            });
        }
        let symbol_live_order_count = self
            .live_orders
            .values()
            .filter(|live| live.symbol == order.symbol)
            .count()
            .saturating_add(1);
        if symbol_live_order_count > self.limits.max_live_order_count_per_symbol {
            return Some(RiskRejectReason::SymbolLiveOrderCount {
                symbol: order.symbol.clone(),
                value: symbol_live_order_count,
                limit: self.limits.max_live_order_count_per_symbol,
            });
        }

        let model = self.instrument_model(&order.symbol);
        let notional = model.notional_usd(order.qty, order.price);
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
        let projected = model.notional_usd(
            position + pending_qty + order.side.factor() * order.qty,
            order.price,
        );
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

    fn stablecoin_reject_reason(
        &self,
        guard: &StablecoinGuardConfig,
        now_ms: TimeMs,
    ) -> Option<RiskRejectReason> {
        let Some(state) = self.stablecoin_rates.get(&guard.symbol) else {
            return Some(RiskRejectReason::StablecoinReferenceNotReady {
                symbol: guard.symbol.clone(),
            });
        };
        if state.conflict {
            return Some(RiskRejectReason::StablecoinReferenceConflict {
                symbol: guard.symbol.clone(),
                ts_ms: state.ts_ms,
            });
        }
        if !state.price.is_finite() || state.price <= 0.0 {
            return Some(RiskRejectReason::StablecoinReferenceInvalid {
                symbol: guard.symbol.clone(),
            });
        }
        let age_ms = now_ms.saturating_sub(state.ts_ms);
        if age_ms > self.limits.stablecoin_max_age_ms {
            return Some(RiskRejectReason::StablecoinReferenceStale {
                symbol: guard.symbol.clone(),
                age_ms,
                limit_ms: self.limits.stablecoin_max_age_ms,
            });
        }
        let minimum_price = 1.0 - guard.max_downside_deviation;
        if state.price < minimum_price {
            return Some(RiskRejectReason::StablecoinDepeg {
                symbol: guard.symbol.clone(),
                price: state.price,
                minimum_price,
            });
        }
        None
    }

    fn evaluate_stablecoin_guards(&mut self, now_ms: TimeMs) -> PostTradeOutcome {
        let mut breach = None;
        for index in 0..self.limits.stablecoin_guards.len() {
            let issue = {
                let guard = &self.limits.stablecoin_guards[index];
                self.stablecoin_reject_reason(guard, now_ms)
            };
            let symbol = &self.limits.stablecoin_guards[index].symbol;
            let Some(issue) = issue else {
                self.stablecoin_breach_since_ms.remove(symbol);
                continue;
            };
            let since_ms = *self
                .stablecoin_breach_since_ms
                .entry(symbol.clone())
                .or_insert(now_ms);
            if self.kill_switch.is_none()
                && breach.is_none()
                && now_ms.saturating_sub(since_ms) >= self.limits.stablecoin_breach_debounce_ms
            {
                breach = Some((symbol.clone(), stablecoin_reason(&issue)));
            }
        }
        let Some((symbol, reason)) = breach else {
            return PostTradeOutcome::default();
        };
        self.activate_risk_breach(now_ms, Some(symbol), reason)
    }

    fn evaluate_post_trade(&mut self, ts_ms: TimeMs, symbol: Option<&str>) -> PostTradeOutcome {
        if self.kill_switch.is_some() {
            return PostTradeOutcome::default();
        }
        let breach = symbol.and_then(|symbol| {
            let mark = self.marks.get(symbol).copied().unwrap_or(0.0);
            let exposure = self
                .instrument_model(symbol)
                .notional_usd(self.position(symbol), mark);
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
            (self.live_orders.len() > self.limits.max_live_order_count).then(|| {
                format!(
                    "live order count {} exceeds {}",
                    self.live_orders.len(),
                    self.limits.max_live_order_count
                )
            })
        });
        let breach = breach.or_else(|| {
            symbol.and_then(|symbol| {
                let count = self
                    .live_orders
                    .values()
                    .filter(|order| order.symbol == symbol)
                    .count();
                (count > self.limits.max_live_order_count_per_symbol).then(|| {
                    format!(
                        "symbol {symbol} live order count {count} exceeds {}",
                        self.limits.max_live_order_count_per_symbol
                    )
                })
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
        self.activate_risk_breach(ts_ms, symbol.map(str::to_string), reason)
    }

    fn activate_risk_breach(
        &mut self,
        ts_ms: TimeMs,
        symbol: Option<Symbol>,
        reason: String,
    ) -> PostTradeOutcome {
        self.kill_switch = Some(reason.clone());
        PostTradeOutcome {
            events: vec![
                SystemEvent {
                    ts_ms,
                    kind: SystemEventKind::RiskBreach,
                    venue: None,
                    account_id: None,
                    symbol: symbol.clone(),
                    reason: reason.clone(),
                },
                SystemEvent {
                    ts_ms,
                    kind: SystemEventKind::KillSwitchActivated,
                    venue: None,
                    account_id: None,
                    symbol,
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

#[derive(Debug, Clone)]
struct StablecoinRateState {
    ts_ms: TimeMs,
    price: f64,
    conflict: bool,
}

#[derive(Debug)]
struct LiveOrderRisk {
    symbol: Symbol,
    notional_usd: f64,
    signed_qty: f64,
}

#[derive(Debug)]
struct OrderRejection {
    ts_ms: TimeMs,
    order_id: String,
    symbol: Symbol,
}

#[derive(Debug)]
struct UnfilledIocCancellation {
    ts_ms: TimeMs,
    order_id: String,
    symbol: Symbol,
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

fn stablecoin_reason(reason: &RiskRejectReason) -> String {
    match reason {
        RiskRejectReason::StablecoinReferenceNotReady { symbol } => {
            format!("stablecoin reference {symbol} is not ready")
        }
        RiskRejectReason::StablecoinReferenceStale {
            symbol,
            age_ms,
            limit_ms,
        } => format!("stablecoin reference {symbol} age {age_ms}ms exceeds {limit_ms}ms"),
        RiskRejectReason::StablecoinReferenceConflict { symbol, ts_ms } => {
            format!("stablecoin reference {symbol} has conflicting values at timestamp {ts_ms}")
        }
        RiskRejectReason::StablecoinReferenceInvalid { symbol } => {
            format!("stablecoin reference {symbol} is invalid")
        }
        RiskRejectReason::StablecoinDepeg {
            symbol,
            price,
            minimum_price,
        } => {
            format!("stablecoin reference {symbol} price {price} is below minimum {minimum_price}")
        }
        _ => "stablecoin reference is unhealthy".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use reap_core::{FillLiquidity, NewOrder, OrderEvent, Side, TimeInForce, TimerEvent};

    use super::*;

    fn order() -> OrderIntent {
        OrderIntent::NewOrder(NewOrder {
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy,
            qty: 1.0,
            price: 100.0,
            time_in_force: TimeInForce::PostOnly,
            reduce_only: false,
            self_trade_prevention: None,
            reason: "quote".to_string(),
        })
    }

    fn live_order_update(order_id: &str, symbol: &str, ts_ms: TimeMs) -> OrderUpdate {
        OrderUpdate {
            ts_ms,
            order_id: order_id.to_string(),
            symbol: symbol.to_string(),
            side: Side::Buy,
            event: OrderEvent::New,
            status: OrderStatus::Live,
            price: 100.0,
            time_in_force: None,
            qty: 1.0,
            open_qty: 1.0,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            last_fill_fee: None,
            reason: "test".to_string(),
        }
    }

    fn rejected_order_update(order_id: &str, symbol: &str, ts_ms: TimeMs) -> OrderUpdate {
        let mut update = live_order_update(order_id, symbol, ts_ms);
        update.event = OrderEvent::Rejected;
        update.status = OrderStatus::Rejected;
        update.open_qty = 0.0;
        update.reason = "okx_private:test rejection".to_string();
        update
    }

    fn unfilled_ioc_cancel_update(order_id: &str, symbol: &str, ts_ms: TimeMs) -> OrderUpdate {
        let mut update = live_order_update(order_id, symbol, ts_ms);
        update.event = OrderEvent::Cancelled;
        update.status = OrderStatus::Cancelled;
        update.time_in_force = Some(TimeInForce::Ioc);
        update.open_qty = 0.0;
        update.reason = "hedge:test".to_string();
        update
    }

    fn ready_gate() -> RiskGate {
        let mut gate = RiskGate::new(RiskLimits::default());
        gate.mark_feed_ready(Venue::Okx, "BTC-USDT", 100);
        gate.mark_private_ready(Venue::Okx, 100);
        gate
    }

    fn stablecoin_limits() -> RiskLimits {
        RiskLimits {
            require_feed_health: false,
            require_private_health: false,
            stablecoin_guards: vec![StablecoinGuardConfig {
                symbol: "USDT-USD".to_string(),
                max_downside_deviation: 0.01,
            }],
            stablecoin_max_age_ms: 10_000,
            stablecoin_breach_debounce_ms: 5_000,
            ..RiskLimits::default()
        }
    }

    fn stablecoin_index(ts_ms: TimeMs, price: f64) -> MarketEvent {
        MarketEvent::IndexPrice {
            ts_ms,
            symbol: "USDT-USD".to_string(),
            price,
        }
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
    fn invalid_limits_activate_kill_switch_at_construction() {
        let gate = RiskGate::new(RiskLimits {
            max_order_notional_usd: f64::NAN,
            require_feed_health: false,
            require_private_health: false,
            ..RiskLimits::default()
        });

        assert!(gate.is_killed());
        assert!(matches!(
            gate.pre_trade(1, order()),
            RiskDecision::Rejected {
                reason: RiskRejectReason::KillSwitch(_),
                ..
            }
        ));
    }

    #[test]
    fn live_order_count_limits_are_validated() {
        for limits in [
            RiskLimits {
                max_live_order_count: 0,
                ..RiskLimits::default()
            },
            RiskLimits {
                max_live_order_count_per_symbol: 0,
                ..RiskLimits::default()
            },
            RiskLimits {
                max_live_order_count: 2,
                max_live_order_count_per_symbol: 3,
                ..RiskLimits::default()
            },
        ] {
            assert!(limits.validation_error().is_some());
        }
    }

    #[test]
    fn order_rejection_limits_are_validated() {
        for limits in [
            RiskLimits {
                order_reject_count_limit: 0,
                ..RiskLimits::default()
            },
            RiskLimits {
                order_reject_count_per_symbol_limit: 0,
                ..RiskLimits::default()
            },
            RiskLimits {
                order_reject_count_limit: 2,
                order_reject_count_per_symbol_limit: 3,
                ..RiskLimits::default()
            },
            RiskLimits {
                order_reject_window_ms: 0,
                ..RiskLimits::default()
            },
        ] {
            assert!(limits.validation_error().is_some());
        }
    }

    #[test]
    fn unfilled_ioc_cancel_limits_are_validated() {
        for limits in [
            RiskLimits {
                unfilled_ioc_cancel_count_per_symbol_limit: 0,
                ..RiskLimits::default()
            },
            RiskLimits {
                unfilled_ioc_cancel_window_ms: 0,
                ..RiskLimits::default()
            },
        ] {
            assert!(limits.validation_error().is_some());
        }
    }

    #[test]
    fn duplicate_rejections_count_once_and_symbol_threshold_latches() {
        let mut gate = RiskGate::new(RiskLimits {
            order_reject_count_limit: 10,
            order_reject_count_per_symbol_limit: 2,
            order_reject_window_ms: 100,
            ..RiskLimits::default()
        });
        let first = rejected_order_update("reject-1", "BTC-USDT", 1);
        assert!(gate.on_order_update(&first).events.is_empty());
        let mut duplicate = first;
        duplicate.ts_ms = 2;
        assert!(gate.on_order_update(&duplicate).events.is_empty());

        let outcome = gate.on_order_update(&rejected_order_update("reject-2", "BTC-USDT", 3));

        assert!(gate.is_killed());
        assert_eq!(outcome.events[0].kind, SystemEventKind::RiskBreach);
        assert_eq!(outcome.events[0].symbol.as_deref(), Some("BTC-USDT"));
        assert!(
            outcome.events[0]
                .reason
                .contains("symbol BTC-USDT order rejection count 2 reached limit 2")
        );
    }

    #[test]
    fn rejection_window_expires_and_global_threshold_spans_symbols() {
        let limits = RiskLimits {
            order_reject_count_limit: 2,
            order_reject_count_per_symbol_limit: 2,
            order_reject_window_ms: 100,
            ..RiskLimits::default()
        };
        let mut expired = RiskGate::new(limits.clone());
        assert!(
            expired
                .on_order_update(&rejected_order_update("reject-1", "BTC-USDT", 1))
                .events
                .is_empty()
        );
        assert!(
            expired
                .on_order_update(&rejected_order_update("reject-2", "ETH-USDT", 102))
                .events
                .is_empty()
        );

        let mut global = RiskGate::new(limits);
        global.on_order_update(&rejected_order_update("reject-1", "BTC-USDT", 1));
        let outcome = global.on_order_update(&rejected_order_update("reject-2", "ETH-USDT", 2));

        assert!(global.is_killed());
        assert!(
            outcome.events[0]
                .reason
                .contains("order rejection count 2 reached limit 2")
        );

        global.apply_system_event(&SystemEvent {
            ts_ms: 3,
            kind: SystemEventKind::KillSwitchReset,
            venue: None,
            account_id: None,
            symbol: None,
            reason: "test reset".to_string(),
        });
        assert!(!global.is_killed());
        assert!(
            global
                .on_order_update(&rejected_order_update("reject-3", "BTC-USDT", 4))
                .events
                .is_empty()
        );
    }

    #[test]
    fn unfilled_ioc_cancellations_are_deduplicated_and_require_zero_fill() {
        let mut gate = RiskGate::new(RiskLimits {
            unfilled_ioc_cancel_count_per_symbol_limit: 2,
            unfilled_ioc_cancel_window_ms: 100,
            ..RiskLimits::default()
        });
        let first = unfilled_ioc_cancel_update("ioc-1", "BTC-USDT", 1);
        assert!(gate.on_order_update(&first).events.is_empty());
        let mut duplicate = first;
        duplicate.ts_ms = 2;
        assert!(gate.on_order_update(&duplicate).events.is_empty());

        let mut partially_filled = unfilled_ioc_cancel_update("ioc-partial", "BTC-USDT", 3);
        partially_filled.filled_qty = 0.1;
        assert!(gate.on_order_update(&partially_filled).events.is_empty());
        let mut post_only = unfilled_ioc_cancel_update("post-only", "BTC-USDT", 4);
        post_only.time_in_force = Some(TimeInForce::PostOnly);
        assert!(gate.on_order_update(&post_only).events.is_empty());

        let outcome = gate.on_order_update(&unfilled_ioc_cancel_update("ioc-2", "BTC-USDT", 5));

        assert!(gate.is_killed());
        assert_eq!(outcome.events[0].kind, SystemEventKind::RiskBreach);
        assert_eq!(outcome.events[0].symbol.as_deref(), Some("BTC-USDT"));
        assert!(
            outcome.events[0]
                .reason
                .contains("symbol BTC-USDT unfilled IOC cancellation count 2 reached limit 2")
        );
    }

    #[test]
    fn unfilled_ioc_cancel_window_is_per_symbol_and_monotonic() {
        let limits = RiskLimits {
            unfilled_ioc_cancel_count_per_symbol_limit: 2,
            unfilled_ioc_cancel_window_ms: 100,
            ..RiskLimits::default()
        };
        let mut expired = RiskGate::new(limits.clone());
        expired.on_order_update(&unfilled_ioc_cancel_update("btc-1", "BTC-USDT", 1));
        expired.on_order_update(&unfilled_ioc_cancel_update("eth-1", "ETH-USDT", 2));
        assert!(!expired.is_killed());
        expired.on_order_update(&unfilled_ioc_cancel_update("btc-2", "BTC-USDT", 102));
        assert!(!expired.is_killed());

        let mut out_of_order = RiskGate::new(limits);
        out_of_order.on_order_update(&unfilled_ioc_cancel_update("btc-1", "BTC-USDT", 100));
        let outcome =
            out_of_order.on_order_update(&unfilled_ioc_cancel_update("btc-2", "BTC-USDT", 50));

        assert!(out_of_order.is_killed());
        assert_eq!(outcome.events[0].ts_ms, 50);
    }

    #[test]
    fn pre_trade_enforces_global_and_per_symbol_live_order_counts() {
        let mut per_symbol = RiskGate::new(RiskLimits {
            max_live_order_count: 10,
            max_live_order_count_per_symbol: 1,
            require_feed_health: false,
            require_private_health: false,
            ..RiskLimits::default()
        });
        per_symbol.on_order_update(&live_order_update("btc-1", "BTC-USDT", 1));
        assert!(matches!(
            per_symbol.pre_trade(2, order()),
            RiskDecision::Rejected {
                reason: RiskRejectReason::SymbolLiveOrderCount {
                    value: 2,
                    limit: 1,
                    ..
                },
                ..
            }
        ));

        let mut global = RiskGate::new(RiskLimits {
            max_live_order_count: 1,
            max_live_order_count_per_symbol: 1,
            require_feed_health: false,
            require_private_health: false,
            ..RiskLimits::default()
        });
        global.on_order_update(&live_order_update("btc-1", "BTC-USDT", 1));
        let mut eth_intent = order();
        let OrderIntent::NewOrder(eth_order) = &mut eth_intent else {
            unreachable!()
        };
        eth_order.symbol = "ETH-USDT".to_string();
        assert!(matches!(
            global.pre_trade(2, eth_intent),
            RiskDecision::Rejected {
                reason: RiskRejectReason::LiveOrderCount { value: 2, limit: 1 },
                ..
            }
        ));
    }

    #[test]
    fn stablecoin_guard_configuration_is_validated() {
        let duplicate = RiskLimits {
            stablecoin_guards: vec![
                StablecoinGuardConfig {
                    symbol: "USDT-USD".to_string(),
                    max_downside_deviation: 0.01,
                },
                StablecoinGuardConfig {
                    symbol: "USDT-USD".to_string(),
                    max_downside_deviation: 0.02,
                },
            ],
            ..RiskLimits::default()
        };
        assert!(
            duplicate
                .validation_error()
                .is_some_and(|error| error.contains("duplicate stablecoin guard"))
        );

        let invalid_threshold = RiskLimits {
            stablecoin_guards: vec![StablecoinGuardConfig {
                symbol: "USDC-USD".to_string(),
                max_downside_deviation: 1.0,
            }],
            ..RiskLimits::default()
        };
        assert!(invalid_threshold.validation_error().is_some());

        for limit in [0, 6] {
            let invalid_indicator = RiskLimits {
                forced_repayment_indicator_limit: limit,
                ..RiskLimits::default()
            };
            assert!(
                invalid_indicator
                    .validation_error()
                    .is_some_and(|error| error.contains("between 1 and 5"))
            );
        }
    }

    #[test]
    fn forced_repayment_indicator_fails_closed_before_account_state_application() {
        let mut gate = ready_gate();
        let outcome = gate.on_account_update(&AccountUpdate {
            ts_ms: 101,
            balances: vec![reap_core::Balance {
                account_id: Some("main".to_string()),
                currency: "USDT".to_string(),
                total: 100.0,
                available: 90.0,
                equity: 100.0,
                liability: 10.0,
                max_loan: 100.0,
                forced_repayment_indicator: Some(1),
            }],
            positions: vec![reap_core::Position {
                symbol: "BTC-USDT".to_string(),
                qty: 2.0,
                avg_price: 100.0,
                margin_mode: None,
            }],
            margins: Vec::new(),
        });

        assert!(gate.is_killed());
        assert_eq!(gate.position("BTC-USDT"), 0.0);
        assert!(outcome.events.iter().any(|event| {
            event.kind == SystemEventKind::RiskBreach
                && event
                    .reason
                    .contains("forced repayment indicator 1 reached limit 1")
        }));
    }

    #[test]
    fn stablecoin_guard_blocks_until_a_fresh_safe_reference_arrives() {
        let mut gate = RiskGate::new(stablecoin_limits());
        assert!(matches!(
            gate.pre_trade(100, order()),
            RiskDecision::Rejected {
                reason: RiskRejectReason::StablecoinReferenceNotReady { .. },
                ..
            }
        ));

        gate.on_market(&stablecoin_index(100, 1.0));
        assert!(matches!(
            gate.pre_trade(100, order()),
            RiskDecision::Allowed(_)
        ));
        gate.on_market(&stablecoin_index(101, 1.02));
        assert!(matches!(
            gate.pre_trade(101, order()),
            RiskDecision::Allowed(_)
        ));
        assert_eq!(
            gate.stablecoin_guard_health(101),
            vec![StablecoinGuardHealth {
                symbol: "USDT-USD".to_string(),
                healthy: true,
                reason: "stablecoin reference is fresh and within limit".to_string(),
            }]
        );
    }

    #[test]
    fn stale_or_downside_depegged_stablecoin_blocks_new_orders() {
        let mut limits = stablecoin_limits();
        limits.stablecoin_max_age_ms = 1_000;
        let mut gate = RiskGate::new(limits);
        gate.on_market(&stablecoin_index(100, 1.0));
        assert!(matches!(
            gate.pre_trade(1_101, order()),
            RiskDecision::Rejected {
                reason: RiskRejectReason::StablecoinReferenceStale { .. },
                ..
            }
        ));

        gate.on_market(&stablecoin_index(1_101, 0.98));
        assert!(matches!(
            gate.pre_trade(1_101, order()),
            RiskDecision::Rejected {
                reason: RiskRejectReason::StablecoinDepeg { .. },
                ..
            }
        ));
    }

    #[test]
    fn stablecoin_replicas_fail_closed_on_same_timestamp_conflicts() {
        let mut gate = RiskGate::new(stablecoin_limits());
        gate.on_market(&stablecoin_index(100, 1.0));
        gate.on_market(&stablecoin_index(99, 0.98));
        assert!(matches!(
            gate.pre_trade(100, order()),
            RiskDecision::Allowed(_)
        ));

        gate.on_market(&stablecoin_index(100, 0.999));
        assert!(matches!(
            gate.pre_trade(100, order()),
            RiskDecision::Rejected {
                reason: RiskRejectReason::StablecoinReferenceConflict { ts_ms: 100, .. },
                ..
            }
        ));
        gate.on_market(&stablecoin_index(101, 1.0));
        assert!(matches!(
            gate.pre_trade(101, order()),
            RiskDecision::Allowed(_)
        ));
    }

    #[test]
    fn transient_stablecoin_breach_recovers_before_durable_latch() {
        let mut gate = RiskGate::new(stablecoin_limits());
        assert!(
            gate.on_normalized_event(&NormalizedEvent::Market(stablecoin_index(10, 0.98)))
                .events
                .is_empty()
        );
        assert!(
            gate.on_normalized_event(&NormalizedEvent::Market(stablecoin_index(4_000, 1.0)))
                .events
                .is_empty()
        );
        assert!(
            gate.on_normalized_event(&NormalizedEvent::Market(stablecoin_index(8_000, 0.98)))
                .events
                .is_empty()
        );
        assert!(
            gate.on_normalized_event(&NormalizedEvent::Timer(TimerEvent {
                ts_ms: 10_000,
                name: "risk".to_string(),
            }))
            .events
            .is_empty()
        );
        assert!(!gate.is_killed());
    }

    #[test]
    fn sustained_stablecoin_breach_emits_one_durable_breach_pair() {
        let mut gate = RiskGate::new(stablecoin_limits());
        assert!(
            gate.on_normalized_event(&NormalizedEvent::Market(stablecoin_index(10, 0.98)))
                .events
                .is_empty()
        );
        assert!(
            gate.on_normalized_event(&NormalizedEvent::Market(stablecoin_index(4_000, 0.98)))
                .events
                .is_empty()
        );
        let outcome =
            gate.on_normalized_event(&NormalizedEvent::Market(stablecoin_index(5_010, 0.98)));
        assert!(gate.is_killed());
        assert_eq!(outcome.events.len(), 2);
        assert_eq!(outcome.events[0].kind, SystemEventKind::RiskBreach);
        assert_eq!(outcome.events[0].symbol.as_deref(), Some("USDT-USD"));
        assert_eq!(outcome.events[1].kind, SystemEventKind::KillSwitchActivated);
        assert!(
            gate.on_normalized_event(&NormalizedEvent::Timer(TimerEvent {
                ts_ms: 6_000,
                name: "risk".to_string(),
            }))
            .events
            .is_empty()
        );
    }

    #[test]
    fn missing_stablecoin_reference_latches_after_debounce() {
        let mut gate = RiskGate::new(stablecoin_limits());
        assert!(
            gate.on_normalized_event(&NormalizedEvent::Timer(TimerEvent {
                ts_ms: 100,
                name: "risk".to_string(),
            }))
            .events
            .is_empty()
        );
        let outcome = gate.on_normalized_event(&NormalizedEvent::Timer(TimerEvent {
            ts_ms: 5_100,
            name: "risk".to_string(),
        }));

        assert!(gate.is_killed());
        assert_eq!(outcome.events.len(), 2);
        assert!(outcome.events[0].reason.contains("is not ready"));
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
            account_id: None,
            symbol: Some("BTC-USDT".to_string()),
            reason: "book".to_string(),
        });
        gate.apply_system_event(&SystemEvent {
            ts_ms: 2_102,
            kind: SystemEventKind::PrivateStreamHeartbeat,
            venue: Some(Venue::Okx),
            account_id: None,
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
            time_in_force: Some(TimeInForce::PostOnly),
            qty: 1.0,
            open_qty: 0.0,
            filled_qty: 1.0,
            avg_fill_price: 100.0,
            last_fill_qty: 1.0,
            last_fill_price: 100.0,
            last_fill_liquidity: Some(FillLiquidity::Taker),
            last_fill_fee: None,
            reason: "test".to_string(),
        });

        assert!(gate.is_killed());
        assert_eq!(outcome.events[0].kind, SystemEventKind::RiskBreach);
        assert!(
            gate.evaluate_post_trade(11, Some("BTC-USDT"))
                .events
                .is_empty()
        );
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
            time_in_force: Some(TimeInForce::PostOnly),
            qty: 1.0,
            open_qty: 1.0,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            last_fill_fee: None,
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
            time_in_force: Some(TimeInForce::PostOnly),
            qty: 1.0,
            open_qty: 1.0,
            filled_qty: 0.0,
            avg_fill_price: 0.0,
            last_fill_qty: 0.0,
            last_fill_price: 0.0,
            last_fill_liquidity: None,
            last_fill_fee: None,
            reason: "test".to_string(),
        });

        assert!(gate.is_killed());
        assert_eq!(outcome.events[0].kind, SystemEventKind::RiskBreach);
    }

    #[test]
    fn post_trade_live_order_count_breach_fails_closed() {
        let limits = RiskLimits {
            max_live_order_count: 10,
            max_live_order_count_per_symbol: 1,
            ..RiskLimits::default()
        };
        let mut gate = RiskGate::new(limits);
        assert!(
            gate.on_order_update(&live_order_update("live-1", "BTC-USDT", 1))
                .events
                .is_empty()
        );

        let outcome = gate.on_order_update(&live_order_update("live-2", "BTC-USDT", 2));

        assert!(gate.is_killed());
        assert_eq!(outcome.events[0].kind, SystemEventKind::RiskBreach);
        assert!(
            outcome.events[0]
                .reason
                .contains("symbol BTC-USDT live order count 2 exceeds 1")
        );
    }

    #[test]
    fn private_health_is_enforced_for_every_account() {
        let limits = RiskLimits {
            require_feed_health: false,
            ..RiskLimits::default()
        };
        let mut gate = RiskGate::new(limits);
        gate.mark_private_account_ready(Venue::Okx, Some("maker".to_string()), 2_101);
        gate.mark_private_account_ready(Venue::Okx, Some("hedge".to_string()), 100);

        let events = gate.check_staleness(2_101);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].account_id.as_deref(), Some("hedge"));
        assert!(matches!(
            gate.pre_trade(2_101, order()),
            RiskDecision::Rejected {
                reason: RiskRejectReason::PrivateStreamStale,
                ..
            }
        ));
    }

    #[test]
    fn derivative_contract_models_drive_all_notional_checks() {
        let limits = RiskLimits {
            max_order_notional_usd: 2_500.0,
            max_abs_position_notional_usd: 2_500.0,
            max_live_order_notional_usd: 2_500.0,
            max_turnover_usd: 2_500.0,
            require_feed_health: false,
            require_private_health: false,
            ..RiskLimits::default()
        };
        let mut inverse_gate = RiskGate::new(limits.clone());
        assert!(inverse_gate.set_instrument_model(
            "BTC-USD-SWAP",
            InstrumentRiskModel::InverseDerivative {
                contract_value: 100.0,
            },
        ));
        let inverse_order = OrderIntent::NewOrder(NewOrder {
            symbol: "BTC-USD-SWAP".to_string(),
            side: Side::Buy,
            qty: 20.0,
            price: 50_000.0,
            time_in_force: TimeInForce::Ioc,
            reduce_only: false,
            self_trade_prevention: None,
            reason: "hedge".to_string(),
        });
        assert!(matches!(
            inverse_gate.pre_trade(1, inverse_order),
            RiskDecision::Allowed(_)
        ));

        let outcome = inverse_gate.on_order_update(&OrderUpdate {
            ts_ms: 2,
            order_id: "inverse-fill".to_string(),
            symbol: "BTC-USD-SWAP".to_string(),
            side: Side::Buy,
            event: OrderEvent::FullyFilled,
            status: OrderStatus::Filled,
            price: 50_000.0,
            time_in_force: Some(TimeInForce::Ioc),
            qty: 20.0,
            open_qty: 0.0,
            filled_qty: 20.0,
            avg_fill_price: 50_000.0,
            last_fill_qty: 20.0,
            last_fill_price: 50_000.0,
            last_fill_liquidity: Some(FillLiquidity::Taker),
            last_fill_fee: None,
            reason: "hedge".to_string(),
        });
        assert!(outcome.events.is_empty());
        assert!(!inverse_gate.is_killed());
        assert_eq!(inverse_gate.turnover_usd(), 2_000.0);

        let mut linear_gate = RiskGate::new(limits);
        assert!(linear_gate.set_instrument_model(
            "BTC-USDT-SWAP",
            InstrumentRiskModel::LinearDerivative {
                contract_value: 0.01,
            },
        ));
        let linear_order = OrderIntent::NewOrder(NewOrder {
            symbol: "BTC-USDT-SWAP".to_string(),
            side: Side::Sell,
            qty: 4.0,
            price: 50_000.0,
            time_in_force: TimeInForce::Ioc,
            reduce_only: false,
            self_trade_prevention: None,
            reason: "hedge".to_string(),
        });
        assert!(matches!(
            linear_gate.pre_trade(1, linear_order),
            RiskDecision::Allowed(_)
        ));

        assert!(!linear_gate.set_instrument_model(
            "BROKEN",
            InstrumentRiskModel::LinearDerivative {
                contract_value: 0.0,
            },
        ));
    }

    #[test]
    fn bootstrap_position_snapshot_is_authoritative_for_pre_trade_risk() {
        let mut gate = RiskGate::new(RiskLimits {
            max_abs_position_notional_usd: 150.0,
            require_feed_health: false,
            require_private_health: false,
            ..RiskLimits::default()
        });
        gate.on_account_update(&reap_core::AccountUpdate {
            ts_ms: 1,
            balances: Vec::new(),
            positions: vec![reap_core::Position {
                symbol: "BTC-USDT".to_string(),
                qty: 1.0,
                avg_price: 100.0,
                margin_mode: None,
            }],
            margins: Vec::new(),
        });

        assert_eq!(gate.position("BTC-USDT"), 1.0);
        assert!(matches!(
            gate.pre_trade(
                2,
                OrderIntent::NewOrder(reap_core::NewOrder {
                    symbol: "BTC-USDT".to_string(),
                    side: reap_core::Side::Buy,
                    qty: 1.0,
                    price: 100.0,
                    time_in_force: reap_core::TimeInForce::PostOnly,
                    reduce_only: false,
                    self_trade_prevention: None,
                    reason: "test".to_string(),
                })
            ),
            RiskDecision::Rejected {
                reason: RiskRejectReason::PositionNotional { .. },
                ..
            }
        ));
    }
}
