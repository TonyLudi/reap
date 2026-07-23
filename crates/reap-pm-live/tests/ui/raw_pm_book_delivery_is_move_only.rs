use reap_polymarket_adapter::PmPublicBookDelivery;

fn replay(delivery: PmPublicBookDelivery) {
    let _first = delivery.clone();
    let _second = delivery;
}

fn main() {}
