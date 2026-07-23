use std::fmt;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use reap_pm_core::{EventOrdering, PmConnectionId, PmProductSource, ReceivedEventClock};
use reap_transport::DeliveryClockError;
use thiserror::Error;

use super::{PmLaneKind, PmServiceKey, SaturationAction};
use crate::public_routes::PmPublicRouteAuthorityId;

/// Process-unique identity of one Run-owned public-lane instance.
///
/// This identity is runtime authority only. It is never serialized, logged as
/// a number, or included in logical event ordering.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct PmLaneAuthorityId(NonZeroU64);

impl PmLaneAuthorityId {
    pub(super) fn allocate() -> Self {
        static NEXT_LANE_AUTHORITY_ID: AtomicU64 = AtomicU64::new(1);
        let value = NEXT_LANE_AUTHORITY_ID
            .fetch_update(
                AtomicOrdering::Relaxed,
                AtomicOrdering::Relaxed,
                |current| current.checked_add(1),
            )
            .expect("process exhausted all nonzero lane authority identities");
        Self(NonZeroU64::new(value).expect("lane authority sequence starts at one"))
    }
}

impl fmt::Debug for PmLaneAuthorityId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmLaneAuthorityId(<opaque>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPublicRouteLaneEvidence {
    pub(super) authority_id: PmPublicRouteAuthorityId,
    pub(super) source: PmProductSource,
    pub(super) head: PmPublicAgedHead,
}

/// Private typed identity of the aged public-lane head.
///
/// This lets the active owner distinguish already-routed unavailable evidence
/// from a newly stale data delivery without exposing a forgeable route token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmPublicAgedHead {
    PmUnavailable(reap_polymarket_adapter::PmPublicSessionFault),
    OkxUnavailable(reap_okx_public_source::OkxPublicSessionFault),
    PmMetadata,
    PmBook,
    PmTickSizeChanged {
        old: reap_pm_core::PmTick,
        new: reap_pm_core::PmTick,
    },
    OkxReference,
}

#[derive(Debug, PartialEq, Eq)]
struct PmPublicLaneFailureProof {
    lane_authority: PmLaneAuthorityId,
    lane_generation: u64,
    lane_capacity: usize,
    rejected_key: PmServiceKey,
}

#[derive(Debug, PartialEq, Eq)]
enum PmPublicLaneFailure<D> {
    Full {
        delivery: D,
        action: SaturationAction,
        proof: PmPublicLaneFailureProof,
    },
    DuplicateKey {
        delivery: D,
        proof: PmPublicLaneFailureProof,
    },
}

/// Exact, move-only evidence that a route-issued delivery was rejected by one
/// concrete public-lane instance.
///
/// Construction and the lane proof are private. The active run must consume
/// this value through the matching Run-owned public lane before it mutates lifecycle
/// state.
#[derive(Debug, PartialEq, Eq)]
pub struct PmPublicLaneEnqueueError<D> {
    failure: PmPublicLaneFailure<D>,
}

impl<D> PmPublicLaneEnqueueError<D> {
    pub(super) fn full(
        delivery: D,
        action: SaturationAction,
        lane_authority: PmLaneAuthorityId,
        lane_generation: u64,
        lane_capacity: usize,
        rejected_key: PmServiceKey,
    ) -> Self {
        Self {
            failure: PmPublicLaneFailure::Full {
                delivery,
                action,
                proof: PmPublicLaneFailureProof {
                    lane_authority,
                    lane_generation,
                    lane_capacity,
                    rejected_key,
                },
            },
        }
    }

    pub(super) fn duplicate_key(
        delivery: D,
        lane_authority: PmLaneAuthorityId,
        lane_generation: u64,
        lane_capacity: usize,
        rejected_key: PmServiceKey,
    ) -> Self {
        Self {
            failure: PmPublicLaneFailure::DuplicateKey {
                delivery,
                proof: PmPublicLaneFailureProof {
                    lane_authority,
                    lane_generation,
                    lane_capacity,
                    rejected_key,
                },
            },
        }
    }

