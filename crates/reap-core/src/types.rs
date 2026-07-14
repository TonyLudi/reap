use serde::{Deserialize, Serialize};

pub type Price = f64;
pub type Quantity = f64;
pub type Symbol = String;
pub type TimeMs = u64;

pub const PINNED_JAVA_REVISION: &str = "b6b120c7b7c466d8431bf082f3229328c5d7b2ae";

/// Stable delay classes shared by live evidence and deterministic backtests.
///
/// The names map to the pinned Java `BackTestDelay` classes except
/// `ReferenceData`, which groups Rust index, funding, mark, and price-limit
/// inputs that have no direct Java delay class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BacktestLatencyClass {
    MarketDepth,
    HistoricalTrade,
    ReferenceData,
    MatchingNew,
    MatchingCancel,
    OrderUpdate,
    OrderFill,
}

impl BacktestLatencyClass {
    pub const ALL: [Self; 7] = [
        Self::MarketDepth,
        Self::HistoricalTrade,
        Self::ReferenceData,
        Self::MatchingNew,
        Self::MatchingCancel,
        Self::OrderUpdate,
        Self::OrderFill,
    ];

    pub const fn stable_tag(self) -> u8 {
        match self {
            Self::MarketDepth => 1,
            Self::HistoricalTrade => 2,
            Self::ReferenceData => 3,
            Self::MatchingNew => 4,
            Self::MatchingCancel => 5,
            Self::OrderUpdate => 6,
            Self::OrderFill => 7,
        }
    }
}

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

    pub fn is_book(&self) -> bool {
        match self {
            Self::Books => true,
            Self::Custom(channel) => {
                matches!(channel.as_str(), "books-l2-tbt" | "books50-l2-tbt")
            }
            _ => false,
        }
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
        #[serde(default)]
        prev_seq_id: i64,
        seq_id: i64,
        #[serde(default)]
        ts_ms: TimeMs,
        #[serde(default)]
        raw_hash: u64,
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
    TimestampHash {
        ts_ms: TimeMs,
        raw_hash: u64,
    },
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

/// The exchange-reported balance delta caused by one fill.
///
/// Charges are negative and rebates are positive. `currency` names the balance
/// that changed; it is not assumed to be the instrument's settlement currency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FillFee {
    pub amount: f64,
    pub currency: String,
}

/// Exchange fill identity scoped to the instrument that issued it.
///
/// An empty symbol is accepted only when reading legacy journals whose
/// bootstrap records stored unscoped IDs. Such a key acts as a conservative
/// wildcard during restart deduplication; newly-created keys are always scoped.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct FillKey {
    pub symbol: Symbol,
    pub fill_id: String,
}

impl FillKey {
    pub fn new(symbol: impl Into<Symbol>, fill_id: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            fill_id: fill_id.into(),
        }
    }

    pub fn legacy_unscoped(fill_id: impl Into<String>) -> Self {
        Self::new(String::new(), fill_id)
    }

    pub fn matches(&self, symbol: &str, fill_id: &str) -> bool {
        self.fill_id == fill_id && (self.symbol.is_empty() || self.symbol == symbol)
    }
}

