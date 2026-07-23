use reap_pm_live::PmLaneSet;
use reap_polymarket_adapter::PmPublicBookDelivery;

fn admit(lanes: &mut PmLaneSet, delivery: PmPublicBookDelivery) {
    lanes.enqueue_pm_book(delivery).unwrap();
}

fn main() {}
