use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::RemoteOrder;

use super::{
    HttpMethod, HttpTransport, OPEN_ORDERS_PATH, OkxOrderWire, OkxResponse, OkxRestClient,
    RestError, decode_okx_response, parse_integer, query_path, timestamp_now,
    validate_optional_text, validate_required_text,
};

const ALGO_ORDERS_PATH: &str = "/api/v5/trade/orders-algo-pending";
const CANCEL_ALGO_ORDERS_PATH: &str = "/api/v5/trade/cancel-algos";
const SPREAD_ORDERS_PATH: &str = "/api/v5/sprd/orders-pending";
const SPREAD_MASS_CANCEL_PATH: &str = "/api/v5/sprd/mass-cancel";
const SPREAD_CANCEL_ALL_AFTER_PATH: &str = "/api/v5/sprd/cancel-all-after";

pub const OKX_PENDING_ORDER_PAGE_LIMIT: usize = 100;
pub const OKX_ALGO_CANCEL_BATCH_LIMIT: usize = 10;
pub const OKX_DEFAULT_MAX_PENDING_ORDER_PAGES: usize = 64;

const REGULAR_DOMAIN: &str = "regular";
const ALGO_DOMAIN: &str = "algo";
const SPREAD_DOMAIN: &str = "spread";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OkxAlgoOrderQuery {
    ConditionalAndOco,
    Chase,
    Trigger,
    MoveOrderStop,
    Iceberg,
    Twap,
    SmartIceberg,
}

