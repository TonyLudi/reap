use reap_polymarket_wire::PmUnsignedClobV2Order;

fn main() {
    let _: PmUnsignedClobV2Order = serde_json::from_str("{}").unwrap();
}
