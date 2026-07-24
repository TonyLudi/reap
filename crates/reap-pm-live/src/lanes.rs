#![allow(
    clippy::result_large_err,
    reason = "fail-closed lane errors retain exact move-only deliveries and full ordering evidence without overload-path allocation"
)]

use reap_pm_core::{
    ConnectionEpoch, EventClock, EventOrdering, IngressSequence, OkxReferenceEvent, PmBookEvent,
    PmBookUpdate, PmConnectionId, PmMarketEvent, PmSourceBound, PmSourceHandle, ReceivedEventClock,
    ReceivedEventEnvelope,
};
use reap_transport::{DeliveryClockError, ImmutableDelivery};

use crate::coordinator::PmBookDecisionProjection;
use crate::public_routes::{
    OkxPublicReferenceDelivery, OkxPublicUnavailable, OkxPublicUnavailableDelivery,
    PmPublicBookDelivery, PmPublicMetadataDelivery, PmPublicRouteAuthorityId, PmPublicUnavailable,
    PmPublicUnavailableDelivery,
};

mod bounded;
mod complete;
mod complete_queue;
mod complete_types;
mod failure;
mod policy;
mod public;
mod service;

use bounded::{Admission, BoundedHeap};
pub use complete::{
    PmCompleteFailClosedMetrics, PmCompleteSchedulerMetrics, PmCompleteServiceCounts,
};
pub(crate) use complete::{PmCompleteInputLanes, PmCompleteLaneService, PmCompleteServiceError};
pub use complete_queue::PmCompleteLaneMetrics;
pub(crate) use complete_queue::{
    PmCompleteLane, PmCompleteLaneAgeFault, PmCompleteLaneBuildError, PmCompleteLaneCheckError,
    PmCompleteLaneEnqueueError,
};
pub(crate) use complete_types::{
    PmCompleteIngress, PmCompleteInputSource, PmCompleteLaneItem, PmCompleteServiced,
    PmCriticalInput, PmPersistenceCarrierError, PmPersistenceInput, PmPrivateInput,
    PmReconciliationInput, PmScopedHalt, PmStopControl, PmTelemetryInput,
};
pub use complete_types::{
    PmCompleteServiceKey, PmCompleteSourceKind, PmPairedReconciliationCut,
    PmPairedReconciliationCutError, PmTelemetryKind,
};
pub use failure::{PmAgedDeliveryEvidence, PmPublicLaneEnqueueError, PmServiceTurnError};
pub(crate) use failure::{PmAuthenticatedPublicLaneFailure, PmPublicAgedHead};
use failure::{PmLaneAuthorityId, PmPublicRouteLaneEvidence};
pub use policy::{
    PM_INPUT_SERVICE_PRIORITY, PmLaneKind, PmLaneMetrics, PmLanePolicy, SaturationAction,
};
pub(crate) use public::PmPublicLaneState;
pub use service::PmPublicLaneService;

#[cfg(test)]
mod complete_tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PmIngressOrder {
    connection: PmConnectionId,
    ordering: EventOrdering,
}

impl PmIngressOrder {
    const fn from_ordering(connection: PmConnectionId, ordering: EventOrdering) -> Self {
        Self {
            connection,
            ordering,
        }
    }

    const fn connection(self) -> PmConnectionId {
        self.connection
    }

    const fn ordering(self) -> EventOrdering {
        self.ordering
    }
}

/// Semantic key for one routed public event. Construction is private.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmServiceKey {
    monotonic_receive_ns: u64,
    source: PmSourceHandle,
    source_kind_rank: u8,
    source_scope_ordinal: u16,
    connection_epoch: ConnectionEpoch,
    local_ingress_sequence: IngressSequence,
    variant_rank: u8,
}

/// Reached Phase-3 public source discriminator retained in every service key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PmServiceSourceKind {
    OkxReference,
    PolymarketMarket,
}

impl PmServiceSourceKind {
    const fn rank(self) -> u8 {
        match self {
            Self::OkxReference => 0,
            Self::PolymarketMarket => 1,
        }
    }
}