    #[must_use]
    pub const fn delivery(&self) -> &D {
        match &self.failure {
            PmPublicLaneFailure::Full { delivery, .. }
            | PmPublicLaneFailure::DuplicateKey { delivery, .. } => delivery,
        }
    }

    #[must_use]
    pub const fn is_full(&self) -> bool {
        matches!(self.failure, PmPublicLaneFailure::Full { .. })
    }

    #[must_use]
    pub const fn action(&self) -> Option<SaturationAction> {
        match self.failure {
            PmPublicLaneFailure::Full { action, .. } => Some(action),
            PmPublicLaneFailure::DuplicateKey { .. } => None,
        }
    }

    #[must_use]
    pub fn into_delivery(self) -> D {
        match self.failure {
            PmPublicLaneFailure::Full { delivery, .. }
            | PmPublicLaneFailure::DuplicateKey { delivery, .. } => delivery,
        }
    }

    pub(super) const fn lane_authority(&self) -> PmLaneAuthorityId {
        match &self.failure {
            PmPublicLaneFailure::Full { proof, .. }
            | PmPublicLaneFailure::DuplicateKey { proof, .. } => proof.lane_authority,
        }
    }

    pub(super) const fn lane_generation(&self) -> u64 {
        match &self.failure {
            PmPublicLaneFailure::Full { proof, .. }
            | PmPublicLaneFailure::DuplicateKey { proof, .. } => proof.lane_generation,
        }
    }

    pub(super) const fn lane_capacity(&self) -> usize {
        match &self.failure {
            PmPublicLaneFailure::Full { proof, .. }
            | PmPublicLaneFailure::DuplicateKey { proof, .. } => proof.lane_capacity,
        }
    }

    pub(super) const fn rejected_key(&self) -> PmServiceKey {
        match &self.failure {
            PmPublicLaneFailure::Full { proof, .. }
            | PmPublicLaneFailure::DuplicateKey { proof, .. } => proof.rejected_key,
        }
    }

    pub(super) fn into_authenticated(self) -> PmAuthenticatedPublicLaneFailure<D> {
        match self.failure {
            PmPublicLaneFailure::Full {
                delivery, action, ..
            } => PmAuthenticatedPublicLaneFailure::Full { delivery, action },
            PmPublicLaneFailure::DuplicateKey { delivery, .. } => {
                PmAuthenticatedPublicLaneFailure::DuplicateKey { delivery }
            }
        }
    }
}

/// A lane failure whose exact originating lane state has just been rechecked.
///
/// This is crate-private so only the active composition root can enact it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PmAuthenticatedPublicLaneFailure<D> {
    Full {
        delivery: D,
        action: SaturationAction,
    },
    DuplicateKey {
        delivery: D,
    },
}

/// Exact, non-consuming evidence for the oldest received delivery when an age
/// policy fails.
///
/// The hidden lane authority and generation bind this move-only evidence to
/// the exact current head of one Run-owned public-lane instance.
#[derive(Debug, PartialEq, Eq)]
pub struct PmAgedDeliveryEvidence {
    key: PmServiceKey,
    connection: PmConnectionId,
    ordering: EventOrdering,
    received_clock: ReceivedEventClock,
    observed_now_ns: u64,
    lane_authority: PmLaneAuthorityId,
    lane_generation: u64,
    public_route: PmPublicRouteLaneEvidence,
}

