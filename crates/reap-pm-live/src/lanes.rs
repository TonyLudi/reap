use reap_pm_core::{
    ConnectionEpoch, EnvelopeError, EventClock, IngressSequence, OkxReferenceEvent,
    PmAllowanceEvent, PmBalanceEvent, PmBookEvent, PmBookUpdate, PmConnectionId, PmFillEvent,
    PmMarketEvent, PmOrderEvent, PmPositionEvent, PmSourceBound, PmSourceHandle,
    ReceivedEventClock,
};
use reap_polymarket_adapter::{
    PmCompleteFillPage, PmCompleteOpenOrdersSnapshot, PmExactOrderDetail,
};
use reap_transport::{DeliveryClockError, ImmutableDelivery};
use thiserror::Error;

mod bounded;
mod policy;
mod scheduled;

use bounded::{Admission, BoundedHeap};
pub use policy::{PmLaneKind, PmLaneMetrics, PmLanePolicy, SaturationAction};
pub use scheduled::{
    PmScheduledAction, PmScheduledActionKind, PmScheduledEnqueueError, PmScheduledKey,
    PmScheduledKeyError, PmScheduledSide, ServicedScheduledAction,
};

/// Checked connection-local ordering facts, independent of event clocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmIngressOrder {
    connection: PmConnectionId,
    connection_epoch: ConnectionEpoch,
    local_ingress_sequence: IngressSequence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmIngressOrderError {
    #[error("connection epoch must be nonzero")]
    ZeroConnectionEpoch,
    #[error("local ingress sequence must be nonzero")]
    ZeroIngressSequence,
}

impl PmIngressOrder {
    pub fn new(
        connection: PmConnectionId,
        connection_epoch: ConnectionEpoch,
        local_ingress_sequence: IngressSequence,
    ) -> Result<Self, PmIngressOrderError> {
        if connection_epoch.value() == 0 {
            return Err(PmIngressOrderError::ZeroConnectionEpoch);
        }
        if local_ingress_sequence.value() == 0 {
            return Err(PmIngressOrderError::ZeroIngressSequence);
        }
        Ok(Self {
            connection,
            connection_epoch,
            local_ingress_sequence,
        })
    }

    #[must_use]
    pub const fn connection(self) -> PmConnectionId {
        self.connection
    }

    #[must_use]
    pub const fn connection_epoch(self) -> ConnectionEpoch {
        self.connection_epoch
    }

    #[must_use]
    pub const fn local_ingress_sequence(self) -> IngressSequence {
        self.local_ingress_sequence
    }
}

/// Semantic key for received events. Construction is intentionally private.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmServiceKey {
    monotonic_receive_ns: u64,
    source: PmSourceHandle,
    connection_epoch: ConnectionEpoch,
    local_ingress_sequence: IngressSequence,
    variant_rank: u8,
}

