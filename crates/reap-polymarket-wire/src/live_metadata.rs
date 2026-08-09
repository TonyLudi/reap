//! Exact, source-specific parsing for the two live CLOB metadata responses.
//!
//! Protocol authority is pinned to `Polymarket/clob-client-v2` revision
//! `f3e1a05f868a1fd0c34ef85dfc45c6ce78f5bb69` (`src/endpoints.ts`,
//! `src/client.ts`, and `src/types/clob.ts`). The tracked integration seed is
//! Predarb object `8222273a9c72033b760e1d2fec813bc77144556d`, specifically
//! `crates/venue-polymarket/src/rest/public.rs`. The long `/markets/{condition}`
//! response owns lifecycle and question identity; the abbreviated
//! `/clob-markets/{condition}` response owns token membership and trading
//! numerics. Neither response is silently widened into the other one's role.

use std::collections::BTreeSet;

use reap_pm_core::{
    PmConditionId, PmMarketId, PmMarketLifecycle, PmOutcomeLabel, PmOutcomeMetadata, PmQuantity,
    PmTick, PmTokenId,
};
use serde::{Deserialize, Deserializer};
use serde_json::value::RawValue;

use crate::limits::{MAX_MARKET_TOKENS, MAX_PUBLIC_REST_BODY_BYTES};
use crate::rest::{PmClobToken, PmLifecycleMetadata, parse_token};
use crate::{PmWireError, PmWireScope};

const MAX_LIFECYCLE_TIME_STRING_BYTES: usize = 128;

/// Typed provenance for the fixed `/clob-markets/{condition}` request.
///
/// The abbreviated response is not allowed to invent a market/question ID.
/// Its omitted condition is bound only by the condition carried by this
/// request scope; a present response condition must match it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmClobV2RequestScope {
    condition: PmConditionId,
    token: PmTokenId,
}

/// One bounded exact string from a long-market lifecycle time field.
///
/// The wire layer deliberately retains the venue string instead of parsing it
/// through a wall-clock type. Canonical UTC policy belongs to the consuming
/// preflight, while this type proves the source value was nonempty, ASCII, and
/// bounded before it left the parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmLifecycleTimeString(Box<str>);

impl PmLifecycleTimeString {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Lifecycle details owned only by the long `GET /markets/{condition}` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmLongMarketLifecycleDetails {
    accepting_order_timestamp: Option<PmLifecycleTimeString>,
    end_date_iso: PmLifecycleTimeString,
    game_start_time: Option<PmLifecycleTimeString>,
    seconds_delay: u64,
}

impl PmLongMarketLifecycleDetails {
    #[must_use]
    pub const fn accepting_order_timestamp(&self) -> Option<&PmLifecycleTimeString> {
        self.accepting_order_timestamp.as_ref()
    }

    #[must_use]
    pub const fn end_date_iso(&self) -> &PmLifecycleTimeString {
        &self.end_date_iso
    }

    #[must_use]
    pub const fn game_start_time(&self) -> Option<&PmLifecycleTimeString> {
        self.game_start_time.as_ref()
    }

    #[must_use]
    pub const fn seconds_delay(&self) -> u64 {
        self.seconds_delay
    }
}

/// Complete typed projection of the long live-market response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmLiveClobMarketLifecycle {
    metadata: PmLifecycleMetadata,
    details: PmLongMarketLifecycleDetails,
}

impl PmLiveClobMarketLifecycle {
    #[must_use]
    pub const fn metadata(&self) -> &PmLifecycleMetadata {
        &self.metadata
    }

    #[must_use]
    pub const fn details(&self) -> &PmLongMarketLifecycleDetails {
        &self.details
    }
}

impl PmClobV2RequestScope {
    #[must_use]
    pub const fn new(condition: PmConditionId, token: PmTokenId) -> Self {
        Self { condition, token }
    }

    #[must_use]
    pub const fn condition(self) -> PmConditionId {
        self.condition
    }

    #[must_use]
    pub const fn token(self) -> PmTokenId {
        self.token
    }
}

