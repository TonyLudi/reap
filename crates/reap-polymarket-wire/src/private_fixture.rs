//! Strict private-response parsing for checked-in fixtures and local fakes.
//!
//! These views preserve wire strings as evidence. They do not establish
//! structural identity, ownership, balance scope, or normalized core events.

use serde::Deserialize;
use thiserror::Error;

use crate::{MAX_PRIVATE_FIXTURE_BYTES, MAX_PRIVATE_FIXTURE_EVENTS};

macro_rules! order_field_getters {
    () => {
        #[must_use]
        pub fn id(&self) -> &str {
            &self.fields.id
        }

        #[must_use]
        pub fn market(&self) -> &str {
            &self.fields.market
        }

        #[must_use]
        pub fn asset_id(&self) -> &str {
            &self.fields.asset_id
        }

        #[must_use]
        pub fn side(&self) -> &str {
            &self.fields.side
        }

        #[must_use]
        pub fn original_size(&self) -> &str {
            &self.fields.original_size
        }

        #[must_use]
        pub fn size_matched(&self) -> &str {
            &self.fields.size_matched
        }

        #[must_use]
        pub fn price(&self) -> &str {
            &self.fields.price
        }

        #[must_use]
        pub fn status(&self) -> &str {
            &self.fields.status
        }

        #[must_use]
        pub fn maker_address(&self) -> &str {
            &self.fields.maker_address
        }
    };
}

/// A fail-closed private fixture parsing failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPrivateFixtureError {
    #[error("private Polymarket fixture exceeds its byte bound")]
    PayloadTooLarge,
    #[error("private Polymarket fixture JSON is malformed or has a wrong wire shape")]
    MalformedJson,
    #[error("private Polymarket user fixture is empty")]
    EmptyUserFrame,
    #[error("private Polymarket user fixture exceeds its event bound")]
    TooManyEvents,
    #[error("private Polymarket trade fixture exceeds its maker-order bound")]
    TooManyMakerOrders,
    #[error("required private Polymarket fixture field `{0}` is empty")]
    EmptyField(&'static str),
}

/// What the legacy scalar allowance fixture can prove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PmFixtureAllowanceScope {
    /// The response has no asset, token, or spender key.
    UnscopedLegacyScalar,
}

/// A strict view of the tracked legacy balance/allowance seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmLegacyBalanceAllowanceFixture {
    balance: String,
    unscoped_allowance: String,
}

impl PmLegacyBalanceAllowanceFixture {
    #[must_use]
    pub fn balance(&self) -> &str {
        &self.balance
    }

    #[must_use]
    pub fn unscoped_allowance(&self) -> &str {
        &self.unscoped_allowance
    }

    #[must_use]
    pub const fn allowance_scope(&self) -> PmFixtureAllowanceScope {
        PmFixtureAllowanceScope::UnscopedLegacyScalar
    }
}

/// A strict, immutable view of one tracked open-order seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmFixtureOpenOrder {
    fields: PmFixtureOrderFields,
}

impl PmFixtureOpenOrder {
    order_field_getters!();
}

/// A strict, immutable view of a user-stream order event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmFixtureUserOrder {
    fields: PmFixtureOrderFields,
    event_kind: String,
}

impl PmFixtureUserOrder {
    order_field_getters!();

    #[must_use]
    pub fn event_kind(&self) -> &str {
        &self.event_kind
    }
}

/// A strict, immutable maker-order leg retained inside a trade fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmFixtureMakerOrder {
    order_id: String,
    asset_id: String,
    side: String,
    price: String,
    matched_amount: String,
    maker_address: Option<String>,
}

impl PmFixtureMakerOrder {
    #[must_use]
    pub fn order_id(&self) -> &str {
        &self.order_id
    }

    #[must_use]
    pub fn asset_id(&self) -> &str {
        &self.asset_id
    }

    #[must_use]
    pub fn side(&self) -> &str {
        &self.side
    }

    #[must_use]
    pub fn price(&self) -> &str {
        &self.price
    }

    #[must_use]
    pub fn matched_amount(&self) -> &str {
        &self.matched_amount
    }

