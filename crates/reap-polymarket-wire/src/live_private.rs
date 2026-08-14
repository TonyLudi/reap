//! Bounded, secret-custody-free views of current authenticated Polymarket responses.
//!
//! These DTOs are independently authored from the current protocol shapes.
//! They preserve only evidence needed by Reap's fixed PM profile, never use
//! floating point, and do not confer account scope, ownership, completeness,
//! or mutation authority by themselves.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use reap_pm_core::{
    EvmAddress, PmBookQuantity, PmConditionId, PmFillId, PmOrderSide, PmPrice, PmQuantity,
    PmTokenId, PmVenueOrderId, U256,
};
use serde::{Deserialize, Deserializer, de::Visitor};
use thiserror::Error;
use zeroize::Zeroizing;

mod associate_trades_wire;
mod result_wire;

use associate_trades_wire::RawAssociateTrades;
use result_wire::{RawCancelResult, RawPlaceResult};

pub const MAX_PM_LIVE_BODY_BYTES: usize = 1_048_576;
pub const MAX_PM_LIVE_PAGE_ITEMS: usize = 128;
const MAX_PM_LIVE_DECLARED_PAGE_LIMIT: usize = 300;
pub const MAX_PM_LIVE_CURSOR_BYTES: usize = 256;
const MAX_PM_LIVE_USER_EVENTS: usize = 64;
const MAX_PM_LIVE_MAKER_ORDERS: usize = 64;
const MAX_PM_LIVE_ALLOWANCES: usize = 32;
const MAX_PM_LIVE_RESULT_ITEMS: usize = 128;
const MAX_PM_LIVE_EVIDENCE_BYTES: usize = 512;
const TERMINAL_CURSOR: &str = "LTE=";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmLiveWireError {
    #[error("authenticated Polymarket payload exceeds its byte bound")]
    PayloadTooLarge,
    #[error("authenticated Polymarket payload is malformed or has a foreign shape")]
    MalformedJson,
    #[error("authenticated Polymarket user frame is empty")]
    EmptyUserFrame,
    #[error("authenticated Polymarket collection exceeds its fixed item bound")]
    TooManyItems,
    #[error("required authenticated Polymarket field `{0}` is empty")]
    EmptyField(&'static str),
    #[error("authenticated Polymarket field `{0}` exceeds its byte bound")]
    FieldTooLong(&'static str),
    #[error("authenticated Polymarket field `{0}` has an invalid identity")]
    InvalidIdentity(&'static str),
    #[error("authenticated Polymarket field `{0}` has an invalid exact numeric value")]
    InvalidNumeric(&'static str),
    #[error("authenticated Polymarket order side is unsupported")]
    UnsupportedSide,
    #[error("authenticated Polymarket cursor is empty, oversized, or non-ASCII")]
    InvalidCursor,
    #[error("authenticated Polymarket page limit/count contradicts its bounded data")]
    InvalidPageMetadata,
    #[error("authenticated Polymarket page declares unsupported limit {observed}")]
    UnsupportedPageLimit { observed: usize },
    #[error("authenticated Polymarket page count {declared} does not match returned rows {actual}")]
    PageCountMismatch { declared: usize, actual: usize },
    #[error("authenticated Polymarket allowance map is absent")]
    MissingAllowanceMap,
    #[error("authenticated Polymarket allowance map contains duplicate canonical spenders")]
    DuplicateAllowance,
    #[error("successful authenticated Polymarket place result omits required evidence")]
    IncompletePlaceSuccess,
    #[error("failed authenticated Polymarket place result omits its required error evidence")]
    IncompletePlaceFailure,
    #[error("authenticated Polymarket place result contradicts its success discriminator")]
    ContradictoryPlaceResult,
    #[error("authenticated Polymarket result contains duplicate identities")]
    DuplicateIdentity,
}

/// Opaque account-scope identity carried by authenticated PM responses.
///
/// The value is deliberately neither `Clone` nor visibly printable. Callers
/// can compare it to their separately held expected owner without extracting
/// or logging the wire credential-owner UUID.
#[derive(PartialEq, Eq)]
pub struct PmCredentialOwner(Zeroizing<Vec<u8>>);

impl PmCredentialOwner {
    #[must_use]
    pub fn matches_exact(&self, expected: &str) -> bool {
        self.0.as_slice() == expected.as_bytes()
    }
}

impl fmt::Debug for PmCredentialOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmCredentialOwner([REDACTED])")
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PmLiveOrder {
    id: PmVenueOrderId,
    condition: PmConditionId,
    token: PmTokenId,
    side: PmOrderSide,
    original_size: PmQuantity,
    size_matched: PmBookQuantity,
    price: PmPrice,
    status: String,
    maker: EvmAddress,
    owner: PmCredentialOwner,
    created_at: u64,
    expiration: u64,
    outcome: Option<String>,
    order_type: Option<String>,
}

impl PmLiveOrder {
    #[must_use]
    pub const fn id(&self) -> PmVenueOrderId {
        self.id
    }
    #[must_use]
    pub const fn condition(&self) -> PmConditionId {
        self.condition
    }
    #[must_use]
    pub const fn token(&self) -> PmTokenId {
        self.token
    }
    #[must_use]
    pub const fn side(&self) -> PmOrderSide {
        self.side
    }
    #[must_use]
    pub const fn original_size(&self) -> PmQuantity {
        self.original_size
    }
    #[must_use]
    pub const fn size_matched(&self) -> PmBookQuantity {
        self.size_matched
    }
    #[must_use]
    pub const fn price(&self) -> PmPrice {
        self.price
    }
    #[must_use]
    pub fn status(&self) -> &str {
        &self.status
    }
    #[must_use]
    pub const fn maker(&self) -> EvmAddress {
        self.maker
    }
    #[must_use]
    pub const fn owner(&self) -> &PmCredentialOwner {
        &self.owner
    }
    #[must_use]
    pub const fn created_at(&self) -> u64 {
        self.created_at
    }
    #[must_use]
    pub const fn expiration(&self) -> u64 {
        self.expiration
    }
    #[must_use]
    pub fn outcome(&self) -> Option<&str> {
        self.outcome.as_deref()
    }
    #[must_use]
    pub fn order_type(&self) -> Option<&str> {
        self.order_type.as_deref()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PmLiveMakerOrder {
    order_id: PmVenueOrderId,
    token: PmTokenId,
    side: PmOrderSide,
    price: PmPrice,
    matched_amount: PmQuantity,
    fee_rate_bps: Option<U256>,
    owner: PmCredentialOwner,
    maker: EvmAddress,
}

impl PmLiveMakerOrder {
    #[must_use]
    pub const fn order_id(&self) -> PmVenueOrderId {
        self.order_id
    }
    #[must_use]
    pub const fn token(&self) -> PmTokenId {
        self.token
    }
    #[must_use]
    pub const fn side(&self) -> PmOrderSide {
        self.side
    }
    #[must_use]
    pub const fn price(&self) -> PmPrice {
        self.price
    }
    #[must_use]
    pub const fn matched_amount(&self) -> PmQuantity {
        self.matched_amount
    }
    /// Exact canonical decimal fee rate for this maker leg, when reported.
    #[must_use]
    pub const fn fee_rate_bps(&self) -> Option<U256> {
        self.fee_rate_bps
    }
    #[must_use]
    pub const fn owner(&self) -> &PmCredentialOwner {
        &self.owner
    }
    #[must_use]
    pub const fn maker(&self) -> EvmAddress {
        self.maker
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PmLiveTrade {
    id: PmFillId,
    condition: PmConditionId,
    token: PmTokenId,
    side: PmOrderSide,
    size: PmQuantity,
    price: PmPrice,
    status: String,
    order_id: Option<PmVenueOrderId>,
    taker_order_id: Option<PmVenueOrderId>,
    trader_side: Option<String>,
    transaction_hash: Option<String>,
    fee_rate_bps: Option<U256>,
    maker_orders: Box<[PmLiveMakerOrder]>,
    maker: Option<EvmAddress>,
    owner: PmCredentialOwner,
    trade_owner: Option<PmCredentialOwner>,
    timestamp: Option<u64>,
    match_time: Option<u64>,
    last_update: Option<u64>,
}

impl PmLiveTrade {
    #[must_use]
    pub const fn id(&self) -> PmFillId {
        self.id
    }
    #[must_use]
    pub const fn condition(&self) -> PmConditionId {
        self.condition
    }
    #[must_use]
    pub const fn token(&self) -> PmTokenId {
        self.token
    }
    #[must_use]
    pub const fn side(&self) -> PmOrderSide {
        self.side
    }
    #[must_use]
    pub const fn size(&self) -> PmQuantity {
        self.size
    }
    #[must_use]
    pub const fn price(&self) -> PmPrice {
        self.price
    }
    #[must_use]
    pub fn status(&self) -> &str {
        &self.status
    }
    #[must_use]
    pub const fn order_id(&self) -> Option<PmVenueOrderId> {
        self.order_id
    }
    #[must_use]
    pub const fn taker_order_id(&self) -> Option<PmVenueOrderId> {
        self.taker_order_id
    }
    #[must_use]
    pub fn trader_side(&self) -> Option<&str> {
        self.trader_side.as_deref()
    }
    #[must_use]
    pub fn transaction_hash(&self) -> Option<&str> {
        self.transaction_hash.as_deref()
    }
    /// Exact canonical decimal rate reported by the venue.
    ///
    /// Absence is distinct from an explicitly reported zero. Consumers must
    /// not infer a fee amount from a nonzero rate without the venue formula.
    #[must_use]
    pub const fn fee_rate_bps(&self) -> Option<U256> {
        self.fee_rate_bps
    }
    #[must_use]
    pub fn maker_orders(&self) -> &[PmLiveMakerOrder] {
        &self.maker_orders
    }
    #[must_use]
    pub const fn maker(&self) -> Option<EvmAddress> {
        self.maker
    }
    #[must_use]
    pub const fn owner(&self) -> &PmCredentialOwner {
        &self.owner
    }
    #[must_use]
    pub const fn trade_owner(&self) -> Option<&PmCredentialOwner> {
        self.trade_owner.as_ref()
    }
    #[must_use]
    pub const fn timestamp(&self) -> Option<u64> {
        self.timestamp
    }
    #[must_use]
    pub const fn match_time(&self) -> Option<u64> {
        self.match_time
    }
    #[must_use]
    pub const fn last_update(&self) -> Option<u64> {
        self.last_update
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PmLiveUserOrder {
    id: PmVenueOrderId,
    condition: PmConditionId,
    token: PmTokenId,
    side: PmOrderSide,
    original_size: PmQuantity,
    size_matched: PmBookQuantity,
    price: PmPrice,
    event_kind: String,
    maker: Option<EvmAddress>,
    expiration: Option<u64>,
    order_type: Option<String>,
    outcome: Option<String>,
    status: Option<String>,
    created_at: Option<u64>,
    associate_trades: Option<Box<[PmFillId]>>,
    owner: PmCredentialOwner,
    order_owner: Option<PmCredentialOwner>,
    timestamp: Option<u64>,
}

impl PmLiveUserOrder {
    #[must_use]
    pub const fn id(&self) -> PmVenueOrderId {
        self.id
    }
    #[must_use]
    pub const fn condition(&self) -> PmConditionId {
        self.condition
    }
    #[must_use]
    pub const fn token(&self) -> PmTokenId {
        self.token
    }
    #[must_use]
    pub const fn side(&self) -> PmOrderSide {
        self.side
    }
    #[must_use]
    pub const fn original_size(&self) -> PmQuantity {
        self.original_size
    }
    #[must_use]
    pub const fn size_matched(&self) -> PmBookQuantity {
        self.size_matched
    }
    #[must_use]
    pub const fn price(&self) -> PmPrice {
        self.price
    }
    #[must_use]
    pub fn event_kind(&self) -> &str {
        &self.event_kind
    }
    #[must_use]
    pub const fn maker(&self) -> Option<EvmAddress> {
        self.maker
    }
    #[must_use]
    pub const fn expiration(&self) -> Option<u64> {
        self.expiration
    }
    #[must_use]
    pub fn order_type(&self) -> Option<&str> {
        self.order_type.as_deref()
    }
    #[must_use]
    pub fn outcome(&self) -> Option<&str> {
        self.outcome.as_deref()
    }
    #[must_use]
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }
    #[must_use]
    pub const fn created_at(&self) -> Option<u64> {
        self.created_at
    }
    #[must_use]
    pub fn associate_trades(&self) -> Option<&[PmFillId]> {
        self.associate_trades.as_deref()
    }
    #[must_use]
    pub const fn owner(&self) -> &PmCredentialOwner {
        &self.owner
    }
    #[must_use]
    pub const fn order_owner(&self) -> Option<&PmCredentialOwner> {
        self.order_owner.as_ref()
    }
    #[must_use]
    pub const fn timestamp(&self) -> Option<u64> {
        self.timestamp
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PmLiveUserEvent {
    Order(Box<PmLiveUserOrder>),
    Trade(Box<PmLiveTrade>),
}

#[derive(Debug, PartialEq, Eq)]
pub struct PmLiveUserFrame {
    events: Box<[PmLiveUserEvent]>,
}

impl PmLiveUserFrame {
    #[must_use]
    pub fn events(&self) -> &[PmLiveUserEvent] {
        &self.events
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmLiveCursor(String);

impl PmLiveCursor {
    pub fn new(value: String) -> Result<Self, PmLiveWireError> {
        if value.is_empty()
            || value == TERMINAL_CURSOR
            || value.len() > MAX_PM_LIVE_CURSOR_BYTES
            || !value.is_ascii()
            || value
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
        {
            Err(PmLiveWireError::InvalidCursor)
        } else {
            Ok(Self(value))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PmLiveOpenOrderPage {
    orders: Box<[PmLiveOrder]>,
    next_cursor: Option<PmLiveCursor>,
    terminal: bool,
    declared_limit: usize,
    declared_count: usize,
}

impl PmLiveOpenOrderPage {
    #[must_use]
    pub fn orders(&self) -> &[PmLiveOrder] {
        &self.orders
    }
    #[must_use]
    pub const fn next_cursor(&self) -> Option<&PmLiveCursor> {
        self.next_cursor.as_ref()
    }
    #[must_use]
    pub const fn terminal(&self) -> bool {
        self.terminal
    }
    #[must_use]
    pub const fn declared_limit(&self) -> usize {
        self.declared_limit
    }
    #[must_use]
    pub const fn declared_count(&self) -> usize {
        self.declared_count
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PmLiveTradePage {
    trades: Box<[PmLiveTrade]>,
    next_cursor: Option<PmLiveCursor>,
    terminal: bool,
    declared_limit: usize,
    declared_count: usize,
}

impl PmLiveTradePage {
    #[must_use]
    pub fn trades(&self) -> &[PmLiveTrade] {
        &self.trades
    }
    #[must_use]
    pub const fn next_cursor(&self) -> Option<&PmLiveCursor> {
        self.next_cursor.as_ref()
    }
    #[must_use]
    pub const fn terminal(&self) -> bool {
        self.terminal
    }
    #[must_use]
    pub const fn declared_limit(&self) -> usize {
        self.declared_limit
    }
    #[must_use]
    pub const fn declared_count(&self) -> usize {
        self.declared_count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmLiveAllowanceEntry {
    spender: EvmAddress,
    amount: U256,
}

impl PmLiveAllowanceEntry {
    #[must_use]
    pub const fn spender(self) -> EvmAddress {
        self.spender
    }
    #[must_use]
    pub const fn amount(self) -> U256 {
        self.amount
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmLiveBalanceAllowance {
    balance: U256,
    allowances: Box<[PmLiveAllowanceEntry]>,
    unscoped_scalar_present: bool,
}

impl PmLiveBalanceAllowance {
    #[must_use]
    pub const fn balance(&self) -> U256 {
        self.balance
    }
    #[must_use]
    pub fn allowances(&self) -> &[PmLiveAllowanceEntry] {
        &self.allowances
    }
    #[must_use]
    pub const fn unscoped_scalar_present(&self) -> bool {
        self.unscoped_scalar_present
    }
    #[must_use]
    pub fn exact_allowance(&self, spender: EvmAddress) -> Option<U256> {
        self.allowances
            .iter()
            .find(|entry| entry.spender == spender)
            .map(|entry| entry.amount)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmLivePlaceResult {
    success: bool,
    order_id: Option<PmVenueOrderId>,
    status: String,
    error_message: Option<String>,
    making_amount: Option<U256>,
    taking_amount: Option<U256>,
    trade_ids: Box<[PmFillId]>,
    transaction_hashes: Box<[String]>,
}

impl PmLivePlaceResult {
    #[must_use]
    pub const fn success(&self) -> bool {
        self.success
    }
    #[must_use]
    pub const fn order_id(&self) -> Option<PmVenueOrderId> {
        self.order_id
    }
    #[must_use]
    pub fn status(&self) -> &str {
        &self.status
    }
    #[must_use]
    pub fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }
    #[must_use]
    pub const fn making_amount(&self) -> Option<U256> {
        self.making_amount
    }
    #[must_use]
    pub const fn taking_amount(&self) -> Option<U256> {
        self.taking_amount
    }
    #[must_use]
    pub fn trade_ids(&self) -> &[PmFillId] {
        &self.trade_ids
    }
    #[must_use]
    pub fn transaction_hashes(&self) -> &[String] {
        &self.transaction_hashes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmLiveCancelResult {
    canceled: Box<[PmVenueOrderId]>,
    not_canceled: Box<[(PmVenueOrderId, String)]>,
}

impl PmLiveCancelResult {
    #[must_use]
    pub fn canceled(&self) -> &[PmVenueOrderId] {
        &self.canceled
    }
    #[must_use]
    pub fn not_canceled(&self) -> &[(PmVenueOrderId, String)] {
        &self.not_canceled
    }
}

pub fn parse_live_user_frame(raw: &[u8]) -> Result<PmLiveUserFrame, PmLiveWireError> {
    check_body(raw)?;
    let wire =
        serde_json::from_slice::<RawUserFrame>(raw).map_err(|_| PmLiveWireError::MalformedJson)?;
    let events = match wire {
        RawUserFrame::One(event) => vec![*event],
        RawUserFrame::Many(events) => events,
    };
    if events.is_empty() {
        return Err(PmLiveWireError::EmptyUserFrame);
    }
    check_items(events.len(), MAX_PM_LIVE_USER_EVENTS)?;
    let events = events
        .into_iter()
        .map(PmLiveUserEvent::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PmLiveUserFrame {
        events: events.into_boxed_slice(),
    })
}

pub fn parse_live_open_order_page(raw: &[u8]) -> Result<PmLiveOpenOrderPage, PmLiveWireError> {
    check_body(raw)?;
    let page = serde_json::from_slice::<RawPage<RawOrder>>(raw)
        .map_err(|_| PmLiveWireError::MalformedJson)?;
    check_items(page.data.len(), MAX_PM_LIVE_PAGE_ITEMS)?;
    validate_page_metadata(page.data.len(), page.limit, page.count)?;
    let orders = page
        .data
        .into_iter()
        .map(PmLiveOrder::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    let (next_cursor, terminal) = parse_cursor(page.next_cursor)?;
    Ok(PmLiveOpenOrderPage {
        orders: orders.into_boxed_slice(),
        next_cursor,
        terminal,
        declared_limit: page.limit,
        declared_count: page.count,
    })
}

pub fn parse_live_trade_page(raw: &[u8]) -> Result<PmLiveTradePage, PmLiveWireError> {
    parse_live_trade_page_with_fee_evidence(raw, true)
}

/// Parse the exact-scope trade projection used solely for owned-fill and
/// position reconciliation. Fee-rate evidence is deliberately omitted: it
/// does not participate in fill identity, side, quantity, price, ownership,
/// or settlement, and current CLOB V2 maker legs may use fractional fee text
/// that the integer fee projection cannot represent.
pub fn parse_live_owned_fill_trade_page(raw: &[u8]) -> Result<PmLiveTradePage, PmLiveWireError> {
    parse_live_trade_page_with_fee_evidence(raw, false)
}

fn parse_live_trade_page_with_fee_evidence(
    raw: &[u8],
    retain_fee_evidence: bool,
) -> Result<PmLiveTradePage, PmLiveWireError> {
    check_body(raw)?;
    let page = serde_json::from_slice::<RawPage<RawRestTrade>>(raw)
        .map_err(|_| PmLiveWireError::MalformedJson)?;
    check_items(page.data.len(), MAX_PM_LIVE_PAGE_ITEMS)?;
    validate_page_metadata(page.data.len(), page.limit, page.count)?;
    let trades = page
        .data
        .into_iter()
        .map(|trade| PmLiveTrade::from_rest(trade, retain_fee_evidence))
        .collect::<Result<Vec<_>, _>>()?;
    let (next_cursor, terminal) = parse_cursor(page.next_cursor)?;
    Ok(PmLiveTradePage {
        trades: trades.into_boxed_slice(),
        next_cursor,
        terminal,
        declared_limit: page.limit,
        declared_count: page.count,
    })
}

pub fn parse_live_order_detail(raw: &[u8]) -> Result<PmLiveOrder, PmLiveWireError> {
    check_body(raw)?;
    let order =
        serde_json::from_slice::<RawOrder>(raw).map_err(|_| PmLiveWireError::MalformedJson)?;
    PmLiveOrder::try_from(order)
}

pub fn parse_live_balance_allowance(raw: &[u8]) -> Result<PmLiveBalanceAllowance, PmLiveWireError> {
    check_body(raw)?;
    let wire = serde_json::from_slice::<RawBalanceAllowance>(raw)
        .map_err(|_| PmLiveWireError::MalformedJson)?;
    let balance = parse_u256(&wire.balance, "balance")?;
    let allowances = wire
        .allowances
        .ok_or(PmLiveWireError::MissingAllowanceMap)?;
    check_items(allowances.len(), MAX_PM_LIVE_ALLOWANCES)?;
    let mut seen = BTreeSet::new();
    let mut parsed = Vec::with_capacity(allowances.len());
    for (spender, amount) in allowances {
        let spender = EvmAddress::parse(&spender)
            .map_err(|_| PmLiveWireError::InvalidIdentity("allowances.spender"))?;
        if !seen.insert(spender) {
            return Err(PmLiveWireError::DuplicateAllowance);
        }
        parsed.push(PmLiveAllowanceEntry {
            spender,
            amount: parse_u256(&amount, "allowances.amount")?,
        });
    }
    parsed.sort_unstable_by_key(|entry| entry.spender);
    Ok(PmLiveBalanceAllowance {
        balance,
        allowances: parsed.into_boxed_slice(),
        unscoped_scalar_present: wire.allowance.is_some(),
    })
}

pub fn parse_live_place_result(raw: &[u8]) -> Result<PmLivePlaceResult, PmLiveWireError> {
    check_body(raw)?;
    let wire = serde_json::from_slice::<RawPlaceResult>(raw)
        .map_err(|_| PmLiveWireError::MalformedJson)?;
    check_items(
        wire.trade_ids
            .len()
            .saturating_add(wire.transaction_hashes.len()),
        MAX_PM_LIVE_RESULT_ITEMS,
    )?;
    let (order_id, status, error_message, making_amount, taking_amount) = if wire.success {
        if !wire.error_message.is_empty() {
            return Err(PmLiveWireError::ContradictoryPlaceResult);
        }
        let order_id = parse_order_id(&wire.order_id, "orderID")
            .map_err(|_| PmLiveWireError::IncompletePlaceSuccess)?;
        let status = parse_place_success_status(wire.status)?;
        let making_amount = parse_optional_place_amount(wire.making_amount, "makingAmount")?;
        let taking_amount = parse_optional_place_amount(wire.taking_amount, "takingAmount")?;
        (Some(order_id), status, None, making_amount, taking_amount)
    } else {
        if wire.error_message.is_empty() {
            return Err(PmLiveWireError::IncompletePlaceFailure);
        }
        if !wire.order_id.is_empty()
            || !wire.status.is_empty()
            || !wire.making_amount.is_empty()
            || !wire.taking_amount.is_empty()
            || !wire.trade_ids.is_empty()
            || !wire.transaction_hashes.is_empty()
        {
            return Err(PmLiveWireError::ContradictoryPlaceResult);
        }
        (
            None,
            String::new(),
            Some(bounded(wire.error_message, "errorMsg")?),
            None,
            None,
        )
    };
    let mut seen = BTreeSet::new();
    let mut trade_ids = Vec::with_capacity(wire.trade_ids.len());
    for id in wire.trade_ids {
        let id = PmFillId::new(&id).map_err(|_| PmLiveWireError::InvalidIdentity("tradeIDs"))?;
        if !seen.insert(id) {
            return Err(PmLiveWireError::DuplicateIdentity);
        }
        trade_ids.push(id);
    }
    let mut transaction_hashes = Vec::with_capacity(wire.transaction_hashes.len());
    for hash in wire.transaction_hashes {
        let hash = bounded(hash, "transactionsHashes")?;
        if !seen.insert(
            PmFillId::new(&hash)
                .map_err(|_| PmLiveWireError::InvalidIdentity("transactionsHashes"))?,
        ) {
            return Err(PmLiveWireError::DuplicateIdentity);
        }
        transaction_hashes.push(hash);
    }
    Ok(PmLivePlaceResult {
        success: wire.success,
        order_id,
        status,
        error_message,
        making_amount,
        taking_amount,
        trade_ids: trade_ids.into_boxed_slice(),
        transaction_hashes: transaction_hashes.into_boxed_slice(),
    })
}

fn parse_place_success_status(value: String) -> Result<String, PmLiveWireError> {
    match value.as_str() {
        "live" | "matched" | "delayed" | "unmatched" => Ok(value),
        _ => Err(PmLiveWireError::IncompletePlaceSuccess),
    }
}

fn parse_optional_place_amount(
    value: String,
    field: &'static str,
) -> Result<Option<U256>, PmLiveWireError> {
    if value.is_empty() {
        Ok(None)
    } else {
        parse_u256(&value, field).map(Some)
    }
}

pub fn parse_live_cancel_result(raw: &[u8]) -> Result<PmLiveCancelResult, PmLiveWireError> {
    check_body(raw)?;
    let wire = serde_json::from_slice::<RawCancelResult>(raw)
        .map_err(|_| PmLiveWireError::MalformedJson)?;
    check_items(
        wire.canceled
            .len()
            .saturating_add(wire.not_canceled.0.len()),
        MAX_PM_LIVE_RESULT_ITEMS,
    )?;
    let mut seen = BTreeSet::new();
    let mut canceled = Vec::with_capacity(wire.canceled.len());
    for id in wire.canceled {
        let id = parse_order_id(&id, "canceled")?;
        if !seen.insert(id) {
            return Err(PmLiveWireError::DuplicateIdentity);
        }
        canceled.push(id);
    }
    let mut not_canceled = Vec::with_capacity(wire.not_canceled.0.len());
    for (id, reason) in wire.not_canceled.0 {
        let id = parse_order_id(&id, "not_canceled")?;
        if !seen.insert(id) {
            return Err(PmLiveWireError::DuplicateIdentity);
        }
        not_canceled.push((id, bounded(reason, "not_canceled.reason")?));
    }
    Ok(PmLiveCancelResult {
        canceled: canceled.into_boxed_slice(),
        not_canceled: not_canceled.into_boxed_slice(),
    })
}

fn check_body(raw: &[u8]) -> Result<(), PmLiveWireError> {
    if raw.len() > MAX_PM_LIVE_BODY_BYTES {
        Err(PmLiveWireError::PayloadTooLarge)
    } else {
        Ok(())
    }
}

fn check_items(len: usize, maximum: usize) -> Result<(), PmLiveWireError> {
    if len > maximum {
        Err(PmLiveWireError::TooManyItems)
    } else {
        Ok(())
    }
}

fn validate_page_metadata(
    data_len: usize,
    limit: usize,
    count: usize,
) -> Result<(), PmLiveWireError> {
    if limit == 0 || data_len > limit {
        return Err(PmLiveWireError::InvalidPageMetadata);
    }
    if limit > MAX_PM_LIVE_DECLARED_PAGE_LIMIT {
        return Err(PmLiveWireError::UnsupportedPageLimit { observed: limit });
    }
    if count != data_len {
        return Err(PmLiveWireError::PageCountMismatch {
            declared: count,
            actual: data_len,
        });
    }
    Ok(())
}

fn bounded(value: String, field: &'static str) -> Result<String, PmLiveWireError> {
    if value.is_empty() {
        return Err(PmLiveWireError::EmptyField(field));
    }
    if value.len() > MAX_PM_LIVE_EVIDENCE_BYTES {
        return Err(PmLiveWireError::FieldTooLong(field));
    }
    if !value.is_ascii() || value.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(PmLiveWireError::InvalidIdentity(field));
    }
    Ok(value)
}

fn optional_bounded(
    value: Option<String>,
    field: &'static str,
) -> Result<Option<String>, PmLiveWireError> {
    value.map(|value| bounded(value, field)).transpose()
}

fn optional_empty_bounded(
    value: Option<String>,
    field: &'static str,
) -> Result<Option<String>, PmLiveWireError> {
    match value {
        None => Ok(None),
        Some(value) if value.is_empty() => Ok(None),
        Some(value) => bounded(value, field).map(Some),
    }
}

fn parse_condition(value: &str) -> Result<PmConditionId, PmLiveWireError> {
    PmConditionId::parse(value).map_err(|_| PmLiveWireError::InvalidIdentity("market"))
}

fn parse_token(value: &str) -> Result<PmTokenId, PmLiveWireError> {
    let units = value
        .parse::<U256>()
        .map_err(|_| PmLiveWireError::InvalidNumeric("asset_id"))?;
    PmTokenId::new(units).map_err(|_| PmLiveWireError::InvalidIdentity("asset_id"))
}

fn parse_order_id(value: &str, field: &'static str) -> Result<PmVenueOrderId, PmLiveWireError> {
    let Some(digits) = value.strip_prefix("0x") else {
        return Err(PmLiveWireError::InvalidIdentity(field));
    };
    if digits.len() != 64 || !digits.bytes().all(is_lower_hex) {
        return Err(PmLiveWireError::InvalidIdentity(field));
    }
    PmVenueOrderId::new(value).map_err(|_| PmLiveWireError::InvalidIdentity(field))
}

fn parse_credential_owner(
    value: RawCredentialOwner,
    field: &'static str,
) -> Result<PmCredentialOwner, PmLiveWireError> {
    let value = value.0;
    let bytes = value.as_slice();
    if bytes.len() != 36
        || bytes.iter().enumerate().any(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte != b'-',
            _ => !is_lower_hex(*byte),
        })
    {
        return Err(PmLiveWireError::InvalidIdentity(field));
    }
    Ok(PmCredentialOwner(value))
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

fn parse_u256(value: &str, field: &'static str) -> Result<U256, PmLiveWireError> {
    value
        .parse()
        .map_err(|_| PmLiveWireError::InvalidNumeric(field))
}

fn parse_optional_fee_rate_bps(
    value: Option<String>,
    field: &'static str,
) -> Result<Option<U256>, PmLiveWireError> {
    value
        .map(|value| {
            if value.is_empty()
                || (value.len() > 1 && value.starts_with('0'))
                || !value.bytes().all(|byte| byte.is_ascii_digit())
            {
                return Err(PmLiveWireError::InvalidNumeric(field));
            }
            parse_u256(&value, field)
        })
        .transpose()
}

fn parse_side(value: &str) -> Result<PmOrderSide, PmLiveWireError> {
    match value {
        "BUY" => Ok(PmOrderSide::Buy),
        "SELL" => Ok(PmOrderSide::Sell),
        _ => Err(PmLiveWireError::UnsupportedSide),
    }
}

fn parse_cursor(value: String) -> Result<(Option<PmLiveCursor>, bool), PmLiveWireError> {
    match value {
        value if value == TERMINAL_CURSOR => Ok((None, true)),
        value => Ok((Some(PmLiveCursor::new(value)?), false)),
    }
}

impl TryFrom<RawOrder> for PmLiveOrder {
    type Error = PmLiveWireError;
    fn try_from(wire: RawOrder) -> Result<Self, Self::Error> {
        let owner = parse_credential_owner(wire.owner, "owner")?;
        Ok(Self {
            id: parse_order_id(&wire.id, "id")?,
            condition: parse_condition(&wire.market)?,
            token: parse_token(&wire.asset_id)?,
            side: parse_side(&wire.side)?,
            original_size: PmQuantity::parse_decimal(&wire.original_size)
                .map_err(|_| PmLiveWireError::InvalidNumeric("original_size"))?,
            size_matched: PmBookQuantity::parse_decimal(&wire.size_matched)
                .map_err(|_| PmLiveWireError::InvalidNumeric("size_matched"))?,
            price: PmPrice::parse_decimal(&wire.price)
                .map_err(|_| PmLiveWireError::InvalidNumeric("price"))?,
            status: bounded(wire.status, "status")?,
            maker: EvmAddress::parse(&wire.maker_address)
                .map_err(|_| PmLiveWireError::InvalidIdentity("maker_address"))?,
            owner,
            created_at: parse_exact_u64(wire.created_at, "created_at", false)?,
            expiration: parse_decimal_u64(&wire.expiration, "expiration", true)?,
            outcome: optional_bounded(wire.outcome, "outcome")?,
            order_type: optional_bounded(wire.order_type, "order_type")?,
        })
    }
}

impl TryFrom<RawMakerOrder> for PmLiveMakerOrder {
    type Error = PmLiveWireError;
    fn try_from(wire: RawMakerOrder) -> Result<Self, Self::Error> {
        Self::from_raw(wire, true)
    }
}

impl PmLiveMakerOrder {
    fn from_raw(wire: RawMakerOrder, retain_fee_evidence: bool) -> Result<Self, PmLiveWireError> {
        let owner = parse_credential_owner(wire.owner, "maker_orders.owner")?;
        Ok(Self {
            order_id: parse_order_id(&wire.order_id, "maker_orders.order_id")?,
            token: parse_token(&wire.asset_id)?,
            side: parse_side(&wire.side)?,
            price: PmPrice::parse_decimal(&wire.price)
                .map_err(|_| PmLiveWireError::InvalidNumeric("maker_orders.price"))?,
            matched_amount: PmQuantity::parse_decimal(&wire.matched_amount)
                .map_err(|_| PmLiveWireError::InvalidNumeric("maker_orders.matched_amount"))?,
            fee_rate_bps: if retain_fee_evidence {
                parse_optional_fee_rate_bps(wire.fee_rate_bps, "maker_orders.fee_rate_bps")?
            } else {
                None
            },
            owner,
            maker: EvmAddress::parse(&wire.maker_address)
                .map_err(|_| PmLiveWireError::InvalidIdentity("maker_orders.maker_address"))?,
        })
    }
}

impl PmLiveTrade {
    fn from_core(
        wire: RawTradeCore,
        owner: PmCredentialOwner,
        last_update: Option<u64>,
        retain_fee_evidence: bool,
    ) -> Result<Self, PmLiveWireError> {
        check_items(wire.maker_orders.len(), MAX_PM_LIVE_MAKER_ORDERS)?;
        let maker_orders = wire
            .maker_orders
            .into_iter()
            .map(|maker| PmLiveMakerOrder::from_raw(maker, retain_fee_evidence))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            id: PmFillId::new(&wire.id).map_err(|_| PmLiveWireError::InvalidIdentity("id"))?,
            condition: parse_condition(&wire.market)?,
            token: parse_token(&wire.asset_id)?,
            side: parse_side(&wire.side)?,
            size: PmQuantity::parse_decimal(&wire.size)
                .map_err(|_| PmLiveWireError::InvalidNumeric("size"))?,
            price: PmPrice::parse_decimal(&wire.price)
                .map_err(|_| PmLiveWireError::InvalidNumeric("price"))?,
            status: bounded(wire.status, "status")?,
            order_id: wire
                .order_id
                .map(|value| parse_order_id(&value, "order_id"))
                .transpose()?,
            taker_order_id: wire
                .taker_order_id
                .map(|value| parse_order_id(&value, "taker_order_id"))
                .transpose()?,
            trader_side: optional_bounded(wire.trader_side, "trader_side")?,
            transaction_hash: optional_empty_bounded(wire.transaction_hash, "transaction_hash")?,
            fee_rate_bps: if retain_fee_evidence {
                parse_optional_fee_rate_bps(wire.fee_rate_bps, "fee_rate_bps")?
            } else {
                None
            },
            maker_orders: maker_orders.into_boxed_slice(),
            maker: None,
            owner,
            trade_owner: None,
            timestamp: None,
            match_time: None,
            last_update,
        })
    }
}

impl TryFrom<RawRestTrade> for PmLiveTrade {
    type Error = PmLiveWireError;

    fn try_from(wire: RawRestTrade) -> Result<Self, Self::Error> {
        Self::from_rest(wire, true)
    }
}

impl PmLiveTrade {
    fn from_rest(wire: RawRestTrade, retain_fee_evidence: bool) -> Result<Self, PmLiveWireError> {
        let owner = parse_credential_owner(wire.owner, "owner")?;
        let match_time = parse_timestamp(&wire.match_time, "match_time")?;
        let last_update = parse_timestamp(&wire.last_update, "last_update")?;
        let mut trade = Self::from_core(wire.trade, owner, Some(last_update), retain_fee_evidence)?;
        trade.maker = Some(
            EvmAddress::parse(&wire.maker_address)
                .map_err(|_| PmLiveWireError::InvalidIdentity("maker_address"))?,
        );
        trade.match_time = Some(match_time);
        Ok(trade)
    }
}

impl TryFrom<RawUserOrder> for PmLiveUserOrder {
    type Error = PmLiveWireError;

    fn try_from(wire: RawUserOrder) -> Result<Self, Self::Error> {
        let associate_trades = wire
            .associate_trades
            .map(parse_associate_trades)
            .transpose()?;
        Ok(Self {
            id: parse_order_id(&wire.id, "id")?,
            condition: parse_condition(&wire.market)?,
            token: parse_token(&wire.asset_id)?,
            side: parse_side(&wire.side)?,
            original_size: PmQuantity::parse_decimal(&wire.original_size)
                .map_err(|_| PmLiveWireError::InvalidNumeric("original_size"))?,
            size_matched: PmBookQuantity::parse_decimal(&wire.size_matched)
                .map_err(|_| PmLiveWireError::InvalidNumeric("size_matched"))?,
            price: PmPrice::parse_decimal(&wire.price)
                .map_err(|_| PmLiveWireError::InvalidNumeric("price"))?,
            event_kind: parse_user_order_event_kind(wire.event_kind)?,
            maker: wire
                .maker_address
                .map(|value| {
                    EvmAddress::parse(&value)
                        .map_err(|_| PmLiveWireError::InvalidIdentity("maker_address"))
                })
                .transpose()?,
            expiration: wire
                .expiration
                .map(|value| parse_decimal_u64(&value, "expiration", true))
                .transpose()?,
            order_type: wire.order_type.map(parse_user_order_type).transpose()?,
            outcome: optional_bounded(wire.outcome, "outcome")?,
            status: wire.status.map(parse_user_order_status).transpose()?,
            created_at: wire
                .created_at
                .map(|value| parse_decimal_u64(&value, "created_at", false))
                .transpose()?,
            associate_trades,
            owner: parse_credential_owner(wire.owner, "owner")?,
            order_owner: wire
                .order_owner
                .map(|owner| parse_credential_owner(owner, "order_owner"))
                .transpose()?,
            timestamp: wire
                .timestamp
                .map(|value| parse_timestamp(&value, "timestamp"))
                .transpose()?,
        })
    }
}

fn parse_associate_trades(raw: RawAssociateTrades) -> Result<Box<[PmFillId]>, PmLiveWireError> {
    let values = raw.0;
    check_items(values.len(), MAX_PM_LIVE_RESULT_ITEMS)?;
    let mut seen = BTreeSet::new();
    let mut parsed = Vec::with_capacity(values.len());
    for value in values {
        let id = PmFillId::new(&value)
            .map_err(|_| PmLiveWireError::InvalidIdentity("associate_trades"))?;
        if !seen.insert(id) {
            return Err(PmLiveWireError::DuplicateIdentity);
        }
        parsed.push(id);
    }
    parsed.sort_unstable();
    Ok(parsed.into_boxed_slice())
}

fn parse_user_order_event_kind(value: String) -> Result<String, PmLiveWireError> {
    match value.as_str() {
        "PLACEMENT" | "UPDATE" | "CANCELLATION" => Ok(value),
        _ => Err(PmLiveWireError::InvalidIdentity("type")),
    }
}

fn parse_user_order_type(value: String) -> Result<String, PmLiveWireError> {
    match value.as_str() {
        "GTC" | "FOK" | "IOC" | "GTD" | "FAK" => Ok(value),
        _ => Err(PmLiveWireError::InvalidIdentity("order_type")),
    }
}

fn parse_user_order_status(value: String) -> Result<String, PmLiveWireError> {
    match value.as_str() {
        "LIVE" | "MATCHED" | "DELAYED" | "UNMATCHED" | "CANCELED" => Ok(value),
        _ => Err(PmLiveWireError::InvalidIdentity("status")),
    }
}

impl TryFrom<RawUserTrade> for PmLiveTrade {
    type Error = PmLiveWireError;

    fn try_from(wire: RawUserTrade) -> Result<Self, Self::Error> {
        let owner = parse_credential_owner(wire.owner, "owner")?;
        let trade_owner = wire
            .trade_owner
            .map(|owner| parse_credential_owner(owner, "trade_owner"))
            .transpose()?;
        let timestamp = wire
            .timestamp
            .map(|value| parse_timestamp(&value, "timestamp"))
            .transpose()?;
        let last_update = wire
            .last_update
            .map(|value| parse_timestamp(&value, "last_update"))
            .transpose()?;
        let match_time = wire
            .match_time
            .map(|value| parse_timestamp(&value, "matchtime"))
            .transpose()?;
        let maker = wire
            .maker_address
            .map(|value| {
                EvmAddress::parse(&value)
                    .map_err(|_| PmLiveWireError::InvalidIdentity("maker_address"))
            })
            .transpose()?;
        let mut trade = Self::from_core(wire.trade, owner, last_update, true)?;
        trade.maker = maker;
        trade.trade_owner = trade_owner;
        trade.timestamp = timestamp;
        trade.match_time = match_time;
        Ok(trade)
    }
}

fn parse_timestamp(value: &str, field: &'static str) -> Result<u64, PmLiveWireError> {
    parse_decimal_u64(value, field, false)
}

fn parse_decimal_u64(
    value: &str,
    field: &'static str,
    allow_zero: bool,
) -> Result<u64, PmLiveWireError> {
    value
        .parse::<u64>()
        .ok()
        .filter(|value| allow_zero || *value > 0)
        .ok_or(PmLiveWireError::InvalidNumeric(field))
}

fn parse_exact_u64(
    value: RawExactU64,
    field: &'static str,
    allow_zero: bool,
) -> Result<u64, PmLiveWireError> {
    match value {
        RawExactU64::Text(value) => parse_decimal_u64(&value, field, allow_zero),
        RawExactU64::Integer(value) if allow_zero || value > 0 => Ok(value),
        RawExactU64::Integer(_) => Err(PmLiveWireError::InvalidNumeric(field)),
    }
}

impl TryFrom<RawUserEvent> for PmLiveUserEvent {
    type Error = PmLiveWireError;
    fn try_from(wire: RawUserEvent) -> Result<Self, Self::Error> {
        match wire {
            RawUserEvent::Order(order) => {
                Ok(Self::Order(Box::new(PmLiveUserOrder::try_from(order)?)))
            }
            RawUserEvent::Trade(trade) => Ok(Self::Trade(Box::new(PmLiveTrade::try_from(trade)?))),
        }
    }
}

struct RawCredentialOwner(Zeroizing<Vec<u8>>);

struct RawCredentialOwnerVisitor;

impl<'de> Visitor<'de> for RawCredentialOwnerVisitor {
    type Value = RawCredentialOwner;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a credential-owner UUID string")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(RawCredentialOwner(Zeroizing::new(
            value.as_bytes().to_vec(),
        )))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(RawCredentialOwner(Zeroizing::new(value.into_bytes())))
    }
}

impl<'de> Deserialize<'de> for RawCredentialOwner {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_string(RawCredentialOwnerVisitor)
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawExactU64 {
    Text(String),
    Integer(u64),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawUserFrame {
    One(Box<RawUserEvent>),
    Many(Vec<RawUserEvent>),
}

#[derive(Deserialize)]
#[serde(tag = "event_type")]
enum RawUserEvent {
    #[serde(rename = "order")]
    Order(RawUserOrder),
    #[serde(rename = "trade")]
    Trade(RawUserTrade),
}

#[derive(Deserialize)]
struct RawUserOrder {
    id: String,
    market: String,
    asset_id: String,
    side: String,
    original_size: String,
    size_matched: String,
    price: String,
    #[serde(rename = "type")]
    event_kind: String,
    #[serde(default)]
    maker_address: Option<String>,
    #[serde(default)]
    expiration: Option<String>,
    #[serde(default)]
    order_type: Option<String>,
    #[serde(default)]
    outcome: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    associate_trades: Option<RawAssociateTrades>,
    owner: RawCredentialOwner,
    #[serde(default)]
    order_owner: Option<RawCredentialOwner>,
    #[serde(default)]
    timestamp: Option<String>,
}

#[derive(Deserialize)]
struct RawUserTrade {
    #[serde(flatten)]
    trade: RawTradeCore,
    owner: RawCredentialOwner,
    #[serde(default)]
    trade_owner: Option<RawCredentialOwner>,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    last_update: Option<String>,
    #[serde(default, rename = "matchtime", alias = "match_time")]
    match_time: Option<String>,
    #[serde(default)]
    maker_address: Option<String>,
}

#[derive(Deserialize)]
struct RawOrder {
    id: String,
    market: String,
    asset_id: String,
    side: String,
    original_size: String,
    size_matched: String,
    price: String,
    status: String,
    maker_address: String,
    owner: RawCredentialOwner,
    expiration: String,
    created_at: RawExactU64,
    #[serde(default)]
    outcome: Option<String>,
    #[serde(default)]
    order_type: Option<String>,
}

#[derive(Deserialize)]
struct RawTradeCore {
    id: String,
    market: String,
    asset_id: String,
    side: String,
    size: String,
    price: String,
    status: String,
    #[serde(default)]
    order_id: Option<String>,
    #[serde(default)]
    taker_order_id: Option<String>,
    #[serde(default)]
    trader_side: Option<String>,
    #[serde(default)]
    transaction_hash: Option<String>,
    #[serde(default)]
    fee_rate_bps: Option<String>,
    #[serde(default)]
    maker_orders: Vec<RawMakerOrder>,
}

#[derive(Deserialize)]
struct RawRestTrade {
    #[serde(flatten)]
    trade: RawTradeCore,
    maker_address: String,
    owner: RawCredentialOwner,
    match_time: String,
    last_update: String,
}

#[derive(Deserialize)]
struct RawMakerOrder {
    order_id: String,
    asset_id: String,
    side: String,
    price: String,
    matched_amount: String,
    #[serde(default)]
    fee_rate_bps: Option<String>,
    owner: RawCredentialOwner,
    maker_address: String,
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
struct RawPage<T> {
    data: Vec<T>,
    next_cursor: String,
    limit: usize,
    count: usize,
}

#[derive(Deserialize)]
struct RawBalanceAllowance {
    balance: String,
    #[serde(default)]
    allowance: Option<serde_json::Value>,
    #[serde(default)]
    allowances: Option<BTreeMap<String, String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONDITION: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
    const QUESTION_MARKET: &str =
        "0x4444444444444444444444444444444444444444444444444444444444444444";
    const MAKER: &str = "0x2222222222222222222222222222222222222222";
    const SPENDER: &str = "0x3333333333333333333333333333333333333333";
    const ORDER_1: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const ORDER_2: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const API_OWNER: &str = "9180014b-33c8-9240-a14b-bdca11c0a465";

    fn order_json() -> String {
        format!(
            r#"{{"id":"{ORDER_1}","market":"{CONDITION}","asset_id":"123","side":"BUY","original_size":"10.000000","size_matched":"0","price":"0.420000","status":"LIVE","maker_address":"{MAKER}","owner":"{API_OWNER}","expiration":"0","created_at":1700000000}}"#
        )
    }

    fn trade_json() -> String {
        format!(
            r#"{{"id":"trade-1","market":"{CONDITION}","asset_id":"123","side":"SELL","size":"2.500000","price":"0.420000","status":"CONFIRMED","match_time":"1700000000002","last_update":"1700000000003","order_id":"{ORDER_1}","maker_orders":[],"maker_address":"{MAKER}","owner":"{API_OWNER}"}}"#
        )
    }

    fn user_order_json() -> String {
        format!(
            r#"{{"event_type":"order","id":"{ORDER_1}","owner":"{API_OWNER}","market":"{CONDITION}","asset_id":"123","side":"BUY","original_size":"10.000000","size_matched":"0","price":"0.420000","type":"PLACEMENT","order_owner":"{API_OWNER}","timestamp":"1700000000000","associate_trades":null,"outcome":"YES","created_at":"1700000000000","expiration":"0","order_type":"GTC","status":"LIVE","maker_address":"{MAKER}"}}"#
        )
    }

    fn user_trade_json() -> String {
        format!(
            r#"{{"event_type":"trade","type":"TRADE","id":"28c4d2eb-bbea-40e7-a9f0-b2fdb56b2c2e","owner":"{API_OWNER}","market":"{CONDITION}","asset_id":"123","side":"SELL","size":"2.500000","price":"0.420000","status":"MATCHED","taker_order_id":"{ORDER_1}","maker_orders":[{{"order_id":"{ORDER_2}","owner":"{API_OWNER}","maker_address":"{MAKER}","matched_amount":"2.500000","price":"0.420000","asset_id":"123","side":"BUY"}}],"trade_owner":"{API_OWNER}","maker_address":"{MAKER}","timestamp":"1700000000001","matchtime":"1700000000002","last_update":"1700000000003","transaction_hash":"","trader_side":"TAKER"}}"#
        )
    }

    #[test]
    fn parses_current_user_order_and_trade_array_without_floats() {
        let raw = format!(r#"[{},{}]"#, user_order_json(), user_trade_json());
        let frame = parse_live_user_frame(raw.as_bytes()).expect("live user frame");
        assert_eq!(frame.events().len(), 2);
        let PmLiveUserEvent::Order(order) = &frame.events()[0] else {
            panic!("order")
        };
        assert_eq!(order.price().to_string(), "0.42");
        assert_eq!(order.original_size().to_string(), "10");
        assert_eq!(order.event_kind(), "PLACEMENT");
        assert_eq!(order.maker(), Some(EvmAddress::parse(MAKER).unwrap()));
        assert_eq!(order.expiration(), Some(0));
        assert_eq!(order.order_type(), Some("GTC"));
        assert_eq!(order.outcome(), Some("YES"));
        assert_eq!(order.status(), Some("LIVE"));
        assert_eq!(order.created_at(), Some(1_700_000_000_000));
        assert!(order.associate_trades().is_none());
        assert!(order.owner().matches_exact(API_OWNER));
        assert!(
            order
                .order_owner()
                .is_some_and(|owner| owner.matches_exact(API_OWNER))
        );
        assert_eq!(order.timestamp(), Some(1_700_000_000_000));
        let PmLiveUserEvent::Trade(trade) = &frame.events()[1] else {
            panic!("trade")
        };
        assert!(trade.owner().matches_exact(API_OWNER));
        assert!(
            trade
                .trade_owner()
                .is_some_and(|owner| owner.matches_exact(API_OWNER))
        );
        assert_eq!(trade.match_time(), Some(1_700_000_000_002));
        assert_eq!(trade.last_update(), Some(1_700_000_000_003));
        assert_eq!(trade.maker_orders().len(), 1);
        assert!(trade.maker_orders()[0].owner().matches_exact(API_OWNER));
    }

    #[test]
    fn authenticated_market_field_is_the_condition_not_the_question_market() {
        let expected = PmConditionId::parse(CONDITION).expect("condition");
        let question = reap_pm_core::PmMarketId::parse(QUESTION_MARKET).expect("question market");

        let order = parse_live_order_detail(order_json().as_bytes()).expect("order detail");
        let _: PmConditionId = order.condition();
        assert_eq!(order.condition(), expected);
        assert_ne!(order.condition().bytes(), question.bytes());

        let trade_page = format!(
            r#"{{"data":[{}],"next_cursor":"LTE=","limit":128,"count":1}}"#,
            trade_json()
        );
        let trades = parse_live_trade_page(trade_page.as_bytes()).expect("trade page");
        let _: PmConditionId = trades.trades()[0].condition();
        assert_eq!(trades.trades()[0].condition(), expected);

        let frame = parse_live_user_frame(user_order_json().as_bytes()).expect("user order");
        let PmLiveUserEvent::Order(user_order) = &frame.events()[0] else {
            panic!("user order")
        };
        let _: PmConditionId = user_order.condition();
        assert_eq!(user_order.condition(), expected);
    }

    #[test]
    fn current_raw_user_order_retains_but_does_not_require_optional_profile_facts() {
        let cases = [
            (",\"outcome\":\"YES\"", "outcome"),
            (",\"created_at\":\"1700000000000\"", "created_at"),
            (",\"expiration\":\"0\"", "expiration"),
            (",\"order_type\":\"GTC\"", "order_type"),
            (",\"status\":\"LIVE\"", "status"),
            (",\"associate_trades\":null", "associate_trades"),
            (&format!(",\"maker_address\":\"{MAKER}\""), "maker_address"),
        ];
        for (field, name) in cases {
            let missing = user_order_json().replace(field, "");
            let parsed = parse_live_user_frame(missing.as_bytes());
            assert!(parsed.is_ok(), "optional {name} must remain parseable");
        }

        let missing_aliases = user_order_json()
            .replace(&format!(",\"order_owner\":\"{API_OWNER}\""), "")
            .replace(",\"timestamp\":\"1700000000000\"", "");
        let parsed = parse_live_user_frame(missing_aliases.as_bytes()).unwrap();
        let [PmLiveUserEvent::Order(order)] = parsed.events() else {
            panic!("one order")
        };
        assert!(order.order_owner().is_none());
        assert!(order.timestamp().is_none());
    }

    #[test]
    fn current_raw_user_order_retains_unique_associated_trade_ids() {
        let raw = user_order_json().replace(
            "\"associate_trades\":null",
            "\"associate_trades\":[\"trade-a\",\"trade-b\"]",
        );
        let frame = parse_live_user_frame(raw.as_bytes()).expect("associated trades");
        let [PmLiveUserEvent::Order(order)] = frame.events() else {
            panic!("one order")
        };
        let associated = order.associate_trades().expect("present association set");
        assert_eq!(associated[0].as_str(), "trade-a");
        assert_eq!(associated[1].as_str(), "trade-b");

        let reversed = user_order_json().replace(
            "\"associate_trades\":null",
            "\"associate_trades\":[\"trade-b\",\"trade-a\"]",
        );
        let reversed = parse_live_user_frame(reversed.as_bytes()).unwrap();
        let [PmLiveUserEvent::Order(reversed)] = reversed.events() else {
            panic!("one order")
        };
        assert_eq!(reversed.associate_trades(), Some(associated));

        let duplicate = user_order_json().replace(
            "\"associate_trades\":null",
            "\"associate_trades\":[\"trade-a\",\"trade-a\"]",
        );
        assert_eq!(
            parse_live_user_frame(duplicate.as_bytes()),
            Err(PmLiveWireError::DuplicateIdentity)
        );
        let malformed = user_order_json().replace(
            "\"associate_trades\":null",
            "\"associate_trades\":\"trade-a\"",
        );
        assert_eq!(
            parse_live_user_frame(malformed.as_bytes()),
            Err(PmLiveWireError::MalformedJson)
        );
    }

    #[test]
    fn duplicate_documented_user_order_keys_fail_closed() {
        let duplicate = user_order_json().replace(
            &format!("\"id\":\"{ORDER_1}\""),
            &format!("\"id\":\"{ORDER_1}\",\"id\":\"{ORDER_2}\""),
        );
        assert_eq!(
            parse_live_user_frame(duplicate.as_bytes()),
            Err(PmLiveWireError::MalformedJson)
        );
    }

    #[test]
    fn current_raw_user_order_rejects_wrong_fact_grammar() {
        for (from, to, expected) in [
            (
                "\"type\":\"PLACEMENT\"",
                "\"type\":\"placed\"",
                PmLiveWireError::InvalidIdentity("type"),
            ),
            (
                "\"order_type\":\"GTC\"",
                "\"order_type\":\"gtc\"",
                PmLiveWireError::InvalidIdentity("order_type"),
            ),
            (
                "\"status\":\"LIVE\"",
                "\"status\":\"live\"",
                PmLiveWireError::InvalidIdentity("status"),
            ),
            (
                "\"created_at\":\"1700000000000\"",
                "\"created_at\":\"1700000000000.5\"",
                PmLiveWireError::InvalidNumeric("created_at"),
            ),
            (
                "\"expiration\":\"0\"",
                "\"expiration\":\"-1\"",
                PmLiveWireError::InvalidNumeric("expiration"),
            ),
        ] {
            let malformed = user_order_json().replace(from, to);
            assert_eq!(parse_live_user_frame(malformed.as_bytes()), Err(expected));
        }
    }

    #[test]
    fn current_raw_trade_accepts_matched_not_broadcasted() {
        let raw = user_trade_json().replace(
            "\"status\":\"MATCHED\"",
            "\"status\":\"MATCHED_NOT_BROADCASTED\"",
        );
        let frame = parse_live_user_frame(raw.as_bytes()).expect("pre-broadcast trade");
        let [PmLiveUserEvent::Trade(trade)] = frame.events() else {
            panic!("one trade")
        };
        assert_eq!(trade.status(), "MATCHED_NOT_BROADCASTED");
    }

    #[test]
    fn live_trade_fee_rates_preserve_exact_role_evidence_and_reject_noncanonical_values() {
        let parse_user = |top_level: Option<serde_json::Value>,
                          maker: Option<serde_json::Value>| {
            let mut value = serde_json::from_str::<serde_json::Value>(&user_trade_json()).unwrap();
            if let Some(top_level) = top_level {
                value["fee_rate_bps"] = top_level;
            }
            if let Some(maker) = maker {
                value["maker_orders"][0]["fee_rate_bps"] = maker;
            }
            let raw = serde_json::to_vec(&value).unwrap();
            parse_live_user_frame(&raw)
        };

        let omitted = parse_user(None, None).expect("omitted fee evidence");
        let [PmLiveUserEvent::Trade(omitted)] = omitted.events() else {
            panic!("one trade")
        };
        assert_eq!(omitted.fee_rate_bps(), None);
        assert_eq!(omitted.maker_orders()[0].fee_rate_bps(), None);

        let zero_and_nonzero = parse_user(
            Some(serde_json::Value::String("0".into())),
            Some(serde_json::Value::String("25".into())),
        )
        .expect("canonical fee rates");
        let [PmLiveUserEvent::Trade(trade)] = zero_and_nonzero.events() else {
            panic!("one trade")
        };
        assert_eq!(trade.fee_rate_bps(), Some(U256::ZERO));
        assert_eq!(
            trade.maker_orders()[0].fee_rate_bps(),
            Some(U256::from_u64(25))
        );

        let mut rest = serde_json::from_str::<serde_json::Value>(&trade_json()).unwrap();
        rest["fee_rate_bps"] = serde_json::Value::String("0".into());
        let page = serde_json::json!({
            "data": [rest],
            "next_cursor": "LTE=",
            "limit": 128,
            "count": 1
        });
        let page =
            parse_live_trade_page(&serde_json::to_vec(&page).unwrap()).expect("REST zero fee rate");
        assert_eq!(page.trades()[0].fee_rate_bps(), Some(U256::ZERO));

        let mut fractional = serde_json::from_str::<serde_json::Value>(&trade_json()).unwrap();
        fractional["maker_orders"] = serde_json::json!([{
            "order_id": ORDER_2,
            "asset_id": "123",
            "side": "BUY",
            "price": "0.420000",
            "matched_amount": "2.500000",
            "fee_rate_bps": "12.5",
            "owner": API_OWNER,
            "maker_address": MAKER,
        }]);
        let fractional_page = serde_json::json!({
            "data": [fractional],
            "next_cursor": "LTE=",
            "limit": 300,
            "count": 1
        });
        let fractional_bytes = serde_json::to_vec(&fractional_page).unwrap();
        assert_eq!(
            parse_live_trade_page(&fractional_bytes),
            Err(PmLiveWireError::InvalidNumeric("maker_orders.fee_rate_bps"))
        );
        let owned = parse_live_owned_fill_trade_page(&fractional_bytes)
            .expect("owned-fill projection ignores unrepresentable fee text");
        assert_eq!(owned.trades()[0].fee_rate_bps(), None);
        assert_eq!(owned.trades()[0].maker_orders()[0].fee_rate_bps(), None);

        assert_eq!(
            parse_user(Some(serde_json::json!(0)), None),
            Err(PmLiveWireError::MalformedJson),
        );
        for malformed in ["", "00", "0.0", "-1", "+1"] {
            assert_eq!(
                parse_user(Some(serde_json::Value::String(malformed.into())), None),
                Err(PmLiveWireError::InvalidNumeric("fee_rate_bps")),
                "present fee rate `{malformed}` must be canonical decimal text",
            );
        }
        let overflow = "9".repeat(79);
        assert_eq!(
            parse_user(Some(serde_json::Value::String(overflow)), None),
            Err(PmLiveWireError::InvalidNumeric("fee_rate_bps")),
        );
        assert_eq!(
            parse_user(None, Some(serde_json::Value::String("00".into()))),
            Err(PmLiveWireError::InvalidNumeric("maker_orders.fee_rate_bps")),
        );
    }

    #[test]
    fn parses_pages_and_terminal_or_continuation_cursor() {
        let first = format!(
            r#"{{"data":[{}],"next_cursor":"cursor-2","limit":128,"count":1}}"#,
            order_json()
        );
        let page = parse_live_open_order_page(first.as_bytes()).expect("first page");
        assert!(!page.terminal());
        assert_eq!(page.next_cursor().expect("cursor").as_str(), "cursor-2");
        assert_eq!(page.declared_limit(), 128);
        assert_eq!(page.declared_count(), 1);
        assert_eq!(page.orders()[0].created_at(), 1_700_000_000);
        assert_eq!(page.orders()[0].expiration(), 0);

        let terminal = format!(
            r#"{{"data":[{}],"next_cursor":"LTE=","limit":128,"count":1}}"#,
            trade_json()
        );
        let page = parse_live_trade_page(terminal.as_bytes()).expect("terminal page");
        assert!(page.terminal());
        assert!(page.next_cursor().is_none());
        assert_eq!(page.trades()[0].match_time(), Some(1_700_000_000_002));
        assert_eq!(page.trades()[0].last_update(), Some(1_700_000_000_003));
    }

    #[test]
    fn rest_trades_require_account_scope_identity_evidence() {
        let terminal = |trade: &str| {
            format!(r#"{{"data":[{trade}],"next_cursor":"LTE=","limit":128,"count":1}}"#)
        };
        let missing_maker = trade_json().replace(&format!(r#","maker_address":"{MAKER}""#), "");
        assert_eq!(
            parse_live_trade_page(terminal(&missing_maker).as_bytes()),
            Err(PmLiveWireError::MalformedJson)
        );

        let missing_owner = trade_json().replace(&format!(r#","owner":"{API_OWNER}""#), "");
        assert_eq!(
            parse_live_trade_page(terminal(&missing_owner).as_bytes()),
            Err(PmLiveWireError::MalformedJson)
        );

        let page = parse_live_trade_page(terminal(&trade_json()).as_bytes())
            .expect("scoped full-account trade page");
        assert_eq!(
            page.trades()[0].maker(),
            Some(EvmAddress::parse(MAKER).unwrap())
        );
        assert!(page.trades()[0].owner().matches_exact(API_OWNER));
    }

    #[test]
    fn credential_owners_are_exact_and_redacted() {
        let frame = parse_live_user_frame(user_order_json().as_bytes()).expect("user order");
        let debug = format!("{frame:?}");
        assert!(!debug.contains(API_OWNER));
        assert!(debug.contains("PmCredentialOwner([REDACTED])"));

        let malformed = user_order_json().replace(API_OWNER, "not-a-credential-owner");
        assert_eq!(
            parse_live_user_frame(malformed.as_bytes()),
            Err(PmLiveWireError::InvalidIdentity("owner"))
        );

        let uppercase = user_order_json().replace(API_OWNER, &API_OWNER.to_ascii_uppercase());
        assert_eq!(
            parse_live_user_frame(uppercase.as_bytes()),
            Err(PmLiveWireError::InvalidIdentity("owner"))
        );

        let uppercase_order = order_json().replace(ORDER_1, &ORDER_1.to_ascii_uppercase());
        assert_eq!(
            parse_live_order_detail(uppercase_order.as_bytes()),
            Err(PmLiveWireError::InvalidIdentity("id"))
        );
    }

    #[test]
    fn required_source_timestamps_fail_closed() {
        let missing_created = order_json().replace(",\"created_at\":1700000000", "");
        assert_eq!(
            parse_live_order_detail(missing_created.as_bytes()),
            Err(PmLiveWireError::MalformedJson)
        );
        let missing_expiration = order_json().replace(",\"expiration\":\"0\"", "");
        assert_eq!(
            parse_live_order_detail(missing_expiration.as_bytes()),
            Err(PmLiveWireError::MalformedJson)
        );
        let float_created = order_json().replace("1700000000}", "1700000000.5}");
        assert_eq!(
            parse_live_order_detail(float_created.as_bytes()),
            Err(PmLiveWireError::MalformedJson)
        );

        let missing_match = trade_json().replace("\"match_time\":\"1700000000002\",", "");
        let page = |trade: &str| {
            format!(r#"{{"data":[{trade}],"next_cursor":"LTE=","limit":128,"count":1}}"#)
        };
        assert_eq!(
            parse_live_trade_page(page(&missing_match).as_bytes()),
            Err(PmLiveWireError::MalformedJson)
        );
        let bad_update = trade_json().replace(
            "\"last_update\":\"1700000000003\"",
            "\"last_update\":\"unknown\"",
        );
        assert_eq!(
            parse_live_trade_page(page(&bad_update).as_bytes()),
            Err(PmLiveWireError::InvalidNumeric("last_update"))
        );

        let missing_user_update =
            user_trade_json().replace(",\"last_update\":\"1700000000003\"", "");
        let frame = parse_live_user_frame(missing_user_update.as_bytes())
            .expect("user update may omit last-update evidence");
        let PmLiveUserEvent::Trade(trade) = &frame.events()[0] else {
            panic!("trade")
        };
        assert_eq!(trade.last_update(), None);
        let malformed_user_update = user_trade_json().replace(
            "\"last_update\":\"1700000000003\"",
            "\"last_update\":\"unknown\"",
        );
        assert_eq!(
            parse_live_user_frame(malformed_user_update.as_bytes()),
            Err(PmLiveWireError::InvalidNumeric("last_update"))
        );
        let without_optional_match =
            user_trade_json().replace(",\"matchtime\":\"1700000000002\"", "");
        let frame = parse_live_user_frame(without_optional_match.as_bytes())
            .expect("user update without pre-match evidence");
        let PmLiveUserEvent::Trade(trade) = &frame.events()[0] else {
            panic!("trade")
        };
        assert_eq!(trade.match_time(), None);
    }

    #[test]
    fn exact_detail_rejects_float_numeric_fields() {
        let raw = order_json().replace("\"price\":\"0.420000\"", "\"price\":0.42");
        assert_eq!(
            parse_live_order_detail(raw.as_bytes()),
            Err(PmLiveWireError::MalformedJson)
        );
    }

    #[test]
    fn balance_requires_map_and_matches_only_exact_spender() {
        let raw = format!(
            r#"{{"balance":"1000000","allowance":"999","allowances":{{"{SPENDER}":"42"}}}}"#
        );
        let parsed = parse_live_balance_allowance(raw.as_bytes()).expect("balance");
        assert_eq!(parsed.balance(), U256::from_u64(1_000_000));
        assert!(parsed.unscoped_scalar_present());
        assert_eq!(
            parsed.exact_allowance(EvmAddress::parse(SPENDER).unwrap()),
            Some(U256::from_u64(42))
        );
        assert_eq!(
            parse_live_balance_allowance(br#"{"balance":"1","allowance":"2"}"#),
            Err(PmLiveWireError::MissingAllowanceMap)
        );
    }

    #[test]
    fn place_and_cancel_results_preserve_exact_identifiers() {
        let raw = format!(
            r#"{{"success":true,"orderID":"{ORDER_1}","status":"live","errorMsg":"","makingAmount":"420000","takingAmount":"1000000","tradeIDs":["trade-1"],"transactionsHashes":["0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"]}}"#
        );
        let place = parse_live_place_result(raw.as_bytes()).expect("place");
        assert!(place.success());
        assert_eq!(place.making_amount(), Some(U256::from_u64(420_000)));
        assert_eq!(place.transaction_hashes().len(), 1);
        let raw =
            format!(r#"{{"canceled":["{ORDER_1}"],"not_canceled":{{"{ORDER_2}":"not found"}}}}"#);
        let cancel = parse_live_cancel_result(raw.as_bytes()).expect("cancel");
        assert_eq!(cancel.canceled().len(), 1);
        assert_eq!(cancel.not_canceled().len(), 1);
    }

    #[test]
    fn place_result_is_an_exact_success_or_failure_union() {
        let failed = br#"{"success":false,"orderID":"","status":"","errorMsg":"insufficient balance","makingAmount":"","takingAmount":"","tradeIDs":[]}"#;
        let parsed = parse_live_place_result(failed).expect("documented failure");
        assert!(!parsed.success());
        assert!(parsed.order_id().is_none());
        assert_eq!(parsed.status(), "");
        assert_eq!(parsed.error_message(), Some("insufficient balance"));

        for contradictory in [
            format!(r#"{{"success":true,"orderID":"{ORDER_1}","status":"live","errorMsg":"failure","makingAmount":"1","takingAmount":"1","tradeIDs":[]}}"#),
            format!(r#"{{"success":false,"orderID":"{ORDER_1}","status":"","errorMsg":"failure","makingAmount":"","takingAmount":"","tradeIDs":[]}}"#),
            r#"{"success":false,"orderID":"","status":"","errorMsg":"","makingAmount":"","takingAmount":"","tradeIDs":[]}"#.into(),
        ] {
            assert!(parse_live_place_result(contradictory.as_bytes()).is_err());
        }

        let delayed = format!(
            r#"{{"success":true,"orderID":"{ORDER_1}","status":"delayed","errorMsg":"","makingAmount":"","takingAmount":"","tradeIDs":[]}}"#
        );
        let delayed = parse_live_place_result(delayed.as_bytes()).expect("delayed success");
        assert_eq!(delayed.status(), "delayed");
        assert_eq!(delayed.making_amount(), None);
        assert_eq!(delayed.taking_amount(), None);

        let v2_sdk_spelling = format!(
            r#"{{"success":true,"orderID":"{ORDER_1}","status":"live","errorMsg":"","makingAmount":"1","takingAmount":"1","tradeIds":[]}}"#
        );
        assert!(parse_live_place_result(v2_sdk_spelling.as_bytes()).is_ok());

        let without_optional_execution_ids = format!(
            r#"{{"success":true,"orderID":"{ORDER_1}","status":"live","errorMsg":"","makingAmount":"1","takingAmount":"1"}}"#
        );
        assert!(parse_live_place_result(without_optional_execution_ids.as_bytes()).is_ok());

        let duplicate_aliases = format!(
            r#"{{"success":true,"orderID":"{ORDER_1}","status":"live","errorMsg":"","makingAmount":"1","takingAmount":"1","tradeIDs":[],"tradeIds":[]}}"#
        );
        assert_eq!(
            parse_live_place_result(duplicate_aliases.as_bytes()),
            Err(PmLiveWireError::MalformedJson)
        );
    }

    #[test]
    fn cancel_result_requires_both_collections_and_rejects_duplicate_map_keys() {
        let missing = format!(r#"{{"canceled":["{ORDER_1}"]}}"#);
        assert_eq!(
            parse_live_cancel_result(missing.as_bytes()),
            Err(PmLiveWireError::MalformedJson)
        );
        let duplicate = format!(
            r#"{{"canceled":[],"not_canceled":{{"{ORDER_1}":"first","{ORDER_1}":"second"}}}}"#
        );
        assert_eq!(
            parse_live_cancel_result(duplicate.as_bytes()),
            Err(PmLiveWireError::MalformedJson)
        );
    }

    #[test]
    fn malformed_foreign_and_oversized_payloads_fail_closed() {
        assert_eq!(
            parse_live_user_frame(br#"{"event_type":"balance","balance":"1"}"#),
            Err(PmLiveWireError::MalformedJson)
        );
        assert_eq!(
            parse_live_user_frame(b"{"),
            Err(PmLiveWireError::MalformedJson)
        );
        let oversized = vec![b' '; MAX_PM_LIVE_BODY_BYTES + 1];
        assert_eq!(
            parse_live_open_order_page(&oversized),
            Err(PmLiveWireError::PayloadTooLarge)
        );
    }

    #[test]
    fn page_item_and_cursor_bounds_are_enforced() {
        let items = std::iter::repeat_with(order_json)
            .take(MAX_PM_LIVE_PAGE_ITEMS + 1)
            .collect::<Vec<_>>()
            .join(",");
        let raw = format!(
            r#"{{"data":[{items}],"next_cursor":"LTE=","limit":128,"count":{}}}"#,
            MAX_PM_LIVE_PAGE_ITEMS + 1
        );
        assert_eq!(
            parse_live_open_order_page(raw.as_bytes()),
            Err(PmLiveWireError::TooManyItems)
        );
        let raw = format!(
            r#"{{"data":[],"next_cursor":"{}","limit":128,"count":0}}"#,
            "x".repeat(MAX_PM_LIVE_CURSOR_BYTES + 1)
        );
        assert_eq!(
            parse_live_trade_page(raw.as_bytes()),
            Err(PmLiveWireError::InvalidCursor)
        );

        assert_eq!(
            parse_live_trade_page(br#"{"data":[],"next_cursor":"LTE=","limit":301,"count":0}"#),
            Err(PmLiveWireError::UnsupportedPageLimit { observed: 301 })
        );
        assert!(
            parse_live_trade_page(br#"{"data":[],"next_cursor":"LTE=","limit":300,"count":0}"#)
                .is_ok()
        );
        assert_eq!(
            parse_live_trade_page(br#"{"data":[],"next_cursor":"LTE=","limit":128,"count":1}"#),
            Err(PmLiveWireError::PageCountMismatch {
                declared: 1,
                actual: 0,
            })
        );
        assert_eq!(
            parse_live_trade_page(br#"{"data":[],"limit":128,"count":0}"#),
            Err(PmLiveWireError::MalformedJson)
        );
    }
}
