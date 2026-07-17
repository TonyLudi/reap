use async_trait::async_trait;
use reap_venue::okx::OkxOrderAck;
pub use reap_venue::okx::okx_order_dispatch_key;
use thiserror::Error;

use crate::{PreparedRegularCancel, PreparedRegularSubmit};

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OrderTransportError {
    #[error("order transport was unavailable before send: {0}")]
    Unavailable(String),
    #[error("order transport outcome is ambiguous after send: {0}")]
    Ambiguous(String),
    #[error("OKX order operation was rejected with code {code}: {message}")]
    Rejected { code: String, message: String },
    #[error("order transport request is invalid: {0}")]
    InvalidRequest(String),
}

impl OrderTransportError {
    pub fn is_ambiguous(&self) -> bool {
        matches!(self, Self::Ambiguous(_))
    }

    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

#[derive(Debug)]
pub struct CancelOrderTransportError {
    error: OrderTransportError,
    prepared: Option<PreparedRegularCancel>,
}

impl CancelOrderTransportError {
    /// Reports a failure proven to have happened before any transport write.
    ///
    /// The capability remains opaque to callers and can only be recovered by
    /// the order gateway for its immediate REST fallback.
    pub fn pre_send_unavailable(
        message: impl Into<String>,
        prepared: PreparedRegularCancel,
    ) -> Self {
        Self {
            error: OrderTransportError::Unavailable(message.into()),
            prepared: Some(prepared),
        }
    }

    pub fn failed(error: OrderTransportError) -> Self {
        let error = match error {
            OrderTransportError::Unavailable(message) => OrderTransportError::Ambiguous(format!(
                "transport reported unavailable without a recoverable pre-send capability: {message}"
            )),
            error => error,
        };
        Self {
            error,
            prepared: None,
        }
    }

    pub(crate) fn into_parts(self) -> (OrderTransportError, Option<PreparedRegularCancel>) {
        (self.error, self.prepared)
    }
}

/// Low-latency command transport only. REST snapshots and reconciliation stay
/// on the gateway's independent authenticated client.
#[async_trait]
pub trait OkxOrderTransport: Send + Sync {
    async fn place_order(
        &self,
        order: PreparedRegularSubmit,
    ) -> Result<OkxOrderAck, OrderTransportError>;

    async fn cancel_order(
        &self,
        order: PreparedRegularCancel,
    ) -> Result<OkxOrderAck, CancelOrderTransportError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn okx_dispatch_key_groups_spot_swap_and_dated_futures() {
        assert_eq!(okx_order_dispatch_key("BTC-USDT"), "BTC-USDT");
        assert_eq!(okx_order_dispatch_key("BTC-USDT-SWAP"), "BTC-USDT");
        assert_eq!(okx_order_dispatch_key("BTC-USDT-260925"), "BTC-USDT");
        assert_eq!(okx_order_dispatch_key("ETH-USDT-SWAP"), "ETH-USDT");
    }
}
