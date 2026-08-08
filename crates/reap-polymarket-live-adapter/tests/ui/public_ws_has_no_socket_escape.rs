use reap_polymarket_live_adapter::{PmPublicMarketWsRole, PmPublicWsConfig};

fn escape(role: &PmPublicMarketWsRole, config: &PmPublicWsConfig) {
    let _socket = role.socket();
    let _endpoint = config.endpoint();
}

fn main() {}
