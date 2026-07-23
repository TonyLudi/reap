use reap_pm_state::{PmExternalBookFault, PmPublicReadinessReason};
use reap_polymarket_adapter::PmPublicSessionFault;
use thiserror::Error;

use crate::capture_roles::PmPublicLaneFaultError;
use crate::lanes::{
    PmAgedDeliveryEvidence, PmPublicLaneEnqueueError, PmServiceTurnError, SaturationAction,
};
use crate::public_routes::{OkxPublicUnavailableDelivery, PmPublicUnavailableDelivery};

/// Exact evidence retained after the active run fail-closes a venue because a
/// route-issued public delivery could not enter the bounded public lane.
#[derive(Debug)]
pub struct PmPublicLaneFaultEnactment<U> {
    pub(super) rejected_ordering: reap_pm_core::EventOrdering,
    pub(super) unavailable_fault: U,
    pub(super) reducer_reason: Option<PmPublicReadinessReason>,
    pub(super) purged_queued_deliveries: usize,
}

impl<U: Copy> PmPublicLaneFaultEnactment<U> {
    #[must_use]
    pub const fn rejected_ordering(&self) -> reap_pm_core::EventOrdering {
        self.rejected_ordering
    }

    #[must_use]
    pub const fn unavailable_fault(&self) -> U {
        self.unavailable_fault
    }

    #[must_use]
    pub const fn reducer_reason(&self) -> Option<PmPublicReadinessReason> {
        self.reducer_reason
    }

