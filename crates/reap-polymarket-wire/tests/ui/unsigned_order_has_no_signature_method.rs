use reap_polymarket_wire::PmUnsignedClobV2Order;

fn main() {
    let order: PmUnsignedClobV2Order = unreachable!();
    let _ = order.signature();
}
