use reap_pm_core::{PmOrderEvent, ReceivedEventClock};
use reap_pm_live::{PmIngressOrder, PmLaneSet, PmServiceKey};

fn replay_key(
    lanes: &mut PmLaneSet,
    event: PmOrderEvent,
    key: PmServiceKey,
    ingress: PmIngressOrder,
    clock: ReceivedEventClock,
) {
    lanes.enqueue_observation(key, clock, ingress, event);
}

fn main() {}
