use std::time::Duration;

use reap_feed::{ConnectionAttemptPacer, ReconnectPolicy};
use reap_okx_live_adapter::{
    OrderCommandWebsocketConfig, OrderCommandWebsocketTransport,
};

fn recreate_internal_pool_config() {
    let _ = OrderCommandWebsocketConfig {
        account_id: "main".to_string(),
        websocket_url: "wss://wspap.okx.com:8443/ws/v5/private".to_string(),
        session_count: 8,
        command_capacity: 8,
        request_expiry: Duration::from_secs(1),
        acknowledgement_timeout: Duration::from_secs(1),
        connection_attempt_pacer: ConnectionAttemptPacer::new(Duration::from_secs(1)),
        reconnect: ReconnectPolicy::default(),
    };
}

fn expose_transport(_transport: OrderCommandWebsocketTransport) {}

fn main() {}
