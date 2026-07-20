use reap_pm_core::{PmOrderEvent, ReceivedEventClock};
use reap_pm_live::{PmIngressOrder, PmLaneSet, PmObservedEvent, PmServiceKey};

fn replay_key(
    lanes: &mut PmLaneSet,
    event: PmOrderEvent,
    key: PmServiceKey,
    ingress: PmIngressOrder,
    clock: ReceivedEventClock,
) {
    PmObservedEvent::enqueue_into(event, lanes, key, ingress, clock);
}

fn main() {}
