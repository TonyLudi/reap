use std::time::Duration;

use reap_polymarket_live_adapter::PmLoopbackMutationConfig;

fn main() {
    let _ = PmLoopbackMutationConfig::loopback_evidence(
        "http://127.0.0.1:18080",
        Duration::from_secs(1),
        Duration::from_secs(1),
    );
}
