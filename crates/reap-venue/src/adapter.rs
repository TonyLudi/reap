use reap_core::{
    AccountUpdate, EventId, FillLiquidity, NormalizedEvent, Price, Quantity, RawEnvelope,
    SequencedBookUpdate, Side, Subscription, Symbol, TimeMs, Venue,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VenueError {
    #[error("invalid {venue:?} payload: {message}")]
    InvalidPayload { venue: Venue, message: String },
    #[error("unsupported {venue:?} channel: {channel}")]
    UnsupportedChannel { venue: Venue, channel: String },
    #[error("failed to serialize venue request: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedEvent {
    pub id: EventId,
    pub event: VenueEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueEvent {
    Book(SequencedBookUpdate),
    Normalized(NormalizedEvent),
    PrivateOrder(PrivateOrderUpdate),
    PrivateFill(RemoteFill),
    Account(AccountUpdate),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivateOrderState {
    Pending,
    Live,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrivateOrderUpdate {
    pub ts_ms: TimeMs,
    pub exchange_order_id: String,
    pub client_order_id: String,
    pub symbol: Symbol,
    pub side: Side,
    pub state: PrivateOrderState,
    pub price: Price,
    pub qty: Quantity,
    pub cumulative_filled_qty: Quantity,
    pub average_fill_price: Price,
    pub last_fill_qty: Quantity,
    pub last_fill_price: Price,
    pub liquidity: Option<FillLiquidity>,
    pub fill_id: Option<String>,
    pub reject_reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteOrder {
    pub exchange_order_id: String,
    pub client_order_id: String,
    pub symbol: Symbol,
    pub side: Side,
    pub state: PrivateOrderState,
    pub price: Price,
    pub qty: Quantity,
    pub cumulative_filled_qty: Quantity,
    pub average_fill_price: Price,
    pub update_time_ms: TimeMs,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteFill {
    pub fill_id: String,
    pub exchange_order_id: String,
    pub client_order_id: String,
    pub symbol: Symbol,
    pub side: Side,
    pub price: Price,
    pub qty: Quantity,
    pub liquidity: FillLiquidity,
    pub ts_ms: TimeMs,
}

pub trait VenueAdapter: Send + Sync {
    fn venue(&self) -> Venue;
    fn websocket_url(&self, private: bool) -> &str;
    fn parse(&self, envelope: &RawEnvelope) -> Result<Vec<ParsedEvent>, VenueError>;
    fn subscription_message(&self, subscriptions: &[Subscription]) -> Result<String, VenueError>;
}