    /// Exact per-leg account proof when the selected fixture contract carries
    /// it. Legacy/pinned legs without this field remain unproven.
    #[must_use]
    pub fn maker_address(&self) -> Option<&str> {
        self.maker_address.as_deref()
    }
}

/// The order-reference evidence carried by one trade fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PmFixtureTradeLinkage {
    Unlinked,
    DirectOrder,
    TakerOrder,
    MakerOrders,
    MultipleReferenceKinds,
}

/// A strict, immutable view of a user-stream trade event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmFixtureUserTrade {
    id: String,
    order_id: Option<String>,
    taker_order_id: Option<String>,
    market: String,
    asset_id: String,
    side: String,
    size: String,
    price: String,
    status: String,
    maker_address: String,
    transaction_hash: String,
    trader_side: Option<String>,
    maker_orders: Option<Vec<PmFixtureMakerOrder>>,
}

impl PmFixtureUserTrade {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn order_id(&self) -> Option<&str> {
        self.order_id.as_deref()
    }

    #[must_use]
    pub fn taker_order_id(&self) -> Option<&str> {
        self.taker_order_id.as_deref()
    }

    #[must_use]
    pub fn market(&self) -> &str {
        &self.market
    }

    #[must_use]
    pub fn asset_id(&self) -> &str {
        &self.asset_id
    }

    #[must_use]
    pub fn side(&self) -> &str {
        &self.side
    }

    #[must_use]
    pub fn size(&self) -> &str {
        &self.size
    }

    #[must_use]
    pub fn price(&self) -> &str {
        &self.price
    }

    #[must_use]
    pub fn status(&self) -> &str {
        &self.status
    }

    #[must_use]
    pub fn maker_address(&self) -> &str {
        &self.maker_address
    }

    #[must_use]
    pub fn transaction_hash(&self) -> &str {
        &self.transaction_hash
    }

    #[must_use]
    pub fn trader_side(&self) -> Option<&str> {
        self.trader_side.as_deref()
    }

    #[must_use]
    pub fn maker_orders(&self) -> Option<&[PmFixtureMakerOrder]> {
        self.maker_orders.as_deref()
    }

    #[must_use]
    pub fn linkage(&self) -> PmFixtureTradeLinkage {
        let direct = self.order_id.is_some();
        let taker = self.taker_order_id.is_some();
        let maker = self
            .maker_orders
            .as_ref()
            .is_some_and(|orders| !orders.is_empty());
        match u8::from(direct) + u8::from(taker) + u8::from(maker) {
            0 => PmFixtureTradeLinkage::Unlinked,
            1 if direct => PmFixtureTradeLinkage::DirectOrder,
            1 if taker => PmFixtureTradeLinkage::TakerOrder,
            1 => PmFixtureTradeLinkage::MakerOrders,
            _ => PmFixtureTradeLinkage::MultipleReferenceKinds,
        }
    }
}

/// One strict user-stream event, still expressed as fixture evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PmFixtureUserEvent {
    Order(PmFixtureUserOrder),
    Trade(PmFixtureUserTrade),
}

/// A bounded object-or-array user fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmFixtureUserFrame {
    events: Vec<PmFixtureUserEvent>,
}

impl PmFixtureUserFrame {
    #[must_use]
    pub fn events(&self) -> &[PmFixtureUserEvent] {
        &self.events
    }
}

/// Parses one user order/trade object or an array of those objects.
pub fn parse_private_user_fixture(raw: &[u8]) -> Result<PmFixtureUserFrame, PmPrivateFixtureError> {
    check_fixture_bound(raw)?;
    let raw_frame = serde_json::from_slice::<RawUserFrame>(raw)
        .map_err(|_| PmPrivateFixtureError::MalformedJson)?;
    let raw_events = match raw_frame {
        RawUserFrame::Object(event) => vec![*event],
        RawUserFrame::Array(events) => events,
    };
    if raw_events.is_empty() {
        return Err(PmPrivateFixtureError::EmptyUserFrame);
    }
    if raw_events.len() > MAX_PRIVATE_FIXTURE_EVENTS {
        return Err(PmPrivateFixtureError::TooManyEvents);
    }

    let events = raw_events
        .into_iter()
        .map(PmFixtureUserEvent::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PmFixtureUserFrame { events })
}

