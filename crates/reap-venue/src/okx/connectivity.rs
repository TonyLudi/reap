/// OKX documents at most three WebSocket connection requests per second per IP.
pub const OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS: u64 = 334;
pub const DEFAULT_OKX_CONNECTION_ATTEMPT_PACER_PATH: &str = "var/reap/okx-connection-attempt.pacer";

/// Groups regular order symbols by the OKX base/quote dispatch family.
pub fn okx_order_dispatch_key(symbol: &str) -> String {
    let mut components = symbol.split('-');
    let Some(base) = components.next().filter(|component| !component.is_empty()) else {
        return symbol.to_string();
    };
    let Some(quote) = components.next().filter(|component| !component.is_empty()) else {
        return symbol.to_string();
    };
    format!("{base}-{quote}")
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
