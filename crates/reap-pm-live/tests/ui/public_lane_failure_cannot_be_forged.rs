use reap_pm_live::{PmPublicBookDelivery, PmPublicLaneEnqueueError};

fn value<T>() -> T {
    panic!("compile-fail fixture")
}

fn forge(delivery: PmPublicBookDelivery) -> PmPublicLaneEnqueueError<PmPublicBookDelivery> {
    let _ = delivery;
    PmPublicLaneEnqueueError { failure: value() }
}

fn main() {}