impl PmServiceKey {
    fn derived(
        clock: ReceivedEventClock,
        source: reap_pm_core::PmProductSource,
        evidence: PmIngressOrder,
        variant_rank: u8,
    ) -> Self {
        let (source_kind, source_scope_ordinal) = match source {
            reap_pm_core::PmProductSource::OkxReference { reference, .. } => {
                (PmServiceSourceKind::OkxReference, reference.ordinal())
            }
            reap_pm_core::PmProductSource::PolymarketMarket { token, .. } => {
                (PmServiceSourceKind::PolymarketMarket, token.ordinal())
            }
            reap_pm_core::PmProductSource::PolymarketAccount { .. } => {
                unreachable!("the reached public lane cannot carry account input")
            }
        };
        Self {
            monotonic_receive_ns: clock.monotonic_receive_ns(),
            source: source.source(),
            source_kind_rank: source_kind.rank(),
            source_scope_ordinal,
            connection_epoch: evidence.ordering().connection_epoch(),
            local_ingress_sequence: evidence.ordering().local_ingress_sequence(),
            variant_rank,
        }
    }

    #[must_use]
    pub const fn monotonic_receive_ns(self) -> u64 {
        self.monotonic_receive_ns
    }

    #[must_use]
    pub const fn source(self) -> PmSourceHandle {
        self.source
    }

    #[must_use]
    pub const fn source_kind_rank(self) -> u8 {
        self.source_kind_rank
    }

    #[must_use]
    pub fn source_kind(self) -> PmServiceSourceKind {
        match self.source_kind_rank {
            0 => PmServiceSourceKind::OkxReference,
            1 => PmServiceSourceKind::PolymarketMarket,
            _ => unreachable!("service source rank is privately constructed"),
        }
    }

    #[must_use]
    pub const fn source_scope_ordinal(self) -> u16 {
        self.source_scope_ordinal
    }

    #[must_use]
    pub const fn connection_epoch(self) -> ConnectionEpoch {
        self.connection_epoch
    }

    #[must_use]
    pub const fn local_ingress_sequence(self) -> IngressSequence {
        self.local_ingress_sequence
    }

    #[must_use]
    pub const fn variant_rank(self) -> u8 {
        self.variant_rank
    }
}

#[derive(Debug, PartialEq, Eq)]
struct LaneItem<T> {
    delivery: ImmutableDelivery<ReceivedLaneValue<T>>,
}

#[derive(Debug, PartialEq, Eq)]
struct ReceivedLaneValue<T> {
    key: PmServiceKey,
    connection: PmConnectionId,
    ordering: EventOrdering,
    received_clock: ReceivedEventClock,
    public_route: PmPublicRouteLaneEvidence,
    value: T,
}

impl<T> LaneItem<T> {
    fn new(
        key: PmServiceKey,
        evidence: PmIngressOrder,
        received_clock: ReceivedEventClock,
        public_route: PmPublicRouteLaneEvidence,
        value: T,
    ) -> Self {
        let payload = ReceivedLaneValue {
            key,
            connection: evidence.connection(),
            ordering: evidence.ordering(),
            received_clock,
            public_route,
            value,
        };
        Self {
            delivery: ImmutableDelivery::new(payload, received_clock.monotonic_receive_ns())
                .expect("checked received clocks are positive"),
        }
    }

    const fn key(&self) -> PmServiceKey {
        self.delivery.payload().key
    }

    const fn connection(&self) -> PmConnectionId {
        self.delivery.payload().connection
    }

    const fn received_clock(&self) -> ReceivedEventClock {
        self.delivery.payload().received_clock
    }

    const fn ordering(&self) -> EventOrdering {
        self.delivery.payload().ordering
    }

    const fn public_route(&self) -> PmPublicRouteLaneEvidence {
        self.delivery.payload().public_route
    }

    fn queue_age_ns(&self, now_ns: u64) -> Result<u64, DeliveryClockError> {
        self.delivery.queue_age_ns(now_ns)
    }