impl PmAgedDeliveryEvidence {
    #[allow(clippy::too_many_arguments)]
    pub(super) const fn new(
        key: PmServiceKey,
        connection: PmConnectionId,
        ordering: EventOrdering,
        received_clock: ReceivedEventClock,
        observed_now_ns: u64,
        lane_authority: PmLaneAuthorityId,
        lane_generation: u64,
        public_route: PmPublicRouteLaneEvidence,
    ) -> Self {
        Self {
            key,
            connection,
            ordering,
            received_clock,
            observed_now_ns,
            lane_authority,
            lane_generation,
            public_route,
        }
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
    pub const fn received_clock(&self) -> ReceivedEventClock {
        self.received_clock
    }

    #[must_use]
    pub const fn observed_now_ns(&self) -> u64 {
        self.observed_now_ns
    }

    pub(super) const fn lane_authority(&self) -> PmLaneAuthorityId {
        self.lane_authority
    }

    pub(super) const fn lane_generation(&self) -> u64 {
        self.lane_generation
    }

    pub(crate) const fn public_authority_id(&self) -> PmPublicRouteAuthorityId {
        self.public_route.authority_id
    }

    pub(crate) const fn public_source(&self) -> PmProductSource {
        self.public_route.source
    }

    pub(crate) const fn public_head(&self) -> PmPublicAgedHead {
        self.public_route.head
    }

    pub(super) const fn public_route(&self) -> PmPublicRouteLaneEvidence {
        self.public_route
    }
}

/// Move-only age-policy failure. Its fields have no public constructor.
#[derive(Debug, PartialEq, Eq, Error)]
#[error("lane contains work older than its maximum admitted age")]
pub struct PmAgedLaneFailure {
    lane: PmLaneKind,
    action: SaturationAction,
    evidence: PmAgedDeliveryEvidence,
}

impl PmAgedLaneFailure {
    pub(super) const fn new(
        lane: PmLaneKind,
        action: SaturationAction,
        evidence: PmAgedDeliveryEvidence,
    ) -> Self {
        Self {
            lane,
            action,
            evidence,
        }
    }

    #[must_use]
    pub const fn lane(&self) -> PmLaneKind {
        self.lane
    }

    #[must_use]
    pub const fn action(&self) -> SaturationAction {
        self.action
    }

    #[must_use]
    pub const fn evidence(&self) -> &PmAgedDeliveryEvidence {
        &self.evidence
    }

    pub(super) fn into_evidence(self) -> PmAgedDeliveryEvidence {
        self.evidence
    }
}

#[allow(
    clippy::large_enum_variant,
    reason = "aged lane authority stays inline and allocation-free on the fail-closed owner path"
)]
#[derive(Debug, PartialEq, Eq, Error)]
pub enum PmServiceTurnError {
    #[error(transparent)]
    Aged(PmAgedLaneFailure),
    #[error("transport delivery clock is invalid at service")]
    DeliveryClock(DeliveryClockError),
    #[error("PM received clock is invalid at service")]
    EventClock(reap_pm_core::EnvelopeError),
    #[error("public lane service requires one active Run with no pending lane fault")]
    PublicRunUnavailable,
    #[error("a public consumer unwound during exact occurrence transfer")]
    ConsumerTransferPoisoned,
}

impl PmServiceTurnError {
    pub(super) const fn aged(
        lane: PmLaneKind,
        action: SaturationAction,
        evidence: PmAgedDeliveryEvidence,
    ) -> Self {
        Self::Aged(PmAgedLaneFailure::new(lane, action, evidence))
    }

    pub(super) const fn aged_failure(&self) -> Option<&PmAgedLaneFailure> {
        match self {
            Self::Aged(failure) => Some(failure),
            _ => None,
        }
    }

    pub(crate) const fn public_aged_head(&self) -> Option<PmPublicAgedHead> {
        match self {
            Self::Aged(failure) => Some(failure.evidence().public_head()),
            _ => None,
        }
    }

    pub(crate) const fn public_aged_evidence(&self) -> Option<&PmAgedDeliveryEvidence> {
        match self {
            Self::Aged(failure) => Some(failure.evidence()),
            _ => None,
        }
    }

    pub(super) fn into_aged_evidence(self) -> Option<PmAgedDeliveryEvidence> {
        match self {
            Self::Aged(failure) => Some(failure.into_evidence()),
            _ => None,
        }
    }
}