/// Trading metadata from the abbreviated CLOB V2 route.
///
/// Deliberately has no market/question accessor: that identity is absent from
/// this wire shape and comes from the independently parsed long response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmClobV2Metadata {
    requested_condition: PmConditionId,
    reported_condition: Option<PmConditionId>,
    tokens: Vec<PmClobToken>,
    configured_outcome: PmOutcomeMetadata,
    tick: PmTick,
    minimum_order_size: PmQuantity,
    negative_risk: bool,
    maker_base_fee_bps: u64,
    taker_base_fee_bps: u64,
    fee_details: PmClobFeeDetails,
    accepting_orders: Option<bool>,
    seconds_delay: Option<u64>,
    game_start_time: Option<PmLifecycleTimeString>,
    cancel_book_on_start: Option<bool>,
    accepting_order_timestamp: Option<PmLifecycleTimeString>,
    rfq_enabled: Option<bool>,
    take_only_delay_enabled: bool,
    bonding_curve_enabled: Option<bool>,
    minimum_order_age_seconds: u64,
}

/// One exact, nonnegative JSON-decimal fee parameter from the CLOB response.
/// The lexeme is retained because binary floating-point is not authoritative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmClobFeeDecimal(Box<str>);

impl PmClobFeeDecimal {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Optional fee-curve values inside the required `fd` object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmClobFeeDetails {
    rate: Option<PmClobFeeDecimal>,
    exponent: Option<PmClobFeeDecimal>,
    taker_only: Option<bool>,
}

impl PmClobFeeDetails {
    #[must_use]
    pub const fn rate(&self) -> Option<&PmClobFeeDecimal> {
        self.rate.as_ref()
    }

    #[must_use]
    pub const fn exponent(&self) -> Option<&PmClobFeeDecimal> {
        self.exponent.as_ref()
    }

    #[must_use]
    pub const fn taker_only(&self) -> Option<bool> {
        self.taker_only
    }
}

impl PmClobV2Metadata {
    #[must_use]
    pub const fn requested_condition(&self) -> PmConditionId {
        self.requested_condition
    }

    #[must_use]
    pub const fn reported_condition(&self) -> Option<PmConditionId> {
        self.reported_condition
    }

    #[must_use]
    pub fn tokens(&self) -> &[PmClobToken] {
        &self.tokens
    }

    #[must_use]
    pub const fn configured_outcome(&self) -> PmOutcomeMetadata {
        self.configured_outcome
    }

    #[must_use]
    pub const fn tick(&self) -> PmTick {
        self.tick
    }

    #[must_use]
    pub const fn minimum_order_size(&self) -> PmQuantity {
        self.minimum_order_size
    }

    #[must_use]
    pub const fn negative_risk(&self) -> bool {
        self.negative_risk
    }

    #[must_use]
    pub const fn maker_base_fee_bps(&self) -> u64 {
        self.maker_base_fee_bps
    }

    #[must_use]
    pub const fn taker_base_fee_bps(&self) -> u64 {
        self.taker_base_fee_bps
    }

    #[must_use]
    pub const fn fee_details(&self) -> &PmClobFeeDetails {
        &self.fee_details
    }

    #[must_use]
    pub const fn accepting_orders(&self) -> Option<bool> {
        self.accepting_orders
    }

    #[must_use]
    pub const fn seconds_delay(&self) -> Option<u64> {
        self.seconds_delay
    }

    #[must_use]
    pub const fn game_start_time(&self) -> Option<&PmLifecycleTimeString> {
        self.game_start_time.as_ref()
    }

    #[must_use]
    pub const fn cancel_book_on_start(&self) -> Option<bool> {
        self.cancel_book_on_start
    }

    #[must_use]
    pub const fn accepting_order_timestamp(&self) -> Option<&PmLifecycleTimeString> {
        self.accepting_order_timestamp.as_ref()
    }

    #[must_use]
    pub const fn rfq_enabled(&self) -> Option<bool> {
        self.rfq_enabled
    }

    #[must_use]
    pub const fn take_only_delay_enabled(&self) -> bool {
        self.take_only_delay_enabled
    }

    #[must_use]
    pub const fn bonding_curve_enabled(&self) -> Option<bool> {
        self.bonding_curve_enabled
    }

