//! Closed typed ingress boundary for the Goal-F PM product.
//!
//! Public observations can enter only after the existing Run-owned public
//! lane has serviced them. Private and reconciliation values are normalized
//! carriers for the complete scheduler; their constructors remain
//! crate-private until the canonical private owner exposes the matching
//! reduction seam.

use reap_pm_core::{
    EventClock, EventOrdering, OkxReferenceEvent, PmBookEvent, PmBookPoint, PmBookQuantity,
    PmBookSide, PmBookTop, PmConnectionId, PmInstrumentHandle, PmMarketEvent, SnapshotRevision,
};
use reap_pm_state::PmBookReducer;
use thiserror::Error;

use crate::lanes::{PmLaneKind, ServicedLaneItem};
use crate::schedule::{PmDueScheduledAction, PmScheduledActionKey, PmScheduledActionKind};

/// Exact public OKX reference occurrence after bounded-lane service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmOkxReferenceInput {
    connection: PmConnectionId,
    ordering: EventOrdering,
    clock: EventClock,
    event: OkxReferenceEvent,
}

impl PmOkxReferenceInput {
    pub(crate) fn from_serviced(
        item: ServicedLaneItem<OkxReferenceEvent>,
    ) -> Result<Self, PmProductInputError> {
        ensure_public_lane(item.lane())?;
        Ok(Self {
            connection: item.connection(),
            ordering: item.ordering(),
            clock: item.clock(),
            event: item.into_value(),
        })
    }

    /// Fixed local-evidence ingress after construction by the sealed runner.
    /// No public caller can stamp this occurrence or obtain the constructor.
    pub(crate) const fn from_evidence(
        connection: PmConnectionId,
        ordering: EventOrdering,
        clock: EventClock,
        event: OkxReferenceEvent,
    ) -> Self {
        Self {
            connection,
            ordering,
            clock,
            event,
        }
    }

    /// Configured route connection carried by the serviced occurrence.
    #[must_use]
    pub const fn connection(self) -> PmConnectionId {
        self.connection
    }

    /// Real venue/local ordering facts. Local ingress is never called a venue
    /// sequence.
    #[must_use]
    pub const fn ordering(self) -> EventOrdering {
        self.ordering
    }

    /// Distinct venue, wall, receive, and service clocks.
    #[must_use]
    pub const fn clock(self) -> EventClock {
        self.clock
    }

    /// Exact positive decimal reference value and configured handle.
    #[must_use]
    pub const fn event(self) -> OkxReferenceEvent {
        self.event
    }
}

/// Exact PM metadata/lifecycle occurrence after bounded-lane service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmMarketInput {
    connection: PmConnectionId,
    ordering: EventOrdering,
    clock: EventClock,
    event: PmMarketEvent,
}

impl PmMarketInput {
    pub(crate) fn from_serviced(
        item: ServicedLaneItem<PmMarketEvent>,
    ) -> Result<Self, PmProductInputError> {
        ensure_public_lane(item.lane())?;
        Ok(Self {
            connection: item.connection(),
            ordering: item.ordering(),
            clock: item.clock(),
            event: item.into_value(),
        })
    }

    pub(crate) const fn from_evidence(
        connection: PmConnectionId,
        ordering: EventOrdering,
        clock: EventClock,
        event: PmMarketEvent,
    ) -> Self {
        Self {
            connection,
            ordering,
            clock,
            event,
        }
    }

    #[must_use]
    pub const fn connection(self) -> PmConnectionId {
        self.connection
    }

    #[must_use]
    pub const fn ordering(self) -> EventOrdering {
        self.ordering
    }

    #[must_use]
    pub const fn clock(self) -> EventClock {
        self.clock
    }

    #[must_use]
    pub const fn event(self) -> PmMarketEvent {
        self.event
    }
}

/// Copied decision-facing projection from the existing canonical PM book
/// owner.
///
/// This is deliberately not a reducer and carries no snapshot/delta mutation
/// method. Construction is kept inside the crate so a public caller cannot
/// pair an arbitrary top with an authenticated serviced book occurrence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmBookDecisionProjection {
    instrument: PmInstrumentHandle,
    metadata_revision: SnapshotRevision,
    snapshot_revision: Option<SnapshotRevision>,
    readiness_revision: u64,
    top: Option<PmBookTop>,
    observed_monotonic_ns: u64,
    ready: bool,
}

