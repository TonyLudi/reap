use reap_pm_core::{
    EventClock, EventOrdering, PmCompleteAccountSnapshot, PmCompleteFillQuery, PmConnectionId,
    PmProductSource, PmSourceHandle, ReceivedEventClock, ReceivedEventEnvelope,
};
use reap_pm_state::{PmPrivateExternalIngressFault, PmRiskHaltScope};
use reap_polymarket_adapter::{
    PmCompleteAccountSnapshotDelivery, PmCompleteOpenOrdersDelivery, PmExactOrderDetailDelivery,
    PmFakePlaceResult, PmFixturePrivateDelivery,
};
use reap_transport::{DeliveryClockError, ImmutableDelivery};
use thiserror::Error;

use crate::coordinator::{PmPendingFakeCancelResult, PmPersistencePoll};
use crate::private_monitor::PmFixturePairedReconciliationDelivery;

use super::{PmLaneKind, PmServiceSourceKind};

/// Stable source discriminator used by every non-public complete-scheduler key.
///
/// Public keys retain the existing `PmServiceSourceKind`; this extension adds
/// the reached account and internal-signal classes without changing the
/// already authenticated public delivery boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PmCompleteSourceKind {
    OkxReference,
    PolymarketMarket,
    PolymarketAccount,
    InternalSignal,
}

impl PmCompleteSourceKind {
    pub(super) const fn rank(self) -> u8 {
        match self {
            Self::OkxReference => 0,
            Self::PolymarketMarket => 1,
            Self::PolymarketAccount => 2,
            Self::InternalSignal => 3,
        }
    }
}

