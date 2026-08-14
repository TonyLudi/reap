//! Closed, credential-free discovery for the currently active BTC Up/Down
//! five-minute market.
//!
//! The source owns the production Gamma origin and derives the sole allowed
//! slug from the current five-minute UTC window. It exposes no arbitrary
//! origin, slug, route, query, authentication, or mutation capability.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reap_pm_core::{PmConditionId, PmMarketId, PmQuantity, PmTick, PmTokenId, U256};
use reap_polymarket_egress_binding::PmLocalEgressSelection;
use reqwest::{
    Client, StatusCode, Url,
    header::{ACCEPT, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_TYPE, HeaderMap, HeaderName},
    redirect::Policy,
};
use serde::Deserialize;
use serde_json::value::RawValue;
use thiserror::Error;

pub const PM_GAMMA_API_PRODUCTION_ORIGIN: &str = "https://gamma-api.polymarket.com";
const BTC_FIVE_MINUTE_WINDOW_SECONDS: u64 = 300;
const MAX_GAMMA_EVENT_BODY_BYTES: usize = 1_048_576;
const MAX_HTTP_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmBtcFiveMinuteSourceError {
    #[error("BTC five-minute Gamma source configuration is invalid")]
    InvalidConfiguration,
    #[error("BTC five-minute Gamma transport could not be built")]
    TransportBuild,
    #[error("selected local egress is supported only on Linux")]
    SelectedLocalEgressUnsupported,
    #[error("BTC five-minute Gamma request timed out")]
    RequestTimeout,
    #[error("BTC five-minute Gamma request failed")]
    RequestFailed,
    #[error("BTC five-minute Gamma response body could not be read")]
    ResponseBodyRead,
    #[error("BTC five-minute Gamma response redirected with status {0}")]
    Redirect(u16),
    #[error("BTC five-minute Gamma response had unexpected status {0}")]
    UnexpectedStatus(u16),
    #[error("BTC five-minute Gamma response has invalid application headers")]
    InvalidApplicationHeaders,
    #[error("BTC five-minute Gamma response exceeds its body bound")]
    ResponseBodyTooLarge,
    #[error("BTC five-minute Gamma response is invalid or outside the exact market profile")]
    InvalidMarket,
    #[error("the five-minute window rolled while market discovery was in flight")]
    WindowRolled,
    #[error("system clock cannot identify the current BTC five-minute window")]
    Clock,
}

/// Exact identity and trading metadata for one current BTC Up/Down five-minute
/// market, parsed from the fixed Gamma event-by-slug response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmBtcFiveMinuteMarket {
    slug: Box<str>,
    title: Box<str>,
    condition: PmConditionId,
    market: PmMarketId,
    up_token: PmTokenId,
    down_token: PmTokenId,
    tick: PmTick,
    minimum_order_size: PmQuantity,
    negative_risk: bool,
    window_start_epoch: u64,
    window_end_epoch: u64,
    observed_at_millis: u64,
}

impl PmBtcFiveMinuteMarket {
    #[must_use]
    pub fn slug(&self) -> &str {
        &self.slug
    }

    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    #[must_use]
    pub const fn condition(&self) -> PmConditionId {
        self.condition
    }

    #[must_use]
    pub const fn market(&self) -> PmMarketId {
        self.market
    }

    #[must_use]
    pub const fn up_token(&self) -> PmTokenId {
        self.up_token
    }

