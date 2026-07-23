use super::*;

/// Complete active-Run reason that the PM book cannot currently authorize a
/// quote.
///
/// This is intentionally broader than reducer readiness: a reducer may retain
/// a valid last book while the Run still owes an exact reduction or lane-fault
/// enactment. Those obligations remain unavailable here until discharged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmPublicBookReadinessReason {
    ArtifactTerminal,
    ConsumerTransferPoisoned,
    LifecycleUnavailable,
    PendingReduction,
    PendingLaneFault,
    Reducer(PmPublicReadinessReason),
    SessionReducerMismatch,
}

/// Copied composite readiness for the PM book owned by one active Run.
///
/// Revisions remain visible for diagnosis while `is_ready()` is the only
/// positive readiness signal. Callers that need levels must additionally
/// obtain [`PmPublicReadyBookView`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPublicBookReadiness {
    reason: Option<PmPublicBookReadinessReason>,
    metadata_revision: Option<reap_pm_core::SnapshotRevision>,
    snapshot_revision: Option<reap_pm_core::SnapshotRevision>,
}

impl PmPublicBookReadiness {
    pub(super) const fn ready(reducer: &PmBookReducer) -> Self {
        let readiness = reducer.readiness();
        Self {
            reason: None,
            metadata_revision: readiness.metadata_revision(),
            snapshot_revision: readiness.snapshot_revision(),
        }
    }

    pub(super) const fn unavailable(
        reducer: &PmBookReducer,
        reason: PmPublicBookReadinessReason,
    ) -> Self {
        let readiness = reducer.readiness();
        Self {
            reason: Some(reason),
            metadata_revision: readiness.metadata_revision(),
            snapshot_revision: readiness.snapshot_revision(),
        }
    }

    #[must_use]
    pub const fn is_ready(self) -> bool {
        self.reason.is_none()
    }

    #[must_use]
    pub const fn reason(self) -> Option<PmPublicBookReadinessReason> {
        self.reason
    }

    #[must_use]
    pub const fn metadata_revision(self) -> Option<reap_pm_core::SnapshotRevision> {
        self.metadata_revision
    }

    #[must_use]
    pub const fn snapshot_revision(self) -> Option<reap_pm_core::SnapshotRevision> {
        self.snapshot_revision
    }
}

/// Borrowed PM book state that is available only when the complete active-Run
/// authority is quote-ready.
///
/// The view cannot outlive or be mutated independently of its Run. Pending
/// reducer obligations, lane faults, disconnects, and terminal state suppress
/// construction even if the reducer's last committed snapshot remains
/// internally intact.
#[derive(Debug, Clone, Copy)]
pub struct PmPublicReadyBookView<'a> {
    levels: &'a [PmBookLevel],
    readiness: PmPublicBookReadiness,
    connection_epoch: reap_pm_core::ConnectionEpoch,
    last_ingress_sequence: Option<IngressSequence>,
    last_verified_snapshot_hash: Option<reap_pm_core::VenueEventHash>,
}

impl<'a> PmPublicReadyBookView<'a> {
    pub(super) fn new(
        reducer: &'a PmBookReducer,
        readiness: PmPublicBookReadiness,
    ) -> Option<Self> {
        if !readiness.is_ready() {
            return None;
        }
        Some(Self {
            levels: reducer.levels(),
            readiness,
            connection_epoch: reducer.connection_epoch()?,
            last_ingress_sequence: reducer.last_ingress_sequence(),
            last_verified_snapshot_hash: reducer.last_verified_snapshot_hash(),
        })
    }

    #[must_use]
    pub const fn levels(self) -> &'a [PmBookLevel] {
        self.levels
    }

    #[must_use]
    pub const fn readiness(self) -> PmPublicBookReadiness {
        self.readiness
    }

    #[must_use]
    pub const fn connection_epoch(self) -> reap_pm_core::ConnectionEpoch {
        self.connection_epoch
    }

    #[must_use]
    pub const fn last_ingress_sequence(self) -> Option<IngressSequence> {
        self.last_ingress_sequence
    }

    #[must_use]
    pub const fn last_verified_snapshot_hash(self) -> Option<reap_pm_core::VenueEventHash> {
        self.last_verified_snapshot_hash
    }
}

