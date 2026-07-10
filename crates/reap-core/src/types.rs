use serde::{Deserialize, Serialize};

pub type Price = f64;
pub type Quantity = f64;
pub type Symbol = String;
pub type TimeMs = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Venue {
    Okx,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Books,
    Trades,
    Orders,
    Fills,
    Account,
    Positions,
    Custom(String),
}

impl Channel {
    pub fn is_private(&self) -> bool {
        matches!(
            self,
            Self::Orders | Self::Fills | Self::Account | Self::Positions
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConnId(pub String);

impl ConnId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl std::fmt::Display for ConnId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedPriority {
    Critical,
    High,
    Normal,
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Subscription {
    pub venue: Venue,
    pub channel: Channel,
    pub symbol: Option<Symbol>,
    pub priority: FeedPriority,
    #[serde(default = "one_connection")]
    pub connections: usize,
}

fn one_connection() -> usize {
    1
}

impl Subscription {
    pub fn public(
        venue: Venue,
        channel: Channel,
        symbol: impl Into<Symbol>,
        priority: FeedPriority,
    ) -> Self {
        debug_assert!(!channel.is_private());
        Self {
            venue,
            channel,
            symbol: Some(symbol.into()),
            priority,
            connections: 1,
        }
    }

    pub fn private(venue: Venue, channel: Channel, priority: FeedPriority) -> Self {
        debug_assert!(channel.is_private());
        Self {
            venue,
            channel,
            symbol: None,
            priority,
            connections: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawEnvelope {
    pub venue: Venue,
    pub conn_id: ConnId,
    pub channel: Channel,
    pub symbol: Option<Symbol>,
    pub recv_ts_ns: u64,
    pub raw_hash: u64,
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventId {
    pub venue: Venue,
    pub channel: Channel,
    pub symbol: Option<Symbol>,
    pub key: EventKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKey {
    BookSequence {
        action: BookAction,
        seq_id: i64,
    },
    Trade(String),
    OrderVersion {
        order_id: String,
        update_time_ms: TimeMs,
        state: String,
        cumulative_fill_bits: u64,
    },
    Fill(String),
    Timestamp(TimeMs),
    RawHash(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn factor(self) -> f64 {
        match self {
            Self::Buy => 1.0,
            Self::Sell => -1.0,
        }
    }

    pub fn reverse(self) -> Self {
        match self {
            Self::Buy => Self::Sell,
            Self::Sell => Self::Buy,
        }
    }

    pub fn crosses(self, order_px: Price, resting_px: Price) -> bool {
        match self {
            Self::Buy => order_px >= resting_px,
            Self::Sell => order_px <= resting_px,
        }
    }

    pub fn is_more_passive(self, candidate: Price, reference: Price) -> bool {
        match self {
            Self::Buy => candidate < reference,
            Self::Sell => candidate > reference,
        }
    }

    pub fn passive_price(self, target: Price, opposite_px: Option<Price>, tick_size: f64) -> Price {
        match (self, opposite_px) {
            (Self::Buy, Some(ask)) => target.min(ask - tick_size),
            (Self::Sell, Some(bid)) => target.max(bid + tick_size),
            _ => target,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentKind {
    Spot,
    Future,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeInForce {
    Gtc,
    Ioc,
    PostOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelfTradePrevention {
    CancelMaker,
    CancelTaker,
    CancelBoth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    PendingNew,
    Live,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderEvent {
    PendingNew,
    New,
    PartialFill,
    FullyFilled,
    Cancelled,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillLiquidity {
    Maker,
    Taker,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Level {
    pub px: Price,
    pub qty: Quantity,
}

impl Level {
    pub fn new(px: Price, qty: Quantity) -> Self {
        Self { px, qty }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderBook {
    pub symbol: Symbol,
    pub ts_ms: TimeMs,
    pub bids: Vec<Level>,
    pub asks: Vec<Level>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookAction {
    Snapshot,
    Update,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SequencedBookUpdate {
    pub action: BookAction,
    pub symbol: Symbol,
    pub ts_ms: TimeMs,
    pub prev_seq_id: i64,
    pub seq_id: i64,
    pub bids: Vec<Level>,
    pub asks: Vec<Level>,
}

impl SequencedBookUpdate {
    pub fn as_book(&self) -> OrderBook {
        OrderBook {
            symbol: self.symbol.clone(),
            ts_ms: self.ts_ms,
            bids: self.bids.clone(),
            asks: self.asks.clone(),
        }
    }
}

impl OrderBook {
    pub fn one_level(symbol: impl Into<Symbol>, ts_ms: TimeMs, bid: Level, ask: Level) -> Self {
        Self {
            symbol: symbol.into(),
            ts_ms,
            bids: vec![bid],
            asks: vec![ask],
        }
    }

    pub fn levels(&self, side: Side) -> &[Level] {
        match side {
            Side::Buy => &self.bids,
            Side::Sell => &self.asks,
        }
    }

    pub fn levels_mut(&mut self, side: Side) -> &mut Vec<Level> {
        match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        }
    }

    pub fn px_at(&self, side: Side, level: usize) -> Option<Price> {
        self.levels(side).get(level).map(|l| l.px)
    }

    pub fn qty_at(&self, side: Side, level: usize) -> Option<Quantity> {
        self.levels(side).get(level).map(|l| l.qty)
    }

    pub fn best_bid(&self) -> Option<Level> {
        self.bids.first().copied()
    }

    pub fn best_ask(&self) -> Option<Level> {
        self.asks.first().copied()
    }

    pub fn mid(&self) -> Option<Price> {
        Some((self.best_bid()?.px + self.best_ask()?.px) * 0.5)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewOrder {
    pub symbol: Symbol,
    pub side: Side,
    pub qty: Quantity,
    pub price: Price,
    pub time_in_force: TimeInForce,
    #[serde(default)]
    pub reduce_only: bool,
    #[serde(default)]
    pub self_trade_prevention: Option<SelfTradePrevention>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OrderIntent {
    NewOrder(NewOrder),
    CancelOrder { order_id: String, reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderUpdate {
    pub ts_ms: TimeMs,
    pub order_id: String,
    pub symbol: Symbol,
    pub side: Side,
    pub event: OrderEvent,
    pub status: OrderStatus,
    pub price: Price,
    pub qty: Quantity,
    pub open_qty: Quantity,
    pub filled_qty: Quantity,
    pub avg_fill_price: Price,
    pub last_fill_qty: Quantity,
    pub last_fill_price: Price,
    pub last_fill_liquidity: Option<FillLiquidity>,
    pub reason: String,
}

impl OrderUpdate {
    pub fn has_fill(&self) -> bool {
        self.last_fill_qty > 0.0
            && matches!(
                self.event,
                OrderEvent::PartialFill | OrderEvent::FullyFilled
            )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MarketEvent {
    Depth(OrderBook),
    Trade {
        ts_ms: TimeMs,
        symbol: Symbol,
        price: Price,
        qty: Quantity,
        taker_side: Side,
    },
    IndexPrice {
        ts_ms: TimeMs,
        symbol: Symbol,
        price: Price,
    },
    FundingRate {
        ts_ms: TimeMs,
        symbol: Symbol,
        rate: f64,
        funding_time_ms: TimeMs,
    },
    BurstSignal {
        ts_ms: TimeMs,
        symbol: Symbol,
        value: f64,
    },
    PriceLimits {
        ts_ms: TimeMs,
        symbol: Symbol,
        mark_price: Price,
        limit_down: Price,
        limit_up: Price,
    },
}

impl MarketEvent {
    pub fn ts_ms(&self) -> TimeMs {
        match self {
            Self::Depth(book) => book.ts_ms,
            Self::Trade { ts_ms, .. }
            | Self::IndexPrice { ts_ms, .. }
            | Self::FundingRate { ts_ms, .. }
            | Self::BurstSignal { ts_ms, .. }
            | Self::PriceLimits { ts_ms, .. } => *ts_ms,
        }
    }

    pub fn symbol(&self) -> &str {
        match self {
            Self::Depth(book) => &book.symbol,
            Self::Trade { symbol, .. }
            | Self::IndexPrice { symbol, .. }
            | Self::FundingRate { symbol, .. }
            | Self::BurstSignal { symbol, .. }
            | Self::PriceLimits { symbol, .. } => symbol,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimerEvent {
    pub ts_ms: TimeMs,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlEvent {
    pub ts_ms: TimeMs,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Balance {
    #[serde(default)]
    pub account_id: Option<String>,
    pub currency: String,
    pub total: Quantity,
    pub available: Quantity,
    #[serde(default)]
    pub equity: Quantity,
    #[serde(default)]
    pub liability: Quantity,
    #[serde(default)]
    pub max_loan: Quantity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub symbol: Symbol,
    pub qty: Quantity,
    pub avg_price: Price,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarginSnapshot {
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub ratio: Option<f64>,
    #[serde(default)]
    pub exchange_ratio: Option<f64>,
    #[serde(default)]
    pub adjusted_equity_usd: Option<f64>,
    #[serde(default)]
    pub notional_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountUpdate {
    pub ts_ms: TimeMs,
    pub balances: Vec<Balance>,
    pub positions: Vec<Position>,
    #[serde(default)]
    pub margins: Vec<MarginSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemEventKind {
    FeedStale,
    FeedGap,
    FeedHeartbeat,
    FeedRecovered,
    BookRecoveryStarted,
    BookRecoveryFailed,
    PrivateStreamStale,
    PrivateStreamHeartbeat,
    PrivateStreamRecovered,
    ReconcileDrift,
    RiskBreach,
    KillSwitchActivated,
    KillSwitchReset,
    SymbolHalted,
    SymbolResumed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemEvent {
    pub ts_ms: TimeMs,
    pub kind: SystemEventKind,
    pub venue: Option<Venue>,
    #[serde(default)]
    pub account_id: Option<String>,
    pub symbol: Option<Symbol>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StrategyEvent {
    Market(MarketEvent),
    Order(OrderUpdate),
    Account(AccountUpdate),
    Timer(TimerEvent),
    Control(ControlEvent),
    System(SystemEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NormalizedEvent {
    Market(MarketEvent),
    Order(OrderUpdate),
    Account(AccountUpdate),
    Timer(TimerEvent),
    Control(ControlEvent),
    System(SystemEvent),
}

impl NormalizedEvent {
    pub fn market(event: MarketEvent) -> Self {
        Self::Market(event)
    }

    pub fn order(update: OrderUpdate) -> Self {
        Self::Order(update)
    }

    pub fn ts_ms(&self) -> TimeMs {
        match self {
            Self::Market(event) => event.ts_ms(),
            Self::Order(update) => update.ts_ms,
            Self::Account(update) => update.ts_ms,
            Self::Timer(event) => event.ts_ms,
            Self::Control(event) => event.ts_ms,
            Self::System(event) => event.ts_ms,
        }
    }

    pub fn into_strategy_event(self) -> StrategyEvent {
        match self {
            Self::Market(event) => StrategyEvent::Market(event),
            Self::Order(update) => StrategyEvent::Order(update),
            Self::Account(update) => StrategyEvent::Account(update),
            Self::Timer(event) => StrategyEvent::Timer(event),
            Self::Control(event) => StrategyEvent::Control(event),
            Self::System(event) => StrategyEvent::System(event),
        }
    }
}

impl From<MarketEvent> for NormalizedEvent {
    fn from(event: MarketEvent) -> Self {
        Self::Market(event)
    }
}

impl From<OrderUpdate> for NormalizedEvent {
    fn from(update: OrderUpdate) -> Self {
        Self::Order(update)
    }
}

impl From<NormalizedEvent> for StrategyEvent {
    fn from(event: NormalizedEvent) -> Self {
        event.into_strategy_event()
    }
}

pub fn round_to_tick(px: Price, tick_size: f64) -> Price {
    if tick_size <= 0.0 || !px.is_finite() {
        return px;
    }
    (px / tick_size).round() * tick_size
}

pub fn round_down_to_lot(qty: Quantity, lot_size: f64) -> Quantity {
    if lot_size <= 0.0 || !qty.is_finite() {
        return qty.max(0.0);
    }
    ((qty / lot_size).floor() * lot_size).max(0.0)
}
