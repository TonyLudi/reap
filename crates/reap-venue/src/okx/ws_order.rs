use serde::Deserialize;
use thiserror::Error;

use super::OkxOrderAck;
use super::capabilities::{WS_CANCEL_REGULAR, WS_PLACE_REGULAR};

pub const OKX_WS_PLACE_ORDER_OP: &str = WS_PLACE_REGULAR.endpoint_or_channel;
pub const OKX_WS_CANCEL_ORDER_OP: &str = WS_CANCEL_REGULAR.endpoint_or_channel;
const MAX_REQUEST_ID_BYTES: usize = 32;

#[derive(Debug, Error)]
pub enum OkxWsOrderProtocolError {
    #[error("OKX websocket order serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("invalid OKX websocket order request id {0:?}")]
    InvalidRequestId(String),
    #[error("invalid OKX websocket order response: {0}")]
    InvalidResponse(String),
    #[error("invalid OKX order argument: {0}")]
    InvalidOrder(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OkxWsOrderOperation {
    Place,
    Cancel,
}

impl OkxWsOrderOperation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Place => OKX_WS_PLACE_ORDER_OP,
            Self::Cancel => OKX_WS_CANCEL_ORDER_OP,
        }
    }

    fn parse(value: &str) -> Result<Self, OkxWsOrderProtocolError> {
        match value {
            OKX_WS_PLACE_ORDER_OP => Ok(Self::Place),
            OKX_WS_CANCEL_ORDER_OP => Ok(Self::Cancel),
            _ => Err(OkxWsOrderProtocolError::InvalidResponse(format!(
                "unsupported op {value:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OkxWsOrderResult {
    Accepted {
        request_id: String,
        operation: OkxWsOrderOperation,
        acknowledgement: OkxOrderAck,
        in_time_us: Option<u64>,
        out_time_us: Option<u64>,
    },
    Rejected {
        request_id: String,
        operation: OkxWsOrderOperation,
        code: String,
        message: String,
    },
}

impl OkxWsOrderResult {
    pub fn request_id(&self) -> &str {
        match self {
            Self::Accepted { request_id, .. } | Self::Rejected { request_id, .. } => request_id,
        }
    }

    pub fn operation(&self) -> OkxWsOrderOperation {
        match self {
            Self::Accepted { operation, .. } | Self::Rejected { operation, .. } => *operation,
        }
    }
}

pub fn parse_okx_ws_order_response(
    payload: &str,
) -> Result<OkxWsOrderResult, OkxWsOrderProtocolError> {
    #[derive(Deserialize)]
    struct Response {
        id: String,
        op: String,
        code: String,
        #[serde(rename = "msg")]
        message: String,
        #[serde(default)]
        data: Vec<Acknowledgement>,
        #[serde(rename = "inTime")]
        in_time: Option<String>,
        #[serde(rename = "outTime")]
        out_time: Option<String>,
    }

    #[derive(Deserialize)]
    struct Acknowledgement {
        #[serde(rename = "ordId", default)]
        exchange_order_id: String,
        #[serde(rename = "clOrdId", default)]
        client_order_id: String,
        #[serde(rename = "sCode", default)]
        code: String,
        #[serde(rename = "sMsg", default)]
        message: String,
    }

    let response: Response = serde_json::from_str(payload)?;
    validate_request_id(&response.id)
        .map_err(|error| OkxWsOrderProtocolError::InvalidResponse(error.to_string()))?;
    let operation = OkxWsOrderOperation::parse(&response.op)?;
    if response.code != "0" {
        return Ok(OkxWsOrderResult::Rejected {
            request_id: response.id,
            operation,
            code: response.code,
            message: response.message,
        });
    }
    if response.data.len() != 1 {
        return Err(OkxWsOrderProtocolError::InvalidResponse(format!(
            "successful {} response contains {} data rows",
            operation.as_str(),
            response.data.len()
        )));
    }
    let acknowledgement = response
        .data
        .into_iter()
        .next()
        .expect("validated one acknowledgement row");
    if acknowledgement.code.is_empty() {
        return Err(OkxWsOrderProtocolError::InvalidResponse(format!(
            "successful {} response is missing sCode",
            operation.as_str()
        )));
    }
    if acknowledgement.code != "0" {
        return Ok(OkxWsOrderResult::Rejected {
            request_id: response.id,
            operation,
            code: acknowledgement.code,
            message: acknowledgement.message,
        });
    }
    if acknowledgement.exchange_order_id.is_empty() {
        return Err(OkxWsOrderProtocolError::InvalidResponse(format!(
            "successful {} response is missing ordId",
            operation.as_str()
        )));
    }
    Ok(OkxWsOrderResult::Accepted {
        request_id: response.id,
        operation,
        acknowledgement: OkxOrderAck {
            exchange_order_id: acknowledgement.exchange_order_id,
            client_order_id: acknowledgement.client_order_id,
        },
        in_time_us: optional_u64("inTime", response.in_time)?,
        out_time_us: optional_u64("outTime", response.out_time)?,
    })
}

fn validate_request_id(request_id: &str) -> Result<(), OkxWsOrderProtocolError> {
    if request_id.is_empty()
        || request_id.len() > MAX_REQUEST_ID_BYTES
        || !request_id.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(OkxWsOrderProtocolError::InvalidRequestId(
            request_id.to_string(),
        ));
    }
    Ok(())
}

fn optional_u64(
    field: &'static str,
    value: Option<String>,
) -> Result<Option<u64>, OkxWsOrderProtocolError> {
    value
        .map(|value| {
            value.parse::<u64>().map_err(|error| {
                OkxWsOrderProtocolError::InvalidResponse(format!(
                    "{field}={value:?} is not an unsigned integer: {error}"
                ))
            })
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_ack_and_exchange_rejection_are_distinct() {
        let accepted = parse_okx_ws_order_response(
            r#"{"id":"a1","op":"order","code":"0","msg":"","data":[{"ordId":"42","clOrdId":"reap123","sCode":"0","sMsg":""}],"inTime":"10","outTime":"11"}"#,
        )
        .unwrap();
        assert!(matches!(
            accepted,
            OkxWsOrderResult::Accepted {
                acknowledgement: OkxOrderAck { ref exchange_order_id, .. },
                ..
            } if exchange_order_id == "42"
        ));

        let rejected = parse_okx_ws_order_response(
            r#"{"id":"a1","op":"order","code":"0","msg":"","data":[{"ordId":"","clOrdId":"reap123","sCode":"51000","sMsg":"bad parameter"}]}"#,
        )
        .unwrap();
        assert!(matches!(
            rejected,
            OkxWsOrderResult::Rejected { ref code, .. } if code == "51000"
        ));
    }

    #[test]
    fn websocket_success_requires_row_status_and_exchange_order_id() {
        for payload in [
            r#"{"id":"a1","op":"order","code":"0","msg":"","data":[{"ordId":"42","clOrdId":"reap123"}]}"#,
            r#"{"id":"a1","op":"order","code":"0","msg":"","data":[{"ordId":"","clOrdId":"reap123","sCode":"0","sMsg":""}]}"#,
        ] {
            assert!(matches!(
                parse_okx_ws_order_response(payload),
                Err(OkxWsOrderProtocolError::InvalidResponse(_))
            ));
        }
    }
}