impl PmServiceKey {
    fn derived(
        clock: ReceivedEventClock,
        source: PmSourceHandle,
        ingress: PmIngressOrder,
        variant_rank: u8,
    ) -> Self {
        Self {
            monotonic_receive_ns: clock.monotonic_receive_ns(),
            source,
            connection_epoch: ingress.connection_epoch(),
            local_ingress_sequence: ingress.local_ingress_sequence(),
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
    received_clock: ReceivedEventClock,
    value: T,
}

impl<T> LaneItem<T> {
    fn new(
        key: PmServiceKey,
        ingress: PmIngressOrder,
        received_clock: ReceivedEventClock,
        value: T,
    ) -> Self {
        let payload = ReceivedLaneValue {
            key,
            connection: ingress.connection(),
            received_clock,
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

    fn queue_age_ns(&self, now_ns: u64) -> Result<u64, DeliveryClockError> {
        self.delivery.queue_age_ns(now_ns)
    }

    fn into_value(self) -> T {
        self.delivery.into_payload().value
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum LaneEnqueueError<T> {
    Full { value: T, action: SaturationAction },
    DuplicateKey { value: T },
}

#[derive(Debug, PartialEq, Eq)]
pub struct ServicedLaneItem<T> {
    lane: PmLaneKind,
    key: PmServiceKey,
    connection: PmConnectionId,
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
    pub const fn clock(&self) -> EventClock {
        self.clock
    }

    #[must_use]
    pub fn into_value(self) -> T {
        self.value
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmServiceTurnError {
    #[error("lane contains work older than its maximum admitted age")]
    Aged {
        lane: PmLaneKind,
        action: SaturationAction,
    },
    #[error("critical burst is insufficient; product must stop")]
    CriticalBurstExceeded,
    #[error("transport delivery clock is invalid at service")]
    DeliveryClock(DeliveryClockError),
    #[error("PM received clock is invalid at service")]
    EventClock(EnvelopeError),
}

/// Closed non-authority markers for later-phase lane payload owners.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmLaneSignalKind {
    Shutdown,
    MarketOrAccountHalt,
    FakeCancelResult,
    FakePlaceResult,
    DurableFailure,
    DurableSuccess,
    PrivateConnectionUnavailable,
    PublicConnectionUnavailable,
    Telemetry,
    ReconciliationRequest,
    CaptureFrame,
    JournalRecord,
    FakePlaceGtcPostOnly,
    FakeCancelOwned,
}

impl PmLaneSignalKind {
    #[must_use]
    pub const fn lane(self) -> PmLaneKind {
        match self {
            Self::Shutdown
            | Self::MarketOrAccountHalt
            | Self::FakeCancelResult
            | Self::FakePlaceResult => PmLaneKind::Critical,
            Self::DurableFailure | Self::DurableSuccess => PmLaneKind::Persistence,
            Self::PrivateConnectionUnavailable => PmLaneKind::Private,
            Self::PublicConnectionUnavailable => PmLaneKind::Public,
            Self::Telemetry => PmLaneKind::Telemetry,
            Self::ReconciliationRequest => PmLaneKind::ReconciliationRequest,
            Self::CaptureFrame => PmLaneKind::Capture,
            Self::JournalRecord => PmLaneKind::Journal,
            Self::FakePlaceGtcPostOnly | Self::FakeCancelOwned => PmLaneKind::FakeEffect,
        }
    }

    #[must_use]
    pub const fn variant_rank(self) -> u8 {
        match self {
            Self::Shutdown
            | Self::DurableFailure
            | Self::PrivateConnectionUnavailable
            | Self::PublicConnectionUnavailable
            | Self::Telemetry
            | Self::ReconciliationRequest
            | Self::CaptureFrame
            | Self::JournalRecord
            | Self::FakePlaceGtcPostOnly => 0,
            Self::MarketOrAccountHalt | Self::DurableSuccess | Self::FakeCancelOwned => 1,
            Self::FakeCancelResult => 2,
            Self::FakePlaceResult => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmLaneSignal {
    kind: PmLaneSignalKind,
    source: PmSourceHandle,
}

impl PmLaneSignal {
    #[must_use]
    pub const fn new(kind: PmLaneSignalKind, source: PmSourceHandle) -> Self {
        Self { kind, source }
    }

    #[must_use]
    pub const fn kind(self) -> PmLaneSignalKind {
        self.kind
    }

    #[must_use]
    pub const fn source(self) -> PmSourceHandle {
        self.source
    }
}

/// Sealed concrete observation classifier and storage conversion.
pub trait PmObservedEvent: observed_event_seal::Sealed + PmSourceBound + Sized {
    fn lane(&self) -> PmLaneKind;
    fn variant_rank(&self) -> u8;

    #[doc(hidden)]
    fn enqueue_into(
        self,
        lanes: &mut PmLaneSet,
        ingress: PmIngressOrder,
        clock: ReceivedEventClock,
    ) -> Result<(), LaneEnqueueError<Self>>;
}

mod observed_event_seal {
    use super::{
        OkxReferenceEvent, PmAllowanceEvent, PmBalanceEvent, PmBookEvent, PmCompleteFillPage,
        PmCompleteOpenOrdersSnapshot, PmExactOrderDetail, PmFillEvent, PmMarketEvent, PmOrderEvent,
        PmPositionEvent,
    };

    pub trait Sealed {}

    impl Sealed for PmMarketEvent {}
    impl Sealed for PmBookEvent {}
    impl Sealed for PmOrderEvent {}
    impl Sealed for PmFillEvent {}
    impl Sealed for PmCompleteOpenOrdersSnapshot {}
    impl Sealed for PmExactOrderDetail {}
    impl Sealed for PmCompleteFillPage {}
    impl Sealed for PmBalanceEvent {}
    impl Sealed for PmAllowanceEvent {}
    impl Sealed for PmPositionEvent {}
    impl Sealed for OkxReferenceEvent {}
}

fn enqueue_received<T, U>(
    queue: &mut BoundedHeap<PmServiceKey, LaneItem<U>>,
    key: PmServiceKey,
    ingress: PmIngressOrder,
    clock: ReceivedEventClock,
    value: T,
    wrap: impl FnOnce(T) -> U,
) -> Result<(), LaneEnqueueError<T>> {
    match queue.prepare(key) {
        Admission::Insert | Admission::Coalesced => {
            queue.insert(key, LaneItem::new(key, ingress, clock, wrap(value)));
            Ok(())
        }
        Admission::Duplicate => Err(LaneEnqueueError::DuplicateKey { value }),
        Admission::Full(action) => Err(LaneEnqueueError::Full { value, action }),
    }
}

#[derive(Debug)]
enum PmPrivateInput {
    Signal(PmLaneSignal),
    Fill(PmFillEvent),
    Order(PmOrderEvent),
}

#[allow(
    clippy::large_enum_variant,
    reason = "fixed inline PM events preserve the zero-allocation owner path; the lane heap is preallocated"
)]
#[derive(Debug)]
enum PmPublicInput {
    Signal(PmLaneSignal),
    Market(PmMarketEvent),
    Book(PmBookEvent),
    Reference(OkxReferenceEvent),
}

#[derive(Debug)]
enum PmReconciliationInput {
    OpenOrders(PmCompleteOpenOrdersSnapshot),
    OrderDetail(PmExactOrderDetail),
    FillPage(PmCompleteFillPage),
    Balance(PmBalanceEvent),
    Allowance(PmAllowanceEvent),
    Position(PmPositionEvent),
}

macro_rules! observed_event {
    ($event:ty, $lane:ident, $rank:expr, $field:ident, $variant:path) => {
        impl PmObservedEvent for $event {
            fn lane(&self) -> PmLaneKind {
                PmLaneKind::$lane
            }

            fn variant_rank(&self) -> u8 {
                $rank
            }

            fn enqueue_into(
                self,
                lanes: &mut PmLaneSet,
                ingress: PmIngressOrder,
                clock: ReceivedEventClock,
            ) -> Result<(), LaneEnqueueError<Self>> {
                let key = PmServiceKey::derived(
                    clock,
                    self.source().source(),
                    ingress,
                    self.variant_rank(),
                );
                enqueue_received(&mut lanes.$field, key, ingress, clock, self, $variant)
            }
        }
    };
}

observed_event!(PmMarketEvent, Public, 1, public, PmPublicInput::Market);
observed_event!(PmOrderEvent, Private, 2, private, PmPrivateInput::Order);
observed_event!(PmFillEvent, Private, 1, private, PmPrivateInput::Fill);
observed_event!(
    PmCompleteOpenOrdersSnapshot,
    Reconciliation,
    0,
    reconciliation,
    PmReconciliationInput::OpenOrders
);
observed_event!(
    PmExactOrderDetail,
    Reconciliation,
    1,
    reconciliation,
    PmReconciliationInput::OrderDetail
);
observed_event!(
    PmCompleteFillPage,
    Reconciliation,
    2,
    reconciliation,
    PmReconciliationInput::FillPage
);
observed_event!(
    PmBalanceEvent,
    Reconciliation,
    3,
    reconciliation,
    PmReconciliationInput::Balance
);
observed_event!(
    PmAllowanceEvent,
    Reconciliation,
    4,
    reconciliation,
    PmReconciliationInput::Allowance
);
observed_event!(
    PmPositionEvent,
    Reconciliation,
    5,
    reconciliation,
    PmReconciliationInput::Position
);
observed_event!(
    OkxReferenceEvent,
    Public,
    5,
    public,
    PmPublicInput::Reference
);

impl PmObservedEvent for PmBookEvent {
    fn lane(&self) -> PmLaneKind {
        PmLaneKind::Public
    }

    fn variant_rank(&self) -> u8 {
        match self.update() {
            PmBookUpdate::SnapshotStart { .. }
            | PmBookUpdate::SnapshotLevel(_)
            | PmBookUpdate::SnapshotComplete { .. } => 2,
            PmBookUpdate::Delta(_) => 3,
            PmBookUpdate::Top(_) => 4,
        }
    }

    fn enqueue_into(
        self,
        lanes: &mut PmLaneSet,
        ingress: PmIngressOrder,
        clock: ReceivedEventClock,
    ) -> Result<(), LaneEnqueueError<Self>> {
        let key =
            PmServiceKey::derived(clock, self.source().source(), ingress, self.variant_rank());
        enqueue_received(
            &mut lanes.public,
            key,
            ingress,
            clock,
            self,
            PmPublicInput::Book,
        )
    }
}

/// Static typed service callbacks; no trait object or erased event payload.
pub trait PmLaneService {
    fn on_signal(&mut self, _item: ServicedLaneItem<PmLaneSignal>) {}
    fn on_scheduled(&mut self, _item: ServicedScheduledAction) {}
    fn on_market(&mut self, _item: ServicedLaneItem<PmMarketEvent>) {}
    fn on_book(&mut self, _item: ServicedLaneItem<PmBookEvent>) {}
    fn on_reference(&mut self, _item: ServicedLaneItem<OkxReferenceEvent>) {}
    fn on_order(&mut self, _item: ServicedLaneItem<PmOrderEvent>) {}
    fn on_fill(&mut self, _item: ServicedLaneItem<PmFillEvent>) {}
    fn on_open_orders(&mut self, _item: ServicedLaneItem<PmCompleteOpenOrdersSnapshot>) {}
    fn on_order_detail(&mut self, _item: ServicedLaneItem<PmExactOrderDetail>) {}
    fn on_fill_page(&mut self, _item: ServicedLaneItem<PmCompleteFillPage>) {}
    fn on_balance(&mut self, _item: ServicedLaneItem<PmBalanceEvent>) {}
    fn on_allowance(&mut self, _item: ServicedLaneItem<PmAllowanceEvent>) {}
    fn on_position(&mut self, _item: ServicedLaneItem<PmPositionEvent>) {}
}

#[derive(Debug)]
pub struct PmLaneSet {
    critical: BoundedHeap<PmServiceKey, LaneItem<PmLaneSignal>>,
    persistence: BoundedHeap<PmServiceKey, LaneItem<PmLaneSignal>>,
    private: BoundedHeap<PmServiceKey, LaneItem<PmPrivateInput>>,
    scheduled: BoundedHeap<PmScheduledKey, PmScheduledAction>,
    public: BoundedHeap<PmServiceKey, LaneItem<PmPublicInput>>,
    reconciliation: BoundedHeap<PmServiceKey, LaneItem<PmReconciliationInput>>,
    telemetry: BoundedHeap<PmServiceKey, LaneItem<PmLaneSignal>>,
    reconciliation_request: BoundedHeap<PmServiceKey, LaneItem<PmLaneSignal>>,
    capture: BoundedHeap<PmServiceKey, LaneItem<PmLaneSignal>>,
    journal: BoundedHeap<PmServiceKey, LaneItem<PmLaneSignal>>,
    fake_effect: BoundedHeap<PmServiceKey, LaneItem<PmLaneSignal>>,
}

impl Default for PmLaneSet {
    fn default() -> Self {
        Self::new()
    }
}

impl PmLaneSet {
    #[must_use]
    pub fn new() -> Self {
        Self {
            critical: BoundedHeap::new(PmLaneKind::Critical),
            persistence: BoundedHeap::new(PmLaneKind::Persistence),
            private: BoundedHeap::new(PmLaneKind::Private),
            scheduled: BoundedHeap::new(PmLaneKind::Scheduled),
            public: BoundedHeap::new(PmLaneKind::Public),
            reconciliation: BoundedHeap::new(PmLaneKind::Reconciliation),
            telemetry: BoundedHeap::new(PmLaneKind::Telemetry),
            reconciliation_request: BoundedHeap::new(PmLaneKind::ReconciliationRequest),
            capture: BoundedHeap::new(PmLaneKind::Capture),
            journal: BoundedHeap::new(PmLaneKind::Journal),
            fake_effect: BoundedHeap::new(PmLaneKind::FakeEffect),
        }
    }

    pub fn enqueue_observation<E: PmObservedEvent>(
        &mut self,
        clock: ReceivedEventClock,
        ingress: PmIngressOrder,
        event: E,
    ) -> Result<(), LaneEnqueueError<E>> {
        event.enqueue_into(self, ingress, clock)
    }

    pub fn enqueue_signal(
        &mut self,
        clock: ReceivedEventClock,
        ingress: PmIngressOrder,
        signal: PmLaneSignal,
    ) -> Result<(), LaneEnqueueError<PmLaneSignal>> {
        let key = PmServiceKey::derived(
            clock,
            signal.source(),
            ingress,
            signal.kind().variant_rank(),
        );
        match signal.kind().lane() {
            PmLaneKind::Critical => enqueue_received(
                &mut self.critical,
                key,
                ingress,
                clock,
                signal,
                std::convert::identity,
            ),
            PmLaneKind::Persistence => enqueue_received(
                &mut self.persistence,
                key,
                ingress,
                clock,
                signal,
                std::convert::identity,
            ),
            PmLaneKind::Private => enqueue_received(
                &mut self.private,
                key,
                ingress,
                clock,
                signal,
                PmPrivateInput::Signal,
            ),
            PmLaneKind::Public => enqueue_received(
                &mut self.public,
                key,
                ingress,
                clock,
                signal,
                PmPublicInput::Signal,
            ),
            PmLaneKind::Telemetry => enqueue_received(
                &mut self.telemetry,
                key,
                ingress,
                clock,
                signal,
                std::convert::identity,
            ),
            PmLaneKind::ReconciliationRequest => enqueue_received(
                &mut self.reconciliation_request,
                key,
                ingress,
                clock,
                signal,
                std::convert::identity,
            ),
            PmLaneKind::Capture => enqueue_received(
                &mut self.capture,
                key,
                ingress,
                clock,
                signal,
                std::convert::identity,
            ),
            PmLaneKind::Journal => enqueue_received(
                &mut self.journal,
                key,
                ingress,
                clock,
                signal,
                std::convert::identity,
            ),
            PmLaneKind::FakeEffect => enqueue_received(
                &mut self.fake_effect,
                key,
                ingress,
                clock,
                signal,
                std::convert::identity,
            ),
            PmLaneKind::Scheduled | PmLaneKind::Reconciliation => {
                unreachable!("closed signal kind has a dedicated typed lane")
            }
        }
    }

    pub fn enqueue_scheduled(
        &mut self,
        monotonic_deadline_ns: u64,
        local_action_sequence: u64,
        action: PmScheduledAction,
    ) -> Result<(), PmScheduledEnqueueError> {
        let key = PmScheduledKey::derived(monotonic_deadline_ns, action, local_action_sequence)
            .map_err(PmScheduledEnqueueError::Key)?;
        match self.scheduled.prepare(key) {
            Admission::Insert | Admission::Coalesced => {
                self.scheduled.insert(key, action);
                Ok(())
            }
            Admission::Duplicate => Err(PmScheduledEnqueueError::DuplicateKey { action }),
            Admission::Full(saturation) => {
                Err(PmScheduledEnqueueError::Full { action, saturation })
            }
        }
    }

    #[must_use]
    pub fn metrics(&self, lane: PmLaneKind) -> PmLaneMetrics {
        match lane {
            PmLaneKind::Critical => self.critical.metrics(),
            PmLaneKind::Persistence => self.persistence.metrics(),
            PmLaneKind::Private => self.private.metrics(),
            PmLaneKind::Scheduled => self.scheduled.metrics(),
            PmLaneKind::Public => self.public.metrics(),
            PmLaneKind::Reconciliation => self.reconciliation.metrics(),
            PmLaneKind::Telemetry => self.telemetry.metrics(),
            PmLaneKind::ReconciliationRequest => self.reconciliation_request.metrics(),
            PmLaneKind::Capture => self.capture.metrics(),
            PmLaneKind::Journal => self.journal.metrics(),
            PmLaneKind::FakeEffect => self.fake_effect.metrics(),
        }
    }

    pub fn service_turn<C: PmLaneService>(
        &mut self,
        now_ns: u64,
        consumer: &mut C,
    ) -> Result<usize, PmServiceTurnError> {
        let mut serviced = 0;

        check_received_age(PmLaneKind::Critical, &self.critical, now_ns)?;
        serviced += service_signal_lane(
            PmLaneKind::Critical,
            &mut self.critical,
            now_ns,
            burst(PmLaneKind::Critical),
            consumer,
        )?;
        if self.critical.metrics().depth() != 0 {
            return Err(PmServiceTurnError::CriticalBurstExceeded);
        }

        check_received_age(PmLaneKind::Persistence, &self.persistence, now_ns)?;
        serviced += service_signal_lane(
            PmLaneKind::Persistence,
            &mut self.persistence,
            now_ns,
            burst(PmLaneKind::Persistence),
            consumer,
        )?;

        check_received_age(PmLaneKind::Private, &self.private, now_ns)?;
        serviced += service_private_lane(
            &mut self.private,
            now_ns,
            burst(PmLaneKind::Private),
            consumer,
        )?;

        check_scheduled_age(&self.scheduled, now_ns)?;
        serviced += service_scheduled_lane(
            &mut self.scheduled,
            now_ns,
            burst(PmLaneKind::Scheduled),
            consumer,
        );

        check_received_age(PmLaneKind::Public, &self.public, now_ns)?;
        serviced += service_public_lane(
            &mut self.public,
            now_ns,
            burst(PmLaneKind::Public),
            consumer,
        )?;

        check_received_age(PmLaneKind::Reconciliation, &self.reconciliation, now_ns)?;
        serviced += service_reconciliation_lane(
            &mut self.reconciliation,
            now_ns,
            burst(PmLaneKind::Reconciliation),
            consumer,
        )?;

        serviced += service_signal_lane(
            PmLaneKind::Telemetry,
            &mut self.telemetry,
            now_ns,
            burst(PmLaneKind::Telemetry),
            consumer,
        )?;
        Ok(serviced)
    }
}

fn burst(lane: PmLaneKind) -> usize {
    PmLanePolicy::for_lane(lane)
        .service_burst()
        .expect("service lane has a frozen burst")
}

fn check_received_age<T>(
    lane: PmLaneKind,
    queue: &BoundedHeap<PmServiceKey, LaneItem<T>>,
    now_ns: u64,
) -> Result<(), PmServiceTurnError> {
    let Some(entry) = queue.peek() else {
        return Ok(());
    };
    let age = entry
        .value
        .queue_age_ns(now_ns)
        .map_err(PmServiceTurnError::DeliveryClock)?;
    if queue
        .policy()
        .maximum_age_ns()
        .is_some_and(|maximum| age > maximum)
    {
        return Err(PmServiceTurnError::Aged {
            lane,
            action: queue.policy().saturation_action(),
        });
    }
    Ok(())
}

fn check_scheduled_age(
    queue: &BoundedHeap<PmScheduledKey, PmScheduledAction>,
    now_ns: u64,
) -> Result<(), PmServiceTurnError> {
    let Some(entry) = queue.peek() else {
        return Ok(());
    };
    if entry.key.monotonic_deadline_ns() > now_ns {
        return Ok(());
    }
    let lateness = now_ns - entry.key.monotonic_deadline_ns();
    if queue
        .policy()
        .maximum_age_ns()
        .is_some_and(|maximum| lateness > maximum)
    {
        return Err(PmServiceTurnError::Aged {
            lane: PmLaneKind::Scheduled,
            action: queue.policy().saturation_action(),
        });
    }
    Ok(())
}

fn into_serviced<T>(
    lane: PmLaneKind,
    item: LaneItem<T>,
    now_ns: u64,
) -> Result<ServicedLaneItem<T>, PmServiceTurnError> {
    let key = item.key();
    let connection = item.connection();
    let clock = item
        .received_clock()
        .service_at(now_ns)
        .map_err(PmServiceTurnError::EventClock)?;
    Ok(ServicedLaneItem {
        lane,
        key,
        connection,
        clock,
        value: item.into_value(),
    })
}

fn pop_received<T>(
    queue: &mut BoundedHeap<PmServiceKey, LaneItem<T>>,
    now_ns: u64,
) -> Result<Option<LaneItem<T>>, PmServiceTurnError> {
    let Some(next) = queue.peek() else {
        return Ok(None);
    };
    next.value
        .received_clock()
        .service_at(now_ns)
        .map_err(PmServiceTurnError::EventClock)?;
    Ok(queue.pop().map(|entry| entry.value))
}

fn map_serviced<U>(
    lane: PmLaneKind,
    key: PmServiceKey,
    connection: PmConnectionId,
    clock: EventClock,
    value: U,
) -> ServicedLaneItem<U> {
    ServicedLaneItem {
        lane,
        key,
        connection,
        clock,
        value,
    }
}

fn service_signal_lane<C: PmLaneService>(
    lane: PmLaneKind,
    queue: &mut BoundedHeap<PmServiceKey, LaneItem<PmLaneSignal>>,
    now_ns: u64,
    limit: usize,
    consumer: &mut C,
) -> Result<usize, PmServiceTurnError> {
    let count = limit.min(queue.len());
    for _ in 0..count {
        let item = pop_received(queue, now_ns)?.expect("bounded count");
        consumer.on_signal(into_serviced(lane, item, now_ns)?);
    }
    Ok(count)
}

fn service_private_lane<C: PmLaneService>(
    queue: &mut BoundedHeap<PmServiceKey, LaneItem<PmPrivateInput>>,
    now_ns: u64,
    limit: usize,
    consumer: &mut C,
) -> Result<usize, PmServiceTurnError> {
    let count = limit.min(queue.len());
    for _ in 0..count {
        let input = pop_received(queue, now_ns)?.expect("bounded count");
        let item = into_serviced(PmLaneKind::Private, input, now_ns)?;
        let ServicedLaneItem {
            lane,
            key,
            connection,
            clock,
            value,
        } = item;
        match value {
            PmPrivateInput::Signal(value) => {
                consumer.on_signal(map_serviced(lane, key, connection, clock, value));
            }
            PmPrivateInput::Fill(value) => {
                consumer.on_fill(map_serviced(lane, key, connection, clock, value));
            }
            PmPrivateInput::Order(value) => {
                consumer.on_order(map_serviced(lane, key, connection, clock, value));
            }
        }
    }
    Ok(count)
}

fn service_scheduled_lane<C: PmLaneService>(
    queue: &mut BoundedHeap<PmScheduledKey, PmScheduledAction>,
    now_ns: u64,
    limit: usize,
    consumer: &mut C,
) -> usize {
    let mut count = 0;
    while count < limit
        && queue
            .peek()
            .is_some_and(|entry| entry.key.monotonic_deadline_ns() <= now_ns)
    {
        let entry = queue.pop().expect("due scheduled item");
        consumer.on_scheduled(ServicedScheduledAction::new(entry.key, now_ns, entry.value));
        count += 1;
    }
    count
}

fn service_public_lane<C: PmLaneService>(
    queue: &mut BoundedHeap<PmServiceKey, LaneItem<PmPublicInput>>,
    now_ns: u64,
    limit: usize,
    consumer: &mut C,
) -> Result<usize, PmServiceTurnError> {
    let count = limit.min(queue.len());
    for _ in 0..count {
        let input = pop_received(queue, now_ns)?.expect("bounded count");
        let item = into_serviced(PmLaneKind::Public, input, now_ns)?;
        let ServicedLaneItem {
            lane,
            key,
            connection,
            clock,
            value,
        } = item;
        match value {
            PmPublicInput::Signal(value) => {
                consumer.on_signal(map_serviced(lane, key, connection, clock, value));
            }
            PmPublicInput::Market(value) => {
                consumer.on_market(map_serviced(lane, key, connection, clock, value));
            }
            PmPublicInput::Book(value) => {
                consumer.on_book(map_serviced(lane, key, connection, clock, value));
            }
            PmPublicInput::Reference(value) => {
                consumer.on_reference(map_serviced(lane, key, connection, clock, value));
            }
        }
    }
    Ok(count)
}

fn service_reconciliation_lane<C: PmLaneService>(
    queue: &mut BoundedHeap<PmServiceKey, LaneItem<PmReconciliationInput>>,
    now_ns: u64,
    limit: usize,
    consumer: &mut C,
) -> Result<usize, PmServiceTurnError> {
    let count = limit.min(queue.len());
    for _ in 0..count {
        let input = pop_received(queue, now_ns)?.expect("bounded count");
        let item = into_serviced(PmLaneKind::Reconciliation, input, now_ns)?;
        let ServicedLaneItem {
            lane,
            key,
            connection,
            clock,
            value,
        } = item;
        match value {
            PmReconciliationInput::OpenOrders(value) => {
                consumer.on_open_orders(map_serviced(lane, key, connection, clock, value));
            }
            PmReconciliationInput::OrderDetail(value) => {
                consumer.on_order_detail(map_serviced(lane, key, connection, clock, value));
            }
            PmReconciliationInput::FillPage(value) => {
                consumer.on_fill_page(map_serviced(lane, key, connection, clock, value));
            }
            PmReconciliationInput::Balance(value) => {
                consumer.on_balance(map_serviced(lane, key, connection, clock, value));
            }
            PmReconciliationInput::Allowance(value) => {
                consumer.on_allowance(map_serviced(lane, key, connection, clock, value));
            }
            PmReconciliationInput::Position(value) => {
                consumer.on_position(map_serviced(lane, key, connection, clock, value));
            }
        }
    }
    Ok(count)
}
