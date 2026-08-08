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
}

/// Parses lifecycle and question identity from `GET /markets/{condition}`.
pub fn parse_live_clob_market_lifecycle(
    raw: &[u8],
    scope: PmWireScope,
) -> Result<PmLifecycleMetadata, PmWireError> {
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
    Ok(PmLifecycleMetadata::from_parts(
        condition, market, lifecycle,
    ))
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

    Ok(PmClobV2Metadata {
        requested_condition: request.condition(),
        reported_condition,
        tokens,
        configured_outcome,
        tick,
        minimum_order_size,
        negative_risk,
    })
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
    #[serde(default, rename = "accepting_order_timestamp")]
    _accepting_order_timestamp: Option<Box<RawValue>>,
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
    #[serde(default, rename = "end_date_iso")]
    _end_date_iso: Option<Box<RawValue>>,
    #[serde(default, rename = "game_start_time")]
    _game_start_time: Option<Box<RawValue>>,
    #[serde(default, rename = "seconds_delay")]
    _seconds_delay: Option<Box<RawValue>>,
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
// client. Only `c`, `t`, `mts`, `mos`, and `nr` are authority-bearing here.
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
    _fee_details: Option<Box<RawValue>>,
    #[serde(default, rename = "mbf")]
    _maker_base_fee: Option<Box<RawValue>>,
    #[serde(default, rename = "tbf")]
    _taker_base_fee: Option<Box<RawValue>>,
    #[serde(default, rename = "ao")]
    _accepting_orders: Option<Box<RawValue>>,
    #[serde(default, rename = "sd")]
    _seconds_delay: Option<Box<RawValue>>,
    #[serde(default, rename = "gst")]
    _game_start_time: Option<Box<RawValue>>,
    #[serde(default, rename = "cbos")]
    _cancel_book_on_start: Option<Box<RawValue>>,
    #[serde(default, rename = "aot")]
    _accepting_order_timestamp: Option<Box<RawValue>>,
    #[serde(default, rename = "rfqe")]
    _rfq_enabled: Option<Box<RawValue>>,
    #[serde(default, rename = "itode")]
    _is_take_only_delay_enabled: Option<Box<RawValue>>,
    #[serde(default, rename = "ibce")]
    _is_bonding_curve_enabled: Option<Box<RawValue>>,
    #[serde(default, rename = "oas")]
    _order_acceptance_status: Option<Box<RawValue>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawShortClobToken {
    #[serde(default, rename = "t")]
    token_id: Option<String>,
    #[serde(default, rename = "o")]
    outcome: Option<String>,
}
