use reap_polymarket_live_adapter::PmLiveMetadataPair;
use reap_polymarket_wire::PmWireScope;

fn forge(scope: PmWireScope) {
    let _ = PmLiveMetadataPair {
        scope,
        market_bytes: b"{}",
        clob_v2_bytes: b"{}",
    };
}

fn main() {}