/// Parses the strict standalone open-order seed shape.
pub fn parse_open_order_fixture(raw: &[u8]) -> Result<PmFixtureOpenOrder, PmPrivateFixtureError> {
    check_fixture_bound(raw)?;
    let raw_order = serde_json::from_slice::<RawOpenOrder>(raw)
        .map_err(|_| PmPrivateFixtureError::MalformedJson)?;
    Ok(PmFixtureOpenOrder {
        fields: PmFixtureOrderFields::try_from(raw_order)?,
    })
}

/// Parses the strict legacy scalar balance/allowance seed shape.
pub fn parse_legacy_balance_allowance_fixture(
    raw: &[u8],
) -> Result<PmLegacyBalanceAllowanceFixture, PmPrivateFixtureError> {
    check_fixture_bound(raw)?;
    let raw_balance = serde_json::from_slice::<RawLegacyBalanceAllowance>(raw)
        .map_err(|_| PmPrivateFixtureError::MalformedJson)?;
    Ok(PmLegacyBalanceAllowanceFixture {
        balance: nonempty(raw_balance.balance, "balance")?,
        unscoped_allowance: nonempty(raw_balance.allowance, "allowance")?,
    })
}

fn check_fixture_bound(raw: &[u8]) -> Result<(), PmPrivateFixtureError> {
    if raw.len() > MAX_PRIVATE_FIXTURE_BYTES {
        Err(PmPrivateFixtureError::PayloadTooLarge)
    } else {
        Ok(())
    }
}

fn nonempty(value: String, field: &'static str) -> Result<String, PmPrivateFixtureError> {
    if value.trim().is_empty() {
        Err(PmPrivateFixtureError::EmptyField(field))
    } else {
        Ok(value)
    }
}