    #[must_use]
    pub const fn down_token(&self) -> PmTokenId {
        self.down_token
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
    pub const fn window_start_epoch(&self) -> u64 {
        self.window_start_epoch
    }

    #[must_use]
    pub const fn window_end_epoch(&self) -> u64 {
        self.window_end_epoch
    }

    #[must_use]
    pub const fn observed_at_millis(&self) -> u64 {
        self.observed_at_millis
    }
}

/// Credential-free capability for exactly one current BTC five-minute Gamma
/// event lookup.
pub struct PmBtcFiveMinuteMarketSource {
    client: Client,
    origin: Url,
}

impl PmBtcFiveMinuteMarketSource {
    pub fn production(
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, PmBtcFiveMinuteSourceError> {
        Self::production_inner(connect_timeout, request_timeout, None)
    }

    pub fn production_on_selected_local_egress(
        connect_timeout: Duration,
        request_timeout: Duration,
        local_egress: &PmLocalEgressSelection,
    ) -> Result<Self, PmBtcFiveMinuteSourceError> {
        local_egress
            .require_production()
            .map_err(|_| PmBtcFiveMinuteSourceError::InvalidConfiguration)?;
        Self::production_inner(connect_timeout, request_timeout, Some(local_egress))
    }

    fn production_inner(
        connect_timeout: Duration,
        request_timeout: Duration,
        local_egress: Option<&PmLocalEgressSelection>,
    ) -> Result<Self, PmBtcFiveMinuteSourceError> {
        if connect_timeout.is_zero()
            || request_timeout.is_zero()
            || connect_timeout > MAX_HTTP_TIMEOUT
            || request_timeout > MAX_HTTP_TIMEOUT
        {
            return Err(PmBtcFiveMinuteSourceError::InvalidConfiguration);
        }
        let origin = Url::parse(PM_GAMMA_API_PRODUCTION_ORIGIN)
            .map_err(|_| PmBtcFiveMinuteSourceError::InvalidConfiguration)?;
        if origin.scheme() != "https"
            || origin.host_str() != Some("gamma-api.polymarket.com")
            || origin.port_or_known_default() != Some(443)
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
        {
            return Err(PmBtcFiveMinuteSourceError::InvalidConfiguration);
        }
        let mut builder = Client::builder()
            .connect_timeout(connect_timeout)
            .timeout(request_timeout)
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .https_only(true);
        if let Some(local_egress) = local_egress {
            #[cfg(target_os = "linux")]
            {
                builder = builder
                    .interface(local_egress.interface_name())
                    .local_address(local_egress.local_source_ip());
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = local_egress;
                return Err(PmBtcFiveMinuteSourceError::SelectedLocalEgressUnsupported);
            }
        }
        let client = builder
            .build()
            .map_err(|_| PmBtcFiveMinuteSourceError::TransportBuild)?;
        Ok(Self { client, origin })
    }

    #[must_use]
    pub const fn production_order_entry_authorized(&self) -> bool {
        false
    }

    pub async fn discover_current(
        &self,
    ) -> Result<PmBtcFiveMinuteMarket, PmBtcFiveMinuteSourceError> {
        let started_millis = current_millis()?;
        let window_start_epoch = current_window_start(started_millis);
        let slug = format!("btc-updown-5m-{window_start_epoch}");
        let mut url = self.origin.clone();
        url.set_path(&format!("/events/slug/{slug}"));

        let mut response = self
            .client
            .get(url)
            .header(ACCEPT, "application/json")
            .header(ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(map_request_error)?;
        let status = response.status();
        if status.is_redirection() {
            return Err(PmBtcFiveMinuteSourceError::Redirect(status.as_u16()));
        }
        if status != StatusCode::OK {
            return Err(PmBtcFiveMinuteSourceError::UnexpectedStatus(
                status.as_u16(),
            ));
        }
        validate_application_headers(response.headers())?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_GAMMA_EVENT_BODY_BYTES as u64)
        {
            return Err(PmBtcFiveMinuteSourceError::ResponseBodyTooLarge);
        }
        let capacity = response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or(0)
            .min(MAX_GAMMA_EVENT_BODY_BYTES);
        let mut body = Vec::with_capacity(capacity);
        while let Some(chunk) = response.chunk().await.map_err(map_body_error)? {
            let next_length = body
                .len()
                .checked_add(chunk.len())
                .ok_or(PmBtcFiveMinuteSourceError::ResponseBodyTooLarge)?;
            if next_length > MAX_GAMMA_EVENT_BODY_BYTES {
                return Err(PmBtcFiveMinuteSourceError::ResponseBodyTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        let observed_at_millis = current_millis()?;
        if current_window_start(observed_at_millis) != window_start_epoch {
            return Err(PmBtcFiveMinuteSourceError::WindowRolled);
        }
        parse_market(&body, &slug, window_start_epoch, observed_at_millis)
    }
}

#[derive(Debug, Deserialize)]
struct RawGammaEvent {
    slug: Option<String>,
    active: Option<bool>,
    closed: Option<bool>,
    archived: Option<bool>,
    markets: Option<Vec<RawGammaMarket>>,
}

#[derive(Debug, Deserialize)]
struct RawGammaMarket {
    question: Option<String>,
    #[serde(rename = "questionID")]
    question_id: Option<String>,
    #[serde(rename = "conditionId")]
    condition_id: Option<String>,
    slug: Option<String>,
    active: Option<bool>,
    closed: Option<bool>,
    archived: Option<bool>,
    #[serde(rename = "enableOrderBook")]
    enable_order_book: Option<bool>,
    #[serde(rename = "acceptingOrders")]
    accepting_orders: Option<bool>,
    #[serde(rename = "clobTokenIds")]
    clob_token_ids: Option<String>,
    outcomes: Option<String>,
    #[serde(rename = "orderPriceMinTickSize")]
    order_price_min_tick_size: Option<Box<RawValue>>,
    #[serde(rename = "orderMinSize")]
    order_min_size: Option<Box<RawValue>>,
    #[serde(rename = "negRisk")]
    negative_risk: Option<bool>,
}

fn parse_market(
    body: &[u8],
    expected_slug: &str,
    window_start_epoch: u64,
    observed_at_millis: u64,
) -> Result<PmBtcFiveMinuteMarket, PmBtcFiveMinuteSourceError> {
    let event: RawGammaEvent =
        serde_json::from_slice(body).map_err(|_| PmBtcFiveMinuteSourceError::InvalidMarket)?;
    if event.slug.as_deref() != Some(expected_slug)
        || event.active != Some(true)
        || event.closed != Some(false)
        || event.archived != Some(false)
    {
        return Err(PmBtcFiveMinuteSourceError::InvalidMarket);
    }
    let matches = event
        .markets
        .ok_or(PmBtcFiveMinuteSourceError::InvalidMarket)?
        .into_iter()
        .filter(|market| market.slug.as_deref() == Some(expected_slug))
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(PmBtcFiveMinuteSourceError::InvalidMarket);
    }
    let market = matches
        .into_iter()
        .next()
        .expect("exactly one market match");
    if market.active != Some(true)
        || market.closed != Some(false)
        || market.archived != Some(false)
        || market.enable_order_book != Some(true)
        || market.accepting_orders != Some(true)
    {
        return Err(PmBtcFiveMinuteSourceError::InvalidMarket);
    }
    let title = market
        .question
        .filter(|value| value.starts_with("Bitcoin Up or Down - "))
        .ok_or(PmBtcFiveMinuteSourceError::InvalidMarket)?;
    let condition = PmConditionId::parse(
        market
            .condition_id
            .as_deref()
            .ok_or(PmBtcFiveMinuteSourceError::InvalidMarket)?,
    )
    .map_err(|_| PmBtcFiveMinuteSourceError::InvalidMarket)?;
    let question = PmMarketId::parse(
        market
            .question_id
            .as_deref()
            .ok_or(PmBtcFiveMinuteSourceError::InvalidMarket)?,
    )
    .map_err(|_| PmBtcFiveMinuteSourceError::InvalidMarket)?;
    let token_ids: Vec<String> = serde_json::from_str(
        market
            .clob_token_ids
            .as_deref()
            .ok_or(PmBtcFiveMinuteSourceError::InvalidMarket)?,
    )
    .map_err(|_| PmBtcFiveMinuteSourceError::InvalidMarket)?;
    let outcomes: Vec<String> = serde_json::from_str(
        market
            .outcomes
            .as_deref()
            .ok_or(PmBtcFiveMinuteSourceError::InvalidMarket)?,
    )
    .map_err(|_| PmBtcFiveMinuteSourceError::InvalidMarket)?;
    if token_ids.len() != 2 || outcomes.as_slice() != ["Up", "Down"] {
        return Err(PmBtcFiveMinuteSourceError::InvalidMarket);
    }
    let up_token = parse_token(&token_ids[0])?;
    let down_token = parse_token(&token_ids[1])?;
    if up_token == down_token {
        return Err(PmBtcFiveMinuteSourceError::InvalidMarket);
    }
    let tick = PmTick::parse_decimal(
        market
            .order_price_min_tick_size
            .as_deref()
            .ok_or(PmBtcFiveMinuteSourceError::InvalidMarket)?
            .get(),
    )
    .map_err(|_| PmBtcFiveMinuteSourceError::InvalidMarket)?;
    let minimum_order_size = PmQuantity::parse_decimal(
        market
            .order_min_size
            .as_deref()
            .ok_or(PmBtcFiveMinuteSourceError::InvalidMarket)?
            .get(),
    )
    .map_err(|_| PmBtcFiveMinuteSourceError::InvalidMarket)?;
    minimum_order_size
        .validate_order(minimum_order_size)
        .map_err(|_| PmBtcFiveMinuteSourceError::InvalidMarket)?;
    let negative_risk = market
        .negative_risk
        .ok_or(PmBtcFiveMinuteSourceError::InvalidMarket)?;
    let window_end_epoch = window_start_epoch
        .checked_add(BTC_FIVE_MINUTE_WINDOW_SECONDS)
        .ok_or(PmBtcFiveMinuteSourceError::Clock)?;
    Ok(PmBtcFiveMinuteMarket {
        slug: expected_slug.into(),
        title: title.into(),
        condition,
        market: question,
        up_token,
        down_token,
        tick,
        minimum_order_size,
        negative_risk,
        window_start_epoch,
        window_end_epoch,
        observed_at_millis,
    })
}

fn parse_token(value: &str) -> Result<PmTokenId, PmBtcFiveMinuteSourceError> {
    let units = value
        .parse::<U256>()
        .map_err(|_| PmBtcFiveMinuteSourceError::InvalidMarket)?;
    PmTokenId::new(units).map_err(|_| PmBtcFiveMinuteSourceError::InvalidMarket)
}

fn current_millis() -> Result<u64, PmBtcFiveMinuteSourceError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| PmBtcFiveMinuteSourceError::Clock)?;
    u64::try_from(elapsed.as_millis()).map_err(|_| PmBtcFiveMinuteSourceError::Clock)
}

const fn current_window_start(now_millis: u64) -> u64 {
    let now_seconds = now_millis / 1_000;
    now_seconds / BTC_FIVE_MINUTE_WINDOW_SECONDS * BTC_FIVE_MINUTE_WINDOW_SECONDS
}

fn validate_application_headers(headers: &HeaderMap) -> Result<(), PmBtcFiveMinuteSourceError> {
    let content_type = exactly_one_header(headers, CONTENT_TYPE)?
        .ok_or(PmBtcFiveMinuteSourceError::InvalidApplicationHeaders)?;
    let content_type = content_type
        .to_str()
        .map_err(|_| PmBtcFiveMinuteSourceError::InvalidApplicationHeaders)?;
    let essence = content_type
        .split(';')
        .next()
        .ok_or(PmBtcFiveMinuteSourceError::InvalidApplicationHeaders)?
        .trim();
    if !essence.eq_ignore_ascii_case("application/json") {
        return Err(PmBtcFiveMinuteSourceError::InvalidApplicationHeaders);
    }
    if let Some(content_encoding) = exactly_one_header(headers, CONTENT_ENCODING)? {
        let content_encoding = content_encoding
            .to_str()
            .map_err(|_| PmBtcFiveMinuteSourceError::InvalidApplicationHeaders)?;
        if !content_encoding.eq_ignore_ascii_case("identity") {
            return Err(PmBtcFiveMinuteSourceError::InvalidApplicationHeaders);
        }
    }
    Ok(())
}

fn exactly_one_header(
    headers: &HeaderMap,
    name: HeaderName,
) -> Result<Option<&reqwest::header::HeaderValue>, PmBtcFiveMinuteSourceError> {
    let mut values = headers.get_all(name).iter();
    let first = values.next();
    if values.next().is_some() {
        return Err(PmBtcFiveMinuteSourceError::InvalidApplicationHeaders);
    }
    Ok(first)
}

fn map_request_error(error: reqwest::Error) -> PmBtcFiveMinuteSourceError {
    if error.is_timeout() {
        PmBtcFiveMinuteSourceError::RequestTimeout
    } else {
        PmBtcFiveMinuteSourceError::RequestFailed
    }
}

fn map_body_error(error: reqwest::Error) -> PmBtcFiveMinuteSourceError {
    if error.is_timeout() {
        PmBtcFiveMinuteSourceError::RequestTimeout
    } else {
        PmBtcFiveMinuteSourceError::ResponseBodyRead
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONDITION: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
    const QUESTION: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";

    fn event() -> String {
        format!(
            r#"{{
                "slug":"btc-updown-5m-1786579200",
                "active":true,
                "closed":false,
                "archived":false,
                "markets":[{{
                    "question":"Bitcoin Up or Down - August 12, 8:00PM-8:05PM ET",
                    "questionID":"{QUESTION}",
                    "conditionId":"{CONDITION}",
                    "slug":"btc-updown-5m-1786579200",
                    "active":true,
                    "closed":false,
                    "archived":false,
                    "enableOrderBook":true,
                    "acceptingOrders":true,
                    "clobTokenIds":"[\"123\", \"456\"]",
                    "outcomes":"[\"Up\", \"Down\"]",
                    "orderPriceMinTickSize":0.01,
                    "orderMinSize":5,
                    "negRisk":false,
                    "endDate":"2026-08-13T00:05:00Z"
                }}]
            }}"#
        )
    }

