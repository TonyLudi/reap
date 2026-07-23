use reap_pm_live::{PmLaneService, PmLaneSet};

struct NoopService;

impl PmLaneService for NoopService {}

fn main() {
    let mut lanes = PmLaneSet::new();
    let _ = lanes.service_turn(0, &mut NoopService);
}