#[derive(Debug)]
pub struct PmPublicCaptureOutcome {
    pub(super) path: PathBuf,
    pub(super) header: PmCaptureHeader,
    pub(super) writer_max_queue_bytes: Option<usize>,
    pub(super) writer_max_reserved_bytes: Option<usize>,
    pub(super) verification: PmCaptureVerification,
    pub(super) projection: PmReplayProjection,
}

/// Failure from the reducer-first PM book pipeline.
///
/// Reducer failures retain the Run-level terminal evidence. Lane admission
/// failures retain the exact already-reduced move-only delivery so callers can
/// enact the authenticated bounded-lane fault.
#[derive(Debug)]
pub enum PmPublicBookPipelineError {
    Reduce(PmPublicCaptureRunError),
    Lane(PmPublicLaneAdmissionError<PmPublicBookDelivery>),
}

impl PmPublicBookPipelineError {
    #[must_use]
    pub const fn lane_failure(&self) -> Option<&PmPublicLaneAdmissionError<PmPublicBookDelivery>> {
        match self {
            Self::Reduce(_) => None,
            Self::Lane(failure) => Some(failure),
        }
    }

    #[must_use]
    pub fn into_lane_failure(self) -> Option<PmPublicLaneAdmissionError<PmPublicBookDelivery>> {
        match self {
            Self::Reduce(_) => None,
            Self::Lane(failure) => Some(failure),
        }
    }
}

/// Failure from an atomic route-issue/classify plus bounded-lane admission.
///
/// A lane failure retains the exact move-only data delivery and the active
/// Run retains the matching Full latch. Successful calls never expose an
/// unqueued route delivery.
#[allow(
    clippy::large_enum_variant,
    reason = "the atomic owner path returns exact inline route and terminal evidence without allocation"
)]
#[derive(Debug)]
pub enum PmPublicDataPipelineError<D> {
    Run(PmPublicCaptureRunError),
    Lane(PmPublicLaneAdmissionError<D>),
}

impl<D> PmPublicDataPipelineError<D> {
    #[must_use]
    pub const fn lane_failure(&self) -> Option<&PmPublicLaneAdmissionError<D>> {
        match self {
            Self::Run(_) => None,
            Self::Lane(failure) => Some(failure),
        }
    }

    #[must_use]
    pub fn into_lane_failure(self) -> Option<PmPublicLaneAdmissionError<D>> {
        match self {
            Self::Run(_) => None,
            Self::Lane(failure) => Some(failure),
        }
    }

    #[must_use]
    pub const fn run_error(&self) -> Option<&PmPublicCaptureRunError> {
        match self {
            Self::Run(source) => Some(source),
            Self::Lane(_) => None,
        }
    }

    #[must_use]
    pub fn into_run_error(self) -> Option<PmPublicCaptureRunError> {
        match self {
            Self::Run(source) => Some(source),
            Self::Lane(_) => None,
        }
    }
}

impl<D> From<PmPublicCaptureRunError> for PmPublicDataPipelineError<D> {
    fn from(source: PmPublicCaptureRunError) -> Self {
        Self::Run(source)
    }
}

/// Copied terminal fact for a must-deliver public notification that could not
/// enter the bounded lane even after its exact invalidated route was purged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPublicNotificationAdmissionFailure {
    #[error("Polymarket public notification {fault:?} could not enter the bounded lane")]
    Polymarket {
        fault: reap_polymarket_adapter::PmPublicSessionFault,
    },
    #[error("OKX public notification {fault:?} could not enter the bounded lane")]
    Okx {
        fault: reap_okx_public_source::OkxPublicSessionFault,
    },
}

impl PmPublicCaptureOutcome {
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    #[must_use]
    pub const fn header(&self) -> &PmCaptureHeader {
        &self.header
    }

    #[must_use]
    pub const fn writer_max_queue_bytes(&self) -> Option<usize> {
        self.writer_max_queue_bytes
    }

    #[must_use]
    pub const fn writer_max_reserved_bytes(&self) -> Option<usize> {
        self.writer_max_reserved_bytes
    }

    #[must_use]
    pub const fn verification(&self) -> &PmCaptureVerification {
        &self.verification
    }

