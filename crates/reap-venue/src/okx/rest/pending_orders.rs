use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::RemoteOrder;

use super::{
    OkxOrderWire, OkxResponse, RestError, decode_okx_response, validate_optional_text,
    validate_required_text,
};

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
#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn regular_parser_derives_cursor_and_pagination_fails_closed_at_bound() {
        let page =
            parse_okx_regular_order_page_response_json(regular_page(100, 100).as_bytes()).unwrap();
        assert_eq!(page.orders.len(), 100);
        assert_eq!(page.next_after.as_deref(), Some("199"));

        let mut pagination = OkxRegularOrderPagination::new(1).unwrap();
        assert!(matches!(
            pagination.accept(page),
            Err(RestError::PendingOrderPaginationLimit {
                domain: REGULAR_DOMAIN,
                pages: 1,
                records: 100,
                ..
            })
        ));
    }

    #[test]
    fn algo_parser_supports_every_query_family() {
        for query in OkxAlgoOrderQuery::ALL {
            let order_type = if query == OkxAlgoOrderQuery::ConditionalAndOco {
                "conditional"
            } else {
                query.as_str()
            };
            let body = serde_json::json!({
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
            .to_string();
            let page = parse_okx_algo_order_page_response_json(body.as_bytes()).unwrap();
            assert_eq!(page.orders.len(), 1);
            assert_eq!(page.orders[0].symbol, "BTC-USDT");
        }
    }

    #[test]
    fn spread_parser_retains_identity_and_rejects_terminal_state() {
        let page = parse_okx_spread_order_page_response_json(
            br#"{"code":"0","msg":"","data":[{"sprdId":"BTC-USDT_BTC-USDT-SWAP","ordId":"123","clOrdId":"client","state":"partially_filled"}]}"#,
        )
        .unwrap();
        assert_eq!(page.orders[0].exchange_order_id, "123");

        assert!(matches!(
            parse_okx_spread_order_page_response_json(
                br#"{"code":"0","msg":"","data":[{"sprdId":"BTC-USDT_BTC-USDT-SWAP","ordId":"123","clOrdId":"client","state":"canceled"}]}"#
            ),
            Err(RestError::InvalidField { field: "state", .. })
        ));
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

    #[test]
    fn cancellation_result_contracts_preserve_per_item_acceptance() {
        assert!(
            OkxAlgoCancelResult {
                algo_id: "1".to_string(),
                client_order_id: String::new(),
                code: "0".to_string(),
                message: String::new(),
            }
            .accepted()
        );
        assert!(
            !OkxAlgoCancelResult {
                algo_id: "2".to_string(),
                client_order_id: String::new(),
                code: "51400".to_string(),
                message: "already canceled".to_string(),
            }
            .accepted()
        );
    }
}
