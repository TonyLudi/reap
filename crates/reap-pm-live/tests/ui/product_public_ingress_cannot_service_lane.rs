use reap_pm_live::{PmProductRun, PmPublicLaneService};
use reap_pm_strategy::PmQuoteModel;

fn bypass<M: PmQuoteModel, C: PmPublicLaneService>(run: &mut PmProductRun<M>, consumer: &mut C) {
    let mut ingress = run.public_ingress();
    let _ = ingress.service_lane_turn(1, consumer);
}

fn main() {}
