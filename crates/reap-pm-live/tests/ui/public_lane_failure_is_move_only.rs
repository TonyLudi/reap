use reap_pm_live::{PmPublicBookDelivery, PmPublicLaneEnqueueError};

fn replay(failure: PmPublicLaneEnqueueError<PmPublicBookDelivery>) {
    let _first = failure;
    let _second = failure;
}

fn main() {}
