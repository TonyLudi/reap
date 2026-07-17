use async_trait::async_trait;
pub use reap_venue::okx::okx_order_dispatch_key;
use reap_venue::okx::{OkxCancelOrder, OkxOrderAck, OkxPlaceOrder};
use thiserror::Error;

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

/// Low-latency command transport only. REST snapshots and reconciliation stay
/// on the gateway's independent authenticated client.
#[async_trait]
pub trait OkxOrderTransport: Send + Sync {
    async fn place_order(&self, order: &OkxPlaceOrder) -> Result<OkxOrderAck, OrderTransportError>;

    async fn cancel_order(
        &self,
        order: &OkxCancelOrder,
    ) -> Result<OkxOrderAck, OrderTransportError>;
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