impl From<PmServiceSourceKind> for PmCompleteSourceKind {
    fn from(value: PmServiceSourceKind) -> Self {
        match value {
            PmServiceSourceKind::OkxReference => Self::OkxReference,
            PmServiceSourceKind::PolymarketMarket => Self::PolymarketMarket,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmCompleteInputSource {
    Product(PmProductSource),
    Internal(PmSourceHandle),
}

impl PmCompleteInputSource {
    const fn facts(self) -> (PmSourceHandle, PmCompleteSourceKind, u16) {
        match self {
            Self::Product(PmProductSource::OkxReference { source, reference }) => (
                source,
                PmCompleteSourceKind::OkxReference,
                reference.ordinal(),
            ),
            Self::Product(PmProductSource::PolymarketMarket { source, token }) => (
                source,
                PmCompleteSourceKind::PolymarketMarket,
                token.ordinal(),
            ),
            Self::Product(PmProductSource::PolymarketAccount { source, account }) => (
                source,
                PmCompleteSourceKind::PolymarketAccount,
                account.ordinal(),
            ),
            Self::Internal(source) => (source, PmCompleteSourceKind::InternalSignal, 0),
        }
    }
}

/// Exact ingress facts used to derive a non-public scheduler key.
///
/// Construction remains crate-private: connectivity/session roles issue
/// these facts and the lane merely orders them. This value carries no
/// mutation, journal, order, or cancellation permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PmCompleteIngress {
    source: PmCompleteInputSource,
    connection: PmConnectionId,
    ordering: EventOrdering,
    clock: ReceivedEventClock,
}

impl PmCompleteIngress {
    pub(crate) const fn product(
        source: PmProductSource,
        connection: PmConnectionId,
        ordering: EventOrdering,
        clock: ReceivedEventClock,
    ) -> Self {
        Self {
            source: PmCompleteInputSource::Product(source),
            connection,
            ordering,
            clock,
        }
    }

    pub(crate) const fn internal(
        source: PmSourceHandle,
        connection: PmConnectionId,
        ordering: EventOrdering,
        clock: ReceivedEventClock,
    ) -> Self {
        Self {
            source: PmCompleteInputSource::Internal(source),
            connection,
            ordering,
            clock,
        }
    }

    pub(crate) const fn source(self) -> PmCompleteInputSource {
        self.source
    }
}

/// Stable exact ordering key for one non-public product input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmCompleteServiceKey {
    monotonic_receive_ns: u64,
    source: PmSourceHandle,
    source_kind_rank: u8,
    source_scope_ordinal: u16,
    connection_epoch: reap_pm_core::ConnectionEpoch,
    local_ingress_sequence: reap_pm_core::IngressSequence,
    variant_rank: u8,
}

impl PmCompleteServiceKey {
    pub(super) const fn derived(ingress: PmCompleteIngress, variant_rank: u8) -> Self {
        let (source, source_kind, source_scope_ordinal) = ingress.source.facts();
        Self {
            monotonic_receive_ns: ingress.clock.monotonic_receive_ns(),
            source,
            source_kind_rank: source_kind.rank(),
            source_scope_ordinal,
            connection_epoch: ingress.ordering.connection_epoch(),
            local_ingress_sequence: ingress.ordering.local_ingress_sequence(),
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
    pub fn source_kind(self) -> PmCompleteSourceKind {
        match self.source_kind_rank {
            0 => PmCompleteSourceKind::OkxReference,
            1 => PmCompleteSourceKind::PolymarketMarket,
            2 => PmCompleteSourceKind::PolymarketAccount,
            3 => PmCompleteSourceKind::InternalSignal,
            _ => unreachable!("complete service source rank is privately constructed"),
        }
    }

    #[must_use]
    pub const fn source_scope_ordinal(self) -> u16 {
        self.source_scope_ordinal
    }

    #[must_use]
    pub const fn connection_epoch(self) -> reap_pm_core::ConnectionEpoch {
        self.connection_epoch
    }

    #[must_use]
    pub const fn local_ingress_sequence(self) -> reap_pm_core::IngressSequence {
        self.local_ingress_sequence
    }

    #[must_use]
    pub const fn variant_rank(self) -> u8 {
        self.variant_rank
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmStopControl {
    Shutdown,
    GlobalStop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PmScopedHalt {
    scope: PmRiskHaltScope,
}

impl PmScopedHalt {
    pub(crate) fn new(scope: PmRiskHaltScope) -> Option<Self> {
        matches!(scope, PmRiskHaltScope::Market | PmRiskHaltScope::Account)
            .then_some(Self { scope })
    }

    pub(crate) const fn scope(self) -> PmRiskHaltScope {
        self.scope
    }
}

/// Reached critical inputs in their frozen within-lane order.
#[derive(Debug)]
pub(crate) enum PmCriticalInput {
    Stop(PmStopControl),
    ScopedHalt(PmScopedHalt),
    FakeCancelResult(PmPendingFakeCancelResult),
    FakePlaceResult(PmFakePlaceResult),
}

impl PmCriticalInput {
    pub(crate) const fn fake_cancel_result_rank() -> u8 {
        2
    }

    pub(crate) const fn fake_place_result_rank() -> u8 {
        3
    }

    pub(crate) const fn variant_rank(&self) -> u8 {
        match self {
            Self::Stop(_) => 0,
            Self::ScopedHalt(_) => 1,
            Self::FakeCancelResult(_) => 2,
            Self::FakePlaceResult(_) => 3,
        }
    }
}

/// Exact take-once persistence result. The inner variants retain any bound
/// fake-effect permit or durable acknowledgement authority.
pub(crate) struct PmPersistenceInput {
    poll: PmPersistencePoll,
    variant_rank: u8,
}

impl PmPersistenceInput {
    pub(crate) fn from_poll(poll: PmPersistencePoll) -> Result<Self, PmPersistenceCarrierError> {
        let variant_rank = match poll {
            PmPersistencePoll::IntentFailed { .. } | PmPersistencePoll::FactFailed(_) => 0,
            PmPersistencePoll::QuoteAcknowledged { .. }
            | PmPersistencePoll::CancelAcknowledged { .. }
            | PmPersistencePoll::FactAcknowledged(_) => 1,
            PmPersistencePoll::Empty => return Err(PmPersistenceCarrierError::Empty),
            PmPersistencePoll::Pending => return Err(PmPersistenceCarrierError::Pending),
        };
        Ok(Self { poll, variant_rank })
    }

    pub(crate) const fn variant_rank(&self) -> u8 {
        self.variant_rank
    }

    pub(crate) fn into_poll(self) -> PmPersistencePoll {
        self.poll
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmPersistenceCarrierError {
    Empty,
    Pending,
}

/// Reached private lifecycle inputs in their frozen within-lane order.
#[allow(
    clippy::large_enum_variant,
    reason = "normalized private delivery remains inline because owner-loop admission must allocate zero heap bytes"
)]
#[derive(Debug)]
pub(crate) enum PmPrivateInput {
    ConnectionAvailable,
    ConnectionUnavailable(PmPrivateExternalIngressFault),
    /// One owner-bound, normalized fixture batch. It occupies the earliest
    /// private lifecycle rank because a single upstream frame may atomically
    /// contain both fills/trades and order progress.
    FixtureBatch(PmFixturePrivateDelivery),
}

impl PmPrivateInput {
    pub(crate) fn fixture_ingress(&self) -> Option<PmCompleteIngress> {
        match self {
            Self::FixtureBatch(delivery) => Some(PmCompleteIngress::product(
                delivery.source(),
                delivery.connection(),
                delivery.ordering(),
                delivery.received_clock(),
            )),
            Self::ConnectionAvailable | Self::ConnectionUnavailable(_) => None,
        }
    }

    pub(crate) const fn variant_rank(&self) -> u8 {
        match self {
            Self::ConnectionAvailable | Self::ConnectionUnavailable(_) => 0,
            Self::FixtureBatch(_) => 1,
        }
    }
}

/// Atomic, complete reconciliation inputs.
///
/// A complete account snapshot contains collateral, authorization, and
/// position facts together. It is therefore ordered at rank 3, the earliest
/// of those three frozen component ranks, and is never split into partial
/// state-bearing messages.
#[allow(
    clippy::enum_variant_names,
    reason = "the suffix records that every reached reconciliation carrier is fixture-only"
)]
#[allow(
    clippy::large_enum_variant,
    reason = "owner-bound reconciliation deliveries remain inline because owner-loop admission must allocate zero heap bytes"
)]
#[derive(Debug)]
pub(crate) enum PmReconciliationInput {
    OpenOrdersFixture(PmCompleteOpenOrdersDelivery),
    OrderDetailFixture(PmExactOrderDetailDelivery),
    /// The two exact role-owner-bound halves cannot be queued or serviced
    /// independently.
    PairedFixture(PmFixturePairedReconciliationDelivery),
    /// A complete account-only fixture refresh.
    StandaloneAccountFixture(PmCompleteAccountSnapshotDelivery),
}

impl PmReconciliationInput {
    pub(crate) const fn variant_rank(&self) -> u8 {
        match self {
            Self::OpenOrdersFixture(_) => 0,
            Self::OrderDetailFixture(_) => 1,
            Self::PairedFixture(_) => 2,
            Self::StandaloneAccountFixture(_) => 3,
        }
    }