    #[must_use]
    pub const fn minimum_order_age_seconds(&self) -> u64 {
        self.minimum_order_age_seconds
    }
}

/// Parses lifecycle and question identity from `GET /markets/{condition}`.
pub fn parse_live_clob_market_lifecycle(
    raw: &[u8],
    scope: PmWireScope,
) -> Result<PmLifecycleMetadata, PmWireError> {
    parse_live_clob_market_lifecycle_details(raw, scope).map(|parsed| *parsed.metadata())
}

/// Parses lifecycle, question identity, and exact lifecycle details from
/// `GET /markets/{condition}`.
pub fn parse_live_clob_market_lifecycle_details(
    raw: &[u8],
    scope: PmWireScope,
) -> Result<PmLiveClobMarketLifecycle, PmWireError> {
    check_rest_bound(raw)?;
    let wire =
        serde_json::from_slice::<RawLongClobMarket>(raw).map_err(|_| PmWireError::MalformedJson)?;
    let condition = parse_condition(required(&wire.condition_id, "condition_id")?)?;
    let market = parse_market(required(&wire.question_id, "question_id")?)?;
    if condition != scope.condition() {
        return Err(PmWireError::ConditionMismatch);
    }
    if market != scope.market() {
        return Err(PmWireError::MarketMismatch);
    }
    let lifecycle = PmMarketLifecycle::new(
        required_copy(wire.active, "active")?,
        required_copy(wire.closed, "closed")?,
        required_copy(wire.archived, "archived")?,
        required_copy(wire.accepting_orders, "accepting_orders")?,
        required_copy(wire.enable_order_book, "enable_order_book")?,
    );
    let accepting_order_timestamp = match wire.accepting_order_timestamp {
        PresentField::Missing => None,
        PresentField::Present(None) => {
            return Err(PmWireError::NullField("accepting_order_timestamp"));
        }
        PresentField::Present(Some(value)) => Some(parse_lifecycle_time_string(
            value,
            "accepting_order_timestamp",
        )?),
    };
    let end_date_iso = match wire.end_date_iso {
        PresentField::Missing => return Err(PmWireError::MissingField("end_date_iso")),
        PresentField::Present(None) => return Err(PmWireError::NullField("end_date_iso")),
        PresentField::Present(Some(value)) => parse_lifecycle_time_string(value, "end_date_iso")?,
    };
    let game_start_time = match wire.game_start_time {
        PresentField::Missing | PresentField::Present(None) => None,
        PresentField::Present(Some(value)) => {
            Some(parse_lifecycle_time_string(value, "game_start_time")?)
        }
    };
    let seconds_delay = match wire.seconds_delay {
        PresentField::Missing => return Err(PmWireError::MissingField("seconds_delay")),
        PresentField::Present(None) => return Err(PmWireError::NullField("seconds_delay")),
        PresentField::Present(Some(value)) => value,
    };
    Ok(PmLiveClobMarketLifecycle {
        metadata: PmLifecycleMetadata::from_parts(condition, market, lifecycle),
        details: PmLongMarketLifecycleDetails {
            accepting_order_timestamp,
            end_date_iso,
            game_start_time,
            seconds_delay,
        },
    })
}

/// Parses exact trading metadata from `GET /clob-markets/{condition}`.
pub fn parse_live_clob_v2_metadata(
    raw: &[u8],
    request: PmClobV2RequestScope,
) -> Result<PmClobV2Metadata, PmWireError> {
    check_rest_bound(raw)?;
    let wire = serde_json::from_slice::<RawShortClobMarket>(raw)
        .map_err(|_| PmWireError::MalformedJson)?;

    let reported_condition = match wire.condition {
        PresentField::Missing => None,
        PresentField::Present(value) => {
            let condition = parse_condition(&value)?;
            if condition != request.condition() {
                return Err(PmWireError::ConditionMismatch);
            }
            Some(condition)
        }
    };

    let raw_tokens = wire.tokens.ok_or(PmWireError::MissingField("tokens"))?;
    if raw_tokens.len() > MAX_MARKET_TOKENS {
        return Err(PmWireError::TooManyMarketTokens);
    }
    if raw_tokens.len() != 2 {
        return Err(PmWireError::UnexpectedMarketTokenCount);
    }
    let mut seen = BTreeSet::new();
    let mut tokens = Vec::with_capacity(raw_tokens.len());
    let mut configured_outcome = None;
    for raw_token in raw_tokens {
        let token = parse_token(required(&raw_token.token_id, "token_id")?)?;
        if !seen.insert(token) {
            return Err(PmWireError::DuplicateToken);
        }
        let label = PmOutcomeLabel::new(required(&raw_token.outcome, "outcome")?)
            .map_err(|_| PmWireError::InvalidIdentity("outcome"))?;
        let outcome = PmOutcomeMetadata::new(token, label);
        if token == request.token() {
            configured_outcome = Some(outcome);
        }
        tokens.push(PmClobToken::from_outcome(outcome));
    }
    let configured_outcome = configured_outcome.ok_or(PmWireError::ConfiguredTokenMissing)?;

    let tick = PmTick::parse_decimal(required_raw(&wire.minimum_tick_size, "minimum_tick_size")?)
        .map_err(|_| PmWireError::InvalidNumeric("minimum_tick_size"))?;
    let minimum_order_size = PmQuantity::parse_decimal(required_raw(
        &wire.minimum_order_size,
        "minimum_order_size",
    )?)
    .map_err(|_| PmWireError::InvalidNumeric("minimum_order_size"))?;
    minimum_order_size
        .validate_order(minimum_order_size)
        .map_err(|_| PmWireError::InvalidNumeric("minimum_order_size"))?;
    let negative_risk = match wire.negative_risk {
        PresentField::Missing => false,
        PresentField::Present(value) => value,
    };
    let maker_base_fee_bps = wire
        .maker_base_fee
        .ok_or(PmWireError::MissingField("maker_base_fee"))?;
    let taker_base_fee_bps = wire
        .taker_base_fee
        .ok_or(PmWireError::MissingField("taker_base_fee"))?;
    let raw_fee_details = match wire.fee_details {
        PresentField::Missing => return Err(PmWireError::MissingField("fee_details")),
        PresentField::Present(None) => return Err(PmWireError::NullField("fee_details")),
        PresentField::Present(Some(details)) => details,
    };
    let fee_details = PmClobFeeDetails {
        rate: raw_fee_details
            .rate
            .as_deref()
            .map(|value| parse_fee_decimal(value, "fee_details.rate"))
            .transpose()?,
        exponent: raw_fee_details
            .exponent
            .as_deref()
            .map(|value| parse_fee_decimal(value, "fee_details.exponent"))
            .transpose()?,
        taker_only: raw_fee_details.taker_only,
    };
    let accepting_orders = optional_nonnull(wire.accepting_orders, "accepting_orders")?;
    let seconds_delay = optional_nonnull(wire.seconds_delay, "seconds_delay")?;
    let game_start_time = match wire.game_start_time {
        PresentField::Missing | PresentField::Present(None) => None,
        PresentField::Present(Some(value)) => {
            Some(parse_lifecycle_time_string(value, "game_start_time")?)
        }
    };
    let cancel_book_on_start = optional_nonnull(wire.cancel_book_on_start, "cancel_book_on_start")?;
    let accepting_order_timestamp =
        optional_nonnull(wire.accepting_order_timestamp, "accepting_order_timestamp")?
            .map(|value| parse_lifecycle_time_string(value, "accepting_order_timestamp"))
            .transpose()?;
    let rfq_enabled = optional_nonnull(wire.rfq_enabled, "rfq_enabled")?;
    let take_only_delay_enabled = match wire.is_take_only_delay_enabled {
        PresentField::Missing => false,
        PresentField::Present(value) => value,
    };
    let bonding_curve_enabled =
        optional_nonnull(wire.is_bonding_curve_enabled, "is_bonding_curve_enabled")?;
    let minimum_order_age_seconds = wire
        .order_acceptance_status
        .ok_or(PmWireError::MissingField("minimum_order_age_seconds"))?;

    Ok(PmClobV2Metadata {
        requested_condition: request.condition(),
        reported_condition,
        tokens,
        configured_outcome,
        tick,
        minimum_order_size,
        negative_risk,
        maker_base_fee_bps,
        taker_base_fee_bps,
        fee_details,
        accepting_orders,
        seconds_delay,
        game_start_time,
        cancel_book_on_start,
        accepting_order_timestamp,
        rfq_enabled,
        take_only_delay_enabled,
        bonding_curve_enabled,
        minimum_order_age_seconds,
    })
}

/// Requires every abbreviated lifecycle fact that is present to agree with
/// the independently parsed long-market authority.
pub fn validate_live_clob_lifecycle_agreement(
    long: &PmLiveClobMarketLifecycle,
    short: &PmClobV2Metadata,
) -> Result<(), PmWireError> {
    let long_metadata = long.metadata();
    let long_details = long.details();
    if short
        .accepting_orders()
        .is_some_and(|value| value != long_metadata.lifecycle().accepting_orders())
    {
        return Err(PmWireError::InvalidIdentity("accepting_orders"));
    }
    if short
        .seconds_delay()
        .is_some_and(|value| value != long_details.seconds_delay())
    {
        return Err(PmWireError::InvalidIdentity("seconds_delay"));
    }
    if let Some(short_timestamp) = short.accepting_order_timestamp() {
        let Some(long_timestamp) = long_details.accepting_order_timestamp() else {
            return Err(PmWireError::InvalidIdentity("accepting_order_timestamp"));
        };
        if short_timestamp != long_timestamp {
            return Err(PmWireError::InvalidIdentity("accepting_order_timestamp"));
        }
    }
    if let Some(short_timestamp) = short.game_start_time() {
        let Some(long_timestamp) = long_details.game_start_time() else {
            return Err(PmWireError::InvalidIdentity("game_start_time"));
        };
        if short_timestamp != long_timestamp {
            return Err(PmWireError::InvalidIdentity("game_start_time"));
        }
    }
    Ok(())
}

fn parse_fee_decimal(
    value: &RawValue,
    field: &'static str,
) -> Result<PmClobFeeDecimal, PmWireError> {
    const MAX_FEE_DECIMAL_BYTES: usize = 64;

    let text = value.get();
    // `RawValue` retains the original JSON-number spelling. The containing
    // typed deserialization has already proved legal JSON, so preserve every
    // bounded nonnegative spelling instead of converting through f64 or
    // inventing a venue-side canonical-decimal grammar.
    if text.is_empty() || text.len() > MAX_FEE_DECIMAL_BYTES || !text.as_bytes()[0].is_ascii_digit()
    {
        return Err(PmWireError::InvalidNumeric(field));
    }
    Ok(PmClobFeeDecimal(text.into()))
}

fn parse_lifecycle_time_string(
    value: String,
    field: &'static str,
) -> Result<PmLifecycleTimeString, PmWireError> {
    if value.is_empty() {
        return Err(PmWireError::InvalidIdentity(field));
    }
    if value.len() > MAX_LIFECYCLE_TIME_STRING_BYTES {
        return Err(PmWireError::FieldTooLong(field));
    }
    if !value.is_ascii() {
        return Err(PmWireError::NonAsciiField(field));
    }
    if value.bytes().any(|byte| !byte.is_ascii_graphic()) {
        return Err(PmWireError::InvalidIdentity(field));
    }
    Ok(PmLifecycleTimeString(value.into()))
}

fn optional_nonnull<T>(
    value: PresentField<Option<T>>,
    field: &'static str,
) -> Result<Option<T>, PmWireError> {
    match value {
        PresentField::Missing => Ok(None),
        PresentField::Present(None) => Err(PmWireError::NullField(field)),
        PresentField::Present(Some(value)) => Ok(Some(value)),
    }
}

fn check_rest_bound(raw: &[u8]) -> Result<(), PmWireError> {
    if raw.len() > MAX_PUBLIC_REST_BODY_BYTES {
        Err(PmWireError::RestBodyTooLarge)
    } else {
        Ok(())
    }
}

fn parse_condition(value: &str) -> Result<PmConditionId, PmWireError> {
    PmConditionId::parse(value).map_err(|_| PmWireError::InvalidIdentity("condition_id"))
}

fn parse_market(value: &str) -> Result<PmMarketId, PmWireError> {
    PmMarketId::parse(value).map_err(|_| PmWireError::InvalidIdentity("market"))
}

fn required<'a>(value: &'a Option<String>, field: &'static str) -> Result<&'a str, PmWireError> {
    value.as_deref().ok_or(PmWireError::MissingField(field))
}

fn required_copy<T: Copy>(value: Option<T>, field: &'static str) -> Result<T, PmWireError> {
    value.ok_or(PmWireError::MissingField(field))
}

fn required_raw<'a>(
    value: &'a Option<Box<RawValue>>,
    field: &'static str,
) -> Result<&'a str, PmWireError> {
    value
        .as_deref()
        .map(RawValue::get)
        .ok_or(PmWireError::MissingField(field))
}

