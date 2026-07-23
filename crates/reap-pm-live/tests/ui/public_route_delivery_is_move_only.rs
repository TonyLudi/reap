use reap_pm_live::PmPublicBookDelivery;

fn replay(delivery: PmPublicBookDelivery) {
    let _first = delivery.clone();
    let _second = delivery;
}

fn main() {}