    fn into_value(self) -> T {
        self.delivery.into_payload().value
    }
}

#[derive(Debug, PartialEq, Eq)]
enum LaneEnqueueError<T> {
    Full { value: T, action: SaturationAction },
    DuplicateKey { value: T },
}

#[derive(Debug, PartialEq, Eq)]
pub struct ServicedLaneItem<T> {
    lane: PmLaneKind,
    key: PmServiceKey,
    connection: PmConnectionId,
    ordering: EventOrdering,
    clock: EventClock,
    value: T,
}

impl<T> ServicedLaneItem<T> {
    #[must_use]
    pub const fn lane(&self) -> PmLaneKind {
        self.lane
    }

    #[must_use]
    pub const fn key(&self) -> PmServiceKey {
        self.key
    }

    #[must_use]
    pub const fn connection(&self) -> PmConnectionId {
        self.connection
    }

    #[must_use]
    pub const fn ordering(&self) -> EventOrdering {
        self.ordering
    }

    #[must_use]
    pub const fn clock(&self) -> EventClock {
        self.clock
    }

    #[must_use]
    pub fn into_value(self) -> T {
        self.value
    }
}

trait PmObservedEvent: PmSourceBound + Sized {
    fn variant_rank(&self) -> u8;
}

fn enqueue_received<T, U>(
    queue: &mut BoundedHeap<PmServiceKey, LaneItem<U>>,
    key: PmServiceKey,
    evidence: PmIngressOrder,
    clock: ReceivedEventClock,
    public_route: PmPublicRouteLaneEvidence,
    value: T,
    wrap: impl FnOnce(T) -> U,
) -> Result<(), LaneEnqueueError<T>> {
    match queue.prepare(key) {
        Admission::Insert | Admission::Coalesced => {
            queue.insert(
                key,
                LaneItem::new(key, evidence, clock, public_route, wrap(value)),
            );
            Ok(())
        }
        Admission::Duplicate => Err(LaneEnqueueError::DuplicateKey { value }),
        Admission::Full(action) => Err(LaneEnqueueError::Full { value, action }),
    }
}

#[allow(
    clippy::large_enum_variant,
    reason = "fixed inline public events preserve the zero-allocation owner path; the heap is preallocated"
)]
#[derive(Debug)]
enum PmPublicInput {
    PmUnavailable(PmPublicUnavailable),
    OkxUnavailable(OkxPublicUnavailable),
    Market(PmMarketEvent),
    Book {
        event: PmBookEvent,
        projection: PmBookDecisionProjection,
    },
    Reference(OkxReferenceEvent),
}

macro_rules! observed_event {
    ($event:ty, $rank:expr) => {
        impl PmObservedEvent for $event {
            fn variant_rank(&self) -> u8 {
                $rank
            }
        }
    };
}

observed_event!(PmMarketEvent, 1);
observed_event!(PmPublicUnavailable, 0);
observed_event!(OkxPublicUnavailable, 0);
observed_event!(OkxReferenceEvent, 5);

impl PmObservedEvent for PmBookEvent {
    fn variant_rank(&self) -> u8 {
        match self.update() {
            PmBookUpdate::TickSizeChanged { .. } => 1,
            PmBookUpdate::Snapshot(_) => 2,
            PmBookUpdate::DeltaBatch(_) => 3,
            PmBookUpdate::TopCheck(_) => 4,
        }
    }
}

#[cfg(test)]
mod authority_tests {
    use reap_pm_core::{
        ConnectionEpoch, IngressSequence, OkxReferenceEvent, OkxReferenceHandle, OkxReferencePrice,
        PmConnectionId, PmProductSource, PmSourceHandle, ReceivedEventClock,
    };

    use super::*;

    fn reference() -> OkxReferenceEvent {
        OkxReferenceEvent::new(
            PmProductSource::okx_reference(
                PmSourceHandle::from_ordinal(1),
                OkxReferenceHandle::from_ordinal(1),
            ),
            OkxReferenceHandle::from_ordinal(1),
            OkxReferencePrice::parse_decimal("50000.125").expect("reference price"),
        )
        .expect("reference event")
    }