#[derive(Default)]
enum PresentField<T> {
    #[default]
    Missing,
    Present(T),
}

impl<'de, T> Deserialize<'de> for PresentField<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        T::deserialize(deserializer).map(Self::Present)
    }
}

// The long endpoint is reviewed field-by-field. Non-authority fields are
// retained as raw JSON solely so numeric-looking extensions never pass through
// an f64 representation before being ignored.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLongClobMarket {
    #[serde(default)]
    condition_id: Option<String>,
    #[serde(default)]
    question_id: Option<String>,
    #[serde(default)]
    active: Option<bool>,
    #[serde(default)]
    closed: Option<bool>,
    #[serde(default)]
    archived: Option<bool>,
    #[serde(default)]
    accepting_orders: Option<bool>,
    #[serde(default)]
    enable_order_book: Option<bool>,
    #[serde(default)]
    accepting_order_timestamp: PresentField<Option<String>>,
    #[serde(default, rename = "minimum_order_size")]
    _minimum_order_size: Option<Box<RawValue>>,
    #[serde(default, rename = "minimum_tick_size")]
    _minimum_tick_size: Option<Box<RawValue>>,
    #[serde(default, rename = "question")]
    _question: Option<Box<RawValue>>,
    #[serde(default, rename = "description")]
    _description: Option<Box<RawValue>>,
    #[serde(default, rename = "market_slug")]
    _market_slug: Option<Box<RawValue>>,
    #[serde(default)]
    end_date_iso: PresentField<Option<String>>,
    #[serde(default)]
    game_start_time: PresentField<Option<String>>,
    #[serde(default)]
    seconds_delay: PresentField<Option<u64>>,
    #[serde(default, rename = "fpmm")]
    _fpmm: Option<Box<RawValue>>,
    #[serde(default, rename = "maker_base_fee")]
    _maker_base_fee: Option<Box<RawValue>>,
    #[serde(default, rename = "taker_base_fee")]
    _taker_base_fee: Option<Box<RawValue>>,
    #[serde(default, rename = "notifications_enabled")]
    _notifications_enabled: Option<Box<RawValue>>,
    #[serde(default, rename = "neg_risk")]
    _neg_risk: Option<Box<RawValue>>,
    #[serde(default, rename = "neg_risk_market_id")]
    _neg_risk_market_id: Option<Box<RawValue>>,
    #[serde(default, rename = "neg_risk_request_id")]
    _neg_risk_request_id: Option<Box<RawValue>>,
    #[serde(default, rename = "icon")]
    _icon: Option<Box<RawValue>>,
    #[serde(default, rename = "image")]
    _image: Option<Box<RawValue>>,
    #[serde(default, rename = "rewards")]
    _rewards: Option<Box<RawValue>>,
    #[serde(default, rename = "is_50_50_outcome")]
    _is_50_50_outcome: Option<Box<RawValue>>,
    #[serde(default, rename = "tokens")]
    _tokens: Option<Box<RawValue>>,
    #[serde(default, rename = "tags")]
    _tags: Option<Box<RawValue>>,
}

