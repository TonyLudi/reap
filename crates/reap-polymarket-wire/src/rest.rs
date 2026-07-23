use std::collections::BTreeSet;

use reap_pm_core::{
    PmConditionId, PmMarketId, PmMarketLifecycle, PmOutcomeLabel, PmOutcomeMetadata, PmQuantity,
    PmTick, PmTokenId, U256,
};
use serde::Deserialize;

use crate::limits::{MAX_MARKET_TOKENS, MAX_PUBLIC_REST_BODY_BYTES};
use crate::ws::parse_book_bytes;
use crate::{PmBookParserConfig, PmBookSnapshot, PmWireError, PmWireScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmLifecycleMetadata {
    condition: PmConditionId,
    market: PmMarketId,
    lifecycle: PmMarketLifecycle,
}

impl PmLifecycleMetadata {
    #[must_use]
    pub const fn condition(self) -> PmConditionId {
        self.condition
    }

    #[must_use]
    pub const fn market(self) -> PmMarketId {
        self.market
    }

    #[must_use]
    pub const fn lifecycle(self) -> PmMarketLifecycle {
        self.lifecycle
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PmClobToken {
    outcome: PmOutcomeMetadata,
}

impl PmClobToken {
    #[must_use]
    pub const fn outcome(self) -> PmOutcomeMetadata {
        self.outcome
    }

    #[must_use]
    pub const fn token(self) -> PmTokenId {
        self.outcome.token()
    }

    #[must_use]
    pub const fn label(self) -> PmOutcomeLabel {
        self.outcome.label()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmClobMetadata {
    condition: PmConditionId,
    market: PmMarketId,
    tokens: Vec<PmClobToken>,
    configured_outcome: PmOutcomeMetadata,
    tick: PmTick,
    minimum_order_size: PmQuantity,
    negative_risk: bool,
}

impl PmClobMetadata {
    #[must_use]
    pub const fn condition(&self) -> PmConditionId {
        self.condition
    }

    #[must_use]
    pub const fn market(&self) -> PmMarketId {
        self.market
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

pub fn parse_server_time(raw: &[u8]) -> Result<u64, PmWireError> {
    check_rest_bound(raw)?;
    let response = serde_json::from_slice::<ServerTimeResponse>(raw)
        .map_err(|_| PmWireError::MalformedJson)?;
    let timestamp = match response {
        ServerTimeResponse::Bare(value) => value,
        ServerTimeResponse::Object { timestamp } => timestamp,
    };
    u64::try_from(timestamp)
        .ok()
        .filter(|value| *value > 0)
        .ok_or(PmWireError::InvalidServerTime)
}

pub fn parse_lifecycle_metadata(
    raw: &[u8],
    scope: PmWireScope,
) -> Result<PmLifecycleMetadata, PmWireError> {
    check_rest_bound(raw)?;
    let wire =
        serde_json::from_slice::<RawLifecycle>(raw).map_err(|_| PmWireError::MalformedJson)?;
    let condition = parse_condition(required(&wire.condition_id, "condition_id")?)?;
    let market = parse_market(required(&wire.market_id, "market_id")?)?;
    validate_metadata_scope(condition, market, scope)?;
    let lifecycle = PmMarketLifecycle::new(
        required_copy(wire.active, "active")?,
        required_copy(wire.closed, "closed")?,
        required_copy(wire.archived, "archived")?,
        required_copy(wire.accepting_orders, "accepting_orders")?,
        required_copy(wire.enable_order_book, "enable_order_book")?,
    );
    Ok(PmLifecycleMetadata {
        condition,
        market,
        lifecycle,
    })
}

pub fn parse_clob_metadata(raw: &[u8], scope: PmWireScope) -> Result<PmClobMetadata, PmWireError> {
    check_rest_bound(raw)?;
    let wire =
        serde_json::from_slice::<RawClobMarket>(raw).map_err(|_| PmWireError::MalformedJson)?;
    let condition = parse_condition(required(&wire.condition_id, "condition_id")?)?;
    let market = parse_market(required(&wire.market_id, "market_id")?)?;
    validate_metadata_scope(condition, market, scope)?;

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
        if token == scope.token() {
            configured_outcome = Some(outcome);
        }
        tokens.push(PmClobToken { outcome });
    }
    let configured_outcome = configured_outcome.ok_or(PmWireError::ConfiguredTokenMissing)?;

    let tick = PmTick::parse_decimal(required(&wire.minimum_tick_size, "minimum_tick_size")?)
        .map_err(|_| PmWireError::InvalidNumeric("minimum_tick_size"))?;
    let minimum_order_size =
        PmQuantity::parse_decimal(required(&wire.minimum_order_size, "minimum_order_size")?)
            .map_err(|_| PmWireError::InvalidNumeric("minimum_order_size"))?;
    minimum_order_size
        .validate_order(minimum_order_size)
        .map_err(|_| PmWireError::InvalidNumeric("minimum_order_size"))?;
    let negative_risk = required_copy(wire.neg_risk, "neg_risk")?;

    Ok(PmClobMetadata {
        condition,
        market,
        tokens,
        configured_outcome,
        tick,
        minimum_order_size,
        negative_risk,
    })
}

pub fn parse_rest_book_snapshot(
    raw: &[u8],
    config: PmBookParserConfig,
) -> Result<PmBookSnapshot, PmWireError> {
    check_rest_bound(raw)?;
    parse_book_bytes(raw, config, false)
}

fn check_rest_bound(raw: &[u8]) -> Result<(), PmWireError> {
    if raw.len() > MAX_PUBLIC_REST_BODY_BYTES {
        Err(PmWireError::RestBodyTooLarge)
    } else {
        Ok(())
    }
}

fn validate_metadata_scope(
    condition: PmConditionId,
    market: PmMarketId,
    scope: PmWireScope,
) -> Result<(), PmWireError> {
    if condition != scope.condition() {
        return Err(PmWireError::ConditionMismatch);
    }
    if market != scope.market() {
        return Err(PmWireError::MarketMismatch);
    }
    Ok(())
}

pub(crate) fn parse_market(value: &str) -> Result<PmMarketId, PmWireError> {
    PmMarketId::parse(value).map_err(|_| PmWireError::InvalidIdentity("market"))
}

pub(crate) fn parse_token(value: &str) -> Result<PmTokenId, PmWireError> {
    let units = value
        .parse::<U256>()
        .map_err(|_| PmWireError::InvalidIdentity("asset_id"))?;
    PmTokenId::new(units).map_err(|_| PmWireError::InvalidIdentity("asset_id"))
}

fn parse_condition(value: &str) -> Result<PmConditionId, PmWireError> {
    PmConditionId::parse(value).map_err(|_| PmWireError::InvalidIdentity("condition_id"))
}

fn required<'a>(value: &'a Option<String>, field: &'static str) -> Result<&'a str, PmWireError> {
    value.as_deref().ok_or(PmWireError::MissingField(field))
}

fn required_copy<T: Copy>(value: Option<T>, field: &'static str) -> Result<T, PmWireError> {
    value.ok_or(PmWireError::MissingField(field))
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ServerTimeResponse {
    Bare(i64),
    Object { timestamp: i64 },
}

// These metadata shapes feed a later authority join. New venue fields must be
// explicitly reviewed and added here; silently ignored extensions cannot
// influence readiness.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLifecycle {
    #[serde(default, alias = "conditionId")]
    condition_id: Option<String>,
    #[serde(default, alias = "question_id", alias = "id")]
    market_id: Option<String>,
    #[serde(default)]
    active: Option<bool>,
    #[serde(default)]
    closed: Option<bool>,
    #[serde(default)]
    archived: Option<bool>,
    #[serde(default, alias = "acceptingOrders")]
    accepting_orders: Option<bool>,
    #[serde(default, alias = "enableOrderBook")]
    enable_order_book: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawClobMarket {
    #[serde(default, alias = "conditionId")]
    condition_id: Option<String>,
    #[serde(default, alias = "question_id", alias = "id")]
    market_id: Option<String>,
    #[serde(default, alias = "min_order_size")]
    minimum_order_size: Option<String>,
    #[serde(default, alias = "tick_size")]
    minimum_tick_size: Option<String>,
    #[serde(default, alias = "nr")]
    neg_risk: Option<bool>,
    #[serde(default, alias = "t")]
    tokens: Option<Vec<RawClobToken>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawClobToken {
    #[serde(default, alias = "t")]
    token_id: Option<String>,
    #[serde(default, alias = "o")]
    outcome: Option<String>,
}