    pub(crate) fn fixture_ingress(&self) -> PmCompleteIngress {
        match self {
            Self::OpenOrdersFixture(delivery) => PmCompleteIngress::product(
                delivery.source(),
                delivery.connection(),
                delivery.ordering(),
                delivery.received_clock(),
            ),
            Self::OrderDetailFixture(delivery) => PmCompleteIngress::product(
                delivery.source(),
                delivery.connection(),
                delivery.ordering(),
                delivery.received_clock(),
            ),
            Self::PairedFixture(delivery) => PmCompleteIngress::product(
                delivery.source(),
                delivery.connection(),
                delivery.ordering(),
                delivery.received_clock(),
            ),
            Self::StandaloneAccountFixture(delivery) => PmCompleteIngress::product(
                delivery.source(),
                delivery.connection(),
                delivery.ordering(),
                delivery.received_clock(),
            ),
        }
    }
}

/// One validated account-plus-fill reconciliation cut.
///
/// The two received envelopes remain inseparable after construction. Exact
/// source, connection, epoch, account scope, request boundary, snapshot, and
/// snapshot revision equality are proved before lane admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmPairedReconciliationCut {
    account: ReceivedEventEnvelope<PmCompleteAccountSnapshot>,
    fills: ReceivedEventEnvelope<PmCompleteFillQuery>,
}

impl PmPairedReconciliationCut {
    pub fn new(
        account: ReceivedEventEnvelope<PmCompleteAccountSnapshot>,
        fills: ReceivedEventEnvelope<PmCompleteFillQuery>,
    ) -> Result<Self, PmPairedReconciliationCutError> {
        if account.source() != fills.source() {
            return Err(PmPairedReconciliationCutError::SourceMismatch);
        }
        if account.connection_id() != fills.connection_id() {
            return Err(PmPairedReconciliationCutError::ConnectionMismatch);
        }
        if account.ordering().connection_epoch() != fills.ordering().connection_epoch() {
            return Err(PmPairedReconciliationCutError::ConnectionEpochMismatch);
        }
        if account.payload().account_scope() != fills.payload().account_scope() {
            return Err(PmPairedReconciliationCutError::AccountScopeMismatch);
        }
        if account.payload().boundary() != fills.payload().boundary() {
            return Err(PmPairedReconciliationCutError::BoundaryMismatch);
        }
        if account.payload().snapshot() != fills.payload().snapshot() {
            return Err(PmPairedReconciliationCutError::SnapshotMismatch);
        }
        if account.ordering().snapshot_revision() != fills.ordering().snapshot_revision() {
            return Err(PmPairedReconciliationCutError::SnapshotRevisionMismatch);
        }
        Ok(Self { account, fills })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPairedReconciliationCutError {
    #[error("paired reconciliation envelopes name different product sources")]
    SourceMismatch,
    #[error("paired reconciliation envelopes name different connections")]
    ConnectionMismatch,
    #[error("paired reconciliation envelopes name different connection epochs")]
    ConnectionEpochMismatch,
    #[error("paired reconciliation payloads name different account scopes")]
    AccountScopeMismatch,
    #[error("paired reconciliation payloads name different request boundaries")]
    BoundaryMismatch,
    #[error("paired reconciliation payloads name different snapshots")]
    SnapshotMismatch,
    #[error("paired reconciliation envelopes name different snapshot revisions")]
    SnapshotRevisionMismatch,
}

/// Closed telemetry label set. Values are numeric and formatting remains off
/// the owner loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmTelemetryKind {
    Health,
    Metric,
    Audit,
}

impl PmTelemetryKind {
    const fn rank(self) -> u8 {
        match self {
            Self::Health => 0,
            Self::Metric => 1,
            Self::Audit => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PmTelemetryInput {
    kind: PmTelemetryKind,
    value: u64,
}

impl PmTelemetryInput {
    pub(crate) const fn new(kind: PmTelemetryKind, value: u64) -> Self {
        Self { kind, value }
    }

    pub(crate) const fn kind(self) -> PmTelemetryKind {
        self.kind
    }

    pub(crate) const fn value(self) -> u64 {
        self.value
    }

    pub(crate) const fn variant_rank(&self) -> u8 {
        self.kind.rank()
    }
}

pub(crate) struct PmCompleteLaneItem<T> {
    delivery: ImmutableDelivery<PmCompleteReceived<T>>,
}

struct PmCompleteReceived<T> {
    key: PmCompleteServiceKey,
    ingress: PmCompleteIngress,
    value: T,
}

impl<T> PmCompleteLaneItem<T> {
    pub(super) fn new(key: PmCompleteServiceKey, ingress: PmCompleteIngress, value: T) -> Self {
        Self {
            delivery: ImmutableDelivery::new(
                PmCompleteReceived {
                    key,
                    ingress,
                    value,
                },
                ingress.clock.monotonic_receive_ns(),
            )
            .expect("received clocks are validated before complete-lane admission"),
        }
    }

    pub(super) const fn key(&self) -> PmCompleteServiceKey {
        self.delivery.payload().key
    }

    pub(super) fn queue_age_ns(&self, now_ns: u64) -> Result<u64, DeliveryClockError> {
        self.delivery.queue_age_ns(now_ns)
    }

    pub(super) fn into_serviced(
        self,
        lane: PmLaneKind,
        monotonic_service_ns: u64,
    ) -> Result<PmCompleteServiced<T>, reap_pm_core::EnvelopeError> {
        let clock = self
            .delivery
            .payload()
            .ingress
            .clock
            .service_at(monotonic_service_ns)?;
        let received = self.delivery.into_payload();
        Ok(PmCompleteServiced {
            lane,
            key: received.key,
            source: received.ingress.source,
            connection: received.ingress.connection,
            ordering: received.ingress.ordering,
            clock,
            value: received.value,
        })
    }
}

/// One exact serviced non-public occurrence transferred to the coordinator.
pub(crate) struct PmCompleteServiced<T> {
    lane: PmLaneKind,
    key: PmCompleteServiceKey,
    source: PmCompleteInputSource,
    connection: PmConnectionId,
    ordering: EventOrdering,
    clock: EventClock,
    value: T,
}

impl<T> PmCompleteServiced<T> {
    pub(crate) const fn lane(&self) -> PmLaneKind {
        self.lane
    }

    pub(crate) const fn key(&self) -> PmCompleteServiceKey {
        self.key
    }

    pub(crate) const fn source(&self) -> PmCompleteInputSource {
        self.source
    }

    pub(crate) const fn connection(&self) -> PmConnectionId {
        self.connection
    }

    pub(crate) const fn ordering(&self) -> EventOrdering {
        self.ordering
    }

    pub(crate) const fn clock(&self) -> EventClock {
        self.clock
    }

    pub(crate) fn into_value(self) -> T {
        self.value
    }
}