impl PmBookDecisionProjection {
    /// Copies the exact decision-facing state from the sole canonical reducer
    /// immediately after it consumed `event`.
    ///
    /// This is intentionally crate-private and is called only by the
    /// reducer-coupled capture Run before that same event enters the public
    /// lane. An internally inconsistent ready reducer is represented as
    /// unavailable, so it cannot accidentally grant quote authority.
    pub(crate) fn from_reduced_owner(
        reducer: &PmBookReducer,
        event: &PmBookEvent,
        ordering: EventOrdering,
        observed_monotonic_ns: u64,
    ) -> Self {
        let readiness = reducer.readiness();
        let metadata_revision = readiness
            .metadata_revision()
            .unwrap_or_else(|| event.metadata_revision());
        let snapshot_revision = readiness.snapshot_revision();
        let top = readiness
            .is_ready()
            .then(|| canonical_top(reducer))
            .flatten();
        let ready = readiness.is_ready() && snapshot_revision.is_some() && top.is_some();
        Self {
            instrument: reducer.instrument(),
            metadata_revision,
            snapshot_revision,
            readiness_revision: ordering.local_ingress_sequence().value(),
            top: ready.then_some(top.expect("ready is bound to a canonical top")),
            observed_monotonic_ns,
            ready,
        }
    }

    #[must_use]
    pub const fn instrument(self) -> PmInstrumentHandle {
        self.instrument
    }

    #[must_use]
    pub const fn metadata_revision(self) -> SnapshotRevision {
        self.metadata_revision
    }

    #[must_use]
    pub const fn snapshot_revision(self) -> Option<SnapshotRevision> {
        self.snapshot_revision
    }

    #[must_use]
    pub const fn readiness_revision(self) -> u64 {
        self.readiness_revision
    }

    #[must_use]
    pub const fn top(self) -> Option<PmBookTop> {
        self.top
    }

    #[must_use]
    pub const fn observed_monotonic_ns(self) -> u64 {
        self.observed_monotonic_ns
    }

    #[must_use]
    pub const fn is_ready(self) -> bool {
        self.ready
    }
}

fn canonical_top(reducer: &PmBookReducer) -> Option<PmBookTop> {
    let bid = reducer
        .levels()
        .iter()
        .find(|level| level.side() == PmBookSide::Bid)?;
    let ask = reducer
        .levels()
        .iter()
        .find(|level| level.side() == PmBookSide::Ask)?;
    let PmBookQuantity::Quantity(bid_quantity) = bid.quantity() else {
        return None;
    };
    let PmBookQuantity::Quantity(ask_quantity) = ask.quantity() else {
        return None;
    };
    PmBookTop::new(
        Some(PmBookPoint::new(bid.price(), bid_quantity)),
        Some(PmBookPoint::new(ask.price(), ask_quantity)),
    )
    .ok()
}

/// One serviced PM book occurrence plus the same canonical owner's copied
/// post-reduction decision projection.
#[derive(Debug, PartialEq, Eq)]
pub struct PmBookInput {
    connection: PmConnectionId,
    ordering: EventOrdering,
    clock: EventClock,
    event: PmBookEvent,
    projection: PmBookDecisionProjection,
}

impl PmBookInput {
    pub(crate) fn from_serviced(
        item: ServicedLaneItem<PmBookEvent>,
        projection: PmBookDecisionProjection,
    ) -> Result<Self, PmProductInputError> {
        ensure_public_lane(item.lane())?;
        let ordering = item.ordering();
        let clock = item.clock();
        let connection = item.connection();
        let event = item.into_value();
        if event.instrument() != projection.instrument()
            || event.metadata_revision() != projection.metadata_revision()
            || projection.observed_monotonic_ns() != clock.monotonic_receive_ns()
        {
            return Err(PmProductInputError::BookProjectionMismatch);
        }
        if projection.is_ready() && ordering.snapshot_revision() != projection.snapshot_revision() {
            return Err(PmProductInputError::BookProjectionMismatch);
        }
        Ok(Self {
            connection,
            ordering,
            clock,
            event,
            projection,
        })
    }