    #[must_use]
    pub const fn projection(&self) -> &PmReplayProjection {
        &self.projection
    }
}

/// First fail-closed cause retained by one active public capture root.
///
/// The cause is deliberately coarse and bounded: detailed source errors are
/// still returned by the operation that failed, while every later mutation
/// and terminal finish reports why the shared artifact was first sealed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmPublicCaptureTerminalCause {
    #[error("capture framing or writer failure")]
    CaptureWriter,
    #[error("public ingress, session, or classification failure")]
    IngressSessionClassification,
    #[error("public route authentication failure")]
    Route,
    #[error("capture lifecycle failure")]
    Lifecycle,
    #[error("snapshot or reducer failure")]
    SnapshotReducer,
    #[error("bounded public lane failure")]
    Lane,
    #[error("Polymarket tick-size change")]
    TickSizeChanged,
    #[error("internal invariant failure")]
    InternalInvariant,
}

#[derive(Debug, Error)]
pub enum PmPublicCaptureRunError {
    #[error(
        "capture artifact is terminal after {cause} and must be rotated before any further mutation"
    )]
    ArtifactTerminal { cause: PmPublicCaptureTerminalCause },
    #[error("active capture lifecycle method was called in the wrong venue phase")]
    InvalidLifecyclePhase,
    #[error(transparent)]
    Plan(#[from] PmPlanError),
    #[error(transparent)]
    Header(#[from] PmCaptureVerifyError),
    #[error(transparent)]
    Write(#[from] PmCaptureWriteError),
    #[error(transparent)]
    PmSession(#[from] PmPublicSessionError),
    #[error(transparent)]
    OkxSession(#[from] OkxPublicSessionError),
    #[error(transparent)]
    Route(#[from] PmPublicRouteError),
    #[error(transparent)]
    Replay(#[from] PmReplayError),
    #[error("PM raw capture rejected before classification: {source}")]
    PmCaptureRejected {
        #[source]
        source: PmCaptureWriteError,
        unavailable: Option<PmPublicUnavailableDelivery>,
    },
    #[error("PM captured frame classification failed: {source}")]
    PmClassify {
        #[source]
        source: PmPublicSessionError,
        unavailable: Option<PmPublicUnavailableDelivery>,
    },
    #[error("PM public heartbeat transition failed: {source}")]
    PmHeartbeat {
        #[source]
        source: PmPublicSessionError,
        unavailable: Option<PmPublicUnavailableDelivery>,
    },
    #[error("PM routed snapshot could not be atomically committed: {source}")]
    PmSnapshotCommit {
        #[source]
        source: PmPublicSnapshotCommitError,
        unavailable: Option<PmPublicUnavailableDelivery>,
    },
    #[error("terminal capture run rejected the exact routed PM snapshot and flow")]
    PmSnapshotCommitRunTerminal {
        delivery: PmPublicBookDelivery,
        flow: PmPublicSnapshotFlow,
    },
    #[error("pending PM book Full enactment rejected the exact routed snapshot and flow")]
    PmSnapshotCommitPendingLaneFault {
        delivery: PmPublicBookDelivery,
        flow: PmPublicSnapshotFlow,
    },
    #[error("capture lifecycle rejected the exact routed PM snapshot and flow")]
    PmSnapshotCommitInvalidPhase {
        delivery: PmPublicBookDelivery,
        flow: PmPublicSnapshotFlow,
    },
    #[error("PM routed delta/top update could not be atomically reduced: {source}")]
    PmBookReduce {
        #[source]
        source: PmPublicBookReduceError,
        unavailable: Option<PmPublicUnavailableDelivery>,
    },
    #[error("terminal capture run rejected the exact routed PM delta/top update")]
    PmBookReduceRunTerminal { delivery: PmPublicBookDelivery },
    #[error("pending PM book Full enactment rejected the exact routed PM delta/top update")]
    PmBookReducePendingLaneFault { delivery: PmPublicBookDelivery },
    #[error("capture lifecycle rejected the exact routed PM delta/top update")]
    PmBookReduceInvalidPhase { delivery: PmPublicBookDelivery },
    #[error("PM session and reducer lifecycle could not be atomically synchronized: {source}")]
    PmReducerSync {
        #[source]
        source: PmPublicReducerSyncError,
        unavailable: Option<PmPublicUnavailableDelivery>,
    },
    #[error("OKX raw capture rejected before classification: {source}")]
    OkxCaptureRejected {
        #[source]
        source: PmCaptureWriteError,
        unavailable: Option<OkxPublicUnavailableDelivery>,
    },
    #[error("OKX captured frame classification failed: {source}")]
    OkxClassify {
        #[source]
        source: OkxPublicSessionError,
        unavailable: Option<OkxPublicUnavailableDelivery>,
    },
    #[error("captured OKX public payload is not UTF-8 text")]
    OkxRawNotUtf8 {
        unavailable: Option<OkxPublicUnavailableDelivery>,
    },
    #[error("raw capture ingress counter overflowed")]
    RawIngressOverflow,
    #[error("session fault did not produce its required unavailable occurrence")]
    MissingUnavailableOccurrence,
    #[error("reconnect delay exceeded the capture schema")]
    ReconnectDelayOverflow,
    #[error("session reconnect transition differed from its pure preview")]
    ReconnectTransitionMismatch,
    #[error("a mandatory public consumer unwound during exact occurrence transfer")]
    PublicConsumerTransferPoisoned,
    #[error(transparent)]
    NotificationAdmission(#[from] PmPublicNotificationAdmissionFailure),
    #[error(
        "PM reconnect is blocked by {pending} queued public delivery occurrence(s) from epoch {epoch}"
    )]
    PendingPmPublicRouteReconnect { epoch: u64, pending: usize },
    #[error(
        "OKX reconnect is blocked by {pending} queued public delivery occurrence(s) from epoch {epoch}"
    )]
    PendingOkxPublicRouteReconnect { epoch: u64, pending: usize },
    #[error("PM heartbeat ping was recorded before the session deadline")]
    HeartbeatPingNotDue,
    #[error("PM heartbeat enactment differed from its immutable preview")]
    HeartbeatTransitionMismatch,
    #[error("capture role returned an event for the wrong venue")]
    InternalRoleMismatch,
    #[error("{pending} route-issued PM book reduction obligation(s) remain pending")]
    PendingPmBookReductions { pending: usize },
    #[error("an already-reduced PM book lane rejection still requires exact fault enactment")]
    PendingPmBookLaneFault,
    #[error("captured PM frame exceeded the bounded reducer-obligation capacity")]
    PendingPmBookReductionOverflow,
    #[error("routed PM snapshot did not match the next exact reducer obligation")]
    PmSnapshotReductionOrderMismatch {
        delivery: PmPublicBookDelivery,
        flow: PmPublicSnapshotFlow,
    },
    #[error("routed PM delta/top update did not match the next exact reducer obligation")]
    PmBookReductionOrderMismatch { delivery: PmPublicBookDelivery },
    #[error("capture writer shutdown failed: {0}")]
    WriterShutdown(JsonlWriterError),
    #[error(
        "capture finished with {pending} unconsumed PM reducer obligation(s) and shutdown error: {shutdown_error:?}"
    )]
    PendingPmBookReductionFinish {
        pending: usize,
        shutdown_error: Option<PmPublicCaptureShutdownError>,
    },
    #[error(
        "capture finished with an unenacted PM book lane fault; shutdown error: {shutdown_error:?}"
    )]
    PendingPmBookLaneFaultFinish {
        shutdown_error: Option<PmPublicCaptureShutdownError>,
    },
    #[error(
        "capture finished with {pending} queued public delivery obligation(s); shutdown error: {shutdown_error:?}"
    )]
    QueuedPublicLaneFinish {
        pending: usize,
        shutdown_error: Option<PmPublicCaptureShutdownError>,
    },
    #[error(
        "terminal capture could not admit a must-deliver public notification ({failure}); shutdown error: {shutdown_error:?}"
    )]
    NotificationAdmissionTerminalFinish {
        failure: PmPublicNotificationAdmissionFailure,
        shutdown_error: Option<PmPublicCaptureShutdownError>,
    },
    #[error(
        "capture finished after a mandatory public consumer unwound; shutdown error: {shutdown_error:?}"
    )]
    PublicConsumerTransferPoisonedFinish {
        shutdown_error: Option<PmPublicCaptureShutdownError>,
    },
    #[error(
        "terminal tick capture finished before exact product-state cleanup ({cleanup_status:?}); shutdown error: {shutdown_error:?}"
    )]
    TerminalTickCleanupIncomplete {
        cleanup_status: PmPublicTerminalTickCleanupStatus,
        shutdown_error: Option<PmPublicCaptureShutdownError>,
    },
    #[error("terminal capture run finished after {cause} with shutdown error: {shutdown_error:?}")]
    TerminalFinish {
        cause: PmPublicCaptureTerminalCause,
        shutdown_error: Option<PmPublicCaptureShutdownError>,
    },
    #[error("capture writer evidence differs from verified artifact evidence")]
    WriterEvidenceMismatch,
}

