use reap_pm_live::PmAgedDeliveryEvidence;

fn value<T>() -> T {
    panic!("compile-fail fixture")
}

fn forge() -> PmAgedDeliveryEvidence {
    PmAgedDeliveryEvidence {
        key: value(),
        connection: value(),
        ordering: value(),
        received_clock: value(),
        observed_now_ns: 1,
        lane_authority: value(),
        lane_generation: 1,
        public_route: None,
    }
}

fn main() {}