    fn public_key(sequence: u64) -> (PmServiceKey, PmIngressOrder, ReceivedEventClock) {
        let clock = ReceivedEventClock::new(None, sequence + 10_000, sequence)
            .expect("positive receive clock");
        let connection = PmConnectionId::new("lane-authority-test").expect("connection");
        let ordering = EventOrdering::new(
            ConnectionEpoch::new(1),
            None,
            None,
            None,
            IngressSequence::new(sequence),
        )
        .expect("ordering");
        let ingress = PmIngressOrder::from_ordering(connection, ordering);
        let event = reference();
        (
            PmServiceKey::derived(clock, event.source(), ingress, event.variant_rank()),
            ingress,
            clock,
        )
    }

    fn fill_public_lane(lane: &mut PmPublicLaneState) {
        let capacity = lane.queue.policy().capacity();
        let public_route = PmPublicRouteLaneEvidence {
            authority_id: PmPublicRouteAuthorityId::for_test(1),
            source: reference().source(),
            head: PmPublicAgedHead::OkxReference,
        };
        for sequence in 1..=capacity {
            let sequence = u64::try_from(sequence).expect("bounded capacity");
            let (key, ingress, clock) = public_key(sequence);
            lane.queue.insert(
                key,
                LaneItem::new(
                    key,
                    ingress,
                    clock,
                    public_route,
                    PmPublicInput::Reference(reference()),
                ),
            );
        }
    }

    fn full_failure(lane: &PmPublicLaneState, rejected: u64) -> PmPublicLaneEnqueueError<u64> {
        let (key, _, _) = public_key(rejected);
        PmPublicLaneEnqueueError::full(
            rejected,
            lane.queue.policy().saturation_action(),
            lane.authority_id,
            lane.queue.generation(),
            lane.queue.policy().capacity(),
            key,
        )
    }

    #[test]
    fn full_failure_rejects_sibling_public_owner_and_retains_input() {
        let mut origin = PmPublicLaneState::new();
        fill_public_lane(&mut origin);
        let sibling = PmPublicLaneState::new();
        let failure = full_failure(&origin, 9_001);

        let returned = sibling
            .authenticate_lane_failure(failure)
            .expect_err("sibling authority cannot consume the failure");
        assert_eq!(*returned.delivery(), 9_001);
        assert!(returned.is_full());
        assert_eq!(
            origin.metrics().depth(),
            PmLanePolicy::for_lane(PmLaneKind::Public).capacity()
        );
        assert_eq!(sibling.metrics().depth(), 0);
    }

    #[test]
    fn full_failure_rejects_changed_capacity_state_and_retains_input() {
        let mut lane = PmPublicLaneState::new();
        fill_public_lane(&mut lane);
        let failure = full_failure(&lane, 9_002);
        let _ = lane.queue.pop().expect("change full lane state");

        let returned = lane
            .authenticate_lane_failure(failure)
            .expect_err("old generation cannot authorize a changed lane");
        assert_eq!(*returned.delivery(), 9_002);
        assert!(returned.is_full());
        assert_eq!(
            lane.metrics().depth(),
            PmLanePolicy::for_lane(PmLaneKind::Public).capacity() - 1
        );
    }

    #[test]
    fn current_full_failure_is_consumed_once_with_exact_input() {
        let mut lane = PmPublicLaneState::new();
        fill_public_lane(&mut lane);
        let failure = full_failure(&lane, 9_003);

        let authenticated = lane
            .authenticate_lane_failure(failure)
            .expect("unchanged originating lane authenticates");
        let PmAuthenticatedPublicLaneFailure::Full { delivery, action } = authenticated else {
            panic!("full proof retains its exact failure kind");
        };
        assert_eq!(delivery, 9_003);
        assert_eq!(action, SaturationAction::InvalidateStreamAndResync);
    }
}
