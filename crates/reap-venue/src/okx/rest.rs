use std::collections::{BTreeMap, HashSet};

use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use reap_core::{
    AccountUpdate, Balance, FillFee, FillKey, FillLiquidity, MarginSnapshot, Position,
    PositionMarginMode, SelfTradePrevention, Side, TimeInForce,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::form_urlencoded;

use crate::{PrivateOrderState, RemoteFill, RemoteOrder};

use super::{AuthError, HttpMethod, OkxSigner, SignedRequest};

const PLACE_ORDER_PATH: &str = "/api/v5/trade/order";
const CANCEL_ORDER_PATH: &str = "/api/v5/trade/cancel-order";
const CANCEL_BATCH_ORDERS_PATH: &str = "/api/v5/trade/cancel-batch-orders";
const CANCEL_ALL_AFTER_PATH: &str = "/api/v5/trade/cancel-all-after";
const PUBLIC_TIME_PATH: &str = "/api/v5/public/time";
const OPEN_ORDERS_PATH: &str = "/api/v5/trade/orders-pending";
const FILLS_PATH: &str = "/api/v5/trade/fills";
const ORDER_DETAILS_PATH: &str = "/api/v5/trade/order";
const ACCOUNT_INSTRUMENTS_PATH: &str = "/api/v5/account/instruments";
const ACCOUNT_CONFIG_PATH: &str = "/api/v5/account/config";
const ACCOUNT_BALANCE_PATH: &str = "/api/v5/account/balance";
const ACCOUNT_POSITIONS_PATH: &str = "/api/v5/account/positions";
pub const OKX_FILLS_PAGE_LIMIT: usize = 100;

/// Parses an unmodified OKX trade-fills or fills-history response.
///
/// Both endpoints use the same response row shape. Keeping this parser beside
/// the signed client prevents offline evidence tooling from drifting from live
/// reconciliation semantics.
pub fn parse_okx_fills_response_json(body: &[u8]) -> Result<Vec<RemoteFill>, RestError> {
    Ok(parse_okx_fill_page_response_json(body)?.fills)
}

/// Parses one unmodified fill page and derives its next `after` cursor.
pub fn parse_okx_fill_page_response_json(body: &[u8]) -> Result<OkxFillPage, RestError> {
    let response: OkxResponse<OkxFillWire> = serde_json::from_slice(body)?;
    if response.code != "0" {
        return Err(RestError::Api {
            code: response.code,
            message: response.message,
        });
    }
    okx_fill_page(response.data)
}

/// Parses an unmodified OKX account-configuration response.
pub fn parse_okx_account_config_response_json(body: &[u8]) -> Result<OkxAccountConfig, RestError> {
    let response: OkxResponse<OkxAccountConfigWire> = decode_okx_response(body)?;
    response
        .data
        .into_iter()
        .next()
        .ok_or(RestError::EmptyData {
            operation: "account config",
        })?
        .try_into()
}

/// Parses an unmodified OKX account-balance response without discarding
/// borrowing and liability evidence.
pub fn parse_okx_account_balance_response_json(
    body: &[u8],
) -> Result<OkxAccountBalanceSnapshot, RestError> {
    let response: OkxResponse<OkxAccountBalanceWire> = decode_okx_response(body)?;
    response
        .data
        .into_iter()
        .next()
        .ok_or(RestError::EmptyData {
            operation: "account balance",
        })?
        .try_into()
}

/// Parses an unmodified OKX positions response and retains margin-loan fields.
pub fn parse_okx_account_positions_response_json(
    body: &[u8],
) -> Result<OkxAccountPositionsSnapshot, RestError> {
    let response: OkxResponse<OkxPositionWire> = decode_okx_response(body)?;
    let positions = response
        .data
        .into_iter()
        .map(OkxPositionRisk::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(OkxAccountPositionsSnapshot {
        update_time_ms: positions
            .iter()
            .map(|position| position.update_time_ms)
            .max()
            .unwrap_or(0),
        positions,
    })
}

fn okx_fill_page(data: Vec<OkxFillWire>) -> Result<OkxFillPage, RestError> {
    if data.len() > OKX_FILLS_PAGE_LIMIT {
        return Err(RestError::InvalidField {
            field: "data",
            value: data.len().to_string(),
            message: format!("fill page exceeded the requested limit {OKX_FILLS_PAGE_LIMIT}"),
        });
    }
    let next_after = if data.len() == OKX_FILLS_PAGE_LIMIT {
        let bill_id = data
            .last()
            .map(|fill| fill.bill_id.trim())
            .filter(|bill_id| !bill_id.is_empty())
            .ok_or_else(|| RestError::InvalidField {
                field: "billId",
                value: String::new(),
                message: "a full fill page requires a pagination cursor".to_string(),
            })?;
        Some(bill_id.to_string())
    } else {
        None
    };
    let fills = data
        .into_iter()
        .map(RemoteFill::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(OkxFillPage { fills, next_after })
}

#[derive(Debug, Error)]
pub enum RestError {
    #[error("request authentication failed: {0}")]
    Auth(#[from] AuthError),
    #[error("request serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("HTTP transport failed: {0}")]
    Transport(String),
    #[error("OKX API error {code}: {message}")]
    Api { code: String, message: String },
    #[error("OKX returned no data for {operation}")]
    EmptyData { operation: &'static str },
    #[error("invalid OKX response field {field}={value:?}: {message}")]
    InvalidField {
        field: &'static str,
        value: String,
        message: String,
    },
    #[error(
        "OKX fill pagination reached the configured limit after {pages} pages and {records} records; next after={next_after}"
    )]
    FillPaginationLimit {
        pages: usize,
        records: usize,
        next_after: String,
    },
    #[error("OKX fill pagination repeated cursor {cursor}")]
    FillPaginationCursor { cursor: String },
    #[error("OKX fill pagination repeated fill {symbol}/{fill_id}")]
    FillPaginationDuplicate { symbol: String, fill_id: String },
}

impl RestError {
    pub fn is_order_not_found(&self) -> bool {
        matches!(self, Self::Api { code, .. } if code == "51603")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

#[async_trait]
pub trait HttpTransport: Send + Sync {
    async fn execute(&self, request: SignedRequest) -> Result<HttpResponse, RestError>;
}

#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
    base_url: String,
}

impl ReqwestTransport {
    pub fn new(base_url: impl Into<String>) -> Result<Self, RestError> {
        Self::with_timeouts(
            base_url,
            std::time::Duration::from_secs(2),
            std::time::Duration::from_secs(5),
        )
    }

    pub fn with_timeouts(
        base_url: impl Into<String>,
        connect_timeout: std::time::Duration,
        request_timeout: std::time::Duration,
    ) -> Result<Self, RestError> {
        let client = reqwest::Client::builder()
            .connect_timeout(connect_timeout)
            .timeout(request_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .tcp_nodelay(true)
            .build()
            .map_err(|error| RestError::Transport(error.to_string()))?;
        Ok(Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        })
    }
}

#[async_trait]
impl HttpTransport for ReqwestTransport {
    async fn execute(&self, request: SignedRequest) -> Result<HttpResponse, RestError> {
        let url = format!("{}{}", self.base_url, request.path);
        let method = match request.method {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
        };
        let mut builder = self.client.request(method, url);
        for (name, value) in request.headers {
            builder = builder.header(&name, value);
        }
        if !request.body.is_empty() {
            builder = builder.body(request.body);
        }
        let response = builder
            .send()
            .await
            .map_err(|error| RestError::Transport(error.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|error| RestError::Transport(error.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(RestError::Transport(format!(
                "HTTP status {status}: {body}"
            )));
        }
        Ok(HttpResponse { status, body })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OkxTradeMode {
    Cash,
    Cross,
    Isolated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OkxInstrumentType {
    #[serde(rename = "SPOT")]
    Spot,
    #[serde(rename = "MARGIN")]
    Margin,
    #[serde(rename = "SWAP")]
    Swap,
    #[serde(rename = "FUTURES")]
    Futures,
    #[serde(rename = "OPTION")]
    Option,
}

impl OkxInstrumentType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spot => "SPOT",
            Self::Margin => "MARGIN",
            Self::Swap => "SWAP",
            Self::Futures => "FUTURES",
            Self::Option => "OPTION",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OkxContractType {
    Linear,
    Inverse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OkxAccountLevel {
    Simple,
    SingleCurrencyMargin,
    MultiCurrencyMargin,
    PortfolioMargin,
}

impl OkxAccountLevel {
    pub fn code(self) -> &'static str {
        match self {
            Self::Simple => "1",
            Self::SingleCurrencyMargin => "2",
            Self::MultiCurrencyMargin => "3",
            Self::PortfolioMargin => "4",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OkxPositionMode {
    LongShortMode,
    NetMode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OkxInstrument {
    pub symbol: String,
    pub instrument_type: OkxInstrumentType,
    pub instrument_family: String,
    pub underlying: String,
    pub base_currency: String,
    pub quote_currency: String,
    pub settle_currency: String,
    pub contract_type: Option<OkxContractType>,
    pub contract_value: Option<f64>,
    pub contract_value_currency: String,
    pub tick_size: f64,
    pub lot_size: f64,
    pub min_size: f64,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OkxAccountConfig {
    pub account_level: OkxAccountLevel,
    pub position_mode: OkxPositionMode,
    pub account_stp_mode: String,
    pub user_id: String,
    pub main_user_id: String,
    /// OKX Spot-mode borrowing switch. `None` means the exchange omitted it.
    pub enable_spot_borrow: Option<bool>,
    /// OKX multi-currency/portfolio automatic borrowing switch.
    pub auto_loan: Option<bool>,
    /// OKX Spot-mode automatic repayment switch.
    pub spot_borrow_auto_repay: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OkxAccountBalanceSnapshot {
    pub update_time_ms: u64,
    pub total_equity_usd: Option<f64>,
    pub adjusted_equity_usd: Option<f64>,
    pub borrow_frozen_usd: Option<f64>,
    pub notional_usd_for_borrow: Option<f64>,
    pub margin_ratio: Option<f64>,
    pub notional_usd: Option<f64>,
    pub details: Vec<OkxBalanceDetail>,
}

impl OkxAccountBalanceSnapshot {
    pub fn account_update(&self) -> AccountUpdate {
        let margins = if self.margin_ratio.is_none()
            && self.adjusted_equity_usd.is_none()
            && self.notional_usd.is_none()
        {
            Vec::new()
        } else {
            vec![MarginSnapshot {
                account_id: None,
                ratio: None,
                exchange_ratio: self.margin_ratio,
                adjusted_equity_usd: self.adjusted_equity_usd,
                notional_usd: self.notional_usd,
            }]
        };
        AccountUpdate {
            ts_ms: self.update_time_ms,
            balances: self
                .details
                .iter()
                .map(|detail| Balance {
                    account_id: None,
                    currency: detail.currency.clone(),
                    total: detail.cash_balance.unwrap_or(0.0),
                    available: detail.available_balance.unwrap_or(0.0),
                    equity: detail.equity.unwrap_or(0.0),
                    liability: detail.liability.unwrap_or(0.0),
                    max_loan: detail.max_loan.unwrap_or(0.0),
                    forced_repayment_indicator: detail.forced_repayment_indicator,
                })
                .collect(),
            positions: Vec::new(),
            margins,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OkxBalanceDetail {
    pub currency: String,
    pub update_time_ms: u64,
    pub cash_balance: Option<f64>,
    pub available_balance: Option<f64>,
    pub equity: Option<f64>,
    pub liability: Option<f64>,
    pub cross_liability: Option<f64>,
    pub isolated_liability: Option<f64>,
    pub unrealized_loss_liability: Option<f64>,
    pub accrued_interest: Option<f64>,
    pub borrow_frozen_usd: Option<f64>,
    pub max_loan: Option<f64>,
    pub forced_repayment_indicator: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OkxAccountPositionsSnapshot {
    pub update_time_ms: u64,
    pub positions: Vec<OkxPositionRisk>,
}

impl OkxAccountPositionsSnapshot {
    pub fn account_update(&self) -> AccountUpdate {
        AccountUpdate {
            ts_ms: self.update_time_ms,
            balances: Vec::new(),
            positions: self
                .positions
                .iter()
                .map(|position| position.position.clone())
                .collect(),
            margins: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OkxPositionRisk {
    pub instrument_type: OkxInstrumentType,
    pub position: Position,
    pub update_time_ms: u64,
    pub liability: Option<f64>,
    pub accrued_interest: Option<f64>,
    pub pending_close_order_liability: Option<f64>,
    pub base_borrowed: Option<f64>,
    pub base_interest: Option<f64>,
    pub quote_borrowed: Option<f64>,
    pub quote_interest: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OkxRawAccountConfig {
    pub request_path: String,
    pub response_body: String,
    pub config: OkxAccountConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OkxRawAccountBalance {
    pub request_path: String,
    pub response_body: String,
    pub snapshot: OkxAccountBalanceSnapshot,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OkxRawAccountPositions {
    pub request_path: String,
    pub response_body: String,
    pub snapshot: OkxAccountPositionsSnapshot,
}

impl OkxTradeMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Cash => "cash",
            Self::Cross => "cross",
            Self::Isolated => "isolated",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OkxPlaceOrder {
    pub symbol: String,
    pub trade_mode: OkxTradeMode,
    pub side: Side,
    pub time_in_force: TimeInForce,
    pub price: f64,
    pub qty: f64,
    pub client_order_id: String,
    pub reduce_only: bool,
    pub self_trade_prevention: Option<SelfTradePrevention>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OkxCancelOrder {
    pub symbol: String,
    pub exchange_order_id: Option<String>,
    pub client_order_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OkxOrderAck {
    pub exchange_order_id: String,
    pub client_order_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OkxCancelOrderResult {
    pub exchange_order_id: String,
    pub client_order_id: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OkxFillPage {
    pub fills: Vec<RemoteFill>,
    pub next_after: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OkxRawFillPage {
    pub request_path: String,
    pub response_body: String,
    pub page: OkxFillPage,
}

#[derive(Debug)]
pub struct OkxFillPagination {
    max_pages: usize,
    pages: usize,
    fills: Vec<RemoteFill>,
    after: Option<String>,
    seen_cursors: HashSet<String>,
    seen_fills: HashSet<FillKey>,
}

impl OkxFillPagination {
    pub fn new(max_pages: usize) -> Result<Self, RestError> {
        if max_pages == 0 {
            return Err(RestError::InvalidField {
                field: "max_pages",
                value: max_pages.to_string(),
                message: "must be positive".to_string(),
            });
        }
        Ok(Self {
            max_pages,
            pages: 0,
            fills: Vec::new(),
            after: None,
            seen_cursors: HashSet::new(),
            seen_fills: HashSet::new(),
        })
    }

    pub fn after(&self) -> Option<&str> {
        self.after.as_deref()
    }

    pub fn accept(&mut self, page: OkxFillPage) -> Result<bool, RestError> {
        self.pages += 1;
        for fill in page.fills {
            if !fill.fill_id.is_empty()
                && !self
                    .seen_fills
                    .insert(FillKey::new(&fill.symbol, &fill.fill_id))
            {
                return Err(RestError::FillPaginationDuplicate {
                    symbol: fill.symbol,
                    fill_id: fill.fill_id,
                });
            }
            self.fills.push(fill);
        }
        let Some(next_after) = page.next_after else {
            return Ok(true);
        };
        if !self.seen_cursors.insert(next_after.clone()) {
            return Err(RestError::FillPaginationCursor { cursor: next_after });
        }
        if self.pages == self.max_pages {
            return Err(RestError::FillPaginationLimit {
                pages: self.pages,
                records: self.fills.len(),
                next_after,
            });
        }
        self.after = Some(next_after);
        Ok(false)
    }

    pub fn into_fills(self) -> Vec<RemoteFill> {
        self.fills
    }
}

impl OkxCancelOrderResult {
    pub fn accepted(&self) -> bool {
        self.code.is_empty() || self.code == "0"
    }
}

#[derive(Clone)]
pub struct OkxRestClient<T> {
    transport: T,
    signer: OkxSigner,
    order_request_expiry_ms: Option<u64>,
}

impl<T> OkxRestClient<T>
where
    T: HttpTransport,
{
    pub fn new(transport: T, signer: OkxSigner) -> Self {
        Self {
            transport,
            signer,
            order_request_expiry_ms: None,
        }
    }

    pub fn with_order_request_expiry(mut self, expiry: std::time::Duration) -> Self {
        self.order_request_expiry_ms = Some(expiry.as_millis().max(1).min(u64::MAX as u128) as u64);
        self
    }

    pub fn signer(&self) -> &OkxSigner {
        &self.signer
    }

    pub async fn place_order(&self, order: &OkxPlaceOrder) -> Result<OkxOrderAck, RestError> {
        let now = Utc::now();
        let timestamp = now.to_rfc3339_opts(SecondsFormat::Millis, true);
        let expiry_ms = self
            .order_request_expiry_ms
            .map(|ttl_ms| (now.timestamp_millis().max(0) as u64).saturating_add(ttl_ms));
        self.place_order_with_expiry_at(&timestamp, expiry_ms, order)
            .await
    }

    pub async fn place_order_at(
        &self,
        timestamp: &str,
        order: &OkxPlaceOrder,
    ) -> Result<OkxOrderAck, RestError> {
        let expiry_ms = self
            .order_request_expiry_ms
            .map(|ttl_ms| timestamp_ms(timestamp).map(|now_ms| now_ms.saturating_add(ttl_ms)))
            .transpose()?;
        self.place_order_with_expiry_at(timestamp, expiry_ms, order)
            .await
    }

    pub async fn place_order_with_expiry_at(
        &self,
        timestamp: &str,
        expiry_ms: Option<u64>,
        order: &OkxPlaceOrder,
    ) -> Result<OkxOrderAck, RestError> {
        let request = self.build_place_request_with_expiry(timestamp, expiry_ms, order)?;
        self.execute_ack(request, "place order").await
    }

    pub async fn server_time_ms(&self) -> Result<u64, RestError> {
        let request = SignedRequest {
            method: HttpMethod::Get,
            path: PUBLIC_TIME_PATH.to_string(),
            body: String::new(),
            headers: BTreeMap::new(),
        };
        let response: OkxResponse<OkxTimeWire> = self.execute(request).await?;
        let wire = response
            .data
            .into_iter()
            .next()
            .ok_or(RestError::EmptyData {
                operation: "server time",
            })?;
        parse_integer("ts", &wire.timestamp)
    }

    pub async fn cancel_all_after(&self, timeout_secs: u64) -> Result<(), RestError> {
        self.cancel_all_after_at(&timestamp_now(), timeout_secs)
            .await
    }

    pub async fn cancel_all_after_at(
        &self,
        timestamp: &str,
        timeout_secs: u64,
    ) -> Result<(), RestError> {
        if timeout_secs != 0 && !(10..=120).contains(&timeout_secs) {
            return Err(RestError::InvalidField {
                field: "timeOut",
                value: timeout_secs.to_string(),
                message: "must be 0 or between 10 and 120 seconds".to_string(),
            });
        }
        #[derive(Serialize)]
        struct Body {
            #[serde(rename = "timeOut")]
            timeout_secs: String,
        }
        let body = serde_json::to_string(&Body {
            timeout_secs: timeout_secs.to_string(),
        })?;
        let request =
            self.signer
                .sign_request(timestamp, HttpMethod::Post, CANCEL_ALL_AFTER_PATH, body)?;
        let response: OkxResponse<OkxCancelAllAfterWire> = self.execute(request).await?;
        let acknowledgement = response
            .data
            .into_iter()
            .next()
            .ok_or(RestError::EmptyData {
                operation: "cancel all after",
            })?;
        if timeout_secs != 0 && parse_integer("triggerTime", &acknowledgement.trigger_time)? == 0 {
            return Err(RestError::InvalidField {
                field: "triggerTime",
                value: acknowledgement.trigger_time,
                message: "must be nonzero when Cancel All After is armed".to_string(),
            });
        }
        Ok(())
    }

    pub async fn cancel_order(&self, order: &OkxCancelOrder) -> Result<OkxOrderAck, RestError> {
        self.cancel_order_at(&timestamp_now(), order).await
    }

    pub async fn cancel_order_at(
        &self,
        timestamp: &str,
        order: &OkxCancelOrder,
    ) -> Result<OkxOrderAck, RestError> {
        let request = self.build_cancel_request(timestamp, order)?;
        self.execute_ack(request, "cancel order").await
    }

    pub async fn cancel_batch_orders(
        &self,
        orders: &[OkxCancelOrder],
    ) -> Result<Vec<OkxCancelOrderResult>, RestError> {
        self.cancel_batch_orders_at(&timestamp_now(), orders).await
    }

    pub async fn cancel_batch_orders_at(
        &self,
        timestamp: &str,
        orders: &[OkxCancelOrder],
    ) -> Result<Vec<OkxCancelOrderResult>, RestError> {
        let request = self.build_cancel_batch_request(timestamp, orders)?;
        let response: OkxResponse<OkxAckWire> = self.execute(request).await?;
        if response.data.is_empty() {
            return Err(RestError::EmptyData {
                operation: "cancel batch orders",
            });
        }
        Ok(response
            .data
            .into_iter()
            .map(|ack| OkxCancelOrderResult {
                exchange_order_id: ack.order_id,
                client_order_id: ack.client_order_id,
                code: ack.sub_code,
                message: ack.sub_message,
            })
            .collect())
    }

    pub async fn open_orders(
        &self,
        instrument_type: Option<&str>,
        symbol: Option<&str>,
    ) -> Result<Vec<RemoteOrder>, RestError> {
        self.open_orders_at(&timestamp_now(), instrument_type, symbol)
            .await
    }

    pub async fn open_orders_at(
        &self,
        timestamp: &str,
        instrument_type: Option<&str>,
        symbol: Option<&str>,
    ) -> Result<Vec<RemoteOrder>, RestError> {
        let path = query_path(
            OPEN_ORDERS_PATH,
            [("instType", instrument_type), ("instId", symbol)],
        );
        let request = self
            .signer
            .sign_request(timestamp, HttpMethod::Get, path, "")?;
        let response: OkxResponse<OkxOrderWire> = self.execute(request).await?;
        response
            .data
            .into_iter()
            .map(RemoteOrder::try_from)
            .collect()
    }

    pub async fn fills(
        &self,
        instrument_type: Option<&str>,
        symbol: Option<&str>,
    ) -> Result<Vec<RemoteFill>, RestError> {
        self.fills_at(&timestamp_now(), instrument_type, symbol)
            .await
    }

    pub async fn fills_at(
        &self,
        timestamp: &str,
        instrument_type: Option<&str>,
        symbol: Option<&str>,
    ) -> Result<Vec<RemoteFill>, RestError> {
        Ok(self
            .fills_page_at(timestamp, instrument_type, symbol, None)
            .await?
            .fills)
    }

    pub async fn fills_page(
        &self,
        instrument_type: Option<&str>,
        symbol: Option<&str>,
        after: Option<&str>,
    ) -> Result<OkxFillPage, RestError> {
        self.fills_page_at(&timestamp_now(), instrument_type, symbol, after)
            .await
    }

    pub async fn fills_page_at(
        &self,
        timestamp: &str,
        instrument_type: Option<&str>,
        symbol: Option<&str>,
        after: Option<&str>,
    ) -> Result<OkxFillPage, RestError> {
        Ok(self
            .fills_page_raw_at(timestamp, instrument_type, symbol, after)
            .await?
            .page)
    }

    /// Retrieves one recent-fill page while retaining the exact response body.
    pub async fn fills_page_raw(
        &self,
        instrument_type: Option<&str>,
        symbol: Option<&str>,
        after: Option<&str>,
    ) -> Result<OkxRawFillPage, RestError> {
        self.fills_page_raw_at(&timestamp_now(), instrument_type, symbol, after)
            .await
    }

    pub async fn fills_page_raw_at(
        &self,
        timestamp: &str,
        instrument_type: Option<&str>,
        symbol: Option<&str>,
        after: Option<&str>,
    ) -> Result<OkxRawFillPage, RestError> {
        let path = query_path(
            FILLS_PATH,
            [
                ("instType", instrument_type),
                ("instId", symbol),
                ("after", after),
                ("limit", Some("100")),
            ],
        );
        let request = self
            .signer
            .sign_request(timestamp, HttpMethod::Get, path.clone(), "")?;
        let response = self.transport.execute(request).await?;
        let page = parse_okx_fill_page_response_json(response.body.as_bytes())?;
        Ok(OkxRawFillPage {
            request_path: path,
            response_body: response.body,
            page,
        })
    }

    /// Retrieves complete recent-fill pages up to a fail-closed bound.
    ///
    /// This transport helper does not pace requests. Live callers use the
    /// account gateway, which reserves one reconciliation request per page.
    pub async fn fills_paginated(
        &self,
        instrument_type: Option<&str>,
        symbol: Option<&str>,
        max_pages: usize,
    ) -> Result<Vec<RemoteFill>, RestError> {
        let mut pagination = OkxFillPagination::new(max_pages)?;
        loop {
            let page = self
                .fills_page(instrument_type, symbol, pagination.after())
                .await?;
            if pagination.accept(page)? {
                return Ok(pagination.into_fills());
            }
        }
    }

    pub async fn order_details(
        &self,
        symbol: &str,
        exchange_order_id: Option<&str>,
        client_order_id: Option<&str>,
    ) -> Result<RemoteOrder, RestError> {
        self.order_details_at(&timestamp_now(), symbol, exchange_order_id, client_order_id)
            .await
    }

    pub async fn order_details_at(
        &self,
        timestamp: &str,
        symbol: &str,
        exchange_order_id: Option<&str>,
        client_order_id: Option<&str>,
    ) -> Result<RemoteOrder, RestError> {
        if exchange_order_id.is_none() && client_order_id.is_none() {
            return Err(RestError::InvalidField {
                field: "ordId/clOrdId",
                value: String::new(),
                message: "one identifier is required".to_string(),
            });
        }
        let path = query_path(
            ORDER_DETAILS_PATH,
            [
                ("instId", Some(symbol)),
                ("ordId", exchange_order_id),
                ("clOrdId", client_order_id),
            ],
        );
        let request = self
            .signer
            .sign_request(timestamp, HttpMethod::Get, path, "")?;
        let response: OkxResponse<OkxOrderWire> = self.execute(request).await?;
        response
            .data
            .into_iter()
            .next()
            .ok_or(RestError::EmptyData {
                operation: "order details",
            })?
            .try_into()
    }

    pub async fn account_instruments(
        &self,
        instrument_type: OkxInstrumentType,
        symbol: Option<&str>,
    ) -> Result<Vec<OkxInstrument>, RestError> {
        self.account_instruments_at(&timestamp_now(), instrument_type, symbol)
            .await
    }

    pub async fn account_instruments_at(
        &self,
        timestamp: &str,
        instrument_type: OkxInstrumentType,
        symbol: Option<&str>,
    ) -> Result<Vec<OkxInstrument>, RestError> {
        let path = query_path(
            ACCOUNT_INSTRUMENTS_PATH,
            [
                ("instType", Some(instrument_type.as_str())),
                ("instId", symbol),
            ],
        );
        let request = self
            .signer
            .sign_request(timestamp, HttpMethod::Get, path, "")?;
        let response: OkxResponse<OkxInstrumentWire> = self.execute(request).await?;
        response
            .data
            .into_iter()
            .map(OkxInstrument::try_from)
            .collect()
    }

    pub async fn account_config(&self) -> Result<OkxAccountConfig, RestError> {
        self.account_config_at(&timestamp_now()).await
    }

    pub async fn account_config_at(&self, timestamp: &str) -> Result<OkxAccountConfig, RestError> {
        Ok(self.account_config_raw_at(timestamp).await?.config)
    }

    /// Retrieves account configuration while retaining the exact response body.
    pub async fn account_config_raw(&self) -> Result<OkxRawAccountConfig, RestError> {
        self.account_config_raw_at(&timestamp_now()).await
    }

    pub async fn account_config_raw_at(
        &self,
        timestamp: &str,
    ) -> Result<OkxRawAccountConfig, RestError> {
        let request =
            self.signer
                .sign_request(timestamp, HttpMethod::Get, ACCOUNT_CONFIG_PATH, "")?;
        let response = self.transport.execute(request).await?;
        let config = parse_okx_account_config_response_json(response.body.as_bytes())?;
        Ok(OkxRawAccountConfig {
            request_path: ACCOUNT_CONFIG_PATH.to_string(),
            response_body: response.body,
            config,
        })
    }

    pub async fn account_balance(&self) -> Result<AccountUpdate, RestError> {
        self.account_balance_at(&timestamp_now()).await
    }

    pub async fn account_balance_at(&self, timestamp: &str) -> Result<AccountUpdate, RestError> {
        Ok(self
            .account_balance_snapshot_at(timestamp)
            .await?
            .account_update())
    }

    pub async fn account_balance_snapshot(&self) -> Result<OkxAccountBalanceSnapshot, RestError> {
        self.account_balance_snapshot_at(&timestamp_now()).await
    }

    pub async fn account_balance_snapshot_at(
        &self,
        timestamp: &str,
    ) -> Result<OkxAccountBalanceSnapshot, RestError> {
        Ok(self.account_balance_raw_at(timestamp).await?.snapshot)
    }

    /// Retrieves account balances while retaining the exact response body.
    pub async fn account_balance_raw(&self) -> Result<OkxRawAccountBalance, RestError> {
        self.account_balance_raw_at(&timestamp_now()).await
    }

    pub async fn account_balance_raw_at(
        &self,
        timestamp: &str,
    ) -> Result<OkxRawAccountBalance, RestError> {
        let request =
            self.signer
                .sign_request(timestamp, HttpMethod::Get, ACCOUNT_BALANCE_PATH, "")?;
        let response = self.transport.execute(request).await?;
        let snapshot = parse_okx_account_balance_response_json(response.body.as_bytes())?;
        Ok(OkxRawAccountBalance {
            request_path: ACCOUNT_BALANCE_PATH.to_string(),
            response_body: response.body,
            snapshot,
        })
    }

    pub async fn account_positions(
        &self,
        instrument_type: Option<OkxInstrumentType>,
        symbol: Option<&str>,
    ) -> Result<AccountUpdate, RestError> {
        self.account_positions_at(&timestamp_now(), instrument_type, symbol)
            .await
    }

    pub async fn account_positions_at(
        &self,
        timestamp: &str,
        instrument_type: Option<OkxInstrumentType>,
        symbol: Option<&str>,
    ) -> Result<AccountUpdate, RestError> {
        Ok(self
            .account_positions_snapshot_at(timestamp, instrument_type, symbol)
            .await?
            .account_update())
    }

    pub async fn account_positions_snapshot(
        &self,
        instrument_type: Option<OkxInstrumentType>,
        symbol: Option<&str>,
    ) -> Result<OkxAccountPositionsSnapshot, RestError> {
        self.account_positions_snapshot_at(&timestamp_now(), instrument_type, symbol)
            .await
    }

    pub async fn account_positions_snapshot_at(
        &self,
        timestamp: &str,
        instrument_type: Option<OkxInstrumentType>,
        symbol: Option<&str>,
    ) -> Result<OkxAccountPositionsSnapshot, RestError> {
        Ok(self
            .account_positions_raw_at(timestamp, instrument_type, symbol)
            .await?
            .snapshot)
    }

    /// Retrieves positions while retaining the exact response body.
    pub async fn account_positions_raw(
        &self,
        instrument_type: Option<OkxInstrumentType>,
        symbol: Option<&str>,
    ) -> Result<OkxRawAccountPositions, RestError> {
        self.account_positions_raw_at(&timestamp_now(), instrument_type, symbol)
            .await
    }

    pub async fn account_positions_raw_at(
        &self,
        timestamp: &str,
        instrument_type: Option<OkxInstrumentType>,
        symbol: Option<&str>,
    ) -> Result<OkxRawAccountPositions, RestError> {
        let path = query_path(
            ACCOUNT_POSITIONS_PATH,
            [
                ("instType", instrument_type.map(OkxInstrumentType::as_str)),
                ("instId", symbol),
            ],
        );
        let request = self
            .signer
            .sign_request(timestamp, HttpMethod::Get, path.clone(), "")?;
        let response = self.transport.execute(request).await?;
        let snapshot = parse_okx_account_positions_response_json(response.body.as_bytes())?;
        Ok(OkxRawAccountPositions {
            request_path: path,
            response_body: response.body,
            snapshot,
        })
    }

    pub fn build_place_request(
        &self,
        timestamp: &str,
        order: &OkxPlaceOrder,
    ) -> Result<SignedRequest, RestError> {
        self.build_place_request_with_expiry(timestamp, None, order)
    }

    pub fn build_place_request_with_expiry(
        &self,
        timestamp: &str,
        expiry_ms: Option<u64>,
        order: &OkxPlaceOrder,
    ) -> Result<SignedRequest, RestError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            #[serde(rename = "instId")]
            symbol: &'a str,
            #[serde(rename = "tdMode")]
            trade_mode: &'static str,
            side: &'static str,
            #[serde(rename = "ordType")]
            order_type: &'static str,
            px: String,
            sz: String,
            #[serde(rename = "clOrdId")]
            client_order_id: &'a str,
            #[serde(rename = "reduceOnly", skip_serializing_if = "Option::is_none")]
            reduce_only: Option<bool>,
            #[serde(rename = "stpMode", skip_serializing_if = "Option::is_none")]
            self_trade_prevention: Option<&'static str>,
        }

        let body = serde_json::to_string(&Body {
            symbol: &order.symbol,
            trade_mode: order.trade_mode.as_str(),
            side: side_string(order.side),
            order_type: time_in_force_string(order.time_in_force),
            px: decimal_string(order.price),
            sz: decimal_string(order.qty),
            client_order_id: &order.client_order_id,
            reduce_only: order.reduce_only.then_some(true),
            self_trade_prevention: order.self_trade_prevention.map(stp_mode_string),
        })?;
        let mut request =
            self.signer
                .sign_request(timestamp, HttpMethod::Post, PLACE_ORDER_PATH, body)?;
        if let Some(expiry_ms) = expiry_ms {
            request
                .headers
                .insert("expTime".to_string(), expiry_ms.to_string());
        }
        Ok(request)
    }

    pub fn build_cancel_request(
        &self,
        timestamp: &str,
        order: &OkxCancelOrder,
    ) -> Result<SignedRequest, RestError> {
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(rename = "instId")]
            symbol: &'a str,
            #[serde(rename = "ordId", skip_serializing_if = "Option::is_none")]
            exchange_order_id: Option<&'a str>,
            #[serde(rename = "clOrdId", skip_serializing_if = "Option::is_none")]
            client_order_id: Option<&'a str>,
        }

        if order.exchange_order_id.is_none() && order.client_order_id.is_none() {
            return Err(RestError::InvalidField {
                field: "ordId/clOrdId",
                value: String::new(),
                message: "one identifier is required".to_string(),
            });
        }
        let body = serde_json::to_string(&Body {
            symbol: &order.symbol,
            exchange_order_id: order.exchange_order_id.as_deref(),
            client_order_id: order.client_order_id.as_deref(),
        })?;
        Ok(self
            .signer
            .sign_request(timestamp, HttpMethod::Post, CANCEL_ORDER_PATH, body)?)
    }

    pub fn build_cancel_batch_request(
        &self,
        timestamp: &str,
        orders: &[OkxCancelOrder],
    ) -> Result<SignedRequest, RestError> {
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(rename = "instId")]
            symbol: &'a str,
            #[serde(rename = "ordId", skip_serializing_if = "Option::is_none")]
            exchange_order_id: Option<&'a str>,
            #[serde(rename = "clOrdId", skip_serializing_if = "Option::is_none")]
            client_order_id: Option<&'a str>,
        }

        if orders.is_empty() || orders.len() > 20 {
            return Err(RestError::InvalidField {
                field: "orders",
                value: orders.len().to_string(),
                message: "cancel batch must contain 1-20 orders".to_string(),
            });
        }
        let mut body = Vec::with_capacity(orders.len());
        for order in orders {
            if order.exchange_order_id.is_none() && order.client_order_id.is_none() {
                return Err(RestError::InvalidField {
                    field: "ordId/clOrdId",
                    value: String::new(),
                    message: "one identifier is required for every cancel".to_string(),
                });
            }
            body.push(Body {
                symbol: &order.symbol,
                exchange_order_id: order.exchange_order_id.as_deref(),
                client_order_id: order.client_order_id.as_deref(),
            });
        }
        Ok(self.signer.sign_request(
            timestamp,
            HttpMethod::Post,
            CANCEL_BATCH_ORDERS_PATH,
            serde_json::to_string(&body)?,
        )?)
    }

    async fn execute_ack(
        &self,
        request: SignedRequest,
        operation: &'static str,
    ) -> Result<OkxOrderAck, RestError> {
        let response: OkxResponse<OkxAckWire> = self.execute(request).await?;
        let ack = response
            .data
            .into_iter()
            .next()
            .ok_or(RestError::EmptyData { operation })?;
        if !ack.sub_code.is_empty() && ack.sub_code != "0" {
            return Err(RestError::Api {
                code: ack.sub_code,
                message: ack.sub_message,
            });
        }
        Ok(OkxOrderAck {
            exchange_order_id: ack.order_id,
            client_order_id: ack.client_order_id,
        })
    }

    async fn execute<R: DeserializeOwned>(
        &self,
        request: SignedRequest,
    ) -> Result<OkxResponse<R>, RestError> {
        let response = self.transport.execute(request).await?;
        let decoded: OkxResponse<R> = serde_json::from_str(&response.body)?;
        if decoded.code != "0" {
            return Err(RestError::Api {
                code: decoded.code,
                message: decoded.message,
            });
        }
        Ok(decoded)
    }
}

fn timestamp_now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn format_okx_timestamp_ms(timestamp_ms: u64) -> Result<String, RestError> {
    let timestamp_ms = i64::try_from(timestamp_ms).map_err(|error| RestError::InvalidField {
        field: "OK-ACCESS-TIMESTAMP",
        value: timestamp_ms.to_string(),
        message: error.to_string(),
    })?;
    let timestamp =
        chrono::DateTime::<Utc>::from_timestamp_millis(timestamp_ms).ok_or_else(|| {
            RestError::InvalidField {
                field: "OK-ACCESS-TIMESTAMP",
                value: timestamp_ms.to_string(),
                message: "timestamp is outside the supported range".to_string(),
            }
        })?;
    Ok(timestamp.to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn timestamp_ms(timestamp: &str) -> Result<u64, RestError> {
    let timestamp = chrono::DateTime::parse_from_rfc3339(timestamp).map_err(|error| {
        RestError::InvalidField {
            field: "OK-ACCESS-TIMESTAMP",
            value: timestamp.to_string(),
            message: error.to_string(),
        }
    })?;
    u64::try_from(timestamp.timestamp_millis()).map_err(|error| RestError::InvalidField {
        field: "OK-ACCESS-TIMESTAMP",
        value: timestamp.to_rfc3339(),
        message: error.to_string(),
    })
}

fn query_path<'a>(
    base: &str,
    parameters: impl IntoIterator<Item = (&'a str, Option<&'a str>)>,
) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (name, value) in parameters {
        if let Some(value) = value {
            serializer.append_pair(name, value);
        }
    }
    let query = serializer.finish();
    if query.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{query}")
    }
}

fn decimal_string(value: f64) -> String {
    value.to_string()
}

fn side_string(side: Side) -> &'static str {
    match side {
        Side::Buy => "buy",
        Side::Sell => "sell",
    }
}

fn time_in_force_string(time_in_force: TimeInForce) -> &'static str {
    match time_in_force {
        TimeInForce::Gtc => "limit",
        TimeInForce::Ioc => "ioc",
        TimeInForce::PostOnly => "post_only",
    }
}

fn stp_mode_string(mode: SelfTradePrevention) -> &'static str {
    match mode {
        SelfTradePrevention::CancelMaker => "cancel_maker",
        SelfTradePrevention::CancelTaker => "cancel_taker",
        SelfTradePrevention::CancelBoth => "cancel_both",
    }
}

#[derive(Debug, Deserialize)]
struct OkxResponse<T> {
    code: String,
    #[serde(rename = "msg")]
    message: String,
    data: Vec<T>,
}

fn decode_okx_response<T: DeserializeOwned>(body: &[u8]) -> Result<OkxResponse<T>, RestError> {
    let decoded: OkxResponse<T> = serde_json::from_slice(body)?;
    if decoded.code != "0" {
        return Err(RestError::Api {
            code: decoded.code,
            message: decoded.message,
        });
    }
    Ok(decoded)
}

#[derive(Debug, Deserialize)]
struct OkxAckWire {
    #[serde(default, rename = "ordId")]
    order_id: String,
    #[serde(default, rename = "clOrdId")]
    client_order_id: String,
    #[serde(default, rename = "sCode")]
    sub_code: String,
    #[serde(default, rename = "sMsg")]
    sub_message: String,
}

#[derive(Debug, Deserialize)]
struct OkxTimeWire {
    #[serde(rename = "ts")]
    timestamp: String,
}

#[derive(Debug, Deserialize)]
struct OkxCancelAllAfterWire {
    #[serde(default, rename = "triggerTime")]
    trigger_time: String,
}

#[derive(Debug, Deserialize)]
struct OkxInstrumentWire {
    #[serde(rename = "instId")]
    symbol: String,
    #[serde(rename = "instType")]
    instrument_type: String,
    #[serde(default, rename = "instFamily")]
    instrument_family: String,
    #[serde(default, rename = "uly")]
    underlying: String,
    #[serde(default, rename = "baseCcy")]
    base_currency: String,
    #[serde(default, rename = "quoteCcy")]
    quote_currency: String,
    #[serde(default, rename = "settleCcy")]
    settle_currency: String,
    #[serde(default, rename = "ctType")]
    contract_type: String,
    #[serde(default, rename = "ctVal")]
    contract_value: String,
    #[serde(default, rename = "ctValCcy")]
    contract_value_currency: String,
    #[serde(rename = "tickSz")]
    tick_size: String,
    #[serde(rename = "lotSz")]
    lot_size: String,
    #[serde(rename = "minSz")]
    min_size: String,
    state: String,
}

impl TryFrom<OkxInstrumentWire> for OkxInstrument {
    type Error = RestError;

    fn try_from(value: OkxInstrumentWire) -> Result<Self, Self::Error> {
        Ok(Self {
            symbol: value.symbol,
            instrument_type: parse_instrument_type(&value.instrument_type)?,
            instrument_family: value.instrument_family,
            underlying: value.underlying,
            base_currency: value.base_currency,
            quote_currency: value.quote_currency,
            settle_currency: value.settle_currency,
            contract_type: parse_contract_type(&value.contract_type)?,
            contract_value: parse_nullable_number("ctVal", &value.contract_value)?,
            contract_value_currency: value.contract_value_currency,
            tick_size: parse_positive_number("tickSz", &value.tick_size)?,
            lot_size: parse_positive_number("lotSz", &value.lot_size)?,
            min_size: parse_positive_number("minSz", &value.min_size)?,
            state: value.state,
        })
    }
}

#[derive(Debug, Deserialize)]
struct OkxAccountConfigWire {
    #[serde(rename = "acctLv")]
    account_level: String,
    #[serde(rename = "posMode")]
    position_mode: String,
    #[serde(default, rename = "acctStpMode")]
    account_stp_mode: String,
    #[serde(default)]
    uid: String,
    #[serde(default, rename = "mainUid")]
    main_user_id: String,
    #[serde(default, rename = "enableSpotBorrow")]
    enable_spot_borrow: Option<bool>,
    #[serde(default, rename = "autoLoan")]
    auto_loan: Option<bool>,
    #[serde(default, rename = "spotBorrowAutoRepay")]
    spot_borrow_auto_repay: Option<bool>,
}

impl TryFrom<OkxAccountConfigWire> for OkxAccountConfig {
    type Error = RestError;

    fn try_from(value: OkxAccountConfigWire) -> Result<Self, Self::Error> {
        Ok(Self {
            account_level: parse_account_level(&value.account_level)?,
            position_mode: parse_position_mode(&value.position_mode)?,
            account_stp_mode: value.account_stp_mode,
            user_id: value.uid,
            main_user_id: value.main_user_id,
            enable_spot_borrow: value.enable_spot_borrow,
            auto_loan: value.auto_loan,
            spot_borrow_auto_repay: value.spot_borrow_auto_repay,
        })
    }
}

#[derive(Debug, Deserialize)]
struct OkxAccountBalanceWire {
    #[serde(default, rename = "uTime")]
    update_time: String,
    #[serde(default, rename = "totalEq")]
    total_equity: String,
    #[serde(default, rename = "mgnRatio")]
    margin_ratio: String,
    #[serde(default, rename = "adjEq")]
    adjusted_equity: String,
    #[serde(default, rename = "borrowFroz")]
    borrow_frozen: String,
    #[serde(default, rename = "notionalUsdForBorrow")]
    notional_usd_for_borrow: String,
    #[serde(default, rename = "notionalUsd")]
    notional_usd: String,
    #[serde(default)]
    details: Vec<OkxBalanceWire>,
}

impl TryFrom<OkxAccountBalanceWire> for OkxAccountBalanceSnapshot {
    type Error = RestError;

    fn try_from(value: OkxAccountBalanceWire) -> Result<Self, Self::Error> {
        let details = value
            .details
            .into_iter()
            .map(|detail| {
                Ok(OkxBalanceDetail {
                    currency: detail.currency,
                    update_time_ms: parse_optional_integer("uTime", &detail.update_time)?,
                    cash_balance: parse_nullable_number("cashBal", &detail.cash_balance)?,
                    available_balance: parse_nullable_number(
                        "availBal",
                        &detail.available_balance,
                    )?,
                    equity: parse_nullable_number("eq", &detail.equity)?,
                    liability: parse_nullable_number("liab", &detail.liability)?,
                    cross_liability: parse_nullable_number("crossLiab", &detail.cross_liability)?,
                    isolated_liability: parse_nullable_number(
                        "isoLiab",
                        &detail.isolated_liability,
                    )?,
                    unrealized_loss_liability: parse_nullable_number(
                        "uplLiab",
                        &detail.unrealized_loss_liability,
                    )?,
                    accrued_interest: parse_nullable_number("interest", &detail.accrued_interest)?,
                    borrow_frozen_usd: parse_nullable_number("borrowFroz", &detail.borrow_frozen)?,
                    max_loan: parse_nullable_number("maxLoan", &detail.max_loan)?,
                    forced_repayment_indicator: parse_forced_repayment_indicator(
                        &detail.forced_repayment_indicator,
                    )?,
                })
            })
            .collect::<Result<Vec<_>, RestError>>()?;
        Ok(OkxAccountBalanceSnapshot {
            update_time_ms: parse_optional_integer("uTime", &value.update_time)?,
            total_equity_usd: parse_nullable_number("totalEq", &value.total_equity)?,
            adjusted_equity_usd: parse_nullable_number("adjEq", &value.adjusted_equity)?,
            borrow_frozen_usd: parse_nullable_number("borrowFroz", &value.borrow_frozen)?,
            notional_usd_for_borrow: parse_nullable_number(
                "notionalUsdForBorrow",
                &value.notional_usd_for_borrow,
            )?,
            margin_ratio: parse_nullable_number("mgnRatio", &value.margin_ratio)?,
            notional_usd: parse_nullable_number("notionalUsd", &value.notional_usd)?,
            details,
        })
    }
}

#[derive(Debug, Deserialize)]
struct OkxBalanceWire {
    #[serde(rename = "ccy")]
    currency: String,
    #[serde(default, rename = "uTime")]
    update_time: String,
    #[serde(default, rename = "cashBal")]
    cash_balance: String,
    #[serde(default, rename = "availBal")]
    available_balance: String,
    #[serde(default, rename = "eq")]
    equity: String,
    #[serde(default, rename = "liab")]
    liability: String,
    #[serde(default, rename = "crossLiab")]
    cross_liability: String,
    #[serde(default, rename = "isoLiab")]
    isolated_liability: String,
    #[serde(default, rename = "uplLiab")]
    unrealized_loss_liability: String,
    #[serde(default, rename = "interest")]
    accrued_interest: String,
    #[serde(default, rename = "borrowFroz")]
    borrow_frozen: String,
    #[serde(default, rename = "maxLoan")]
    max_loan: String,
    #[serde(default, rename = "twap")]
    forced_repayment_indicator: String,
}

#[derive(Debug, Deserialize)]
struct OkxPositionWire {
    #[serde(rename = "instType")]
    instrument_type: String,
    #[serde(rename = "instId")]
    symbol: String,
    #[serde(rename = "pos")]
    qty: String,
    #[serde(default, rename = "avgPx")]
    average_price: String,
    #[serde(default, rename = "posSide")]
    position_side: String,
    #[serde(rename = "mgnMode")]
    margin_mode: String,
    #[serde(default, rename = "uTime")]
    update_time: String,
    #[serde(default, rename = "liab")]
    liability: String,
    #[serde(default, rename = "interest")]
    accrued_interest: String,
    #[serde(default, rename = "pendingCloseOrdLiabVal")]
    pending_close_order_liability: String,
    #[serde(default, rename = "baseBorrowed")]
    base_borrowed: String,
    #[serde(default, rename = "baseInterest")]
    base_interest: String,
    #[serde(default, rename = "quoteBorrowed")]
    quote_borrowed: String,
    #[serde(default, rename = "quoteInterest")]
    quote_interest: String,
}

impl TryFrom<OkxPositionWire> for OkxPositionRisk {
    type Error = RestError;

    fn try_from(value: OkxPositionWire) -> Result<Self, Self::Error> {
        let OkxPositionWire {
            instrument_type,
            symbol,
            qty,
            average_price,
            position_side,
            margin_mode,
            update_time,
            liability,
            accrued_interest,
            pending_close_order_liability,
            base_borrowed,
            base_interest,
            quote_borrowed,
            quote_interest,
        } = value;
        let mut qty = parse_number("pos", &qty)?;
        match position_side.as_str() {
            "" | "net" | "long" => {}
            "short" => qty = -qty.abs(),
            other => {
                return Err(RestError::InvalidField {
                    field: "posSide",
                    value: other.to_string(),
                    message: "expected net, long, or short".to_string(),
                });
            }
        }
        Ok(Self {
            instrument_type: parse_instrument_type(&instrument_type)?,
            position: Position {
                symbol,
                qty,
                avg_price: parse_optional_number("avgPx", &average_price)?,
                margin_mode: Some(parse_position_margin_mode(&margin_mode)?),
            },
            update_time_ms: parse_optional_integer("uTime", &update_time)?,
            liability: parse_nullable_number("liab", &liability)?,
            accrued_interest: parse_nullable_number("interest", &accrued_interest)?,
            pending_close_order_liability: parse_nullable_number(
                "pendingCloseOrdLiabVal",
                &pending_close_order_liability,
            )?,
            base_borrowed: parse_nullable_number("baseBorrowed", &base_borrowed)?,
            base_interest: parse_nullable_number("baseInterest", &base_interest)?,
            quote_borrowed: parse_nullable_number("quoteBorrowed", &quote_borrowed)?,
            quote_interest: parse_nullable_number("quoteInterest", &quote_interest)?,
        })
    }
}

#[derive(Debug, Deserialize)]
struct OkxOrderWire {
    #[serde(default, rename = "ordId")]
    order_id: String,
    #[serde(default, rename = "clOrdId")]
    client_order_id: String,
    #[serde(rename = "instId")]
    symbol: String,
    side: String,
    state: String,
    #[serde(default)]
    px: String,
    #[serde(default)]
    sz: String,
    #[serde(default, rename = "accFillSz")]
    cumulative_filled_qty: String,
    #[serde(default, rename = "avgPx")]
    average_fill_price: String,
    #[serde(default, rename = "uTime")]
    update_time: String,
}

impl TryFrom<OkxOrderWire> for RemoteOrder {
    type Error = RestError;

    fn try_from(value: OkxOrderWire) -> Result<Self, Self::Error> {
        Ok(Self {
            exchange_order_id: value.order_id,
            client_order_id: value.client_order_id,
            symbol: value.symbol,
            side: parse_side(&value.side)?,
            state: parse_state(&value.state)?,
            price: parse_optional_number("px", &value.px)?,
            qty: parse_optional_number("sz", &value.sz)?,
            cumulative_filled_qty: parse_optional_number(
                "accFillSz",
                &value.cumulative_filled_qty,
            )?,
            average_fill_price: parse_optional_number("avgPx", &value.average_fill_price)?,
            update_time_ms: parse_optional_integer("uTime", &value.update_time)?,
        })
    }
}

#[derive(Debug, Deserialize)]
struct OkxFillWire {
    #[serde(default, rename = "billId")]
    bill_id: String,
    #[serde(default, rename = "tradeId")]
    fill_id: String,
    #[serde(default, rename = "ordId")]
    order_id: String,
    #[serde(default, rename = "clOrdId")]
    client_order_id: String,
    #[serde(rename = "instId")]
    symbol: String,
    side: String,
    #[serde(rename = "fillPx")]
    price: String,
    #[serde(rename = "fillSz")]
    qty: String,
    #[serde(default, rename = "execType")]
    execution_type: String,
    #[serde(default)]
    fee: String,
    #[serde(default, rename = "feeCcy")]
    fee_currency: String,
    #[serde(rename = "fillTime")]
    fill_time: String,
}

impl TryFrom<OkxFillWire> for RemoteFill {
    type Error = RestError;

    fn try_from(value: OkxFillWire) -> Result<Self, Self::Error> {
        validate_required_text("tradeId", &value.fill_id)?;
        validate_required_text("ordId", &value.order_id)?;
        validate_required_text("instId", &value.symbol)?;
        let client_order_id = if value.client_order_id == "0" {
            String::new()
        } else {
            value.client_order_id
        };
        let price = parse_number("fillPx", &value.price)?;
        if price <= 0.0 {
            return Err(RestError::InvalidField {
                field: "fillPx",
                value: value.price,
                message: "must be positive".to_string(),
            });
        }
        let qty = parse_number("fillSz", &value.qty)?;
        if qty <= 0.0 {
            return Err(RestError::InvalidField {
                field: "fillSz",
                value: value.qty,
                message: "must be positive".to_string(),
            });
        }
        let ts_ms = parse_integer("fillTime", &value.fill_time)?;
        if ts_ms == 0 {
            return Err(RestError::InvalidField {
                field: "fillTime",
                value: value.fill_time,
                message: "must be positive".to_string(),
            });
        }
        Ok(Self {
            fill_id: value.fill_id,
            exchange_order_id: value.order_id,
            client_order_id,
            symbol: value.symbol,
            side: parse_side(&value.side)?,
            price,
            qty,
            liquidity: match value.execution_type.as_str() {
                "M" => FillLiquidity::Maker,
                "T" | "" => FillLiquidity::Taker,
                other => {
                    return Err(RestError::InvalidField {
                        field: "execType",
                        value: other.to_string(),
                        message: "expected M or T".to_string(),
                    });
                }
            },
            fee: parse_fill_fee(&value.fee, &value.fee_currency)?,
            ts_ms,
        })
    }
}

fn validate_required_text(field: &'static str, value: &str) -> Result<(), RestError> {
    if value.is_empty() || value.trim() != value {
        return Err(RestError::InvalidField {
            field,
            value: value.to_string(),
            message: "must be non-empty and contain no surrounding whitespace".to_string(),
        });
    }
    Ok(())
}

fn parse_side(value: &str) -> Result<Side, RestError> {
    match value {
        "buy" => Ok(Side::Buy),
        "sell" => Ok(Side::Sell),
        _ => Err(RestError::InvalidField {
            field: "side",
            value: value.to_string(),
            message: "expected buy or sell".to_string(),
        }),
    }
}

fn parse_instrument_type(value: &str) -> Result<OkxInstrumentType, RestError> {
    match value {
        "SPOT" => Ok(OkxInstrumentType::Spot),
        "MARGIN" => Ok(OkxInstrumentType::Margin),
        "SWAP" => Ok(OkxInstrumentType::Swap),
        "FUTURES" => Ok(OkxInstrumentType::Futures),
        "OPTION" => Ok(OkxInstrumentType::Option),
        _ => Err(RestError::InvalidField {
            field: "instType",
            value: value.to_string(),
            message: "unsupported instrument type".to_string(),
        }),
    }
}

fn parse_contract_type(value: &str) -> Result<Option<OkxContractType>, RestError> {
    match value {
        "" => Ok(None),
        "linear" => Ok(Some(OkxContractType::Linear)),
        "inverse" => Ok(Some(OkxContractType::Inverse)),
        _ => Err(RestError::InvalidField {
            field: "ctType",
            value: value.to_string(),
            message: "expected linear or inverse".to_string(),
        }),
    }
}

fn parse_account_level(value: &str) -> Result<OkxAccountLevel, RestError> {
    match value {
        "1" => Ok(OkxAccountLevel::Simple),
        "2" => Ok(OkxAccountLevel::SingleCurrencyMargin),
        "3" => Ok(OkxAccountLevel::MultiCurrencyMargin),
        "4" => Ok(OkxAccountLevel::PortfolioMargin),
        _ => Err(RestError::InvalidField {
            field: "acctLv",
            value: value.to_string(),
            message: "expected account level 1 through 4".to_string(),
        }),
    }
}

fn parse_position_mode(value: &str) -> Result<OkxPositionMode, RestError> {
    match value {
        "long_short_mode" => Ok(OkxPositionMode::LongShortMode),
        "net_mode" => Ok(OkxPositionMode::NetMode),
        _ => Err(RestError::InvalidField {
            field: "posMode",
            value: value.to_string(),
            message: "expected long_short_mode or net_mode".to_string(),
        }),
    }
}

fn parse_position_margin_mode(value: &str) -> Result<PositionMarginMode, RestError> {
    match value {
        "cross" => Ok(PositionMarginMode::Cross),
        "isolated" => Ok(PositionMarginMode::Isolated),
        other => Err(RestError::InvalidField {
            field: "mgnMode",
            value: other.to_string(),
            message: "expected cross or isolated".to_string(),
        }),
    }
}

fn parse_forced_repayment_indicator(value: &str) -> Result<Option<u8>, RestError> {
    if value.is_empty() {
        return Ok(None);
    }
    let indicator = value
        .parse::<u8>()
        .map_err(|error| RestError::InvalidField {
            field: "twap",
            value: value.to_string(),
            message: error.to_string(),
        })?;
    if indicator > 5 {
        return Err(RestError::InvalidField {
            field: "twap",
            value: value.to_string(),
            message: "expected 0 through 5".to_string(),
        });
    }
    Ok(Some(indicator))
}

fn parse_state(value: &str) -> Result<PrivateOrderState, RestError> {
    match value {
        "live" => Ok(PrivateOrderState::Live),
        "partially_filled" => Ok(PrivateOrderState::PartiallyFilled),
        "filled" => Ok(PrivateOrderState::Filled),
        "canceled" | "mmp_canceled" => Ok(PrivateOrderState::Cancelled),
        "order_failed" => Ok(PrivateOrderState::Rejected),
        _ => Err(RestError::InvalidField {
            field: "state",
            value: value.to_string(),
            message: "unsupported order state".to_string(),
        }),
    }
}

fn parse_number(field: &'static str, value: &str) -> Result<f64, RestError> {
    let parsed =
        value
            .parse()
            .map_err(|error: std::num::ParseFloatError| RestError::InvalidField {
                field,
                value: value.to_string(),
                message: error.to_string(),
            })?;
    if !f64::is_finite(parsed) {
        return Err(RestError::InvalidField {
            field,
            value: value.to_string(),
            message: "expected a finite number".to_string(),
        });
    }
    Ok(parsed)
}

fn parse_positive_number(field: &'static str, value: &str) -> Result<f64, RestError> {
    let parsed = parse_number(field, value)?;
    if parsed <= 0.0 {
        return Err(RestError::InvalidField {
            field,
            value: value.to_string(),
            message: "expected a positive number".to_string(),
        });
    }
    Ok(parsed)
}

fn parse_nullable_number(field: &'static str, value: &str) -> Result<Option<f64>, RestError> {
    if value.is_empty() {
        Ok(None)
    } else {
        parse_number(field, value).map(Some)
    }
}

fn parse_optional_number(field: &'static str, value: &str) -> Result<f64, RestError> {
    if value.is_empty() {
        Ok(0.0)
    } else {
        parse_number(field, value)
    }
}

fn parse_fill_fee(amount: &str, currency: &str) -> Result<Option<FillFee>, RestError> {
    let amount = amount.trim();
    let currency = currency.trim();
    if amount.is_empty() && currency.is_empty() {
        return Ok(None);
    }
    if amount.is_empty() || currency.is_empty() {
        return Err(RestError::InvalidField {
            field: "fee",
            value: format!("fee={amount:?}, feeCcy={currency:?}"),
            message: "fee and feeCcy must either both be present or both be absent".to_string(),
        });
    }
    Ok(Some(FillFee {
        amount: parse_number("fee", amount)?,
        currency: currency.to_ascii_uppercase(),
    }))
}

fn parse_integer(field: &'static str, value: &str) -> Result<u64, RestError> {
    value
        .parse()
        .map_err(|error: std::num::ParseIntError| RestError::InvalidField {
            field,
            value: value.to_string(),
            message: error.to_string(),
        })
}

fn parse_optional_integer(field: &'static str, value: &str) -> Result<u64, RestError> {
    if value.is_empty() {
        Ok(0)
    } else {
        parse_integer(field, value)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::okx::OkxCredentials;

    #[derive(Clone)]
    struct MockTransport {
        responses: Arc<Mutex<Vec<String>>>,
        requests: Arc<Mutex<Vec<SignedRequest>>>,
    }

    #[async_trait]
    impl HttpTransport for MockTransport {
        async fn execute(&self, request: SignedRequest) -> Result<HttpResponse, RestError> {
            self.requests.lock().unwrap().push(request);
            let body = self.responses.lock().unwrap().remove(0);
            Ok(HttpResponse { status: 200, body })
        }
    }

    fn client(
        responses: Vec<&str>,
    ) -> (OkxRestClient<MockTransport>, Arc<Mutex<Vec<SignedRequest>>>) {
        client_owned(responses.into_iter().map(str::to_string).collect())
    }

    fn client_owned(
        responses: Vec<String>,
    ) -> (OkxRestClient<MockTransport>, Arc<Mutex<Vec<SignedRequest>>>) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let transport = MockTransport {
            responses: Arc::new(Mutex::new(responses)),
            requests: Arc::clone(&requests),
        };
        let signer = OkxSigner::new(OkxCredentials::new("key", "secret", "pass"), false);
        (OkxRestClient::new(transport, signer), requests)
    }

    fn fill_response(first: usize, count: usize) -> String {
        let data = (first..first + count)
            .map(|index| {
                serde_json::json!({
                    "billId": format!("bill-{index}"),
                    "tradeId": format!("fill-{index}"),
                    "ordId": format!("order-{index}"),
                    "clOrdId": format!("client-{index}"),
                    "instId": "BTC-USDT",
                    "side": "buy",
                    "fillPx": "100",
                    "fillSz": "0.01",
                    "execType": "M",
                    "fee": "-0.00001",
                    "feeCcy": "BTC",
                    "fillTime": "1000"
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({"code": "0", "msg": "", "data": data}).to_string()
    }

    #[test]
    fn offline_fill_response_parser_preserves_exact_fee_fields() {
        let response = fill_response(7, 1);

        let fills = parse_okx_fills_response_json(response.as_bytes()).unwrap();

        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].fill_id, "fill-7");
        assert_eq!(fills[0].client_order_id, "client-7");
        assert_eq!(
            fills[0].fee,
            Some(FillFee {
                amount: -0.00001,
                currency: "BTC".to_string(),
            })
        );
    }

    #[test]
    fn offline_fill_response_parser_rejects_api_errors() {
        let error = parse_okx_fills_response_json(
            br#"{"code":"50011","msg":"rate limit reached","data":[]}"#,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RestError::Api { ref code, .. } if code == "50011"
        ));
    }

    #[test]
    fn offline_fill_response_parser_rejects_missing_trade_identity() {
        let error = parse_okx_fills_response_json(
            br#"{"code":"0","msg":"","data":[{"tradeId":"","ordId":"order-1","instId":"BTC-USDT","side":"buy","fillPx":"100","fillSz":"0.01","execType":"M","fee":"-0.001","feeCcy":"BTC","fillTime":"1000"}]}"#,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RestError::InvalidField {
                field: "tradeId",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn signed_place_and_cancel_requests_use_client_id() {
        let (client, requests) = client(vec![
            r#"{"code":"0","msg":"","data":[{"ordId":"123","clOrdId":"reap1","sCode":"0","sMsg":""}]}"#,
            r#"{"code":"0","msg":"","data":[{"ordId":"123","clOrdId":"reap1","sCode":"0","sMsg":""}]}"#,
        ]);
        let ack = client
            .place_order_at(
                "2020-12-08T09:08:57.715Z",
                &OkxPlaceOrder {
                    symbol: "BTC-USDT".to_string(),
                    trade_mode: OkxTradeMode::Cash,
                    side: Side::Buy,
                    time_in_force: TimeInForce::PostOnly,
                    price: 100.5,
                    qty: 0.1,
                    client_order_id: "reap1".to_string(),
                    reduce_only: false,
                    self_trade_prevention: Some(SelfTradePrevention::CancelMaker),
                },
            )
            .await
            .unwrap();
        assert_eq!(ack.exchange_order_id, "123");

        client
            .cancel_order_at(
                "2020-12-08T09:08:58.000Z",
                &OkxCancelOrder {
                    symbol: "BTC-USDT".to_string(),
                    exchange_order_id: None,
                    client_order_id: Some("reap1".to_string()),
                },
            )
            .await
            .unwrap();

        let requests = requests.lock().unwrap();
        assert_eq!(requests[0].path, PLACE_ORDER_PATH);
        assert!(requests[0].body.contains(r#""clOrdId":"reap1""#));
        assert!(requests[0].body.contains(r#""stpMode":"cancel_maker""#));
        assert_eq!(requests[1].path, CANCEL_ORDER_PATH);
        assert!(requests[1].body.contains(r#""clOrdId":"reap1""#));
        assert!(
            requests
                .iter()
                .all(|request| request.headers.contains_key("OK-ACCESS-SIGN"))
        );
    }

    #[tokio::test]
    async fn batch_cancel_preserves_per_order_acceptance_results() {
        let (client, requests) = client(vec![
            r#"{"code":"0","msg":"","data":[{"ordId":"123","clOrdId":"reap1","sCode":"0","sMsg":""},{"ordId":"456","clOrdId":"reap2","sCode":"51400","sMsg":"order already canceled"}]}"#,
        ]);
        let results = client
            .cancel_batch_orders_at(
                "2020-12-08T09:08:58.000Z",
                &[
                    OkxCancelOrder {
                        symbol: "BTC-USDT".to_string(),
                        exchange_order_id: Some("123".to_string()),
                        client_order_id: None,
                    },
                    OkxCancelOrder {
                        symbol: "ETH-USDT".to_string(),
                        exchange_order_id: Some("456".to_string()),
                        client_order_id: None,
                    },
                ],
            )
            .await
            .unwrap();

        assert!(results[0].accepted());
        assert!(!results[1].accepted());
        assert_eq!(results[1].code, "51400");
        let requests = requests.lock().unwrap();
        assert_eq!(requests[0].path, CANCEL_BATCH_ORDERS_PATH);
        assert!(requests[0].body.contains(r#""instId":"BTC-USDT""#));
        assert!(requests[0].body.contains(r#""ordId":"456""#));
    }

    #[test]
    fn batch_cancel_and_exchange_timestamp_are_bounded() {
        let (client, _) = client(Vec::new());
        assert!(matches!(
            client.build_cancel_batch_request("time", &[]),
            Err(RestError::InvalidField {
                field: "orders",
                ..
            })
        ));
        assert_eq!(
            format_okx_timestamp_ms(1_607_418_537_715).unwrap(),
            "2020-12-08T09:08:57.715Z"
        );
    }

    #[tokio::test]
    async fn place_order_sets_exchange_expiry_header() {
        let (client, requests) = client(vec![
            r#"{"code":"0","msg":"","data":[{"ordId":"123","clOrdId":"reap1","sCode":"0","sMsg":""}]}"#,
        ]);
        let client = client.with_order_request_expiry(std::time::Duration::from_millis(750));
        let timestamp = "2020-12-08T09:08:57.715Z";
        client
            .place_order_at(
                timestamp,
                &OkxPlaceOrder {
                    symbol: "BTC-USDT".to_string(),
                    trade_mode: OkxTradeMode::Cash,
                    side: Side::Buy,
                    time_in_force: TimeInForce::PostOnly,
                    price: 100.5,
                    qty: 0.1,
                    client_order_id: "reap1".to_string(),
                    reduce_only: false,
                    self_trade_prevention: None,
                },
            )
            .await
            .unwrap();

        let requests = requests.lock().unwrap();
        assert_eq!(
            requests[0].headers["expTime"],
            (timestamp_ms(timestamp).unwrap() + 750).to_string()
        );
    }

    #[tokio::test]
    async fn public_time_and_cancel_all_after_have_expected_wire_contract() {
        let (client, requests) = client(vec![
            r#"{"code":"0","msg":"","data":[{"ts":"1597026383085"}]}"#,
            r#"{"code":"0","msg":"","data":[{"triggerTime":"1597026443","tag":"","ts":"1597026383"}]}"#,
            r#"{"code":"0","msg":"","data":[{"triggerTime":"0","tag":"","ts":"1597026384"}]}"#,
        ]);

        assert_eq!(client.server_time_ms().await.unwrap(), 1_597_026_383_085);
        client
            .cancel_all_after_at("2020-12-08T09:08:57.715Z", 30)
            .await
            .unwrap();
        client
            .cancel_all_after_at("2020-12-08T09:08:58.715Z", 0)
            .await
            .unwrap();

        let requests = requests.lock().unwrap();
        assert_eq!(requests[0].path, PUBLIC_TIME_PATH);
        assert!(requests[0].headers.is_empty());
        assert_eq!(requests[1].path, CANCEL_ALL_AFTER_PATH);
        assert_eq!(requests[1].body, r#"{"timeOut":"30"}"#);
        assert!(requests[1].headers.contains_key("OK-ACCESS-SIGN"));
        assert_eq!(requests[2].body, r#"{"timeOut":"0"}"#);
    }

    #[tokio::test]
    async fn cancel_all_after_rejects_unsafe_timeout() {
        let (invalid_timeout_client, _) = client(Vec::new());
        assert!(matches!(
            invalid_timeout_client
                .cancel_all_after_at("2020-12-08T09:08:57.715Z", 9)
                .await,
            Err(RestError::InvalidField {
                field: "timeOut",
                ..
            })
        ));

        let (client, _) = client(vec![
            r#"{"code":"0","msg":"","data":[{"triggerTime":"0"}]}"#,
        ]);
        assert!(matches!(
            client
                .cancel_all_after_at("2020-12-08T09:08:57.715Z", 10)
                .await,
            Err(RestError::InvalidField {
                field: "triggerTime",
                ..
            })
        ));
    }

    #[tokio::test]
    async fn parses_open_orders_and_fills_for_reconciliation() {
        let (client, requests) = client(vec![
            r#"{"code":"0","msg":"","data":[{"ordId":"123","clOrdId":"reap1","instId":"BTC-USDT","side":"buy","state":"partially_filled","px":"100","sz":"1","accFillSz":"0.4","avgPx":"99.5","uTime":"1000"}]}"#,
            r#"{"code":"0","msg":"","data":[{"tradeId":"fill1","ordId":"123","clOrdId":"reap1","instId":"BTC-USDT","side":"buy","fillPx":"99.5","fillSz":"0.4","execType":"M","fee":"-0.0004","feeCcy":"btc","fillTime":"1000"}]}"#,
        ]);
        let orders = client
            .open_orders_at("time", Some("SPOT"), Some("BTC-USDT"))
            .await
            .unwrap();
        let fills = client
            .fills_at("time", Some("SPOT"), Some("BTC-USDT"))
            .await
            .unwrap();

        assert_eq!(orders[0].state, PrivateOrderState::PartiallyFilled);
        assert_eq!(fills[0].liquidity, FillLiquidity::Maker);
        assert_eq!(
            fills[0].fee,
            Some(FillFee {
                amount: -0.0004,
                currency: "BTC".to_string(),
            })
        );
        let requests = requests.lock().unwrap();
        assert_eq!(
            requests[0].path,
            "/api/v5/trade/orders-pending?instType=SPOT&instId=BTC-USDT"
        );
        assert_eq!(
            requests[1].path,
            "/api/v5/trade/fills?instType=SPOT&instId=BTC-USDT&limit=100"
        );
    }

    #[tokio::test]
    async fn fill_reconciliation_paginates_until_a_short_page() {
        let (client, requests) = client_owned(vec![fill_response(100, 100), fill_response(200, 2)]);

        let fills = client
            .fills_paginated(Some("SPOT"), Some("BTC-USDT"), 3)
            .await
            .unwrap();

        assert_eq!(fills.len(), 102);
        assert_eq!(fills.first().unwrap().fill_id, "fill-100");
        assert_eq!(fills.last().unwrap().fill_id, "fill-201");
        let requests = requests.lock().unwrap();
        assert_eq!(
            requests[0].path,
            "/api/v5/trade/fills?instType=SPOT&instId=BTC-USDT&limit=100"
        );
        assert_eq!(
            requests[1].path,
            "/api/v5/trade/fills?instType=SPOT&instId=BTC-USDT&after=bill-199&limit=100"
        );
    }

    #[tokio::test]
    async fn fill_reconciliation_fails_closed_at_page_bound() {
        let (client, requests) = client_owned(vec![fill_response(100, 100)]);

        let error = client.fills_paginated(None, None, 1).await.unwrap_err();

        assert!(matches!(
            error,
            RestError::FillPaginationLimit {
                pages: 1,
                records: 100,
                ref next_after,
            } if next_after == "bill-199"
        ));
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn queries_terminal_order_details_by_client_id() {
        let (client, requests) = client(vec![
            r#"{"code":"0","msg":"","data":[{"ordId":"123","clOrdId":"reap1","instId":"BTC-USDT","side":"buy","state":"canceled","px":"100","sz":"1","accFillSz":"0","avgPx":"","uTime":"1000"}]}"#,
        ]);
        let order = client
            .order_details_at("time", "BTC-USDT", None, Some("reap1"))
            .await
            .unwrap();

        assert_eq!(order.state, PrivateOrderState::Cancelled);
        assert_eq!(
            requests.lock().unwrap()[0].path,
            "/api/v5/trade/order?instId=BTC-USDT&clOrdId=reap1"
        );
    }

    #[tokio::test]
    async fn parses_signed_bootstrap_metadata_and_account_state() {
        let (client, requests) = client(vec![
            r#"{"code":"0","msg":"","data":[{"instId":"BTC-USDT-SWAP","instType":"SWAP","baseCcy":"BTC","quoteCcy":"USDT","settleCcy":"USDT","ctType":"linear","ctVal":"0.01","ctValCcy":"BTC","tickSz":"0.1","lotSz":"1","minSz":"1","state":"live"}]}"#,
            r#"{"code":"0","msg":"","data":[{"acctLv":"2","posMode":"net_mode","acctStpMode":"cancel_maker","uid":"7","mainUid":"6","enableSpotBorrow":false,"autoLoan":false,"spotBorrowAutoRepay":false}]}"#,
            r#"{"code":"0","msg":"","data":[{"uTime":"1000","totalEq":"11000","mgnRatio":"12.5","adjEq":"10000","borrowFroz":"0","notionalUsdForBorrow":"0","notionalUsd":"2000","details":[{"ccy":"USDT","uTime":"999","cashBal":"9000","availBal":"8000","eq":"10000","liab":"0","crossLiab":"0","isoLiab":"0","uplLiab":"0","interest":"0","borrowFroz":"0","maxLoan":"500","twap":"2"}]}]}"#,
            r#"{"code":"0","msg":"","data":[{"instType":"SWAP","instId":"BTC-USDT-SWAP","pos":"2","posSide":"net","mgnMode":"cross","avgPx":"50000","uTime":"1001","liab":"","interest":""}]}"#,
        ]);

        let instruments = client
            .account_instruments_at("time", OkxInstrumentType::Swap, Some("BTC-USDT-SWAP"))
            .await
            .unwrap();
        let account = client.account_config_at("time").await.unwrap();
        let balance = client.account_balance_at("time").await.unwrap();
        let positions = client
            .account_positions_at("time", Some(OkxInstrumentType::Swap), Some("BTC-USDT-SWAP"))
            .await
            .unwrap();

        assert_eq!(instruments[0].contract_type, Some(OkxContractType::Linear));
        assert_eq!(instruments[0].contract_value, Some(0.01));
        assert_eq!(account.account_level, OkxAccountLevel::SingleCurrencyMargin);
        assert_eq!(account.position_mode, OkxPositionMode::NetMode);
        assert_eq!(account.enable_spot_borrow, Some(false));
        assert_eq!(balance.balances[0].available, 8000.0);
        assert_eq!(balance.balances[0].forced_repayment_indicator, Some(2));
        assert_eq!(balance.margins[0].exchange_ratio, Some(12.5));
        assert_eq!(positions.positions[0].qty, 2.0);
        assert_eq!(
            positions.positions[0].margin_mode,
            Some(PositionMarginMode::Cross)
        );

        let requests = requests.lock().unwrap();
        assert_eq!(
            requests[0].path,
            "/api/v5/account/instruments?instType=SWAP&instId=BTC-USDT-SWAP"
        );
        assert_eq!(requests[1].path, ACCOUNT_CONFIG_PATH);
        assert_eq!(requests[2].path, ACCOUNT_BALANCE_PATH);
        assert_eq!(
            requests[3].path,
            "/api/v5/account/positions?instType=SWAP&instId=BTC-USDT-SWAP"
        );
        assert!(
            requests
                .iter()
                .all(|request| request.headers.contains_key("OK-ACCESS-SIGN"))
        );
    }

    #[test]
    fn rejects_non_finite_exchange_numbers() {
        assert!(matches!(
            parse_number("px", "NaN"),
            Err(RestError::InvalidField { .. })
        ));
        assert!(matches!(
            parse_forced_repayment_indicator("6"),
            Err(RestError::InvalidField { field: "twap", .. })
        ));
    }

    #[test]
    fn offline_account_parsers_preserve_borrowing_evidence() {
        let account = parse_okx_account_config_response_json(
            br#"{"code":"0","msg":"","data":[{"acctLv":"1","posMode":"net_mode","uid":"7","mainUid":"6","enableSpotBorrow":false,"autoLoan":false,"spotBorrowAutoRepay":true}]}"#,
        )
        .unwrap();
        assert_eq!(account.enable_spot_borrow, Some(false));
        assert_eq!(account.auto_loan, Some(false));
        assert_eq!(account.spot_borrow_auto_repay, Some(true));

        let balance = parse_okx_account_balance_response_json(
            br#"{"code":"0","msg":"","data":[{"uTime":"1000","totalEq":"100","adjEq":"99","borrowFroz":"2","notionalUsdForBorrow":"3","details":[{"ccy":"USDT","uTime":"999","cashBal":"100","availBal":"90","eq":"99","liab":"1","crossLiab":"0.5","isoLiab":"0.25","uplLiab":"0.1","interest":"0.01","borrowFroz":"2","maxLoan":"50","twap":"1"}]}]}"#,
        )
        .unwrap();
        assert_eq!(balance.borrow_frozen_usd, Some(2.0));
        assert_eq!(balance.notional_usd_for_borrow, Some(3.0));
        assert_eq!(balance.details[0].liability, Some(1.0));
        assert_eq!(balance.details[0].accrued_interest, Some(0.01));

        let positions = parse_okx_account_positions_response_json(
            br#"{"code":"0","msg":"","data":[{"instType":"MARGIN","instId":"BTC-USDT","pos":"1","posSide":"net","mgnMode":"cross","uTime":"1001","liab":"20","interest":"0.02","pendingCloseOrdLiabVal":"1","baseBorrowed":"0","baseInterest":"0","quoteBorrowed":"20","quoteInterest":"0.02"}]}"#,
        )
        .unwrap();
        assert_eq!(
            positions.positions[0].instrument_type,
            OkxInstrumentType::Margin
        );
        assert_eq!(positions.positions[0].liability, Some(20.0));
        assert_eq!(positions.positions[0].quote_interest, Some(0.02));
    }

    #[tokio::test]
    async fn rejects_unsupported_position_margin_mode() {
        let (client, _) = client(vec![
            r#"{"code":"0","msg":"","data":[{"instType":"SWAP","instId":"BTC-USDT-SWAP","pos":"2","posSide":"net","mgnMode":"portfolio","uTime":"1001"}]}"#,
        ]);

        let error = client
            .account_positions_at("time", Some(OkxInstrumentType::Swap), None)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            RestError::InvalidField {
                field: "mgnMode",
                ..
            }
        ));
    }
}
