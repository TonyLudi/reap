use reap_core::Venue;
use reap_pm_core::{
    EventClock, EventEnvelope, EventOrdering, PmConnectionId, PmProductSource,
};

fn validated_envelope_fields_cannot_be_forged<P>(
    venue: Venue,
    source: PmProductSource,
    connection_id: PmConnectionId,
    clock: EventClock,
    ordering: EventOrdering,
    payload: P,
) {
    let _ = EventEnvelope {
        venue,
        source,
        connection_id,
        clock,
        ordering,
        payload,
    };
}

fn main() {}
