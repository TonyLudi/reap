use std::collections::BTreeSet;

use reap_pm_core::{
    PmBookLevel, PmBookQuantity, PmBookSide, PmMarketId, PmPrice, PmQuantity, PmTick, PmTokenId,
};
use serde_json::Value;

use crate::exact::ExactText;
use crate::hash::{required, verify_raw_snapshot_hash};
use crate::limits::{MAX_BOOK_LEVELS, MAX_PUBLIC_WS_FRAME_BYTES, MAX_WS_EVENTS_PER_FRAME};
use crate::raw::{
    RawBestBidAskEvent, RawBook, RawBookLevel, RawPriceChangeEvent, RawTickSizeChangeEvent,
};
use crate::rest::{parse_market, parse_token};
use crate::{PmBookParserConfig, PmWireError, SnapshotHash};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmExactBookLevel {
    level: PmBookLevel,
    raw_price: ExactText,
    raw_size: ExactText,
}

impl PmExactBookLevel {
    #[must_use]
    pub const fn level(self) -> PmBookLevel {
        self.level
    }

    #[must_use]
    pub fn raw_price(&self) -> &str {
        self.raw_price.as_str()
    }

    #[must_use]
    pub fn raw_size(&self) -> &str {
        self.raw_size.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmBookSnapshot {
    market: PmMarketId,
    token: PmTokenId,
    timestamp: ExactText,
    timestamp_millis: u64,
    bids: Vec<PmExactBookLevel>,
    asks: Vec<PmExactBookLevel>,
    minimum_order_size: PmQuantity,
    tick: PmTick,
    negative_risk: bool,
    verified_hash: SnapshotHash,
}

impl PmBookSnapshot {
    #[must_use]
    pub const fn market(&self) -> PmMarketId {
        self.market
    }

    #[must_use]
    pub const fn token(&self) -> PmTokenId {
        self.token
    }

    #[must_use]
    pub fn raw_timestamp(&self) -> &str {
        self.timestamp.as_str()
    }

    #[must_use]
    pub const fn timestamp_millis(&self) -> u64 {
        self.timestamp_millis
    }

    #[must_use]
    pub fn bids(&self) -> &[PmExactBookLevel] {
        &self.bids
    }

    #[must_use]
    pub fn asks(&self) -> &[PmExactBookLevel] {
        &self.asks
    }

    #[must_use]
    pub const fn minimum_order_size(&self) -> PmQuantity {
        self.minimum_order_size
    }

    #[must_use]
    pub const fn tick(&self) -> PmTick {
        self.tick
    }

    #[must_use]
    pub const fn negative_risk(&self) -> bool {
        self.negative_risk
    }

    #[must_use]
    pub const fn verified_hash(&self) -> SnapshotHash {
        self.verified_hash
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmBestPrices {
    bid: PmPrice,
    ask: PmPrice,
    raw_bid: ExactText,
    raw_ask: ExactText,
}

impl PmBestPrices {
    #[must_use]
    pub const fn bid(self) -> PmPrice {
        self.bid
    }

    #[must_use]
    pub const fn ask(self) -> PmPrice {
        self.ask
    }

    #[must_use]
    pub fn raw_bid(&self) -> &str {
        self.raw_bid.as_str()
    }

    #[must_use]
    pub fn raw_ask(&self) -> &str {
        self.raw_ask.as_str()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmExactPriceChange {
    level: PmBookLevel,
    raw_price: ExactText,
    raw_size: ExactText,
    transaction_hash: Option<ExactText>,
    best_prices: Option<PmBestPrices>,
}

impl PmExactPriceChange {
    #[must_use]
    pub const fn level(self) -> PmBookLevel {
        self.level
    }

    #[must_use]
    pub fn raw_price(&self) -> &str {
        self.raw_price.as_str()
    }

    #[must_use]
    pub fn raw_size(&self) -> &str {
        self.raw_size.as_str()
    }

    #[must_use]
    pub fn transaction_hash(&self) -> Option<&str> {
        self.transaction_hash.as_ref().map(ExactText::as_str)
    }

    #[must_use]
    pub const fn best_prices(self) -> Option<PmBestPrices> {
        self.best_prices
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmPriceChangeBatch {
    market: PmMarketId,
    token: PmTokenId,
    timestamp: ExactText,
    timestamp_millis: u64,
    changes: Vec<PmExactPriceChange>,
    final_best_prices: PmBestPrices,
}

impl PmPriceChangeBatch {
    #[must_use]
    pub const fn market(&self) -> PmMarketId {
        self.market
    }

    #[must_use]
    pub const fn token(&self) -> PmTokenId {
        self.token
    }

    #[must_use]
    pub fn raw_timestamp(&self) -> &str {
        self.timestamp.as_str()
    }

    #[must_use]
    pub const fn timestamp_millis(&self) -> u64 {
        self.timestamp_millis
    }

    #[must_use]
    pub fn changes(&self) -> &[PmExactPriceChange] {
        &self.changes
    }

    #[must_use]
    pub const fn final_best_prices(&self) -> PmBestPrices {
        self.final_best_prices
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmBestBidAsk {
    market: PmMarketId,
    token: PmTokenId,
    timestamp: ExactText,
    timestamp_millis: u64,
    prices: PmBestPrices,
}

impl PmBestBidAsk {
    #[must_use]
    pub const fn market(self) -> PmMarketId {
        self.market
    }

    #[must_use]
    pub const fn token(self) -> PmTokenId {
        self.token
    }

    #[must_use]
    pub fn raw_timestamp(&self) -> &str {
        self.timestamp.as_str()
    }

    #[must_use]
    pub const fn timestamp_millis(self) -> u64 {
        self.timestamp_millis
    }

    #[must_use]
    pub const fn prices(self) -> PmBestPrices {
        self.prices
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmTickSizeChange {
    market: PmMarketId,
    token: PmTokenId,
    timestamp: ExactText,
    timestamp_millis: u64,
    old_tick: PmTick,
    new_tick: PmTick,
}

impl PmTickSizeChange {
    #[must_use]
    pub const fn market(self) -> PmMarketId {
        self.market
    }

    #[must_use]
    pub const fn token(self) -> PmTokenId {
        self.token
    }

    #[must_use]
    pub fn raw_timestamp(&self) -> &str {
        self.timestamp.as_str()
    }

    #[must_use]
    pub const fn timestamp_millis(self) -> u64 {
        self.timestamp_millis
    }

    #[must_use]
    pub const fn old_tick(self) -> PmTick {
        self.old_tick
    }

    #[must_use]
    pub const fn new_tick(self) -> PmTick {
        self.new_tick
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PmIgnoredEvent {
    PublicTrade,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PmWsEvent {
    BookSnapshot(PmBookSnapshot),
    PriceChanges(PmPriceChangeBatch),
    BestBidAsk(PmBestBidAsk),
    TickSizeChange(PmTickSizeChange),
    Ignored(PmIgnoredEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmWsFrame {
    events: Vec<PmWsEvent>,
    was_array: bool,
}

impl PmWsFrame {
    #[must_use]
    pub fn events(&self) -> &[PmWsEvent] {
        &self.events
    }

    #[must_use]
    pub const fn was_array(&self) -> bool {
        self.was_array
    }
}

pub fn parse_ws_frame(raw: &[u8], config: PmBookParserConfig) -> Result<PmWsFrame, PmWireError> {
    if raw.len() > MAX_PUBLIC_WS_FRAME_BYTES {
        return Err(PmWireError::WsFrameTooLarge);
    }
    let value = serde_json::from_slice::<Value>(raw).map_err(|_| PmWireError::MalformedJson)?;
    let (values, was_array) = match value {
        Value::Array(values) => (values, true),
        Value::Object(_) => (vec![value], false),
        _ => return Err(PmWireError::MalformedJson),
    };
    if values.is_empty() {
        return Err(PmWireError::EmptyEnvelope);
    }
    if values.len() > MAX_WS_EVENTS_PER_FRAME {
        return Err(PmWireError::TooManyEvents);
    }
    let events = values
        .into_iter()
        .map(|value| parse_ws_event(value, config))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PmWsFrame { events, was_array })
}

pub(crate) fn parse_book_bytes(
    raw: &[u8],
    config: PmBookParserConfig,
    require_ws_event_type: bool,
) -> Result<PmBookSnapshot, PmWireError> {
    let book = serde_json::from_slice::<RawBook>(raw).map_err(|_| PmWireError::MalformedJson)?;
    parse_raw_book(book, config, require_ws_event_type)
}

fn parse_ws_event(value: Value, config: PmBookParserConfig) -> Result<PmWsEvent, PmWireError> {
    let event_type = value
        .get("event_type")
        .and_then(Value::as_str)
        .ok_or(PmWireError::MissingField("event_type"))?;
    ExactText::new("event_type", event_type)?;
    match event_type {
        "book" => {
            let book =
                serde_json::from_value::<RawBook>(value).map_err(|_| PmWireError::MalformedJson)?;
            parse_raw_book(book, config, true).map(PmWsEvent::BookSnapshot)
        }
        "price_change" => {
            let change = serde_json::from_value::<RawPriceChangeEvent>(value)
                .map_err(|_| PmWireError::MalformedJson)?;
            parse_price_change(change, config).map(PmWsEvent::PriceChanges)
        }
        "best_bid_ask" => {
            let top = serde_json::from_value::<RawBestBidAskEvent>(value)
                .map_err(|_| PmWireError::MalformedJson)?;
            parse_best_bid_ask(top, config).map(PmWsEvent::BestBidAsk)
        }
        "tick_size_change" => {
            let tick = serde_json::from_value::<RawTickSizeChangeEvent>(value)
                .map_err(|_| PmWireError::MalformedJson)?;
            parse_tick_size_change(tick, config).map(PmWsEvent::TickSizeChange)
        }
        // `/ws/market` is a multiplexed stream. Goal F has no public-trade
        // capability, parser, or normalized event, so only the discriminator
        // is recognized; no other field in this object is inspected.
        "last_trade_price" => Ok(PmWsEvent::Ignored(PmIgnoredEvent::PublicTrade)),
        _ => Err(PmWireError::UnsupportedEventType),
    }
}

fn parse_raw_book(
    book: RawBook,
    config: PmBookParserConfig,
    require_ws_event_type: bool,
) -> Result<PmBookSnapshot, PmWireError> {
    if require_ws_event_type && book.event_type.as_deref() != Some("book") {
        return Err(PmWireError::UnsupportedEventType);
    }
    if book
        .bids
        .as_ref()
        .map_or(0, Vec::len)
        .saturating_add(book.asks.as_ref().map_or(0, Vec::len))
        > MAX_BOOK_LEVELS
    {
        return Err(PmWireError::TooManyBookLevels);
    }

    let market = parse_market(required(&book.market, "market")?)?;
    let token = parse_token(required(&book.asset_id, "asset_id")?)?;
    validate_scope(market, token, config)?;
    let (timestamp, timestamp_millis) = parse_timestamp(required(&book.timestamp, "timestamp")?)?;

    let tick_raw = required(&book.tick_size, "tick_size")?;
    let tick =
        PmTick::parse_decimal(tick_raw).map_err(|_| PmWireError::InvalidNumeric("tick_size"))?;
    if tick != config.tick() {
        return Err(PmWireError::MetadataTickMismatch);
    }
    let minimum_raw = required(&book.min_order_size, "min_order_size")?;
    let minimum_order_size = PmQuantity::parse_decimal(minimum_raw)
        .map_err(|_| PmWireError::InvalidNumeric("min_order_size"))?;
    minimum_order_size
        .validate_order(minimum_order_size)
        .map_err(|_| PmWireError::InvalidNumeric("min_order_size"))?;
    if minimum_order_size != config.minimum_order_size() {
        return Err(PmWireError::MetadataMinimumMismatch);
    }
    let negative_risk = book.neg_risk.ok_or(PmWireError::MissingField("neg_risk"))?;
    if negative_risk != config.negative_risk() {
        return Err(PmWireError::MetadataNegativeRiskMismatch);
    }
    validate_integrity_only_last_trade_price(required(
        &book.last_trade_price,
        "last_trade_price",
    )?)?;

    let mut bids = parse_snapshot_levels(
        book.bids
            .as_deref()
            .ok_or(PmWireError::MissingField("bids"))?,
        PmBookSide::Bid,
        config.tick(),
    )?;
    let mut asks = parse_snapshot_levels(
        book.asks
            .as_deref()
            .ok_or(PmWireError::MissingField("asks"))?,
        PmBookSide::Ask,
        config.tick(),
    )?;
    if bids.is_empty() || asks.is_empty() {
        return Err(PmWireError::EmptyBook);
    }
    bids.sort_unstable_by_key(|level| level.level().price());
    asks.sort_unstable_by_key(|level| level.level().price());
    let best_bid = bids.last().expect("nonempty bids").level().price();
    let best_ask = asks.first().expect("nonempty asks").level().price();
    if best_bid >= best_ask {
        return Err(PmWireError::CrossedBook);
    }

    let verified_hash = verify_raw_snapshot_hash(&book)?;
    Ok(PmBookSnapshot {
        market,
        token,
        timestamp,
        timestamp_millis,
        bids,
        asks,
        minimum_order_size,
        tick,
        negative_risk,
        verified_hash,
    })
}

fn parse_snapshot_levels(
    raw_levels: &[RawBookLevel],
    side: PmBookSide,
    tick: PmTick,
) -> Result<Vec<PmExactBookLevel>, PmWireError> {
    let mut seen = BTreeSet::new();
    raw_levels
        .iter()
        .map(|raw| {
            let raw_price_value = required(&raw.price, "price")?;
            let raw_size_value = required(&raw.size, "size")?;
            let price = parse_price(raw_price_value, tick)?;
            if !seen.insert(price) {
                return Err(PmWireError::DuplicateLevel);
            }
            let quantity = PmQuantity::parse_decimal(raw_size_value)
                .map_err(|_| PmWireError::InvalidNumeric("size"))?;
            let level = PmBookLevel::new(side, price, PmBookQuantity::Quantity(quantity));
            Ok(PmExactBookLevel {
                level,
                raw_price: ExactText::new("price", raw_price_value)?,
                raw_size: ExactText::new("size", raw_size_value)?,
            })
        })
        .collect()
}

fn parse_price_change(
    event: RawPriceChangeEvent,
    config: PmBookParserConfig,
) -> Result<PmPriceChangeBatch, PmWireError> {
    let market = parse_market(required(&event.market, "market")?)?;
    if market != config.scope().market() {
        return Err(PmWireError::MarketMismatch);
    }
    let (timestamp, timestamp_millis) = parse_timestamp(required(&event.timestamp, "timestamp")?)?;
    let raw_changes = event
        .price_changes
        .ok_or(PmWireError::MissingField("price_changes"))?;
    if raw_changes.is_empty() {
        return Err(PmWireError::EmptyPriceChanges);
    }
    if raw_changes.len() > MAX_BOOK_LEVELS {
        return Err(PmWireError::TooManyBookLevels);
    }

    let mut seen_bids = BTreeSet::new();
    let mut seen_asks = BTreeSet::new();
    let mut changes = Vec::with_capacity(raw_changes.len());
    for raw in raw_changes {
        let token = parse_token(required(&raw.asset_id, "asset_id")?)?;
        if token != config.scope().token() {
            return Err(PmWireError::TokenMismatch);
        }
        let raw_price_value = required(&raw.price, "price")?;
        let raw_size_value = required(&raw.size, "size")?;
        let price = parse_price(raw_price_value, config.tick())?;
        let quantity = PmBookQuantity::parse_decimal(raw_size_value)
            .map_err(|_| PmWireError::InvalidNumeric("size"))?;
        let side = match required(&raw.side, "side")? {
            "BUY" => PmBookSide::Bid,
            "SELL" => PmBookSide::Ask,
            _ => return Err(PmWireError::InvalidSide),
        };
        let seen = match side {
            PmBookSide::Bid => &mut seen_bids,
            PmBookSide::Ask => &mut seen_asks,
        };
        if !seen.insert(price) {
            return Err(PmWireError::DuplicateLevel);
        }
        let transaction_hash = raw
            .hash
            .as_deref()
            .map(|hash| ExactText::new("hash", hash))
            .transpose()?;
        let best_prices =
            parse_optional_best_prices(raw.best_bid.as_deref(), raw.best_ask.as_deref(), config)?;
        changes.push(PmExactPriceChange {
            level: PmBookLevel::new(side, price, quantity),
            raw_price: ExactText::new("price", raw_price_value)?,
            raw_size: ExactText::new("size", raw_size_value)?,
            transaction_hash,
            best_prices,
        });
    }
    let final_best_prices = changes
        .last()
        .and_then(|change| change.best_prices)
        .ok_or(PmWireError::MissingBestPrices)?;
    Ok(PmPriceChangeBatch {
        market,
        token: config.scope().token(),
        timestamp,
        timestamp_millis,
        changes,
        final_best_prices,
    })
}

fn parse_best_bid_ask(
    event: RawBestBidAskEvent,
    config: PmBookParserConfig,
) -> Result<PmBestBidAsk, PmWireError> {
    let (market, token) = validate_raw_scope(&event.market, &event.asset_id, config)?;
    let (timestamp, timestamp_millis) = parse_timestamp(required(&event.timestamp, "timestamp")?)?;
    let prices =
        parse_required_best_prices(event.best_bid.as_deref(), event.best_ask.as_deref(), config)?;
    validate_optional_best_sizes(event.bid_size.as_deref(), event.ask_size.as_deref())?;
    Ok(PmBestBidAsk {
        market,
        token,
        timestamp,
        timestamp_millis,
        prices,
    })
}

fn parse_tick_size_change(
    event: RawTickSizeChangeEvent,
    config: PmBookParserConfig,
) -> Result<PmTickSizeChange, PmWireError> {
    let (market, token) = validate_raw_scope(&event.market, &event.asset_id, config)?;
    let (timestamp, timestamp_millis) = parse_timestamp(required(&event.timestamp, "timestamp")?)?;
    let old_tick = PmTick::parse_decimal(required(&event.old_tick_size, "old_tick_size")?)
        .map_err(|_| PmWireError::InvalidNumeric("old_tick_size"))?;
    if old_tick != config.tick() {
        return Err(PmWireError::TickChangeOldMismatch);
    }
    let new_tick = PmTick::parse_decimal(required(&event.new_tick_size, "new_tick_size")?)
        .map_err(|_| PmWireError::InvalidNumeric("new_tick_size"))?;
    if new_tick == old_tick {
        return Err(PmWireError::TickSizeUnchanged);
    }
    Ok(PmTickSizeChange {
        market,
        token,
        timestamp,
        timestamp_millis,
        old_tick,
        new_tick,
    })
}

fn parse_optional_best_prices(
    bid: Option<&str>,
    ask: Option<&str>,
    config: PmBookParserConfig,
) -> Result<Option<PmBestPrices>, PmWireError> {
    match (bid, ask) {
        (None, None) => Ok(None),
        (Some(bid), Some(ask)) => parse_best_prices(bid, ask, config).map(Some),
        _ => Err(PmWireError::PartialBestPrices),
    }
}

fn parse_required_best_prices(
    bid: Option<&str>,
    ask: Option<&str>,
    config: PmBookParserConfig,
) -> Result<PmBestPrices, PmWireError> {
    match (bid, ask) {
        (Some(bid), Some(ask)) => parse_best_prices(bid, ask, config),
        (None, None) => Err(PmWireError::MissingBestPrices),
        _ => Err(PmWireError::PartialBestPrices),
    }
}

fn parse_best_prices(
    bid: &str,
    ask: &str,
    config: PmBookParserConfig,
) -> Result<PmBestPrices, PmWireError> {
    let parsed_bid = parse_price(bid, config.tick())?;
    let parsed_ask = parse_price(ask, config.tick())?;
    if parsed_bid >= parsed_ask {
        return Err(PmWireError::CrossedBestPrices);
    }
    Ok(PmBestPrices {
        bid: parsed_bid,
        ask: parsed_ask,
        raw_bid: ExactText::new("best_bid", bid)?,
        raw_ask: ExactText::new("best_ask", ask)?,
    })
}

fn parse_price(value: &str, tick: PmTick) -> Result<PmPrice, PmWireError> {
    let price = PmPrice::parse_decimal(value).map_err(|_| PmWireError::InvalidNumeric("price"))?;
    price
        .validate_tick(tick)
        .map_err(|_| PmWireError::PriceOffConfiguredTick)
}

/// Validates snapshot hash evidence without minting an executable `PmPrice`.
///
/// The venue includes `last_trade_price` in the snapshot checksum payload.
/// Terminal `0` and `1` are valid evidence even though neither can be a live
/// executable book level. Arbitrary precision is retained lexically for the
/// hash; this check only proves an exact unsigned decimal in the closed unit
/// interval.
fn validate_integrity_only_last_trade_price(value: &str) -> Result<(), PmWireError> {
    ExactText::new("last_trade_price", value)?;
    let mut components = value.split('.');
    let integer = components
        .next()
        .expect("split always yields one component");
    let fractional = components.next();
    if components.next().is_some()
        || integer.is_empty()
        || !integer.bytes().all(|byte| byte.is_ascii_digit())
        || fractional.is_some_and(|digits| {
            digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return Err(PmWireError::InvalidNumeric("last_trade_price"));
    }

    let significant_integer = integer.trim_start_matches('0');
    match significant_integer {
        "" => Ok(()),
        "1" if fractional.is_none_or(|digits| digits.bytes().all(|byte| byte == b'0')) => Ok(()),
        _ => Err(PmWireError::InvalidNumeric("last_trade_price")),
    }
}

fn validate_optional_best_sizes(
    bid_size: Option<&str>,
    ask_size: Option<&str>,
) -> Result<(), PmWireError> {
    match (bid_size, ask_size) {
        (None, None) => Ok(()),
        (Some(bid), Some(ask)) => {
            validate_positive_quantity("bid_size", bid)?;
            validate_positive_quantity("ask_size", ask)
        }
        _ => Err(PmWireError::PartialBestSizes),
    }
}

fn validate_positive_quantity(field: &'static str, value: &str) -> Result<(), PmWireError> {
    ExactText::new(field, value)?;
    PmQuantity::parse_decimal(value)
        .map(|_| ())
        .map_err(|_| PmWireError::InvalidNumeric(field))
}

fn parse_timestamp(value: &str) -> Result<(ExactText, u64), PmWireError> {
    let exact = ExactText::new("timestamp", value)?;
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(PmWireError::InvalidNumeric("timestamp"));
    }
    let timestamp = value
        .parse::<u64>()
        .ok()
        .filter(|timestamp| *timestamp > 0)
        .ok_or(PmWireError::InvalidNumeric("timestamp"))?;
    Ok((exact, timestamp))
}

fn validate_raw_scope(
    raw_market: &Option<String>,
    raw_token: &Option<String>,
    config: PmBookParserConfig,
) -> Result<(PmMarketId, PmTokenId), PmWireError> {
    let market = parse_market(required(raw_market, "market")?)?;
    let token = parse_token(required(raw_token, "asset_id")?)?;
    validate_scope(market, token, config)?;
    Ok((market, token))
}

fn validate_scope(
    market: PmMarketId,
    token: PmTokenId,
    config: PmBookParserConfig,
) -> Result<(), PmWireError> {
    if market != config.scope().market() {
        return Err(PmWireError::MarketMismatch);
    }
    if token != config.scope().token() {
        return Err(PmWireError::TokenMismatch);
    }
    Ok(())
}