fn optional_nonempty(
    value: Option<String>,
    field: &'static str,
) -> Result<Option<String>, PmPrivateFixtureError> {
    value.map(|value| nonempty(value, field)).transpose()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PmFixtureOrderFields {
    id: String,
    market: String,
    asset_id: String,
    side: String,
    original_size: String,
    size_matched: String,
    price: String,
    status: String,
    maker_address: String,
}

impl TryFrom<RawOpenOrder> for PmFixtureOrderFields {
    type Error = PmPrivateFixtureError;

    fn try_from(raw: RawOpenOrder) -> Result<Self, Self::Error> {
        Self::from_raw(RawOrderFields {
            id: raw.id,
            market: raw.market,
            asset_id: raw.asset_id,
            side: raw.side,
            original_size: raw.original_size,
            size_matched: raw.size_matched,
            price: raw.price,
            status: raw.status,
            maker_address: raw.maker_address,
        })
    }
}

impl PmFixtureOrderFields {
    fn from_raw(raw: RawOrderFields) -> Result<Self, PmPrivateFixtureError> {
        Ok(Self {
            id: nonempty(raw.id, "id")?,
            market: nonempty(raw.market, "market")?,
            asset_id: nonempty(raw.asset_id, "asset_id")?,
            side: nonempty(raw.side, "side")?,
            original_size: nonempty(raw.original_size, "original_size")?,
            size_matched: nonempty(raw.size_matched, "size_matched")?,
            price: nonempty(raw.price, "price")?,
            status: nonempty(raw.status, "status")?,
            maker_address: nonempty(raw.maker_address, "maker_address")?,
        })
    }
}

impl TryFrom<RawUserEvent> for PmFixtureUserEvent {
    type Error = PmPrivateFixtureError;

    fn try_from(raw: RawUserEvent) -> Result<Self, Self::Error> {
        match raw {
            RawUserEvent::Order(raw) => {
                let fields = PmFixtureOrderFields::from_raw(RawOrderFields {
                    id: raw.id,
                    market: raw.market,
                    asset_id: raw.asset_id,
                    side: raw.side,
                    original_size: raw.original_size,
                    size_matched: raw.size_matched,
                    price: raw.price,
                    status: raw.status,
                    maker_address: raw.maker_address,
                })?;
                Ok(Self::Order(PmFixtureUserOrder {
                    fields,
                    event_kind: nonempty(raw.event_kind, "type")?,
                }))
            }
            RawUserEvent::Trade(raw) => Ok(Self::Trade(PmFixtureUserTrade::try_from(raw)?)),
        }
    }
}

impl TryFrom<RawUserTrade> for PmFixtureUserTrade {
    type Error = PmPrivateFixtureError;

    fn try_from(raw: RawUserTrade) -> Result<Self, Self::Error> {
        if raw
            .maker_orders
            .as_ref()
            .is_some_and(|orders| orders.len() > MAX_PRIVATE_FIXTURE_EVENTS)
        {
            return Err(PmPrivateFixtureError::TooManyMakerOrders);
        }
        let maker_orders = raw
            .maker_orders
            .map(|orders| {
                orders
                    .into_iter()
                    .map(PmFixtureMakerOrder::try_from)
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;
        Ok(Self {
            id: nonempty(raw.id, "id")?,
            order_id: optional_nonempty(raw.order_id, "order_id")?,
            taker_order_id: optional_nonempty(raw.taker_order_id, "taker_order_id")?,
            market: nonempty(raw.market, "market")?,
            asset_id: nonempty(raw.asset_id, "asset_id")?,
            side: nonempty(raw.side, "side")?,
            size: nonempty(raw.size, "size")?,
            price: nonempty(raw.price, "price")?,
            status: nonempty(raw.status, "status")?,
            maker_address: nonempty(raw.maker_address, "maker_address")?,
            transaction_hash: nonempty(raw.transaction_hash, "transaction_hash")?,
            trader_side: optional_nonempty(raw.trader_side, "trader_side")?,
            maker_orders,
        })
    }
}

impl TryFrom<RawMakerOrder> for PmFixtureMakerOrder {
    type Error = PmPrivateFixtureError;

    fn try_from(raw: RawMakerOrder) -> Result<Self, Self::Error> {
        Ok(Self {
            order_id: nonempty(raw.order_id, "maker_orders.order_id")?,
            asset_id: nonempty(raw.asset_id, "maker_orders.asset_id")?,
            side: nonempty(raw.side, "maker_orders.side")?,
            price: nonempty(raw.price, "maker_orders.price")?,
            matched_amount: nonempty(raw.matched_amount, "maker_orders.matched_amount")?,
            maker_address: optional_nonempty(raw.maker_address, "maker_orders.maker_address")?,
        })
    }
}

struct RawOrderFields {
    id: String,
    market: String,
    asset_id: String,
    side: String,
    original_size: String,
    size_matched: String,
    price: String,
    status: String,
    maker_address: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLegacyBalanceAllowance {
    balance: String,
    allowance: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOpenOrder {
    id: String,
    market: String,
    asset_id: String,
    side: String,
    original_size: String,
    size_matched: String,
    price: String,
    status: String,
    maker_address: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawUserFrame {
    Object(Box<RawUserEvent>),
    Array(Vec<RawUserEvent>),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawUserEvent {
    Order(RawUserOrder),
    Trade(RawUserTrade),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawUserOrder {
    #[serde(rename = "event_type")]
    _event_type: RawOrderEventType,
    id: String,
    market: String,
    asset_id: String,
    side: String,
    original_size: String,
    size_matched: String,
    price: String,
    status: String,
    maker_address: String,
    #[serde(rename = "type")]
    event_kind: String,
}

#[derive(Deserialize)]
enum RawOrderEventType {
    #[serde(rename = "order")]
    Order,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawUserTrade {
    #[serde(rename = "event_type")]
    _event_type: RawTradeEventType,
    id: String,
    order_id: Option<String>,
    taker_order_id: Option<String>,
    market: String,
    asset_id: String,
    side: String,
    size: String,
    price: String,
    status: String,
    maker_address: String,
    transaction_hash: String,
    trader_side: Option<String>,
    maker_orders: Option<Vec<RawMakerOrder>>,
}

#[derive(Deserialize)]
enum RawTradeEventType {
    #[serde(rename = "trade")]
    Trade,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMakerOrder {
    order_id: String,
    asset_id: String,
    side: String,
    price: String,
    matched_amount: String,
    maker_address: Option<String>,
}