    #[must_use]
    pub const fn purged_queued_deliveries(&self) -> usize {
        self.purged_queued_deliveries
    }

    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        reap_pm_core::EventOrdering,
        U,
        Option<PmPublicReadinessReason>,
        usize,
    ) {
        (
            self.rejected_ordering,
            self.unavailable_fault,
            self.reducer_reason,
            self.purged_queued_deliveries,
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PmPublicLaneAdmissionError<D> {
    RunTerminal { delivery: D },
    PendingPmBookAuthority { delivery: D },
    RouteAuthorityMismatch { delivery: D },
    RouteScopeMismatch { delivery: D },
    Lane(PmPublicLaneEnqueueError<D>),
}

impl<D> PmPublicLaneAdmissionError<D> {
    #[must_use]
    pub const fn delivery(&self) -> &D {
        match self {
            Self::RunTerminal { delivery }
            | Self::PendingPmBookAuthority { delivery }
            | Self::RouteAuthorityMismatch { delivery }
            | Self::RouteScopeMismatch { delivery } => delivery,
            Self::Lane(failure) => failure.delivery(),
        }
    }

    #[must_use]
    pub fn into_delivery(self) -> D {
        match self {
            Self::RunTerminal { delivery }
            | Self::PendingPmBookAuthority { delivery }
            | Self::RouteAuthorityMismatch { delivery }
            | Self::RouteScopeMismatch { delivery } => delivery,
            Self::Lane(failure) => failure.into_delivery(),
        }
    }
}

#[derive(Debug, Error)]
pub enum PmPublicLaneEnactError<D> {
    #[error("terminal capture run rejected the exact public lane admission failure")]
    RunTerminal {
        failure: PmPublicLaneAdmissionError<D>,
    },
    #[error("public PM book lane failure is not the exact pending reduced delivery")]
    PendingBookFaultMismatch {
        failure: PmPublicLaneAdmissionError<D>,
    },
    #[error("an exact reduced PM book Full proof must be enacted before another PM lane fault")]
    PendingBookFaultBlocksMutation {
        failure: PmPublicLaneAdmissionError<D>,
    },
    #[error("pending PM book reducer obligations block this unrelated lane mutation")]
    PendingBookReductionBlocksMutation {
        failure: PmPublicLaneAdmissionError<D>,
    },
    #[error("public delivery belongs to a sibling capture authority")]
    RouteAuthorityMismatch { delivery: D },
    #[error("public delivery does not match the active venue source, connection, and epoch")]
    RouteScopeMismatch { delivery: D },
    #[error("duplicate public lane key is not a capacity fault")]
    DuplicateKey { delivery: D },
    #[error("public lane state no longer authenticates the exact admission failure")]
    LaneStateMismatch {
        failure: PmPublicLaneAdmissionError<D>,
    },
    #[error("PM tick-size change terminalized the capture run")]
    TickSizeChanged {
        delivery: D,
        action: SaturationAction,
        old: reap_pm_core::PmTick,
        new: reap_pm_core::PmTick,
        purged_queued_deliveries: usize,
        terminal_pm_unavailable: Option<PmPublicUnavailableDelivery>,
        terminal_okx_unavailable: Option<OkxPublicUnavailableDelivery>,
    },
    #[error("public lane failure did not require stream invalidation and resync")]
    UnexpectedAction {
        delivery: D,
        action: SaturationAction,
    },
    #[error("must-deliver public notification could not enter the bounded lane: {failure}")]
    NotificationAdmission {
        delivery: D,
        failure: super::PmPublicNotificationAdmissionFailure,
    },
    #[error("rejected must-deliver notification terminalized admission: {failure}")]
    NotificationAdmissionTerminal {
        failure: super::PmPublicNotificationAdmissionFailure,
    },
    #[error("public lane failure occurred outside the live lifecycle phase")]
    InvalidLifecyclePhase {
        delivery: D,
        action: SaturationAction,
    },
    #[error("capture could not record the public lane disconnect before invalidation: {source}")]
    LifecycleWrite {
        delivery: D,
        action: SaturationAction,
        #[source]
        source: crate::capture::PmCaptureWriteError,
        purged_queued_deliveries: usize,
        terminal_pm_unavailable: Option<PmPublicUnavailableDelivery>,
        terminal_okx_unavailable: Option<OkxPublicUnavailableDelivery>,
    },
    #[error("active run could not enact the public lane fault: {source}")]
    Fault {
        delivery: D,
        #[source]
        source: PmPublicLaneFaultError,
        purged_queued_deliveries: usize,
        terminal_pm_unavailable: Option<PmPublicUnavailableDelivery>,
        terminal_okx_unavailable: Option<OkxPublicUnavailableDelivery>,
    },
}

#[derive(Debug)]
pub enum PmPublicAgedLaneFaultEnactment {
    Polymarket {
        unavailable_fault: PmPublicSessionFault,
        reducer_reason: PmPublicReadinessReason,
        purged_queued_deliveries: usize,
    },
    Okx {
        unavailable_fault: reap_okx_public_source::OkxPublicSessionFault,
        purged_queued_deliveries: usize,
    },
}

#[derive(Debug, Error)]
pub enum PmPublicAgedLaneEnactError {
    #[error("terminal capture run rejected the exact aged-lane failure")]
    RunTerminal { failure: PmServiceTurnError },
    #[error("an exact reduced PM book Full proof must be enacted before aged-lane recovery")]
    PendingBookFault { failure: PmServiceTurnError },
    #[error("pending PM book reducer obligations block unrelated OKX aged-lane recovery")]
    PendingBookReduction { failure: PmServiceTurnError },
    #[error("service failure is not an evidenced public-lane age fault requiring resync")]
    InvalidFailure { failure: PmServiceTurnError },
    #[error("aged public-lane evidence does not match a configured public venue")]
    EvidenceMismatch { evidence: PmAgedDeliveryEvidence },
    #[error("must-deliver aged-lane notification could not enter the bounded lane: {failure}")]
    NotificationAdmission {
        evidence: PmAgedDeliveryEvidence,
        failure: super::PmPublicNotificationAdmissionFailure,
    },
    #[error("aged public lane failure occurred outside the expected lifecycle phase")]
    InvalidLifecyclePhase { evidence: PmAgedDeliveryEvidence },
    #[error(
        "capture could not record the aged public lane disconnect before invalidation: {source}"
    )]
    LifecycleWrite {
        evidence: PmAgedDeliveryEvidence,
        #[source]
        source: crate::capture::PmCaptureWriteError,
        purged_queued_deliveries: usize,
        terminal_pm_unavailable: Option<PmPublicUnavailableDelivery>,
        terminal_okx_unavailable: Option<OkxPublicUnavailableDelivery>,
    },
    #[error("aged PM tick-size change terminalized the capture run")]
    TickSizeChanged {
        evidence: PmAgedDeliveryEvidence,
        old: reap_pm_core::PmTick,
        new: reap_pm_core::PmTick,
        purged_queued_deliveries: usize,
        terminal_pm_unavailable: Option<PmPublicUnavailableDelivery>,
        terminal_okx_unavailable: Option<OkxPublicUnavailableDelivery>,
    },
    #[error("active run could not enact the aged public-lane fault: {source}")]
    Fault {
        evidence: PmAgedDeliveryEvidence,
        #[source]
        source: PmPublicLaneFaultError,
        purged_queued_deliveries: usize,
        terminal_pm_unavailable: Option<PmPublicUnavailableDelivery>,
        terminal_okx_unavailable: Option<OkxPublicUnavailableDelivery>,
    },
}

pub(super) const fn pm_unavailable_reducer_fault(
    fault: PmPublicSessionFault,
) -> (PmExternalBookFault, PmPublicReadinessReason) {
    match fault {
        PmPublicSessionFault::Disconnect => (
            PmExternalBookFault::Disconnect,
            PmPublicReadinessReason::Disconnected,
        ),
        PmPublicSessionFault::Gap => (PmExternalBookFault::Gap, PmPublicReadinessReason::Gap),
        PmPublicSessionFault::Overflow => (
            PmExternalBookFault::Overflow,
            PmPublicReadinessReason::Overflow,
        ),
        PmPublicSessionFault::Stale => (
            PmExternalBookFault::BacklogAged,
            PmPublicReadinessReason::BookStale,
        ),
        PmPublicSessionFault::HeartbeatTimeout => (
            PmExternalBookFault::HeartbeatTimeout,
            PmPublicReadinessReason::HeartbeatTimeout,
        ),
        PmPublicSessionFault::HashMismatch => (
            PmExternalBookFault::HashMismatch,
            PmPublicReadinessReason::HashMismatch,
        ),
        PmPublicSessionFault::InvalidTransition
        | PmPublicSessionFault::TickSizeChanged
        | PmPublicSessionFault::ReducerRejected => (
            PmExternalBookFault::InvalidTransition,
            PmPublicReadinessReason::InvalidTransition,
        ),
    }
}
