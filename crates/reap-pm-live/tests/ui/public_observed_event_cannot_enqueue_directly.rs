use reap_pm_core::ReceivedEventClock;
use reap_pm_live::{PmIngressOrder, PmLaneSet, PmObservedEvent};

fn bypass<E: PmObservedEvent>(
    event: E,
    lanes: &mut PmLaneSet,
    ingress: PmIngressOrder,
    clock: ReceivedEventClock,
) {
    <E as PmObservedEvent>::enqueue_into(event, lanes, ingress, clock).unwrap();
}

fn main() {}
