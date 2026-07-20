use reap_pm_core::{PmOrderEvent, ReceivedEventClock};
use reap_pm_live::{PmIngressOrder, PmLaneKind, PmLaneSet};

fn forge_public_order(
    lanes: &mut PmLaneSet,
    clock: ReceivedEventClock,
    ingress: PmIngressOrder,
    order: PmOrderEvent,
) {
    lanes.enqueue_observation(PmLaneKind::Public, clock, ingress, order);
}

fn main() {}