    pub(crate) fn from_evidence(
        connection: PmConnectionId,
        ordering: EventOrdering,
        clock: EventClock,
        event: PmBookEvent,
        projection: PmBookDecisionProjection,
    ) -> Result<Self, PmProductInputError> {
        if event.instrument() != projection.instrument()
            || event.metadata_revision() != projection.metadata_revision()
            || projection.observed_monotonic_ns() != clock.monotonic_receive_ns()
            || (projection.is_ready()
                && ordering.snapshot_revision() != projection.snapshot_revision())
        {
            return Err(PmProductInputError::BookProjectionMismatch);
        }
        Ok(Self {
            connection,
            ordering,
            clock,
            event,
            projection,
        })
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
    pub const fn event(&self) -> &PmBookEvent {
        &self.event
    }

    #[must_use]
    pub const fn projection(&self) -> PmBookDecisionProjection {
        self.projection
    }
}

/// Coordinator-owned due timer occurrence.
///
/// Construction consumes the existing schedule owner's move-only due value;
/// a caller cannot stamp an arbitrary action key or deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmTimerInput {
    key: PmScheduledActionKey,
    deadline_ns: u64,
    scheduled_at_ns: u64,
    monotonic_service_ns: u64,
    due_age_ns: u64,
    local_action_sequence: u64,
    decision_wall_timestamp_ms: u64,
}

impl PmTimerInput {
    pub(crate) fn from_due(due: PmDueScheduledAction) -> Result<Self, PmProductInputError> {
        if due.local_action_sequence() == 0 || due.decision_wall_timestamp_ms() == 0 {
            return Err(PmProductInputError::ZeroRevisionOrTime);
        }
        Ok(Self {
            key: due.key(),
            deadline_ns: due.deadline_ns(),
            scheduled_at_ns: due.scheduled_at_ns(),
            monotonic_service_ns: due.serviced_at_ns(),
            due_age_ns: due.due_age_ns(),
            local_action_sequence: due.local_action_sequence(),
            decision_wall_timestamp_ms: due.decision_wall_timestamp_ms(),
        })
    }

    #[must_use]
    pub const fn key(&self) -> PmScheduledActionKey {
        self.key
    }

    #[must_use]
    pub const fn kind(&self) -> PmScheduledActionKind {
        self.key.kind()
    }

    #[must_use]
    pub const fn deadline_ns(&self) -> u64 {
        self.deadline_ns
    }

    #[must_use]
    pub const fn scheduled_at_ns(&self) -> u64 {
        self.scheduled_at_ns
    }

    /// Captured monotonic authority time. Service time is used only to prove
    /// due age and may not change a replayed decision.
    #[must_use]
    pub const fn decision_monotonic_ns(&self) -> u64 {
        self.deadline_ns
    }

    #[must_use]
    pub const fn monotonic_service_ns(&self) -> u64 {
        self.monotonic_service_ns
    }

    #[must_use]
    pub const fn due_age_ns(&self) -> u64 {
        self.due_age_ns
    }

    #[must_use]
    pub const fn local_action_sequence(&self) -> u64 {
        self.local_action_sequence
    }

    #[must_use]
    pub const fn decision_wall_timestamp_ms(&self) -> u64 {
        self.decision_wall_timestamp_ms
    }
}

/// Fixed, allocation-free control reason vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmControlReason {
    RequestedShutdown,
    RecoveredSafetyHalt,
    PublicUnavailable,
    PrivateUnavailable,
    RiskLimit,
    PersistenceUnavailable,
    SchedulerOverload,
    ContractViolation,
}

fn ensure_public_lane(lane: PmLaneKind) -> Result<(), PmProductInputError> {
    if lane == PmLaneKind::Public {
        Ok(())
    } else {
        Err(PmProductInputError::WrongLane)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmProductInputError {
    #[error("product input came from the wrong scheduler lane")]
    WrongLane,
    #[error("product input contains a zero revision or timestamp")]
    ZeroRevisionOrTime,
    #[error("a ready PM book projection lacks a snapshot or exact top")]
    ReadyBookMissingEvidence,
    #[error("an unavailable PM book projection carries executable top state")]
    UnavailableBookCarriesTop,
    #[error("the PM book projection does not match its serviced occurrence")]
    BookProjectionMismatch,
}
