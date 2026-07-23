use reap_pm_core::{
    EventClock, EventOrdering, OkxReferenceEvent, PmBookEvent, PmConnectionId, PmMarketEvent,
};

use super::{
    BoundedHeap, LaneItem, PmAgedDeliveryEvidence, PmLaneKind, PmLanePolicy, PmPublicInput,
    PmPublicLaneState, PmPublicUnavailable, PmServiceKey, PmServiceTurnError, ServicedLaneItem,
};
use crate::public_routes::OkxPublicUnavailable;

/// Reached Phase-3 public callbacks; no trait object or erased event payload.
///
/// Every handler is mandatory. Invocation transfers one exact serviced
/// occurrence, and normal synchronous return commits its consumption from the
/// bounded Run-owned queue. Implementors must apply a total deterministic
/// transition during the call rather than forwarding the occurrence to an
/// unproved or unbounded side channel. Phase 3 authenticates and orders that
/// transfer; semantic strategy-state proof remains the later sealed
/// coordinator's responsibility.
pub trait PmPublicLaneService {
    fn on_pm_public_unavailable(&mut self, item: ServicedLaneItem<PmPublicUnavailable>);
    fn on_okx_public_unavailable(&mut self, item: ServicedLaneItem<OkxPublicUnavailable>);
    fn on_market(&mut self, item: ServicedLaneItem<PmMarketEvent>);
    fn on_book(&mut self, item: ServicedLaneItem<PmBookEvent>);
    fn on_reference(&mut self, item: ServicedLaneItem<OkxReferenceEvent>);
}

impl PmPublicLaneState {
    pub(crate) fn service_turn<C: PmPublicLaneService>(
        &mut self,
        now_ns: u64,
        consumer: &mut C,
    ) -> Result<usize, PmServiceTurnError> {
        if self.consumer_transfer_poisoned() {
            return Err(PmServiceTurnError::ConsumerTransferPoisoned);
        }
        service_public_lane(
            self,
            now_ns,
            PmLanePolicy::for_lane(PmLaneKind::Public)
                .service_burst()
                .expect("public lane has a frozen burst"),
            consumer,
        )
    }
}

fn check_public_age(public: &PmPublicLaneState, now_ns: u64) -> Result<(), PmServiceTurnError> {
    let Some(entry) = public.queue.peek() else {
        return Ok(());
    };
    if matches!(
        entry.value.public_route().head,
        super::PmPublicAgedHead::PmUnavailable(_) | super::PmPublicAgedHead::OkxUnavailable(_)
    ) {
        return Ok(());
    }
    let age = entry
        .value
        .queue_age_ns(now_ns)
        .map_err(PmServiceTurnError::DeliveryClock)?;
    if public
        .queue
        .policy()
        .maximum_age_ns()
        .is_some_and(|maximum| age > maximum)
    {
        return Err(PmServiceTurnError::aged(
            PmLaneKind::Public,
            public.queue.policy().saturation_action(),
            PmAgedDeliveryEvidence::new(
                entry.value.key(),
                entry.value.connection(),
                entry.value.ordering(),
                entry.value.received_clock(),
                now_ns,
                public.authority_id,
                public.queue.generation(),
                entry.value.public_route(),
            ),
        ));
    }
    Ok(())
}

fn public_head_is_unavailable(public: &PmPublicLaneState) -> bool {
    public.queue.peek().is_some_and(|entry| {
        matches!(
            entry.value.public_route().head,
            super::PmPublicAgedHead::PmUnavailable(_) | super::PmPublicAgedHead::OkxUnavailable(_)
        )
    })
}

fn into_serviced<T>(item: LaneItem<T>, clock: EventClock) -> ServicedLaneItem<T> {
    let key = item.key();
    let connection = item.connection();
    let ordering = item.ordering();
    ServicedLaneItem {
        lane: PmLaneKind::Public,
        key,
        connection,
        ordering,
        clock,
        value: item.into_value(),
    }
}

fn pop_received<T>(
    queue: &mut BoundedHeap<PmServiceKey, LaneItem<T>>,
    now_ns: u64,
) -> Result<Option<(LaneItem<T>, EventClock)>, PmServiceTurnError> {
    let Some(next) = queue.peek() else {
        return Ok(None);
    };
    let clock = next
        .value
        .received_clock()
        .service_at(now_ns)
        .map_err(PmServiceTurnError::EventClock)?;
    Ok(queue.pop().map(|entry| (entry.value, clock)))
}

fn map_serviced<U>(
    key: PmServiceKey,
    connection: PmConnectionId,
    ordering: EventOrdering,
    clock: EventClock,
    value: U,
) -> ServicedLaneItem<U> {
    ServicedLaneItem {
        lane: PmLaneKind::Public,
        key,
        connection,
        ordering,
        clock,
        value,
    }
}

fn service_public_lane<C: PmPublicLaneService>(
    public: &mut PmPublicLaneState,
    now_ns: u64,
    limit: usize,
    consumer: &mut C,
) -> Result<usize, PmServiceTurnError> {
    let count = if public_head_is_unavailable(public) {
        usize::from(public.queue.len() != 0)
    } else {
        limit.min(public.queue.len())
    };
    let mut serviced = 0;
    for _ in 0..count {
        if let Err(source) = check_public_age(public, now_ns) {
            if serviced != 0 {
                return Ok(serviced);
            }
            return Err(source);
        }
        let (input, clock) = match pop_received(&mut public.queue, now_ns) {
            Ok(Some(input)) => input,
            Ok(None) => break,
            Err(_) if serviced != 0 => return Ok(serviced),
            Err(source) => return Err(source),
        };
        let item = into_serviced(input, clock);
        let ServicedLaneItem {
            key,
            connection,
            ordering,
            clock,
            value,
            ..
        } = item;
        public.consumer_transfer_in_flight = true;
        match value {
            PmPublicInput::PmUnavailable(value) => consumer
                .on_pm_public_unavailable(map_serviced(key, connection, ordering, clock, value)),
            PmPublicInput::OkxUnavailable(value) => consumer
                .on_okx_public_unavailable(map_serviced(key, connection, ordering, clock, value)),
            PmPublicInput::Market(value) => {
                consumer.on_market(map_serviced(key, connection, ordering, clock, value));
            }
            PmPublicInput::Book(value) => {
                consumer.on_book(map_serviced(key, connection, ordering, clock, value));
            }
            PmPublicInput::Reference(value) => {
                consumer.on_reference(map_serviced(key, connection, ordering, clock, value));
            }
        }
        public.consumer_transfer_in_flight = false;
        serviced += 1;
    }
    Ok(serviced)
}