#[derive(Debug, Error)]
pub enum PmPublicCaptureShutdownError {
    #[error(transparent)]
    Capture(#[from] PmCaptureWriteError),
    #[error(transparent)]
    Writer(#[from] JsonlWriterError),
}

impl PmPublicCaptureRunError {
    pub(super) fn from_role_start(error: PmCaptureRoleStartError) -> Self {
        match error {
            PmCaptureRoleStartError::Header(source) => Self::Header(source),
            PmCaptureRoleStartError::PmSession(source) => Self::PmSession(source),
            PmCaptureRoleStartError::OkxSession(source) => Self::OkxSession(source),
            PmCaptureRoleStartError::MetadataContract(source) => {
                Self::Replay(PmReplayError::MetadataContract(source))
            }
            PmCaptureRoleStartError::Route(source) => Self::Route(source),
        }
    }

    #[must_use]
    pub fn pm_unavailable(&self) -> Option<&PmPublicUnavailableDelivery> {
        match self {
            Self::PmCaptureRejected { unavailable, .. }
            | Self::PmClassify { unavailable, .. }
            | Self::PmHeartbeat { unavailable, .. }
            | Self::PmSnapshotCommit { unavailable, .. }
            | Self::PmBookReduce { unavailable, .. }
            | Self::PmReducerSync { unavailable, .. } => unavailable.as_ref(),
            _ => None,
        }
    }

    #[must_use]
    pub fn okx_unavailable(&self) -> Option<&OkxPublicUnavailableDelivery> {
        match self {
            Self::OkxCaptureRejected { unavailable, .. }
            | Self::OkxClassify { unavailable, .. }
            | Self::OkxRawNotUtf8 { unavailable } => unavailable.as_ref(),
            _ => None,
        }
    }

    #[must_use]
    pub const fn snapshot_terminal_inputs(
        &self,
    ) -> Option<(&PmPublicBookDelivery, &PmPublicSnapshotFlow)> {
        match self {
            Self::PmSnapshotCommitRunTerminal { delivery, flow }
            | Self::PmSnapshotCommitInvalidPhase { delivery, flow } => Some((delivery, flow)),
            _ => None,
        }
    }

    #[must_use]
    pub const fn terminal_shutdown_error(&self) -> Option<&PmPublicCaptureShutdownError> {
        match self {
            Self::TerminalFinish {
                shutdown_error: Some(source),
                ..
            }
            | Self::PendingPmBookReductionFinish {
                shutdown_error: Some(source),
                ..
            }
            | Self::PendingPmBookLaneFaultFinish {
                shutdown_error: Some(source),
            }
            | Self::QueuedPublicLaneFinish {
                shutdown_error: Some(source),
                ..
            }
            | Self::NotificationAdmissionTerminalFinish {
                shutdown_error: Some(source),
                ..
            }
            | Self::PublicConsumerTransferPoisonedFinish {
                shutdown_error: Some(source),
            }
            | Self::TerminalTickCleanupIncomplete {
                shutdown_error: Some(source),
                ..
            } => Some(source),
            _ => None,
        }
    }

    #[must_use]
    pub const fn terminal_cause(&self) -> Option<PmPublicCaptureTerminalCause> {
        match self {
            Self::ArtifactTerminal { cause } | Self::TerminalFinish { cause, .. } => Some(*cause),
            Self::TerminalTickCleanupIncomplete { .. } => {
                Some(PmPublicCaptureTerminalCause::TickSizeChanged)
            }
            _ => None,
        }
    }
}