impl OkxAlgoOrderQuery {
    pub const ALL: [Self; 7] = [
        Self::ConditionalAndOco,
        Self::Chase,
        Self::Trigger,
        Self::MoveOrderStop,
        Self::Iceberg,
        Self::Twap,
        Self::SmartIceberg,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ConditionalAndOco => "conditional,oco",
            Self::Chase => "chase",
            Self::Trigger => "trigger",
            Self::MoveOrderStop => "move_order_stop",
            Self::Iceberg => "iceberg",
            Self::Twap => "twap",
            Self::SmartIceberg => "smart_iceberg",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OkxAlgoOrderType {
    Conditional,
    Oco,
    Chase,
    Trigger,
    MoveOrderStop,
    Iceberg,
    Twap,
    SmartIceberg,
}

impl TryFrom<&str> for OkxAlgoOrderType {
    type Error = RestError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "conditional" => Ok(Self::Conditional),
            "oco" => Ok(Self::Oco),
            "chase" => Ok(Self::Chase),
            "trigger" => Ok(Self::Trigger),
            "move_order_stop" => Ok(Self::MoveOrderStop),
            "iceberg" => Ok(Self::Iceberg),
            "twap" => Ok(Self::Twap),
            "smart_iceberg" => Ok(Self::SmartIceberg),
            _ => Err(RestError::InvalidField {
                field: "ordType",
                value: value.to_string(),
                message: "unsupported pending algo order type".to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OkxAlgoOrder {
    pub algo_id: String,
    pub client_order_id: String,
    pub symbol: String,
    pub order_type: OkxAlgoOrderType,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OkxSpreadOrder {
    pub spread_id: String,
    pub exchange_order_id: String,
    pub client_order_id: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OkxRegularOrderPage {
    pub orders: Vec<RemoteOrder>,
    pub next_after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OkxAlgoOrderPage {
    pub orders: Vec<OkxAlgoOrder>,
    pub next_after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OkxSpreadOrderPage {
    pub orders: Vec<OkxSpreadOrder>,
    pub next_end_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OkxCancelAlgoOrder {
    pub symbol: String,
    pub algo_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OkxAlgoCancelResult {
    pub algo_id: String,
    pub client_order_id: String,
    pub code: String,
    pub message: String,
}

impl OkxAlgoCancelResult {
    pub fn accepted(&self) -> bool {
        self.code.is_empty() || self.code == "0"
    }
}

#[derive(Debug)]
struct PendingOrderPagination<T> {
    domain: &'static str,
    max_pages: usize,
    pages: usize,
    cursor: Option<String>,
    orders: Vec<T>,
    seen_cursors: BTreeSet<String>,
    seen_order_ids: BTreeSet<String>,
}

impl<T> PendingOrderPagination<T> {
    fn new(domain: &'static str, max_pages: usize) -> Result<Self, RestError> {
        if max_pages == 0 {
            return Err(RestError::InvalidField {
                field: "max_pending_order_pages",
                value: max_pages.to_string(),
                message: "must be positive".to_string(),
            });
        }
        Ok(Self {
            domain,
            max_pages,
            pages: 0,
            cursor: None,
            orders: Vec::new(),
            seen_cursors: BTreeSet::new(),
            seen_order_ids: BTreeSet::new(),
        })
    }

    fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    fn accept(
        &mut self,
        orders: Vec<T>,
        next_cursor: Option<String>,
        identity: impl Fn(&T) -> &str,
    ) -> Result<bool, RestError> {
        self.pages = self.pages.saturating_add(1);
        for order in &orders {
            let order_id = identity(order);
            validate_required_text("pendingOrderId", order_id)?;
            if !self.seen_order_ids.insert(order_id.to_string()) {
                return Err(RestError::PendingOrderPaginationDuplicate {
                    domain: self.domain,
                    order_id: order_id.to_string(),
                });
            }
        }
        self.orders.extend(orders);
        let Some(next_cursor) = next_cursor else {
            self.cursor = None;
            return Ok(true);
        };
        if !self.seen_cursors.insert(next_cursor.clone()) {
            return Err(RestError::PendingOrderPaginationCursor {
                domain: self.domain,
                cursor: next_cursor,
            });
        }
        if self.pages >= self.max_pages {
            return Err(RestError::PendingOrderPaginationLimit {
                domain: self.domain,
                pages: self.pages,
                records: self.orders.len(),
                next_cursor,
            });
        }
        self.cursor = Some(next_cursor);
        Ok(false)
    }
}

#[derive(Debug)]
pub struct OkxRegularOrderPagination(PendingOrderPagination<RemoteOrder>);

impl OkxRegularOrderPagination {
    pub fn new(max_pages: usize) -> Result<Self, RestError> {
        PendingOrderPagination::new(REGULAR_DOMAIN, max_pages).map(Self)
    }

    pub fn after(&self) -> Option<&str> {
        self.0.cursor()
    }

    pub fn accept(&mut self, page: OkxRegularOrderPage) -> Result<bool, RestError> {
        self.0.accept(page.orders, page.next_after, |order| {
            &order.exchange_order_id
        })
    }

    pub fn into_orders(self) -> Vec<RemoteOrder> {
        self.0.orders
    }
}

#[derive(Debug)]
pub struct OkxAlgoOrderPagination(PendingOrderPagination<OkxAlgoOrder>);

impl OkxAlgoOrderPagination {
    pub fn new(max_pages: usize) -> Result<Self, RestError> {
        PendingOrderPagination::new(ALGO_DOMAIN, max_pages).map(Self)
    }

    pub fn after(&self) -> Option<&str> {
        self.0.cursor()
    }

    pub fn accept(&mut self, page: OkxAlgoOrderPage) -> Result<bool, RestError> {
        self.0
            .accept(page.orders, page.next_after, |order| &order.algo_id)
    }

    pub fn into_orders(self) -> Vec<OkxAlgoOrder> {
        self.0.orders
    }
}

#[derive(Debug)]
pub struct OkxSpreadOrderPagination(PendingOrderPagination<OkxSpreadOrder>);

impl OkxSpreadOrderPagination {
    pub fn new(max_pages: usize) -> Result<Self, RestError> {
        PendingOrderPagination::new(SPREAD_DOMAIN, max_pages).map(Self)
    }

    pub fn end_id(&self) -> Option<&str> {
        self.0.cursor()
    }

    pub fn accept(&mut self, page: OkxSpreadOrderPage) -> Result<bool, RestError> {
        self.0.accept(page.orders, page.next_end_id, |order| {
            &order.exchange_order_id
        })
    }

    pub fn into_orders(self) -> Vec<OkxSpreadOrder> {
        self.0.orders
    }
}

pub fn parse_okx_regular_order_page_response_json(
    body: &[u8],
) -> Result<OkxRegularOrderPage, RestError> {
    let response: OkxResponse<OkxOrderWire> = decode_okx_response(body)?;
    validate_page_size(response.data.len(), REGULAR_DOMAIN)?;
    for order in &response.data {
        validate_required_text("ordId", &order.order_id)?;
    }
    let next_after = full_page_cursor(
        response.data.len(),
        response.data.last().map(|order| order.order_id.as_str()),
        "ordId",
    )?;
    let orders = response
        .data
        .into_iter()
        .map(RemoteOrder::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(OkxRegularOrderPage { orders, next_after })
}

pub fn parse_okx_algo_order_page_response_json(body: &[u8]) -> Result<OkxAlgoOrderPage, RestError> {
    let response: OkxResponse<OkxAlgoOrderWire> = decode_okx_response(body)?;
    validate_page_size(response.data.len(), ALGO_DOMAIN)?;
    let next_after = full_page_cursor(
        response.data.len(),
        response.data.last().map(|order| order.algo_id.as_str()),
        "algoId",
    )?;
    let orders = response
        .data
        .into_iter()
        .map(OkxAlgoOrder::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(OkxAlgoOrderPage { orders, next_after })
}

pub fn parse_okx_spread_order_page_response_json(
    body: &[u8],
) -> Result<OkxSpreadOrderPage, RestError> {
    let response: OkxResponse<OkxSpreadOrderWire> = decode_okx_response(body)?;
    validate_page_size(response.data.len(), SPREAD_DOMAIN)?;
    let next_end_id = full_page_cursor(
        response.data.len(),
        response
            .data
            .last()
            .map(|order| order.exchange_order_id.as_str()),
        "ordId",
    )?;
    let orders = response
        .data
        .into_iter()
        .map(OkxSpreadOrder::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(OkxSpreadOrderPage {
        orders,
        next_end_id,
    })
}

fn validate_page_size(size: usize, domain: &'static str) -> Result<(), RestError> {
    if size > OKX_PENDING_ORDER_PAGE_LIMIT {
        return Err(RestError::InvalidField {
            field: "data",
            value: size.to_string(),
            message: format!(
                "{domain} pending-order page exceeded requested limit {OKX_PENDING_ORDER_PAGE_LIMIT}"
            ),
        });
    }
    Ok(())
}

fn full_page_cursor(
    size: usize,
    cursor: Option<&str>,
    field: &'static str,
) -> Result<Option<String>, RestError> {
    if size < OKX_PENDING_ORDER_PAGE_LIMIT {
        return Ok(None);
    }
    let cursor = cursor.unwrap_or_default();
    validate_required_text(field, cursor)?;
    Ok(Some(cursor.to_string()))
}

impl<T> OkxRestClient<T>
where
    T: HttpTransport,
{
    pub async fn regular_pending_orders_page(
        &self,
        instrument_type: Option<&str>,
        symbol: Option<&str>,
        after: Option<&str>,
    ) -> Result<OkxRegularOrderPage, RestError> {
        self.regular_pending_orders_page_at(&timestamp_now(), instrument_type, symbol, after)
            .await
    }

    pub async fn regular_pending_orders_page_at(
        &self,
        timestamp: &str,
        instrument_type: Option<&str>,
        symbol: Option<&str>,
        after: Option<&str>,
    ) -> Result<OkxRegularOrderPage, RestError> {
        let path = query_path(
            OPEN_ORDERS_PATH,
            [
                ("instType", instrument_type),
                ("instId", symbol),
                ("after", after),
                ("limit", Some("100")),
            ],
        );
        let request = self
            .signer
            .sign_request(timestamp, HttpMethod::Get, path, "")?;
        let response = self.transport.execute(request).await?;
        parse_okx_regular_order_page_response_json(response.body.as_bytes())
    }

    pub async fn algo_pending_orders_page(
        &self,
        query: OkxAlgoOrderQuery,
        after: Option<&str>,
    ) -> Result<OkxAlgoOrderPage, RestError> {
        self.algo_pending_orders_page_at(&timestamp_now(), query, after)
            .await
    }

    pub async fn algo_pending_orders_page_at(
        &self,
        timestamp: &str,
        query: OkxAlgoOrderQuery,
        after: Option<&str>,
    ) -> Result<OkxAlgoOrderPage, RestError> {
        let path = query_path(
            ALGO_ORDERS_PATH,
            [
                ("ordType", Some(query.as_str())),
                ("after", after),
                ("limit", Some("100")),
            ],
        );
        let request = self
            .signer
            .sign_request(timestamp, HttpMethod::Get, path, "")?;
        let response = self.transport.execute(request).await?;
        parse_okx_algo_order_page_response_json(response.body.as_bytes())
    }

    pub async fn spread_pending_orders_page(
        &self,
        end_id: Option<&str>,
    ) -> Result<OkxSpreadOrderPage, RestError> {
        self.spread_pending_orders_page_at(&timestamp_now(), end_id)
            .await
    }

    pub async fn spread_pending_orders_page_at(
        &self,
        timestamp: &str,
        end_id: Option<&str>,
    ) -> Result<OkxSpreadOrderPage, RestError> {
        let path = query_path(
            SPREAD_ORDERS_PATH,
            [("endId", end_id), ("limit", Some("100"))],
        );
        let request = self
            .signer
            .sign_request(timestamp, HttpMethod::Get, path, "")?;
        let response = self.transport.execute(request).await?;
        parse_okx_spread_order_page_response_json(response.body.as_bytes())
    }

    pub async fn cancel_algo_orders(
        &self,
        orders: &[OkxCancelAlgoOrder],
    ) -> Result<Vec<OkxAlgoCancelResult>, RestError> {
        self.cancel_algo_orders_at(&timestamp_now(), orders).await
    }

    pub async fn cancel_algo_orders_at(
        &self,
        timestamp: &str,
        orders: &[OkxCancelAlgoOrder],
    ) -> Result<Vec<OkxAlgoCancelResult>, RestError> {
        if orders.is_empty() || orders.len() > OKX_ALGO_CANCEL_BATCH_LIMIT {
            return Err(RestError::InvalidField {
                field: "orders",
                value: orders.len().to_string(),
                message: format!(
                    "algo cancel batch must contain 1-{OKX_ALGO_CANCEL_BATCH_LIMIT} orders"
                ),
            });
        }
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(rename = "instId")]
            symbol: &'a str,
            #[serde(rename = "algoId")]
            algo_id: &'a str,
        }
        let mut body = Vec::with_capacity(orders.len());
        for order in orders {
            validate_required_text("instId", &order.symbol)?;
            validate_required_text("algoId", &order.algo_id)?;
            body.push(Body {
                symbol: &order.symbol,
                algo_id: &order.algo_id,
            });
        }
        let request = self.signer.sign_request(
            timestamp,
            HttpMethod::Post,
            CANCEL_ALGO_ORDERS_PATH,
            serde_json::to_string(&body)?,
        )?;
        let response: OkxResponse<OkxAlgoCancelWire> = self.execute(request).await?;
        if response.data.is_empty() {
            return Err(RestError::EmptyData {
                operation: "cancel algo orders",
            });
        }
        response
            .data
            .into_iter()
            .map(OkxAlgoCancelResult::try_from)
            .collect()
    }

    pub async fn spread_mass_cancel(&self) -> Result<(), RestError> {
        self.spread_mass_cancel_at(&timestamp_now()).await
    }

    pub async fn spread_mass_cancel_at(&self, timestamp: &str) -> Result<(), RestError> {
        let request =
            self.signer
                .sign_request(timestamp, HttpMethod::Post, SPREAD_MASS_CANCEL_PATH, "{}")?;
        let mut response: OkxResponse<OkxSpreadMassCancelWire> = self.execute(request).await?;
        if response.data.len() != 1 {
            return Err(RestError::InvalidField {
                field: "data",
                value: response.data.len().to_string(),
                message: "spread mass cancel must return exactly one result".to_string(),
            });
        }
        if !response.data.remove(0).result {
            return Err(RestError::InvalidField {
                field: "result",
                value: "false".to_string(),
                message: "spread mass cancel was not accepted".to_string(),
            });
        }
        Ok(())
    }

    pub async fn spread_cancel_all_after(&self, timeout_secs: u64) -> Result<(), RestError> {
        self.spread_cancel_all_after_at(&timestamp_now(), timeout_secs)
            .await
    }

    pub async fn spread_cancel_all_after_at(
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
        let request = self.signer.sign_request(
            timestamp,
            HttpMethod::Post,
            SPREAD_CANCEL_ALL_AFTER_PATH,
            serde_json::to_string(&Body {
                timeout_secs: timeout_secs.to_string(),
            })?,
        )?;
        let mut response: OkxResponse<OkxSpreadCancelAllAfterWire> = self.execute(request).await?;
        if response.data.len() != 1 {
            return Err(RestError::InvalidField {
                field: "data",
                value: response.data.len().to_string(),
                message: "spread Cancel All After must return exactly one acknowledgement"
                    .to_string(),
            });
        }
        let acknowledgement = response.data.remove(0);
        if timeout_secs != 0 && parse_integer("triggerTime", &acknowledgement.trigger_time)? == 0 {
            return Err(RestError::InvalidField {
                field: "triggerTime",
                value: acknowledgement.trigger_time,
                message: "must be nonzero when spread Cancel All After is armed".to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct OkxAlgoOrderWire {
    #[serde(rename = "algoId")]
    algo_id: String,
    #[serde(default, rename = "algoClOrdId")]
    client_order_id: String,
    #[serde(rename = "instId")]
    symbol: String,
    #[serde(rename = "ordType")]
    order_type: String,
    state: String,
}

impl TryFrom<OkxAlgoOrderWire> for OkxAlgoOrder {
    type Error = RestError;

    fn try_from(value: OkxAlgoOrderWire) -> Result<Self, Self::Error> {
        validate_required_text("algoId", &value.algo_id)?;
        validate_required_text("instId", &value.symbol)?;
        validate_optional_text("algoClOrdId", value.client_order_id.clone())?;
        if !matches!(value.state.as_str(), "live" | "pause") {
            return Err(RestError::InvalidField {
                field: "state",
                value: value.state,
                message: "pending algo state must be live or pause".to_string(),
            });
        }
        Ok(Self {
            algo_id: value.algo_id,
            client_order_id: value.client_order_id,
            symbol: value.symbol,
            order_type: value.order_type.as_str().try_into()?,
            state: value.state,
        })
    }
}

#[derive(Debug, Deserialize)]
struct OkxSpreadOrderWire {
    #[serde(rename = "sprdId")]
    spread_id: String,
    #[serde(rename = "ordId")]
    exchange_order_id: String,
    #[serde(default, rename = "clOrdId")]
    client_order_id: String,
    state: String,
}

impl TryFrom<OkxSpreadOrderWire> for OkxSpreadOrder {
    type Error = RestError;

    fn try_from(value: OkxSpreadOrderWire) -> Result<Self, Self::Error> {
        validate_required_text("sprdId", &value.spread_id)?;
        validate_required_text("ordId", &value.exchange_order_id)?;
        validate_optional_text("clOrdId", value.client_order_id.clone())?;
        if !matches!(value.state.as_str(), "live" | "partially_filled") {
            return Err(RestError::InvalidField {
                field: "state",
                value: value.state,
                message: "pending spread state must be live or partially_filled".to_string(),
            });
        }
        Ok(Self {
            spread_id: value.spread_id,
            exchange_order_id: value.exchange_order_id,
            client_order_id: value.client_order_id,
            state: value.state,
        })
    }
}

#[derive(Debug, Deserialize)]
struct OkxAlgoCancelWire {
    #[serde(rename = "algoId")]
    algo_id: String,
    #[serde(default, rename = "algoClOrdId")]
    client_order_id: String,
    #[serde(default, rename = "sCode")]
    code: String,
    #[serde(default, rename = "sMsg")]
    message: String,
}

impl TryFrom<OkxAlgoCancelWire> for OkxAlgoCancelResult {
    type Error = RestError;

    fn try_from(value: OkxAlgoCancelWire) -> Result<Self, Self::Error> {
        validate_required_text("algoId", &value.algo_id)?;
        validate_optional_text("algoClOrdId", value.client_order_id.clone())?;
        Ok(Self {
            algo_id: value.algo_id,
            client_order_id: value.client_order_id,
            code: value.code,
            message: value.message,
        })
    }
}

#[derive(Debug, Deserialize)]
struct OkxSpreadMassCancelWire {
    result: bool,
}

#[derive(Debug, Deserialize)]
struct OkxSpreadCancelAllAfterWire {
    #[serde(rename = "triggerTime")]
    trigger_time: String,
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;
    use crate::okx::{HttpResponse, OkxCredentials, OkxSigner, SignedRequest};

    #[derive(Clone)]
    struct MockTransport {
        responses: Arc<Mutex<Vec<String>>>,
        requests: Arc<Mutex<Vec<SignedRequest>>>,
    }

    #[async_trait]
    impl HttpTransport for MockTransport {
        async fn execute(&self, request: SignedRequest) -> Result<HttpResponse, RestError> {
            self.requests.lock().unwrap().push(request);
            Ok(HttpResponse {
                status: 200,
                body: self.responses.lock().unwrap().remove(0),
            })
        }
    }

    fn client(
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

    fn regular_page(first: usize, count: usize) -> String {
        let data = (first..first + count)
            .map(|index| {
                serde_json::json!({
                    "ordId": format!("{index}"),
                    "clOrdId": format!("client-{index}"),
                    "instId": "BTC-USDT",
                    "side": "buy",
                    "state": "live",
                    "px": "100",
                    "sz": "1",
                    "accFillSz": "0",
                    "avgPx": "",
                    "uTime": "1000"
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({"code": "0", "msg": "", "data": data}).to_string()
    }

    #[tokio::test]
    async fn regular_pages_use_after_and_fail_closed_at_the_bound() {
        let (client, requests) = client(vec![regular_page(100, 100)]);
        let mut pagination = OkxRegularOrderPagination::new(1).unwrap();
        let page = client
            .regular_pending_orders_page_at("time", None, None, pagination.after())
            .await
            .unwrap();
        let error = pagination.accept(page).unwrap_err();
        assert!(matches!(
            error,
            RestError::PendingOrderPaginationLimit {
                domain: REGULAR_DOMAIN,
                pages: 1,
                records: 100,
                ..
            }
        ));
        assert_eq!(
            requests.lock().unwrap()[0].path,
            "/api/v5/trade/orders-pending?limit=100"
        );
    }

    #[tokio::test]
    async fn parses_all_algo_queries_and_builds_bounded_cancel() {
        let responses = OkxAlgoOrderQuery::ALL
            .into_iter()
            .map(|query| {
                let order_type = if query == OkxAlgoOrderQuery::ConditionalAndOco {
                    "conditional"
                } else {
                    query.as_str()
                };
                serde_json::json!({
                    "code": "0",
                    "msg": "",
                    "data": [{
                        "algoId": format!("algo-{order_type}"),
                        "algoClOrdId": "client",
                        "instId": "BTC-USDT",
                        "ordType": order_type,
                        "state": "live"
                    }]
                })
                .to_string()
            })
            .chain(std::iter::once(
                r#"{"code":"0","msg":"","data":[{"algoId":"algo-trigger","algoClOrdId":"client","sCode":"0","sMsg":""}]}"#.to_string(),
            ))
            .collect();
        let (client, requests) = client(responses);
        for query in OkxAlgoOrderQuery::ALL {
            let page = client
                .algo_pending_orders_page_at("time", query, None)
                .await
                .unwrap();
            assert_eq!(page.orders.len(), 1);
        }
        let results = client
            .cancel_algo_orders_at(
                "time",
                &[OkxCancelAlgoOrder {
                    symbol: "BTC-USDT".to_string(),
                    algo_id: "algo-trigger".to_string(),
                }],
            )
            .await
            .unwrap();
        assert!(results[0].accepted());
        let requests = requests.lock().unwrap();
        assert_eq!(
            requests[0].path,
            "/api/v5/trade/orders-algo-pending?ordType=conditional%2Coco&limit=100"
        );
        assert_eq!(requests.last().unwrap().path, CANCEL_ALGO_ORDERS_PATH);
        assert_eq!(
            requests.last().unwrap().body,
            r#"[{"instId":"BTC-USDT","algoId":"algo-trigger"}]"#
        );
    }

    #[tokio::test]
    async fn spread_contract_uses_end_id_mass_cancel_and_its_own_deadman() {
        let (client, requests) = client(vec![
            r#"{"code":"0","msg":"","data":[{"sprdId":"BTC-USDT_BTC-USDT-SWAP","ordId":"123","clOrdId":"client","state":"partially_filled"}]}"#.to_string(),
            r#"{"code":"0","msg":"","data":[{"result":true}]}"#.to_string(),
            r#"{"code":"0","msg":"","data":[{"triggerTime":"1000","ts":"1"}]}"#.to_string(),
        ]);
        let page = client
            .spread_pending_orders_page_at("time", Some("456"))
            .await
            .unwrap();
        assert_eq!(page.orders[0].exchange_order_id, "123");
        client.spread_mass_cancel_at("time").await.unwrap();
        client.spread_cancel_all_after_at("time", 10).await.unwrap();
        let requests = requests.lock().unwrap();
        assert_eq!(
            requests[0].path,
            "/api/v5/sprd/orders-pending?endId=456&limit=100"
        );
        assert_eq!(requests[1].path, SPREAD_MASS_CANCEL_PATH);
        assert_eq!(requests[1].body, "{}");
        assert_eq!(requests[2].path, SPREAD_CANCEL_ALL_AFTER_PATH);
    }

    #[test]
    fn pagination_rejects_duplicate_orders_and_repeated_cursors() {
        let mut duplicate = OkxAlgoOrderPagination::new(3).unwrap();
        let order = OkxAlgoOrder {
            algo_id: "1".to_string(),
            client_order_id: String::new(),
            symbol: "BTC-USDT".to_string(),
            order_type: OkxAlgoOrderType::Trigger,
            state: "live".to_string(),
        };
        duplicate
            .accept(OkxAlgoOrderPage {
                orders: vec![order.clone()],
                next_after: Some("1".to_string()),
            })
            .unwrap();
        assert!(matches!(
            duplicate.accept(OkxAlgoOrderPage {
                orders: vec![order],
                next_after: None,
            }),
            Err(RestError::PendingOrderPaginationDuplicate {
                domain: ALGO_DOMAIN,
                ..
            })
        ));

        let mut repeated_cursor = OkxSpreadOrderPagination::new(3).unwrap();
        repeated_cursor
            .accept(OkxSpreadOrderPage {
                orders: Vec::new(),
                next_end_id: Some("1".to_string()),
            })
            .unwrap();
        assert!(matches!(
            repeated_cursor.accept(OkxSpreadOrderPage {
                orders: Vec::new(),
                next_end_id: Some("1".to_string()),
            }),
            Err(RestError::PendingOrderPaginationCursor {
                domain: SPREAD_DOMAIN,
                ..
            })
        ));
    }
}