// This spelling mirrors `MarketDetails` in the pinned official TypeScript
// client plus `oas` from the frozen v3 OpenAPI response contract.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawShortClobMarket {
    #[serde(default, rename = "c")]
    condition: PresentField<String>,
    #[serde(default, rename = "t")]
    tokens: Option<Vec<RawShortClobToken>>,
    #[serde(default, rename = "mts")]
    minimum_tick_size: Option<Box<RawValue>>,
    #[serde(default, rename = "mos")]
    minimum_order_size: Option<Box<RawValue>>,
    #[serde(default, rename = "nr")]
    negative_risk: PresentField<bool>,
    #[serde(default, rename = "r")]
    _rewards: Option<Box<RawValue>>,
    #[serde(default, rename = "fd")]
    fee_details: PresentField<Option<RawFeeDetails>>,
    #[serde(default, rename = "mbf")]
    maker_base_fee: Option<u64>,
    #[serde(default, rename = "tbf")]
    taker_base_fee: Option<u64>,
    #[serde(default, rename = "ao")]
    accepting_orders: PresentField<Option<bool>>,
    #[serde(default, rename = "sd")]
    seconds_delay: PresentField<Option<u64>>,
    #[serde(default, rename = "gst")]
    game_start_time: PresentField<Option<String>>,
    #[serde(default, rename = "cbos")]
    cancel_book_on_start: PresentField<Option<bool>>,
    #[serde(default, rename = "aot")]
    accepting_order_timestamp: PresentField<Option<String>>,
    #[serde(default, rename = "rfqe")]
    rfq_enabled: PresentField<Option<bool>>,
    #[serde(default, rename = "itode")]
    is_take_only_delay_enabled: PresentField<bool>,
    #[serde(default, rename = "ibce")]
    is_bonding_curve_enabled: PresentField<Option<bool>>,
    #[serde(default, rename = "oas")]
    order_acceptance_status: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFeeDetails {
    #[serde(default, rename = "r")]
    rate: Option<Box<RawValue>>,
    #[serde(default, rename = "e")]
    exponent: Option<Box<RawValue>>,
    #[serde(default, rename = "to")]
    taker_only: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawShortClobToken {
    #[serde(default, rename = "t")]
    token_id: Option<String>,
    #[serde(default, rename = "o")]
    outcome: Option<String>,
}