    #[test]
    fn parses_exact_current_btc_five_minute_market() {
        let parsed = parse_market(
            event().as_bytes(),
            "btc-updown-5m-1786579200",
            1_786_579_200,
            1_786_579_201_000,
        )
        .unwrap();
        assert_eq!(parsed.slug(), "btc-updown-5m-1786579200");
        assert_eq!(parsed.up_token().units(), U256::from_u64(123));
        assert_eq!(parsed.down_token().units(), U256::from_u64(456));
        assert_eq!(parsed.tick().to_string(), "0.01");
        assert_eq!(parsed.minimum_order_size().to_string(), "5");
        assert_eq!(parsed.window_end_epoch(), 1_786_579_500);
    }

    #[test]
    fn rejects_non_live_or_mislabelled_market() {
        let closed = event().replacen("\"closed\":false", "\"closed\":true", 1);
        assert_eq!(
            parse_market(
                closed.as_bytes(),
                "btc-updown-5m-1786579200",
                1_786_579_200,
                1_786_579_201_000,
            ),
            Err(PmBtcFiveMinuteSourceError::InvalidMarket)
        );
        let labels = event().replace("[\\\"Up\\\", \\\"Down\\\"]", "[\\\"Yes\\\", \\\"No\\\"]");
        assert_eq!(
            parse_market(
                labels.as_bytes(),
                "btc-updown-5m-1786579200",
                1_786_579_200,
                1_786_579_201_000,
            ),
            Err(PmBtcFiveMinuteSourceError::InvalidMarket)
        );
    }
}