impl<'de> Deserialize<'de> for FillKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum FillKeyWire {
            Legacy(String),
            Scoped { symbol: Symbol, fill_id: String },
        }

        Ok(match FillKeyWire::deserialize(deserializer)? {
            FillKeyWire::Legacy(fill_id) => Self::legacy_unscoped(fill_id),
            FillKeyWire::Scoped { symbol, fill_id } => Self::new(symbol, fill_id),
        })
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_in_force: Option<TimeInForce>,
    pub qty: Quantity,
    pub open_qty: Quantity,
    pub filled_qty: Quantity,
    pub avg_fill_price: Price,
    pub last_fill_qty: Quantity,
    pub last_fill_price: Price,
    pub last_fill_liquidity: Option<FillLiquidity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fill_fee: Option<FillFee>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settlement: Option<FundingSettlement>,
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FundingSettlement {
    pub funding_time_ms: TimeMs,
    pub rate: f64,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forced_repayment_indicator: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub symbol: Symbol,
    pub qty: Quantity,
    pub avg_price: Price,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin_mode: Option<PositionMarginMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionMarginMode {
    Cross,
    Isolated,
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
    OrderTransportStale,
    OrderTransportHeartbeat,
    OrderTransportRecovered,
    ReconcileDrift,
    RiskBreach,
    KillSwitchActivated,
    KillSwitchReset,
    AccountHalted,
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

#[cfg(test)]
mod tests {
    use super::{Balance, Channel, FillKey, MarketEvent, NormalizedEvent, OrderUpdate, Position};

    #[test]
    fn okx_depth_variants_are_book_channels() {
        assert!(Channel::Books.is_book());
        assert!(Channel::Custom("books-l2-tbt".to_string()).is_book());
        assert!(Channel::Custom("books50-l2-tbt".to_string()).is_book());
        assert!(!Channel::Custom("mark-price".to_string()).is_book());
    }

    #[test]
    fn position_margin_mode_is_backward_compatible_with_existing_jsonl() {
        let position: Position =
            serde_json::from_str(r#"{"symbol":"BTC-USDT-SWAP","qty":2.0,"avg_price":50000.0}"#)
                .unwrap();

        assert_eq!(position.margin_mode, None);
        assert!(
            !serde_json::to_string(&position)
                .unwrap()
                .contains("margin_mode")
        );
    }

    #[test]
    fn forced_repayment_indicator_is_backward_compatible_with_existing_jsonl() {
        let balance: Balance = serde_json::from_str(
            r#"{"account_id":"main","currency":"USDT","total":100.0,"available":90.0,"equity":100.0,"liability":0.0,"max_loan":0.0}"#,
        )
        .unwrap();

        assert_eq!(balance.forced_repayment_indicator, None);
        assert!(
            !serde_json::to_string(&balance)
                .unwrap()
                .contains("forced_repayment_indicator")
        );
    }

    #[test]
    fn order_time_in_force_is_backward_compatible_with_existing_jsonl() {
        let update: OrderUpdate = serde_json::from_str(
            r#"{"ts_ms":1,"order_id":"order-1","symbol":"BTC-USDT","side":"buy","event":"new","status":"live","price":100.0,"qty":1.0,"open_qty":1.0,"filled_qty":0.0,"avg_fill_price":0.0,"last_fill_qty":0.0,"last_fill_price":0.0,"last_fill_liquidity":null,"reason":"quote"}"#,
        )
        .unwrap();

        assert_eq!(update.time_in_force, None);
        assert!(
            !serde_json::to_string(&update)
                .unwrap()
                .contains("time_in_force")
        );
    }

    #[test]
    fn funding_settlement_is_backward_compatible_with_existing_jsonl() {
        let event: NormalizedEvent = serde_json::from_str(
            r#"{"Market":{"FundingRate":{"ts_ms":1,"symbol":"BTC-USDT-SWAP","rate":0.0001,"funding_time_ms":2}}}"#,
        )
        .unwrap();

        assert!(matches!(
            &event,
            NormalizedEvent::Market(MarketEvent::FundingRate {
                settlement: None,
                ..
            })
        ));
        assert!(
            !serde_json::to_string(&event)
                .unwrap()
                .contains("settlement")
        );
    }

    #[test]
    fn fill_key_reads_legacy_ids_and_round_trips_scoped_identity() {
        let legacy: FillKey = serde_json::from_str(r#""fill-1""#).unwrap();
        assert!(legacy.matches("BTC-USDT", "fill-1"));
        assert!(legacy.matches("ETH-USDT", "fill-1"));

        let scoped = FillKey::new("BTC-USDT", "fill-1");
        let decoded: FillKey =
            serde_json::from_str(&serde_json::to_string(&scoped).unwrap()).unwrap();
        assert_eq!(decoded, scoped);
        assert!(decoded.matches("BTC-USDT", "fill-1"));
        assert!(!decoded.matches("ETH-USDT", "fill-1"));
    }
}
