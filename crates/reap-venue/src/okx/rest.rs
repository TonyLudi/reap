use std::collections::{BTreeSet, HashSet};

use reap_core::{
    AccountUpdate, Balance, FillFee, FillKey, FillLiquidity, MarginSnapshot, Position,
    PositionMarginMode, SelfTradePrevention, Side, TimeInForce,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{PrivateOrderState, RemoteFill, RemoteOrder};

mod pending_orders;
pub use pending_orders::{
    OKX_ALGO_CANCEL_BATCH_LIMIT, OKX_DEFAULT_MAX_PENDING_ORDER_PAGES, OKX_PENDING_ORDER_PAGE_LIMIT,
    OkxAlgoCancelResult, OkxAlgoOrder, OkxAlgoOrderPage, OkxAlgoOrderPagination, OkxAlgoOrderQuery,
    OkxAlgoOrderType, OkxCancelAlgoOrder, OkxRegularOrderPage, OkxRegularOrderPagination,
    OkxSpreadOrder, OkxSpreadOrderPage, OkxSpreadOrderPagination,
    parse_okx_algo_order_page_response_json, parse_okx_regular_order_page_response_json,
    parse_okx_spread_order_page_response_json,
};

pub const OKX_FILLS_PAGE_LIMIT: usize = 100;
pub const OKX_BILLS_PAGE_LIMIT: usize = 100;
pub const OKX_MIN_ACCOUNT_INSTRUMENT_REQUEST_INTERVAL_MS: u64 = 100;
pub const OKX_MIN_TRADE_FEE_REQUEST_INTERVAL_MS: u64 = 400;

/// Parses an unmodified OKX trade-fills or fills-history response.
///
/// Both endpoints use the same response row shape. Keeping one credential-free
/// parser prevents evidence and live reconciliation semantics from drifting.
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

/// Parses one unmodified account-bills page and derives its next `after`
/// cursor. The account-wide endpoint is the economic statement source used by
/// the pinned Java `BillDetails`/`OkexV5BillFetchTask` path.
pub fn parse_okx_bill_page_response_json(body: &[u8]) -> Result<OkxBillPage, RestError> {
    let response: OkxResponse<OkxBillWire> = serde_json::from_slice(body)?;
    if response.code != "0" {
        return Err(RestError::Api {
            code: response.code,
            message: response.message,
        });
    }
    okx_bill_page(response.data)
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

/// Parses one unmodified public OKX index-ticker response.
///
/// The pinned Java `OkexV5RestClient.getIndex` uses the same endpoint and
/// retains `instId`, `idxPx`, and `ts` for account and strategy valuation.
pub fn parse_okx_index_ticker_response_json(
    body: &[u8],
) -> Result<OkxIndexTickerSnapshot, RestError> {
    let mut response: OkxResponse<OkxIndexTickerWire> = decode_okx_response(body)?;
    if response.data.len() != 1 {
        return Err(RestError::InvalidField {
            field: "data",
            value: response.data.len().to_string(),
            message: "index ticker response must contain exactly one row".to_string(),
        });
    }
    response
        .data
        .pop()
        .expect("checked one index ticker row")
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

/// Parses an unmodified OKX pending-orders response.
pub fn parse_okx_open_orders_response_json(body: &[u8]) -> Result<Vec<RemoteOrder>, RestError> {
    let response: OkxResponse<OkxOrderWire> = decode_okx_response(body)?;
    response
        .data
        .into_iter()
        .map(RemoteOrder::try_from)
        .collect()
}

/// Parses an unmodified OKX system-status response.
pub fn parse_okx_system_status_response_json(
    body: &[u8],
) -> Result<Vec<OkxSystemStatus>, RestError> {
    let response: OkxResponse<OkxSystemStatusWire> = decode_okx_response(body)?;
    response
        .data
        .into_iter()
        .map(OkxSystemStatus::try_from)
        .collect()
}

/// Parses the current OKX fee-group response without using deprecated
/// top-level maker/taker fields.
pub fn parse_okx_trade_fee_response_json(body: &[u8]) -> Result<Vec<OkxTradeFeeRate>, RestError> {
    let mut response: OkxResponse<OkxTradeFeeScheduleWire> = decode_okx_response(body)?;
    if response.data.len() != 1 {
        return Err(RestError::InvalidField {
            field: "data",
            value: response.data.len().to_string(),
            message: "trade fee response must contain exactly one schedule".to_string(),
        });
    }
    parse_trade_fee_schedule(response.data.pop().expect("checked one trade-fee schedule"))
}

/// Parses one unmodified OKX order-details response while retaining the
/// exchange cancellation attribution needed by deadman certification.
pub fn parse_okx_order_details_response_json(body: &[u8]) -> Result<OkxOrderDetails, RestError> {
    let mut response: OkxResponse<OkxOrderWire> = decode_okx_response(body)?;
    if response.data.is_empty() {
        return Err(RestError::EmptyData {
            operation: "order details",
        });
    }
    if response.data.len() != 1 {
        return Err(RestError::InvalidField {
            field: "data",
            value: response.data.len().to_string(),
            message: "order details must contain exactly one row".to_string(),
        });
    }
    response
        .data
        .pop()
        .expect("checked one order-detail row")
        .try_into()
}

/// Parses the credential-free response returned by the public time endpoint.
pub fn parse_okx_server_time_response_json(body: &[u8]) -> Result<u64, RestError> {
    let response: OkxResponse<OkxTimeWire> = decode_okx_response(body)?;
    let wire = response
        .data
        .into_iter()
        .next()
        .ok_or(RestError::EmptyData {
            operation: "server time",
        })?;
    parse_integer("ts", &wire.timestamp)
}

/// Parses account instrument metadata without granting authenticated transport.
pub fn parse_okx_account_instruments_response_json(
    body: &[u8],
) -> Result<Vec<OkxInstrument>, RestError> {
    let response: OkxResponse<OkxInstrumentWire> = decode_okx_response(body)?;
    response
        .data
        .into_iter()
        .map(OkxInstrument::try_from)
        .collect()
}

/// Parses a regular place/cancel acknowledgement.
pub fn parse_okx_order_ack_response_json(
    body: &[u8],
    operation: &'static str,
) -> Result<OkxOrderAck, RestError> {
    let response: OkxResponse<OkxAckWire> = decode_okx_response(body)?;
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

/// Parses all rows returned by regular batch cancellation.
pub fn parse_okx_cancel_order_results_response_json(
    body: &[u8],
) -> Result<Vec<OkxCancelOrderResult>, RestError> {
    let response: OkxResponse<OkxAckWire> = decode_okx_response(body)?;
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

/// Validates the acknowledgement returned by regular Cancel All After.
pub fn parse_okx_cancel_all_after_response_json(
    body: &[u8],
    timeout_secs: u64,
) -> Result<(), RestError> {
    let response: OkxResponse<OkxCancelAllAfterWire> = decode_okx_response(body)?;
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

fn okx_bill_page(data: Vec<OkxBillWire>) -> Result<OkxBillPage, RestError> {
    if data.len() > OKX_BILLS_PAGE_LIMIT {
        return Err(RestError::InvalidField {
            field: "data",
            value: data.len().to_string(),
            message: format!("bill page exceeded the requested limit {OKX_BILLS_PAGE_LIMIT}"),
        });
    }
    let next_after = if data.len() == OKX_BILLS_PAGE_LIMIT {
        let bill_id = data
            .last()
            .map(|bill| bill.bill_id.trim())
            .filter(|bill_id| !bill_id.is_empty())
            .ok_or_else(|| RestError::InvalidField {
                field: "billId",
                value: String::new(),
                message: "a full bill page requires a pagination cursor".to_string(),
            })?;
        Some(bill_id.to_string())
    } else {
        None
    };
    let bills = data
        .into_iter()
        .map(OkxBill::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(OkxBillPage { bills, next_after })
}

#[derive(Debug, Error)]
pub enum RestError {
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
    #[error(
        "OKX bill pagination reached the configured limit after {pages} pages and {records} records; next after={next_after}"
    )]
    BillPaginationLimit {
        pages: usize,
        records: usize,
        next_after: String,
    },
    #[error("OKX bill pagination repeated cursor {cursor}")]
    BillPaginationCursor { cursor: String },
    #[error("OKX bill pagination repeated bill {bill_id}")]
    BillPaginationDuplicate { bill_id: String },
    #[error(
        "OKX {domain} pending-order pagination reached the configured limit after {pages} pages and {records} records; next cursor={next_cursor}"
    )]
    PendingOrderPaginationLimit {
        domain: &'static str,
        pages: usize,
        records: usize,
        next_cursor: String,
    },
    #[error("OKX {domain} pending-order pagination repeated cursor {cursor}")]
    PendingOrderPaginationCursor {
        domain: &'static str,
        cursor: String,
    },
    #[error("OKX {domain} pending-order pagination repeated order {order_id}")]
    PendingOrderPaginationDuplicate {
        domain: &'static str,
        order_id: String,
    },
}

impl RestError {
    pub fn is_order_not_found(&self) -> bool {
        matches!(self, Self::Api { code, .. } if code == "51603")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OkxSystemStatusState {
    Scheduled,
    Ongoing,
    PreOpen,
    Completed,
    Canceled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OkxSystemServiceType {
    WebSocket,
    Trading,
    BlockTrading,
    TradingBot,
    TradingAccounts,
    TradingProducts,
    SpreadTrading,
    CopyTrading,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OkxSystemMaintenanceType {
    Scheduled,
    Unscheduled,
    Disruption,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OkxSystemEnvironment {
    Production,
    Demo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OkxSystemStatus {
    pub title: String,
    pub description: String,
    pub state: OkxSystemStatusState,
    pub begin_time_ms: u64,
    pub end_time_ms: u64,
    pub pre_open_begin_time_ms: Option<u64>,
    pub service_type: OkxSystemServiceType,
    pub maintenance_type: OkxSystemMaintenanceType,
    pub environment: OkxSystemEnvironment,
    pub system: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OkxTradeFeeRate {
    pub instrument_type: OkxInstrumentType,
    pub group_id: String,
    pub level: String,
    /// Signed exchange rate: negative is a commission and positive is a rebate.
    pub maker_rate: f64,
    /// Signed exchange rate: negative is a commission and positive is a rebate.
    pub taker_rate: f64,
    pub timestamp_ms: u64,
}

impl OkxTradeFeeRate {
    pub fn maker_cost_rate(&self) -> f64 {
        -self.maker_rate
    }

    pub fn taker_cost_rate(&self) -> f64 {
        -self.taker_rate
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
    pub trade_fee_group_id: String,
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
    pub max_limit_size: f64,
    pub max_market_size: f64,
    pub max_limit_amount_usd: Option<f64>,
    pub max_market_amount_usd: Option<f64>,
    pub state: String,
    pub upcoming_changes: Vec<OkxInstrumentChange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OkxInstrumentChangeParameter {
    TickSize,
    MinimumSize,
    MaximumMarketSize,
}

impl OkxInstrumentChangeParameter {
    pub fn as_okx_str(self) -> &'static str {
        match self {
            Self::TickSize => "tickSz",
            Self::MinimumSize => "minSz",
            Self::MaximumMarketSize => "maxMktSz",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OkxInstrumentChange {
    pub parameter: OkxInstrumentChangeParameter,
    pub new_value: f64,
    pub effective_time_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OkxAccountConfig {
    pub account_level: OkxAccountLevel,
    pub position_mode: OkxPositionMode,
    pub account_stp_mode: String,
    pub user_id: String,
    pub main_user_id: String,
    /// Note assigned to the API key that authenticated this request.
    pub api_key_label: String,
    /// Permissions attached to the API key that authenticated this request.
    pub api_key_permissions: BTreeSet<OkxApiKeyPermission>,
    /// IP addresses or network segments bound to the authenticating API key.
    pub api_key_ip_bindings: BTreeSet<String>,
    /// OKX Spot-mode borrowing switch. `None` means the exchange omitted it.
    pub enable_spot_borrow: Option<bool>,
    /// OKX multi-currency/portfolio automatic borrowing switch.
    pub auto_loan: Option<bool>,
    /// OKX Spot-mode automatic repayment switch.
    pub spot_borrow_auto_repay: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OkxApiKeyPermission {
    ReadOnly,
    Trade,
    Withdraw,
}

impl OkxApiKeyPermission {
    pub fn as_okx_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Trade => "trade",
            Self::Withdraw => "withdraw",
        }
    }
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
    /// Exchange-converted currency equity in USD (`eqUsd`).
    pub equity_usd: Option<f64>,
    /// Haircut-adjusted currency equity in USD (`disEq`).
    pub discounted_equity_usd: Option<f64>,
    /// Currency-level unrealized PnL (`upl`).
    pub unrealized_pnl: Option<f64>,
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
pub struct OkxIndexTickerSnapshot {
    pub symbol: String,
    pub index_price: f64,
    pub timestamp_ms: u64,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OkxOrderDetails {
    pub order: RemoteOrder,
    pub cancel_source: String,
    pub cancel_source_reason: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OkxBillExecutionType {
    Maker,
    Taker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OkxBillMarginMode {
    Cash,
    Cross,
    Isolated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OkxBillAccountType {
    Funding,
    Trading,
}

/// One balance-changing OKX account bill.
///
/// This retains the pinned Java `BillDetails` fields and the current trade
/// identity fields required to bind trade bills back to exact fills.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OkxBill {
    pub bill_id: String,
    pub bill_type: String,
    pub sub_type: String,
    pub timestamp_ms: u64,
    pub currency: String,
    pub balance_change: f64,
    pub balance: Option<f64>,
    pub position_balance_change: Option<f64>,
    pub position_balance: Option<f64>,
    pub quantity: Option<f64>,
    pub price: Option<f64>,
    pub pnl: Option<f64>,
    pub fee: Option<f64>,
    pub interest: Option<f64>,
    pub instrument_type: Option<OkxInstrumentType>,
    pub symbol: String,
    pub margin_mode: Option<OkxBillMarginMode>,
    pub order_id: String,
    pub client_order_id: String,
    pub trade_id: String,
    pub fill_time_ms: Option<u64>,
    pub execution_type: Option<OkxBillExecutionType>,
    pub from_account: Option<OkxBillAccountType>,
    pub to_account: Option<OkxBillAccountType>,
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OkxBillPage {
    pub bills: Vec<OkxBill>,
    pub next_after: Option<String>,
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

#[derive(Debug)]
pub struct OkxBillPagination {
    max_pages: usize,
    pages: usize,
    bills: Vec<OkxBill>,
    after: Option<String>,
    seen_cursors: HashSet<String>,
    seen_bills: HashSet<String>,
}

impl OkxBillPagination {
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
            bills: Vec::new(),
            after: None,
            seen_cursors: HashSet::new(),
            seen_bills: HashSet::new(),
        })
    }

    pub fn after(&self) -> Option<&str> {
        self.after.as_deref()
    }

    pub fn accept(&mut self, page: OkxBillPage) -> Result<bool, RestError> {
        self.pages += 1;
        for bill in page.bills {
            if !self.seen_bills.insert(bill.bill_id.clone()) {
                return Err(RestError::BillPaginationDuplicate {
                    bill_id: bill.bill_id,
                });
            }
            self.bills.push(bill);
        }
        let Some(next_after) = page.next_after else {
            return Ok(true);
        };
        if !self.seen_cursors.insert(next_after.clone()) {
            return Err(RestError::BillPaginationCursor { cursor: next_after });
        }
        if self.pages == self.max_pages {
            return Err(RestError::BillPaginationLimit {
                pages: self.pages,
                records: self.bills.len(),
                next_after,
            });
        }
        self.after = Some(next_after);
        Ok(false)
    }

    pub fn into_bills(self) -> Vec<OkxBill> {
        self.bills
    }
}

impl OkxCancelOrderResult {
    pub fn accepted(&self) -> bool {
        self.code.is_empty() || self.code == "0"
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
struct OkxIndexTickerWire {
    #[serde(rename = "instId")]
    symbol: String,
    #[serde(rename = "idxPx")]
    index_price: String,
    #[serde(rename = "ts")]
    timestamp: String,
}

impl TryFrom<OkxIndexTickerWire> for OkxIndexTickerSnapshot {
    type Error = RestError;

    fn try_from(value: OkxIndexTickerWire) -> Result<Self, Self::Error> {
        if value.symbol.trim().is_empty() || value.symbol.trim() != value.symbol {
            return Err(RestError::InvalidField {
                field: "instId",
                value: value.symbol,
                message: "must be non-empty and contain no surrounding whitespace".to_string(),
            });
        }
        let index_price = parse_number("idxPx", &value.index_price)?;
        if index_price <= 0.0 {
            return Err(RestError::InvalidField {
                field: "idxPx",
                value: value.index_price,
                message: "must be positive".to_string(),
            });
        }
        let timestamp_ms = parse_integer("ts", &value.timestamp)?;
        if timestamp_ms == 0 {
            return Err(RestError::InvalidField {
                field: "ts",
                value: value.timestamp,
                message: "must be positive".to_string(),
            });
        }
        Ok(Self {
            symbol: value.symbol,
            index_price,
            timestamp_ms,
        })
    }
}

#[derive(Debug, Deserialize)]
struct OkxSystemStatusWire {
    #[serde(default)]
    title: String,
    state: String,
    begin: String,
    end: String,
    #[serde(default, rename = "preOpenBegin")]
    pre_open_begin: String,
    #[serde(rename = "serviceType")]
    service_type: String,
    #[serde(rename = "maintType")]
    maintenance_type: String,
    env: String,
    system: String,
    #[serde(default, rename = "scheDesc")]
    description: String,
}

#[derive(Debug, Deserialize)]
struct OkxTradeFeeScheduleWire {
    #[serde(rename = "instType")]
    instrument_type: String,
    level: String,
    #[serde(rename = "ts")]
    timestamp: String,
    #[serde(rename = "feeGroup")]
    fee_groups: Vec<OkxTradeFeeGroupWire>,
}

#[derive(Debug, Deserialize)]
struct OkxTradeFeeGroupWire {
    #[serde(rename = "groupId")]
    group_id: String,
    maker: String,
    taker: String,
}

#[derive(Debug, Deserialize)]
struct OkxCancelAllAfterWire {
    #[serde(default, rename = "triggerTime")]
    trigger_time: String,
}

impl TryFrom<OkxSystemStatusWire> for OkxSystemStatus {
    type Error = RestError;

    fn try_from(value: OkxSystemStatusWire) -> Result<Self, Self::Error> {
        let state = match value.state.trim().to_ascii_lowercase().as_str() {
            "scheduled" => OkxSystemStatusState::Scheduled,
            "ongoing" => OkxSystemStatusState::Ongoing,
            "pre_open" => OkxSystemStatusState::PreOpen,
            "completed" => OkxSystemStatusState::Completed,
            "canceled" => OkxSystemStatusState::Canceled,
            _ => {
                return Err(RestError::InvalidField {
                    field: "state",
                    value: value.state,
                    message: "unknown system maintenance state".to_string(),
                });
            }
        };
        let service_type = match value.service_type.trim() {
            "0" => OkxSystemServiceType::WebSocket,
            "5" => OkxSystemServiceType::Trading,
            "6" => OkxSystemServiceType::BlockTrading,
            "7" => OkxSystemServiceType::TradingBot,
            "8" => OkxSystemServiceType::TradingAccounts,
            "9" => OkxSystemServiceType::TradingProducts,
            "10" => OkxSystemServiceType::SpreadTrading,
            "11" => OkxSystemServiceType::CopyTrading,
            "99" => OkxSystemServiceType::Other,
            _ => {
                return Err(RestError::InvalidField {
                    field: "serviceType",
                    value: value.service_type,
                    message: "unknown system maintenance service type".to_string(),
                });
            }
        };
        let maintenance_type = match value.maintenance_type.trim() {
            "1" => OkxSystemMaintenanceType::Scheduled,
            "2" => OkxSystemMaintenanceType::Unscheduled,
            "3" => OkxSystemMaintenanceType::Disruption,
            _ => {
                return Err(RestError::InvalidField {
                    field: "maintType",
                    value: value.maintenance_type,
                    message: "unknown system maintenance type".to_string(),
                });
            }
        };
        let environment = match value.env.trim() {
            "1" => OkxSystemEnvironment::Production,
            "2" => OkxSystemEnvironment::Demo,
            _ => {
                return Err(RestError::InvalidField {
                    field: "env",
                    value: value.env,
                    message: "unknown system maintenance environment".to_string(),
                });
            }
        };
        let begin_time_ms = parse_integer("begin", &value.begin)?;
        if begin_time_ms == 0 {
            return Err(RestError::InvalidField {
                field: "begin",
                value: value.begin,
                message: "system maintenance begin time must be positive".to_string(),
            });
        }
        let end_time_ms = parse_integer("end", &value.end)?;
        if end_time_ms < begin_time_ms {
            return Err(RestError::InvalidField {
                field: "end",
                value: value.end,
                message: "system maintenance end time must not precede begin time".to_string(),
            });
        }
        let pre_open_begin_time_ms =
            match parse_optional_integer("preOpenBegin", value.pre_open_begin.trim())? {
                0 => None,
                timestamp => Some(timestamp),
            };
        validate_required_text("system", &value.system)?;

        Ok(Self {
            title: value.title,
            description: value.description,
            state,
            begin_time_ms,
            end_time_ms,
            pre_open_begin_time_ms,
            service_type,
            maintenance_type,
            environment,
            system: value.system.trim().to_ascii_lowercase(),
        })
    }
}

fn parse_trade_fee_schedule(
    value: OkxTradeFeeScheduleWire,
) -> Result<Vec<OkxTradeFeeRate>, RestError> {
    let instrument_type = parse_instrument_type(&value.instrument_type)?;
    validate_required_text("level", &value.level)?;
    let timestamp_ms = parse_integer("ts", &value.timestamp)?;
    if timestamp_ms == 0 {
        return Err(RestError::InvalidField {
            field: "ts",
            value: value.timestamp,
            message: "trade fee timestamp must be positive".to_string(),
        });
    }
    if value.fee_groups.is_empty() {
        return Err(RestError::EmptyData {
            operation: "trade fee groups",
        });
    }
    let mut group_ids = HashSet::new();
    value
        .fee_groups
        .into_iter()
        .map(|group| {
            validate_required_text("feeGroup.groupId", &group.group_id)?;
            let group_id = group.group_id.trim().to_string();
            if !group_ids.insert(group_id.clone()) {
                return Err(RestError::InvalidField {
                    field: "feeGroup.groupId",
                    value: group_id,
                    message: "trade fee response repeated a group".to_string(),
                });
            }
            Ok(OkxTradeFeeRate {
                instrument_type,
                group_id,
                level: value.level.trim().to_string(),
                maker_rate: parse_number("feeGroup.maker", &group.maker)?,
                taker_rate: parse_number("feeGroup.taker", &group.taker)?,
                timestamp_ms,
            })
        })
        .collect()
}

#[derive(Debug, Deserialize)]
struct OkxInstrumentWire {
    #[serde(rename = "instId")]
    symbol: String,
    #[serde(rename = "instType")]
    instrument_type: String,
    #[serde(default, rename = "instFamily")]
    instrument_family: String,
    #[serde(default, rename = "groupId")]
    trade_fee_group_id: String,
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
    #[serde(rename = "maxLmtSz")]
    max_limit_size: String,
    #[serde(rename = "maxMktSz")]
    max_market_size: String,
    #[serde(rename = "maxLmtAmt")]
    max_limit_amount_usd: String,
    #[serde(rename = "maxMktAmt")]
    max_market_amount_usd: String,
    state: String,
    #[serde(rename = "upcChg")]
    upcoming_changes: Vec<OkxInstrumentChangeWire>,
}

#[derive(Debug, Deserialize)]
struct OkxInstrumentChangeWire {
    param: String,
    #[serde(rename = "newValue")]
    new_value: String,
    #[serde(rename = "effTime")]
    effective_time: String,
}

impl TryFrom<OkxInstrumentWire> for OkxInstrument {
    type Error = RestError;

    fn try_from(value: OkxInstrumentWire) -> Result<Self, Self::Error> {
        let min_size = parse_positive_number("minSz", &value.min_size)?;
        let max_limit_size = parse_positive_number("maxLmtSz", &value.max_limit_size)?;
        if max_limit_size < min_size {
            return Err(RestError::InvalidField {
                field: "maxLmtSz",
                value: value.max_limit_size,
                message: format!(
                    "maximum limit-order size must be at least minimum size {min_size}"
                ),
            });
        }
        Ok(Self {
            symbol: value.symbol,
            instrument_type: parse_instrument_type(&value.instrument_type)?,
            instrument_family: value.instrument_family,
            trade_fee_group_id: value.trade_fee_group_id,
            underlying: value.underlying,
            base_currency: value.base_currency,
            quote_currency: value.quote_currency,
            settle_currency: value.settle_currency,
            contract_type: parse_contract_type(&value.contract_type)?,
            contract_value: parse_nullable_number("ctVal", &value.contract_value)?,
            contract_value_currency: value.contract_value_currency,
            tick_size: parse_positive_number("tickSz", &value.tick_size)?,
            lot_size: parse_positive_number("lotSz", &value.lot_size)?,
            min_size,
            max_limit_size,
            max_market_size: parse_positive_number("maxMktSz", &value.max_market_size)?,
            max_limit_amount_usd: parse_nullable_positive_number(
                "maxLmtAmt",
                &value.max_limit_amount_usd,
            )?,
            max_market_amount_usd: parse_nullable_positive_number(
                "maxMktAmt",
                &value.max_market_amount_usd,
            )?,
            state: value.state,
            upcoming_changes: value
                .upcoming_changes
                .into_iter()
                .map(OkxInstrumentChange::try_from)
                .collect::<Result<_, _>>()?,
        })
    }
}

impl TryFrom<OkxInstrumentChangeWire> for OkxInstrumentChange {
    type Error = RestError;

    fn try_from(value: OkxInstrumentChangeWire) -> Result<Self, Self::Error> {
        let parameter = match value.param.as_str() {
            "tickSz" => OkxInstrumentChangeParameter::TickSize,
            "minSz" => OkxInstrumentChangeParameter::MinimumSize,
            "maxMktSz" => OkxInstrumentChangeParameter::MaximumMarketSize,
            _ => {
                return Err(RestError::InvalidField {
                    field: "upcChg.param",
                    value: value.param,
                    message: "unsupported upcoming instrument change parameter".to_string(),
                });
            }
        };
        let effective_time_ms = parse_integer("upcChg.effTime", &value.effective_time)?;
        if effective_time_ms == 0 {
            return Err(RestError::InvalidField {
                field: "upcChg.effTime",
                value: value.effective_time,
                message: "upcoming instrument change time must be positive".to_string(),
            });
        }
        Ok(Self {
            parameter,
            new_value: parse_positive_number("upcChg.newValue", &value.new_value)?,
            effective_time_ms,
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
    #[serde(default)]
    label: String,
    #[serde(default)]
    perm: String,
    #[serde(default)]
    ip: String,
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
            api_key_label: value.label,
            api_key_permissions: parse_api_key_permissions(&value.perm)?,
            api_key_ip_bindings: parse_api_key_ip_bindings(&value.ip)?,
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
                    equity_usd: parse_nullable_number("eqUsd", &detail.equity_usd)?,
                    discounted_equity_usd: parse_nullable_number(
                        "disEq",
                        &detail.discounted_equity_usd,
                    )?,
                    unrealized_pnl: parse_nullable_number("upl", &detail.unrealized_pnl)?,
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
    #[serde(default, rename = "eqUsd")]
    equity_usd: String,
    #[serde(default, rename = "disEq")]
    discounted_equity_usd: String,
    #[serde(default, rename = "upl")]
    unrealized_pnl: String,
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
    #[serde(default, rename = "cancelSource")]
    cancel_source: String,
    #[serde(default, rename = "cancelSourceReason")]
    cancel_source_reason: String,
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

impl TryFrom<OkxOrderWire> for OkxOrderDetails {
    type Error = RestError;

    fn try_from(value: OkxOrderWire) -> Result<Self, Self::Error> {
        let cancel_source = value.cancel_source.clone();
        let cancel_source_reason = value.cancel_source_reason.clone();
        Ok(Self {
            order: value.try_into()?,
            cancel_source,
            cancel_source_reason,
        })
    }
}

#[derive(Debug, Deserialize)]
struct OkxBillWire {
    #[serde(default, rename = "billId")]
    bill_id: String,
    #[serde(default, rename = "type")]
    bill_type: String,
    #[serde(default, rename = "subType")]
    sub_type: String,
    #[serde(default, rename = "ts")]
    timestamp: String,
    #[serde(default, rename = "ccy")]
    currency: String,
    #[serde(default, rename = "balChg")]
    balance_change: String,
    #[serde(default, rename = "bal")]
    balance: String,
    #[serde(default, rename = "posBalChg")]
    position_balance_change: String,
    #[serde(default, rename = "posBal")]
    position_balance: String,
    #[serde(default, rename = "sz")]
    quantity: String,
    #[serde(default, rename = "px")]
    price: String,
    #[serde(default, rename = "pnl")]
    pnl: String,
    #[serde(default, rename = "fee")]
    fee: String,
    #[serde(default, rename = "interest")]
    interest: String,
    #[serde(default, rename = "instType")]
    instrument_type: String,
    #[serde(default, rename = "instId")]
    symbol: String,
    #[serde(default, rename = "mgnMode")]
    margin_mode: String,
    #[serde(default, rename = "ordId")]
    order_id: String,
    #[serde(default, rename = "clOrdId")]
    client_order_id: String,
    #[serde(default, rename = "tradeId")]
    trade_id: String,
    #[serde(default, rename = "fillTime")]
    fill_time: String,
    #[serde(default, rename = "execType")]
    execution_type: String,
    #[serde(default, rename = "from")]
    from_account: String,
    #[serde(default, rename = "to")]
    to_account: String,
    #[serde(default, rename = "notes")]
    notes: String,
}

impl TryFrom<OkxBillWire> for OkxBill {
    type Error = RestError;

    fn try_from(value: OkxBillWire) -> Result<Self, Self::Error> {
        validate_required_text("billId", &value.bill_id)?;
        validate_ascii_digits("type", &value.bill_type)?;
        validate_ascii_digits("subType", &value.sub_type)?;
        validate_required_text("ccy", &value.currency)?;
        let timestamp_ms = parse_integer("ts", &value.timestamp)?;
        if timestamp_ms == 0 {
            return Err(RestError::InvalidField {
                field: "ts",
                value: value.timestamp,
                message: "must be positive".to_string(),
            });
        }
        let symbol = validate_optional_text("instId", value.symbol)?;
        let order_id = validate_optional_text("ordId", value.order_id)?;
        let client_order_id = match validate_optional_text("clOrdId", value.client_order_id)? {
            value if value == "0" => String::new(),
            value => value,
        };
        let trade_id = match validate_optional_text("tradeId", value.trade_id)? {
            value if value == "0" => String::new(),
            value => value,
        };
        Ok(Self {
            bill_id: value.bill_id,
            bill_type: value.bill_type,
            sub_type: value.sub_type,
            timestamp_ms,
            currency: value.currency.to_ascii_uppercase(),
            balance_change: parse_number("balChg", &value.balance_change)?,
            balance: parse_nullable_number("bal", &value.balance)?,
            position_balance_change: parse_nullable_number(
                "posBalChg",
                &value.position_balance_change,
            )?,
            position_balance: parse_nullable_number("posBal", &value.position_balance)?,
            quantity: parse_nullable_number("sz", &value.quantity)?,
            price: parse_nullable_number("px", &value.price)?,
            pnl: parse_nullable_number("pnl", &value.pnl)?,
            fee: parse_nullable_number("fee", &value.fee)?,
            interest: parse_nullable_number("interest", &value.interest)?,
            instrument_type: parse_optional_instrument_type(&value.instrument_type)?,
            symbol,
            margin_mode: parse_bill_margin_mode(&value.margin_mode)?,
            order_id,
            client_order_id,
            trade_id,
            fill_time_ms: parse_nullable_integer("fillTime", &value.fill_time)?,
            execution_type: parse_bill_execution_type(&value.execution_type)?,
            from_account: parse_bill_account_type("from", &value.from_account)?,
            to_account: parse_bill_account_type("to", &value.to_account)?,
            notes: value.notes,
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

fn validate_ascii_digits(field: &'static str, value: &str) -> Result<(), RestError> {
    validate_required_text(field, value)?;
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(RestError::InvalidField {
            field,
            value: value.to_string(),
            message: "must contain only ASCII digits".to_string(),
        });
    }
    Ok(())
}

fn validate_optional_text(field: &'static str, value: String) -> Result<String, RestError> {
    if value.trim() != value {
        return Err(RestError::InvalidField {
            field,
            value,
            message: "must contain no surrounding whitespace".to_string(),
        });
    }
    Ok(value)
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

fn parse_optional_instrument_type(value: &str) -> Result<Option<OkxInstrumentType>, RestError> {
    if value.is_empty() {
        Ok(None)
    } else {
        parse_instrument_type(value).map(Some)
    }
}

fn parse_bill_execution_type(value: &str) -> Result<Option<OkxBillExecutionType>, RestError> {
    match value {
        "" => Ok(None),
        "M" => Ok(Some(OkxBillExecutionType::Maker)),
        "T" => Ok(Some(OkxBillExecutionType::Taker)),
        other => Err(RestError::InvalidField {
            field: "execType",
            value: other.to_string(),
            message: "expected M or T".to_string(),
        }),
    }
}

fn parse_bill_margin_mode(value: &str) -> Result<Option<OkxBillMarginMode>, RestError> {
    match value {
        "" => Ok(None),
        "cash" => Ok(Some(OkxBillMarginMode::Cash)),
        "cross" => Ok(Some(OkxBillMarginMode::Cross)),
        "isolated" => Ok(Some(OkxBillMarginMode::Isolated)),
        other => Err(RestError::InvalidField {
            field: "mgnMode",
            value: other.to_string(),
            message: "expected cash, cross, or isolated".to_string(),
        }),
    }
}

fn parse_bill_account_type(
    field: &'static str,
    value: &str,
) -> Result<Option<OkxBillAccountType>, RestError> {
    match value {
        "" => Ok(None),
        "6" => Ok(Some(OkxBillAccountType::Funding)),
        "18" => Ok(Some(OkxBillAccountType::Trading)),
        other => Err(RestError::InvalidField {
            field,
            value: other.to_string(),
            message: "expected account type 6 or 18".to_string(),
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

fn parse_api_key_permissions(value: &str) -> Result<BTreeSet<OkxApiKeyPermission>, RestError> {
    if value.trim().is_empty() {
        return Ok(BTreeSet::new());
    }
    value
        .split(',')
        .map(|permission| match permission.trim() {
            "read_only" => Ok(OkxApiKeyPermission::ReadOnly),
            "trade" => Ok(OkxApiKeyPermission::Trade),
            "withdraw" => Ok(OkxApiKeyPermission::Withdraw),
            other => Err(RestError::InvalidField {
                field: "perm",
                value: other.to_string(),
                message: "expected read_only, trade, or withdraw".to_string(),
            }),
        })
        .collect()
}

fn parse_api_key_ip_bindings(value: &str) -> Result<BTreeSet<String>, RestError> {
    if value.trim().is_empty() {
        return Ok(BTreeSet::new());
    }
    value
        .split(',')
        .map(|binding| {
            let binding = binding.trim();
            if binding.is_empty() {
                Err(RestError::InvalidField {
                    field: "ip",
                    value: value.to_string(),
                    message: "IP bindings must be non-empty comma-separated values".to_string(),
                })
            } else {
                Ok(binding.to_string())
            }
        })
        .collect()
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

fn parse_nullable_positive_number(
    field: &'static str,
    value: &str,
) -> Result<Option<f64>, RestError> {
    if value.is_empty() {
        Ok(None)
    } else {
        parse_positive_number(field, value).map(Some)
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

fn parse_nullable_integer(field: &'static str, value: &str) -> Result<Option<u64>, RestError> {
    if value.is_empty() {
        Ok(None)
    } else {
        let parsed = parse_integer(field, value)?;
        if parsed == 0 {
            return Err(RestError::InvalidField {
                field,
                value: value.to_string(),
                message: "must be positive when present".to_string(),
            });
        }
        Ok(Some(parsed))
    }
}
#[cfg(test)]
mod tests {
    use super::*;

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

    fn bill_response(first: usize, count: usize) -> String {
        let data = (first..first + count)
            .map(|index| {
                serde_json::json!({
                    "bal": "1000.01",
                    "balChg": "0.01",
                    "billId": format!("bill-{index}"),
                    "ccy": "USDT",
                    "clOrdId": format!("client-{index}"),
                    "execType": "M",
                    "fee": "-0.01",
                    "fillTime": "1000",
                    "from": "",
                    "instId": "BTC-USDT-SWAP",
                    "instType": "SWAP",
                    "interest": "0",
                    "mgnMode": "cross",
                    "notes": "",
                    "ordId": format!("order-{index}"),
                    "pnl": "0.02",
                    "posBal": "2",
                    "posBalChg": "1",
                    "px": "50000",
                    "subType": "3",
                    "sz": "1",
                    "to": "",
                    "tradeId": format!("trade-{index}"),
                    "ts": "1001",
                    "type": "2"
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({"code": "0", "msg": "", "data": data}).to_string()
    }

    #[test]
    fn fill_parser_and_pagination_preserve_identity_and_fail_closed() {
        let first = parse_okx_fill_page_response_json(fill_response(100, 100).as_bytes()).unwrap();
        assert_eq!(first.next_after.as_deref(), Some("bill-199"));
        assert_eq!(first.fills[0].fill_id, "fill-100");
        assert_eq!(
            first.fills[0].fee,
            Some(FillFee {
                amount: -0.00001,
                currency: "BTC".to_string(),
            })
        );

        let mut complete = OkxFillPagination::new(3).unwrap();
        assert!(!complete.accept(first).unwrap());
        let last = parse_okx_fill_page_response_json(fill_response(200, 2).as_bytes()).unwrap();
        assert!(complete.accept(last).unwrap());
        assert_eq!(complete.into_fills().len(), 102);

        let mut bounded = OkxFillPagination::new(1).unwrap();
        let full = parse_okx_fill_page_response_json(fill_response(300, 100).as_bytes()).unwrap();
        assert!(matches!(
            bounded.accept(full),
            Err(RestError::FillPaginationLimit {
                pages: 1,
                records: 100,
                ..
            })
        ));
    }

    #[test]
    fn bill_parser_and_pagination_preserve_economic_identity_and_fail_closed() {
        let first = parse_okx_bill_page_response_json(bill_response(100, 100).as_bytes()).unwrap();
        assert_eq!(first.next_after.as_deref(), Some("bill-199"));
        assert_eq!(first.bills[0].trade_id, "trade-100");
        assert_eq!(first.bills[0].margin_mode, Some(OkxBillMarginMode::Cross));

        let mut complete = OkxBillPagination::new(3).unwrap();
        assert!(!complete.accept(first).unwrap());
        let last = parse_okx_bill_page_response_json(bill_response(200, 2).as_bytes()).unwrap();
        assert!(complete.accept(last).unwrap());
        assert_eq!(complete.into_bills().len(), 102);

        let mut bounded = OkxBillPagination::new(1).unwrap();
        let full = parse_okx_bill_page_response_json(bill_response(300, 100).as_bytes()).unwrap();
        assert!(matches!(
            bounded.accept(full),
            Err(RestError::BillPaginationLimit {
                pages: 1,
                records: 100,
                ..
            })
        ));
    }

    #[test]
    fn system_status_and_trade_fee_parsers_preserve_current_contracts() {
        let statuses = parse_okx_system_status_response_json(
            br#"{"code":"0","data":[{"begin":"1784016000000","end":"1784017200000","env":"1","maintType":"1","preOpenBegin":"","scheDesc":"","serviceType":"11","state":"ongoing","system":"unified","title":"Copy trading maintenance"}],"msg":""}"#,
        )
        .unwrap();
        assert_eq!(statuses[0].state, OkxSystemStatusState::Ongoing);
        assert_eq!(statuses[0].service_type, OkxSystemServiceType::CopyTrading);
        assert_eq!(statuses[0].environment, OkxSystemEnvironment::Production);

        let rates = parse_okx_trade_fee_response_json(
            br#"{"code":"0","msg":"","data":[{"feeGroup":[{"groupId":"1","maker":"-0.0002","taker":"-0.0005"}],"instType":"SPOT","level":"Lv1","ts":"1763979985847"}]}"#,
        )
        .unwrap();
        assert_eq!(rates[0].maker_cost_rate(), 0.0002);
        assert_eq!(rates[0].taker_cost_rate(), 0.0005);
    }

    #[test]
    fn account_instrument_parser_retains_typed_upcoming_changes() {
        let instruments = parse_okx_account_instruments_response_json(
            br#"{"code":"0","msg":"","data":[{"instId":"BTC-USDT-SWAP","instType":"SWAP","instFamily":"BTC-USDT","groupId":"2","baseCcy":"BTC","quoteCcy":"USDT","settleCcy":"USDT","ctType":"linear","ctVal":"0.01","ctValCcy":"BTC","tickSz":"0.1","lotSz":"1","minSz":"1","maxLmtSz":"1000000","maxMktSz":"1000000","maxLmtAmt":"","maxMktAmt":"","state":"live","upcChg":[{"param":"tickSz","newValue":"0.01","effTime":"1763979985847"}]}]}"#,
        )
        .unwrap();
        assert_eq!(instruments.len(), 1);
        assert_eq!(instruments[0].symbol, "BTC-USDT-SWAP");
        assert_eq!(
            instruments[0].upcoming_changes[0].parameter,
            OkxInstrumentChangeParameter::TickSize
        );
    }

    #[test]
    fn server_time_and_acknowledgement_parsers_are_credential_free() {
        assert_eq!(
            parse_okx_server_time_response_json(
                br#"{"code":"0","msg":"","data":[{"ts":"1597026383085"}]}"#
            )
            .unwrap(),
            1_597_026_383_085
        );
        let ack = parse_okx_order_ack_response_json(
            br#"{"code":"0","msg":"","data":[{"ordId":"123","clOrdId":"reap1","sCode":"0","sMsg":""}]}"#,
            "cancel order",
        )
        .unwrap();
        assert_eq!(ack.exchange_order_id, "123");
        assert!(
            parse_okx_cancel_all_after_response_json(
                br#"{"code":"0","msg":"","data":[{"triggerTime":"1597026443"}]}"#,
                30,
            )
            .is_ok()
        );
        assert!(matches!(
            parse_okx_cancel_all_after_response_json(
                br#"{"code":"0","msg":"","data":[{"triggerTime":"0"}]}"#,
                30,
            ),
            Err(RestError::InvalidField {
                field: "triggerTime",
                ..
            })
        ));
    }

    #[test]
    fn index_open_order_and_order_detail_parsers_retain_identity() {
        let ticker = parse_okx_index_ticker_response_json(
            br#"{"code":"0","msg":"","data":[{"instId":"BTC-USD","idxPx":"50000.25","ts":"1597026383085"}]}"#,
        )
        .unwrap();
        assert_eq!(ticker.symbol, "BTC-USD");
        assert_eq!(ticker.index_price, 50_000.25);

        let orders = parse_okx_open_orders_response_json(
            br#"{"code":"0","msg":"","data":[{"ordId":"123","clOrdId":"reap1","instId":"BTC-USDT","side":"buy","state":"partially_filled","px":"100","sz":"1","accFillSz":"0.4","avgPx":"99.5","uTime":"1000"}]}"#,
        )
        .unwrap();
        assert_eq!(orders[0].state, PrivateOrderState::PartiallyFilled);

        let details = parse_okx_order_details_response_json(
            br#"{"code":"0","msg":"","data":[{"ordId":"123","clOrdId":"reap1","instId":"BTC-USDT","side":"buy","state":"canceled","px":"100","sz":"1","accFillSz":"0","avgPx":"","uTime":"1000","cancelSource":"20","cancelSourceReason":"Cancel all after triggered"}]}"#,
        )
        .unwrap();
        assert_eq!(details.order.state, PrivateOrderState::Cancelled);
        assert_eq!(details.cancel_source, "20");
    }

    #[test]
    fn account_parsers_preserve_borrowing_and_position_evidence() {
        let account = parse_okx_account_config_response_json(
            br#"{"code":"0","msg":"","data":[{"acctLv":"1","posMode":"net_mode","uid":"7","mainUid":"6","enableSpotBorrow":false,"autoLoan":false,"spotBorrowAutoRepay":true}]}"#,
        )
        .unwrap();
        assert_eq!(account.enable_spot_borrow, Some(false));

        let balance = parse_okx_account_balance_response_json(
            br#"{"code":"0","msg":"","data":[{"uTime":"1000","totalEq":"100","adjEq":"99","borrowFroz":"2","notionalUsdForBorrow":"3","details":[{"ccy":"USDT","uTime":"999","cashBal":"100","availBal":"90","eq":"99","eqUsd":"98.5","disEq":"97","upl":"-1","liab":"1","crossLiab":"0.5","isoLiab":"0.25","uplLiab":"0.1","interest":"0.01","borrowFroz":"2","maxLoan":"50","twap":"1"}]}]}"#,
        )
        .unwrap();
        assert_eq!(balance.borrow_frozen_usd, Some(2.0));
        assert_eq!(balance.details[0].liability, Some(1.0));

        let positions = parse_okx_account_positions_response_json(
            br#"{"code":"0","msg":"","data":[{"instType":"MARGIN","instId":"BTC-USDT","pos":"1","posSide":"net","mgnMode":"cross","uTime":"1001","liab":"20","interest":"0.02","pendingCloseOrdLiabVal":"1","baseBorrowed":"0","baseInterest":"0","quoteBorrowed":"20","quoteInterest":"0.02"}]}"#,
        )
        .unwrap();
        assert_eq!(
            positions.positions[0].instrument_type,
            OkxInstrumentType::Margin
        );
        assert_eq!(positions.positions[0].quote_interest, Some(0.02));
    }

    #[test]
    fn parsers_reject_api_errors_unknown_permissions_and_non_finite_numbers() {
        assert!(matches!(
            parse_okx_fills_response_json(
                br#"{"code":"50011","msg":"rate limit reached","data":[]}"#
            ),
            Err(RestError::Api { ref code, .. }) if code == "50011"
        ));
        assert!(matches!(
            parse_okx_account_config_response_json(
                br#"{"code":"0","msg":"","data":[{"acctLv":"1","posMode":"net_mode","perm":"read_only,transfer"}]}"#
            ),
            Err(RestError::InvalidField { field: "perm", .. })
        ));
        assert!(matches!(
            parse_number("px", "NaN"),
            Err(RestError::InvalidField { .. })
        ));
    }
}
