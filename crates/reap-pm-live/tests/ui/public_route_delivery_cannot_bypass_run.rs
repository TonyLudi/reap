use reap_pm_live::{PmLaneSet, PmPublicBookDelivery};

fn bypass_run(lanes: &mut PmLaneSet, delivery: PmPublicBookDelivery) {
    lanes.enqueue_pm_book(delivery).unwrap();
}

fn main() {}
