use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use reap_core::{FillLiquidity, SelfTradePrevention, Side, TimeInForce};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::form_urlencoded;

use crate::{PrivateOrderState, RemoteFill, RemoteOrder};

use super::{AuthError, HttpMethod, OkxSigner, SignedRequest};

const PLACE_ORDER_PATH: &str = "/api/v5/trade/order";
const CANCEL_ORDER_PATH: &str = "/api/v5/trade/cancel-order";
const OPEN_ORDERS_PATH: &str = "/api/v5/trade/orders-pending";
const FILLS_PATH: &str = "/api/v5/trade/fills";

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
        let client = reqwest::Client::builder()
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

pub struct OkxRestClient<T> {
    transport: T,
    signer: OkxSigner,
}

impl<T> OkxRestClient<T>
where
    T: HttpTransport,
{
    pub fn new(transport: T, signer: OkxSigner) -> Self {
        Self { transport, signer }
    }

    pub fn signer(&self) -> &OkxSigner {
        &self.signer
    }

    pub async fn place_order(&self, order: &OkxPlaceOrder) -> Result<OkxOrderAck, RestError> {
        self.place_order_at(&timestamp_now(), order).await
    }

    pub async fn place_order_at(
        &self,
        timestamp: &str,
        order: &OkxPlaceOrder,
    ) -> Result<OkxOrderAck, RestError> {
        let request = self.build_place_request(timestamp, order)?;
        self.execute_ack(request, "place order").await
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
        let path = query_path(
            FILLS_PATH,
            [("instType", instrument_type), ("instId", symbol)],
        );
        let request = self
            .signer
            .sign_request(timestamp, HttpMethod::Get, path, "")?;
        let response: OkxResponse<OkxFillWire> = self.execute(request).await?;
        response
            .data
            .into_iter()
            .map(RemoteFill::try_from)
            .collect()
    }

    pub fn build_place_request(
        &self,
        timestamp: &str,
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
        Ok(self
            .signer
            .sign_request(timestamp, HttpMethod::Post, PLACE_ORDER_PATH, body)?)
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
    #[serde(rename = "fillTime")]
    fill_time: String,
}

impl TryFrom<OkxFillWire> for RemoteFill {
    type Error = RestError;

    fn try_from(value: OkxFillWire) -> Result<Self, Self::Error> {
        Ok(Self {
            fill_id: value.fill_id,
            exchange_order_id: value.order_id,
            client_order_id: value.client_order_id,
            symbol: value.symbol,
            side: parse_side(&value.side)?,
            price: parse_number("fillPx", &value.price)?,
            qty: parse_number("fillSz", &value.qty)?,
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
            ts_ms: parse_integer("fillTime", &value.fill_time)?,
        })
    }
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
    value
        .parse()
        .map_err(|error: std::num::ParseFloatError| RestError::InvalidField {
            field,
            value: value.to_string(),
            message: error.to_string(),
        })
}

fn parse_optional_number(field: &'static str, value: &str) -> Result<f64, RestError> {
    if value.is_empty() {
        Ok(0.0)
    } else {
        parse_number(field, value)
    }
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
        let requests = Arc::new(Mutex::new(Vec::new()));
        let transport = MockTransport {
            responses: Arc::new(Mutex::new(
                responses.into_iter().map(str::to_string).collect(),
            )),
            requests: Arc::clone(&requests),
        };
        let signer = OkxSigner::new(OkxCredentials::new("key", "secret", "pass"), false);
        (OkxRestClient::new(transport, signer), requests)
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
    async fn parses_open_orders_and_fills_for_reconciliation() {
        let (client, requests) = client(vec![
            r#"{"code":"0","msg":"","data":[{"ordId":"123","clOrdId":"reap1","instId":"BTC-USDT","side":"buy","state":"partially_filled","px":"100","sz":"1","accFillSz":"0.4","avgPx":"99.5","uTime":"1000"}]}"#,
            r#"{"code":"0","msg":"","data":[{"tradeId":"fill1","ordId":"123","clOrdId":"reap1","instId":"BTC-USDT","side":"buy","fillPx":"99.5","fillSz":"0.4","execType":"M","fillTime":"1000"}]}"#,
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
        let requests = requests.lock().unwrap();
        assert_eq!(
            requests[0].path,
            "/api/v5/trade/orders-pending?instType=SPOT&instId=BTC-USDT"
        );
        assert_eq!(
            requests[1].path,
            "/api/v5/trade/fills?instType=SPOT&instId=BTC-USDT"
        );
    }
}
