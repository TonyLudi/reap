#![allow(
    clippy::result_large_err,
    reason = "the active run and lane boundaries return exact move-only routed and terminal evidence without heap allocation"
)]

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use reap_capture_framing::JsonlWriterError;
use reap_okx_public_source::OkxPublicSessionError;
use reap_pm_core::{
    EventOrdering, IngressSequence, PmBookLevel, PmConnectionId, PmMetadataError, PmProductSource,
    ReceivedEventClock,
};
use reap_pm_live_contracts::{
    ConstructedRoleBinding, PmConnectionRoute, PmConnectivityPlan, PmPlanError,
    PmPublicConnectivityConfig, PmRoleKind,
};
use reap_pm_state::{
    PmBookCounters, PmBookReducer, PmBookTransition, PmDomainFingerprint, PmExternalBookFault,
    PmMetadataContract, PmMetadataFingerprint, PmMetadataObservation,
    PmPendingExternalBookFaultAuthority, PmPrivateConfigError, PmPrivateStateError,
    PmPublicReadinessReason,
};
use reap_polymarket_adapter::{
    PmAccountPositionRoleError, PmPrivateLifecycleRoleError, PmPublicHeartbeatAction,
    PmPublicRoleError, PmPublicSessionError, PmReconciliationContractError,
};
use thiserror::Error;

use crate::capture::{
    OkxCaptureDisconnectReason, OkxCaptureLifecycle, PmCaptureDisconnectReason, PmCaptureHeader,
    PmCaptureLifecycle, PmCaptureProvenance, PmCaptureScope, PmCaptureSessionPolicy,
    PmCaptureVerification, PmCaptureVerifyError, PmCaptureWriteError, PmPublicCaptureWriter,
    verify_pm_public_capture,
};
use crate::capture_roles::{
    OkxCaptureRoleEvent, OkxPublicCaptureEvent, PmCaptureBlueprint, PmCaptureBookReduceFailure,
    PmCaptureRoleIngressError, PmCaptureRoleStartError, PmCaptureRoles,
    PmCaptureSnapshotCommitFailure, PmPublicBookReduceError, PmPublicCaptureBatch,
    PmPublicFreshnessTimerOutcome, PmPublicReducerSyncError, PmPublicSnapshotCommitError,
    PmPublicSnapshotFlow,
};
use crate::lanes::{
    PmAgedDeliveryEvidence, PmAuthenticatedPublicLaneFailure, PmLaneMetrics, PmPublicAgedHead,
    PmPublicLaneService, PmPublicLaneState, PmServiceTurnError, SaturationAction,
};
use crate::public_routes::{
    OkxPublicReferenceDelivery, OkxPublicUnavailableDelivery, PmPublicBookDelivery,
    PmPublicMetadataDelivery, PmPublicRouteAuthorityId, PmPublicRouteError,
    PmPublicUnavailableDelivery,
};
use crate::replay::{PmReplayError, PmReplayProjection, replay_pm_public_capture};

mod lane_enact;
mod product;
mod run_capture;
mod run_lane_aged;
mod run_lane_full;
mod run_lane_service;
mod run_lifecycle;
mod run_reduce;
mod run_state;
mod run_terminal_tick;
mod run_types;

use lane_enact::pm_unavailable_reducer_fault;
pub use lane_enact::{
    PmPublicAgedLaneEnactError, PmPublicAgedLaneFaultEnactment, PmPublicLaneAdmissionError,
    PmPublicLaneEnactError, PmPublicLaneFaultEnactment,
};
pub use product::{
    PmProduct, PmProductPublicAgedEnactError, PmProductPublicAgedRetryReason,
    PmProductPublicIngress, PmProductPublicIngressError, PmProductPublicIngressOutcome,
    PmProductRun, PmProductRunError, PmProductStartError,
};
use run_state::{
    MAX_PENDING_PM_BOOK_REDUCTIONS, PendingOtherAgedLaneFault, PendingPmAgedLaneFault,
    PendingPmBookKind, PendingPmBookLaneFault, PendingPmBookReduction, PendingPmMetadataLaneFault,
    PendingPmRouteFaultIdentity, PublicLifecyclePhase,
};
pub use run_terminal_tick::{PmPublicTerminalTickApplyError, PmPublicTerminalTickCleanupStatus};
pub use run_types::{
    PmPublicBookPipelineError, PmPublicBookReadiness, PmPublicBookReadinessReason,
    PmPublicCaptureOutcome, PmPublicCaptureRunError, PmPublicCaptureShutdownError,
    PmPublicCaptureTerminalCause, PmPublicDataPipelineError, PmPublicNotificationAdmissionFailure,
    PmPublicReadyBookView,
};

#[derive(Debug, Error)]
pub enum PmCompositionError {
    #[error(transparent)]
    Plan(#[from] PmPlanError),
    #[error(transparent)]
    AccountRole(#[from] PmAccountPositionRoleError),
    #[error(transparent)]
    PublicRole(#[from] PmPublicRoleError),
    #[error(transparent)]
    PrivateRole(#[from] PmPrivateLifecycleRoleError),
    #[error(transparent)]
    ReconciliationRole(#[from] PmReconciliationContractError),
    #[error(transparent)]
    InstrumentScope(#[from] PmMetadataError),
    #[error(transparent)]
    PrivateConfig(#[from] PmPrivateConfigError),
    #[error(transparent)]
    PrivateState(#[from] PmPrivateStateError),
}

#[derive(Debug)]
pub struct PmPublicCapture {
    plan: PmConnectivityPlan,
    bindings: Vec<ConstructedRoleBinding>,
    capture: PmCaptureBlueprint,
}

impl PmPublicCapture {
    pub fn new(config: PmPublicConnectivityConfig) -> Result<Self, PmCompositionError> {
        let plan = PmConnectivityPlan::public_capture(config)?;
        Self::from_plan(plan)
    }

    fn from_plan(plan: PmConnectivityPlan) -> Result<Self, PmCompositionError> {
        let config = plan
            .public_config()
            .expect("public plan carries public config");
        let capture = PmCaptureBlueprint::new(config)?;
        let bindings = capture.bindings(config);
        plan.validate_bindings(&bindings)?;
        Ok(Self {
            plan,
            bindings,
            capture,
        })
    }

    #[must_use]
    pub fn reached_roles(&self) -> &[PmRoleKind] {
        self.plan.reached_roles()
    }

    #[must_use]
    pub fn binding_count(&self) -> usize {
        let config = self
            .plan
            .public_config()
            .expect("public plan carries public config");
        debug_assert_eq!(self.capture.reference(config), config.okx_reference());
        debug_assert_eq!(self.capture.instrument(), config.instrument());
        self.bindings.len()
    }

    /// Starts the one credential-free public capture run authorized by this
    /// static plan.
    ///
    /// The returned owner keeps both venue sessions, route authority, writer,
    /// and raw ingress counters private so callers cannot substitute an epoch,
    /// local ingress sequence, or same-shaped session from another root.
    pub async fn start(
        self,
        path: PathBuf,
        authoritative: reap_polymarket_adapter::PmAuthoritativeMetadata,
        session_policy: PmCaptureSessionPolicy,
        provenance: PmCaptureProvenance,
    ) -> Result<PmPublicCaptureRun, PmPublicCaptureRunError> {
        let config = self
            .plan
            .public_config()
            .expect("public plan carries public config")
            .clone();
        let pm_reducer = prepare_pm_reducer(&config, &authoritative, session_policy)?;
        let scope = PmCaptureScope::new(&config, &authoritative)?;
        let header = PmCaptureHeader::new(scope, session_policy, provenance)?;
        let roles = PmCaptureRoles::start(self.capture, &config, authoritative, session_policy)
            .map_err(PmPublicCaptureRunError::from_role_start)?;
        self.plan.validate_bindings(&self.bindings)?;
        roles
            .validate_pm_reducer_start(&pm_reducer)
            .map_err(|source| PmPublicCaptureRunError::PmReducerSync {
                source,
                unavailable: None,
            })?;
        let writer = PmPublicCaptureWriter::start(path.clone(), header.clone()).await?;
        Ok(PmPublicCaptureRun {
            path,
            header,
            writer,
            roles,
            pm_route: config.polymarket_route(),
            okx_route: config.okx_route(),
            public_lane: PmPublicLaneState::new(),
            pm_raw_ingress: RawIngressCounter::default(),
            okx_raw_ingress: RawIngressCounter::default(),
            terminal_cause: None,
            pm_disconnected_epoch: None,
            okx_disconnected_epoch: None,
            terminal_pm_unavailable: None,
            terminal_okx_unavailable: None,
            pm_lifecycle: PublicLifecyclePhase::AwaitingConnection,
            okx_lifecycle: PublicLifecyclePhase::AwaitingConnection,
            pending_pm_book_reductions: VecDeque::with_capacity(MAX_PENDING_PM_BOOK_REDUCTIONS),
            pending_pm_book_lane_fault: None,
            pending_pm_metadata_lane_fault: None,
            pending_pm_aged_lane_fault: None,
            pending_okx_reference_lane_fault: None,
            pending_pm_unavailable_lane_fault: None,
            pending_okx_unavailable_lane_fault: None,
            pending_other_aged_lane_fault: None,
            terminal_notification_admission_failure: None,
            pm_reducer,
            terminal_tick_cleanup: PmPublicTerminalTickCleanupStatus::NotRequired,
        })
    }
}

fn prepare_pm_reducer(
    config: &PmPublicConnectivityConfig,
    authoritative: &reap_polymarket_adapter::PmAuthoritativeMetadata,
    session_policy: PmCaptureSessionPolicy,
) -> Result<PmBookReducer, PmPublicCaptureRunError> {
    let fingerprint =
        PmMetadataFingerprint::new(authoritative.metadata_fingerprint()).map_err(|_| {
            PmPublicCaptureRunError::PmReducerSync {
                source: PmPublicReducerSyncError::ReducerConfigurationMismatch,
                unavailable: None,
            }
        })?;
    let domain = PmDomainFingerprint::new(authoritative.domain_fingerprint()).map_err(|_| {
        PmPublicCaptureRunError::PmReducerSync {
            source: PmPublicReducerSyncError::ReducerConfigurationMismatch,
            unavailable: None,
        }
    })?;
    let contract = PmMetadataContract::goal_f_clob_v2(config.expected_metadata(), domain);
    let mut reducer = PmBookReducer::new(
        config.instrument(),
        fingerprint,
        contract,
        session_policy.freshness()?,
    )
    .map_err(|source| PmPublicCaptureRunError::PmReducerSync {
        source: PmPublicReducerSyncError::Reducer(source),
        unavailable: None,
    })?;
    let event = authoritative.event();
    let observation = PmMetadataObservation::new(
        event.instrument(),
        event.metadata_revision(),
        fingerprint,
        contract,
        authoritative.monotonic_receive_ns(),
    )
    .map_err(|_| PmPublicCaptureRunError::PmReducerSync {
        source: PmPublicReducerSyncError::ReducerConfigurationMismatch,
        unavailable: None,
    })?;
    reducer.apply_metadata(observation).map_err(|source| {
        PmPublicCaptureRunError::PmReducerSync {
            source: PmPublicReducerSyncError::Reducer(source),
            unavailable: None,
        }
    })?;
    reducer
        .begin_epoch(session_policy.pm_initial_epoch())
        .map_err(|source| PmPublicCaptureRunError::PmReducerSync {
            source: PmPublicReducerSyncError::Reducer(source),
            unavailable: None,
        })?;
    Ok(reducer)
}

#[derive(Debug, Default)]
struct RawIngressCounter {
    epoch: u64,
    sequence: u64,
}

impl RawIngressCounter {
    fn next(&self, epoch: u64) -> Result<u64, PmPublicCaptureRunError> {
        if epoch == 0 {
            return Err(PmPublicCaptureRunError::RawIngressOverflow);
        }
        if self.epoch == epoch {
            self.sequence
                .checked_add(1)
                .ok_or(PmPublicCaptureRunError::RawIngressOverflow)
        } else {
            Ok(1)
        }
    }

    fn commit(&mut self, epoch: u64, sequence: u64) {
        self.epoch = epoch;
        self.sequence = sequence;
    }
}

/// Active least-authority owner for one PM+OKX public capture.
#[derive(Debug)]
pub struct PmPublicCaptureRun {
    path: PathBuf,
    header: PmCaptureHeader,
    writer: PmPublicCaptureWriter,
    roles: PmCaptureRoles,
    pm_route: PmConnectionRoute,
    okx_route: PmConnectionRoute,
    public_lane: PmPublicLaneState,
    pm_raw_ingress: RawIngressCounter,
    okx_raw_ingress: RawIngressCounter,
    terminal_cause: Option<PmPublicCaptureTerminalCause>,
    pm_disconnected_epoch: Option<u64>,
    okx_disconnected_epoch: Option<u64>,
    terminal_pm_unavailable: Option<PmPublicUnavailableDelivery>,
    terminal_okx_unavailable: Option<OkxPublicUnavailableDelivery>,
    pm_lifecycle: PublicLifecyclePhase,
    okx_lifecycle: PublicLifecyclePhase,
    pending_pm_book_reductions: VecDeque<PendingPmBookReduction>,
    pending_pm_book_lane_fault: Option<PendingPmBookLaneFault>,
    pending_pm_metadata_lane_fault: Option<PendingPmMetadataLaneFault>,
    pending_pm_aged_lane_fault: Option<PendingPmAgedLaneFault>,
    pending_okx_reference_lane_fault: Option<PendingPmRouteFaultIdentity>,
    pending_pm_unavailable_lane_fault: Option<PendingPmRouteFaultIdentity>,
    pending_okx_unavailable_lane_fault: Option<PendingPmRouteFaultIdentity>,
    pending_other_aged_lane_fault: Option<PendingOtherAgedLaneFault>,
    terminal_notification_admission_failure: Option<PmPublicNotificationAdmissionFailure>,
    pm_reducer: PmBookReducer,
    terminal_tick_cleanup: PmPublicTerminalTickCleanupStatus,
}

impl PmPublicCaptureRun {
    fn ensure_active(&self) -> Result<(), PmPublicCaptureRunError> {
        match self.terminal_cause {
            Some(cause) => Err(PmPublicCaptureRunError::ArtifactTerminal { cause }),
            None if self.public_lane.consumer_transfer_poisoned() => {
                Err(PmPublicCaptureRunError::PublicConsumerTransferPoisoned)
            }
            None => Ok(()),
        }
    }

    fn terminalize_plain(&mut self, cause: PmPublicCaptureTerminalCause) {
        if self.terminal_cause.is_some() {
            return;
        }
        self.purge_current_public_routes();
        self.terminal_cause = Some(cause);
        self.roles.terminalize_plain(
            reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
            reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
        );
    }

    fn terminalize_with_receive_evidence(
        &mut self,
        cause: PmPublicCaptureTerminalCause,
        pm_fault: reap_polymarket_adapter::PmPublicSessionFault,
        okx_fault: reap_okx_public_source::OkxPublicSessionFault,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> (
        Option<PmPublicUnavailableDelivery>,
        Option<OkxPublicUnavailableDelivery>,
    ) {
        if self.terminal_cause.is_some() {
            return (None, None);
        }
        self.purge_current_public_routes();
        self.terminal_cause = Some(cause);
        self.roles.terminalize_with_receive_evidence(
            pm_fault,
            okx_fault,
            local_wall_receive_ns,
            monotonic_receive_ns,
        )
    }

    fn purge_current_public_routes(&mut self) {
        let authority = self.roles.authority_id();
        let _ = self.public_lane.purge_public_route(
            authority,
            self.pm_route.source(),
            self.pm_route.connection(),
            reap_pm_core::ConnectionEpoch::new(self.roles.pm_epoch()),
        );
        let _ = self.public_lane.purge_public_route(
            authority,
            self.okx_route.source(),
            self.okx_route.connection(),
            reap_pm_core::ConnectionEpoch::new(self.roles.okx_epoch()),
        );
    }

    fn pending_pm_public_route_depth(&self) -> usize {
        self.public_lane.public_route_depth(
            self.roles.authority_id(),
            self.pm_route.source(),
            self.pm_route.connection(),
            reap_pm_core::ConnectionEpoch::new(self.roles.pm_epoch()),
        )
    }

    fn pending_okx_public_route_depth(&self) -> usize {
        self.public_lane.public_route_depth(
            self.roles.authority_id(),
            self.okx_route.source(),
            self.okx_route.connection(),
            reap_pm_core::ConnectionEpoch::new(self.roles.okx_epoch()),
        )
    }

    #[must_use]
    pub const fn artifact_terminal(&self) -> bool {
        self.terminal_cause.is_some()
    }

    #[must_use]
    pub const fn terminal_cause(&self) -> Option<PmPublicCaptureTerminalCause> {
        self.terminal_cause
    }

    #[must_use]
    pub const fn terminal_pm_unavailable(&self) -> Option<&PmPublicUnavailableDelivery> {
        self.terminal_pm_unavailable.as_ref()
    }

    #[must_use]
    pub const fn terminal_okx_unavailable(&self) -> Option<&OkxPublicUnavailableDelivery> {
        self.terminal_okx_unavailable.as_ref()
    }

    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    #[must_use]
    pub fn public_lane_metrics(&self) -> PmLaneMetrics {
        self.public_lane.metrics()
    }

    pub(crate) const fn public_consumer_transfer_poisoned(&self) -> bool {
        self.public_lane.consumer_transfer_poisoned()
    }

    pub(crate) fn public_lane_reserved_capacity_bytes(&self) -> usize {
        self.public_lane.reserved_capacity_bytes()
    }

    pub(crate) fn capture_writer_reserved_capacity_bytes(&self) -> usize {
        self.writer.reserved_capacity_bytes()
    }

    #[cfg(test)]
    pub(crate) fn pending_capture_record_depth_for_evidence(&self) -> usize {
        self.writer.pending_record_depth_for_evidence()
    }

    pub(crate) fn reserved_capacity_bytes(&self) -> usize {
        self.public_lane_reserved_capacity_bytes()
            .saturating_add(self.capture_writer_reserved_capacity_bytes())
    }

    #[must_use]
    pub const fn header(&self) -> &PmCaptureHeader {
        &self.header
    }

    #[must_use]
    pub fn pm_subscription_bytes(&self) -> &[u8] {
        self.roles.pm_subscription_bytes()
    }

    #[must_use]
    pub fn okx_subscription_bytes(&self) -> &[u8] {
        self.roles.okx_subscription_bytes()
    }

    pub async fn record_pm_connection_started(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<(), PmPublicCaptureRunError> {
        self.ensure_active()?;
        self.ensure_no_pending_pm_book_reductions()?;
        if !self.pm_lifecycle.accepts_connection_start() {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            return Err(PmPublicCaptureRunError::InvalidLifecyclePhase);
        }
        if let Err(source) = self
            .writer
            .record_lifecycle(
                reap_pm_core::ConnectionEpoch::new(self.roles.pm_epoch()),
                monotonic_ns,
                PmCaptureLifecycle::ConnectionStarted,
            )
            .await
        {
            self.terminalize_plain(PmPublicCaptureTerminalCause::CaptureWriter);
            return Err(source.into());
        }
        self.pm_disconnected_epoch = None;
        self.pm_lifecycle = PublicLifecyclePhase::AwaitingSubscription;
        Ok(())
    }

    pub async fn record_okx_connection_started(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<(), PmPublicCaptureRunError> {
        self.ensure_active()?;
        self.ensure_no_pending_pm_book_reductions()?;
        if !self.okx_lifecycle.accepts_connection_start() {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            return Err(PmPublicCaptureRunError::InvalidLifecyclePhase);
        }
        if let Err(source) = self
            .writer
            .record_okx_lifecycle(
                self.roles.okx_epoch(),
                monotonic_ns,
                OkxCaptureLifecycle::ConnectionStarted,
            )
            .await
        {
            self.terminalize_plain(PmPublicCaptureTerminalCause::CaptureWriter);
            return Err(source.into());
        }
        self.okx_disconnected_epoch = None;
        self.okx_lifecycle = PublicLifecyclePhase::AwaitingSubscription;
        Ok(())
    }

    pub async fn record_pm_subscription_sent(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<(), PmPublicCaptureRunError> {
        self.ensure_active()?;
        self.ensure_no_pending_pm_book_reductions()?;
        if !self.pm_lifecycle.accepts_subscription() {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            return Err(PmPublicCaptureRunError::InvalidLifecyclePhase);
        }
        if let Err(source) = self.roles.preflight_pm_subscription_sent(monotonic_ns) {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            return Err(source.into());
        }
        if let Err(source) = self.writer.preflight_pm_lifecycle(
            reap_pm_core::ConnectionEpoch::new(self.roles.pm_epoch()),
            monotonic_ns,
            PmCaptureLifecycle::SubscriptionSent,
        ) {
            self.terminalize_plain(PmPublicCaptureTerminalCause::CaptureWriter);
            return Err(source.into());
        }
        if let Err(source) = self
            .writer
            .record_lifecycle(
                reap_pm_core::ConnectionEpoch::new(self.roles.pm_epoch()),
                monotonic_ns,
                PmCaptureLifecycle::SubscriptionSent,
            )
            .await
        {
            self.terminalize_plain(PmPublicCaptureTerminalCause::CaptureWriter);
            return Err(source.into());
        }
        if let Err(source) = self.roles.mark_pm_subscription_sent(monotonic_ns) {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            return Err(source.into());
        }
        self.pm_lifecycle = PublicLifecyclePhase::Live;
        Ok(())
    }

    pub async fn record_okx_subscription_sent(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<(), PmPublicCaptureRunError> {
        self.ensure_active()?;
        self.ensure_no_pending_pm_book_reductions()?;
        if !self.okx_lifecycle.accepts_subscription() {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            return Err(PmPublicCaptureRunError::InvalidLifecyclePhase);
        }
        if let Err(source) = self
            .writer
            .record_okx_lifecycle(
                self.roles.okx_epoch(),
                monotonic_ns,
                OkxCaptureLifecycle::SubscriptionSent,
            )
            .await
        {
            self.terminalize_plain(PmPublicCaptureTerminalCause::CaptureWriter);
            return Err(source.into());
        }
        self.okx_lifecycle = PublicLifecyclePhase::Live;
        Ok(())
    }

    /// Issues the one authoritative PM metadata occurrence and admits it to
    /// the Run-owned public lane in the same synchronous operation.
    pub fn issue_and_enqueue_pm_metadata(
        &mut self,
        local_wall_receive_ns: u64,
    ) -> Result<(), PmPublicDataPipelineError<PmPublicMetadataDelivery>> {
        let delivery = self
            .issue_pm_metadata(local_wall_receive_ns)
            .map_err(PmPublicDataPipelineError::Run)?;
        self.enqueue_pm_metadata(delivery)
            .map_err(PmPublicDataPipelineError::Lane)
    }

    fn issue_pm_metadata(
        &mut self,
        local_wall_receive_ns: u64,
    ) -> Result<PmPublicMetadataDelivery, PmPublicCaptureRunError> {
        self.ensure_active()?;
        self.ensure_no_pending_pm_book_reductions()?;
        if !self.pm_lifecycle.accepts_live_input() {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            return Err(PmPublicCaptureRunError::InvalidLifecyclePhase);
        }
        match self.roles.pm_metadata(local_wall_receive_ns) {
            Ok(delivery) => Ok(delivery),
            Err(source) => {
                self.terminalize_plain(PmPublicCaptureTerminalCause::Route);
                Err(source.into())
            }
        }
    }

    #[allow(
        clippy::result_large_err,
        reason = "admission returns the exact move-only routed delivery without allocation"
    )]
    pub(crate) fn enqueue_pm_metadata(
        &mut self,
        delivery: PmPublicMetadataDelivery,
    ) -> Result<(), PmPublicLaneAdmissionError<PmPublicMetadataDelivery>> {
        if !self.pending_pm_book_reductions.is_empty() || self.has_pending_pm_lane_fault() {
            return Err(PmPublicLaneAdmissionError::PendingPmBookAuthority { delivery });
        }
        if self.artifact_terminal() {
            return Err(PmPublicLaneAdmissionError::RunTerminal { delivery });
        }
        let (authority_id, source, connection, ordering) = {
            let envelope = delivery.envelope();
            (
                delivery.authority_id(),
                envelope.source(),
                envelope.connection_id(),
                envelope.ordering(),
            )
        };
        if authority_id != self.roles.authority_id() {
            return Err(PmPublicLaneAdmissionError::RouteAuthorityMismatch { delivery });
        }
        if !self.pm_lifecycle.accepts_live_input()
            || !self
                .roles
                .pm_delivery_is_current(authority_id, source, connection, ordering)
        {
            return Err(PmPublicLaneAdmissionError::RouteScopeMismatch { delivery });
        }
        let pending_identity = PendingPmRouteFaultIdentity::from_metadata(&delivery);
        match self.public_lane.enqueue_pm_metadata(delivery) {
            Ok(()) => Ok(()),
            Err(failure) => {
                if failure.is_full() {
                    let epoch = failure.delivery().envelope().ordering().connection_epoch();
                    match self
                        .pm_reducer
                        .begin_pending_external_fault(epoch, PmExternalBookFault::Overflow)
                    {
                        Ok(reducer_fault_authority) => {
                            let pending = PendingPmMetadataLaneFault::new(
                                pending_identity,
                                reducer_fault_authority,
                            );
                            if let Err(pending) =
                                self.register_pending_pm_metadata_lane_fault(pending)
                            {
                                let _ = self.pm_reducer.finalize_pending_external_fault(
                                    pending.reducer_fault_authority(),
                                    PmExternalBookFault::InvalidTransition,
                                );
                                self.terminalize_plain(
                                    PmPublicCaptureTerminalCause::InternalInvariant,
                                );
                            }
                        }
                        Err(_) => {
                            self.terminalize_plain(PmPublicCaptureTerminalCause::InternalInvariant);
                        }
                    }
                } else {
                    let clock = failure.delivery().envelope().received_clock();
                    let _ = self.roles.apply_pm_reducer_external_fault(
                        &mut self.pm_reducer,
                        PmExternalBookFault::InvalidTransition,
                        PmPublicReadinessReason::InvalidTransition,
                    );
                    let (pm_unavailable, okx_unavailable) = self.terminalize_with_receive_evidence(
                        PmPublicCaptureTerminalCause::Lane,
                        reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                        reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                        clock.local_wall_receive_ns(),
                        clock.monotonic_receive_ns(),
                    );
                    self.terminal_pm_unavailable = pm_unavailable;
                    self.terminal_okx_unavailable = okx_unavailable;
                }
                Err(PmPublicLaneAdmissionError::Lane(failure))
            }
        }
    }

    pub(crate) fn enqueue_okx_reference(
        &mut self,
        delivery: OkxPublicReferenceDelivery,
    ) -> Result<(), PmPublicLaneAdmissionError<OkxPublicReferenceDelivery>> {
        if !self.pending_pm_book_reductions.is_empty() || self.has_pending_pm_lane_fault() {
            return Err(PmPublicLaneAdmissionError::PendingPmBookAuthority { delivery });
        }
        if self.artifact_terminal() {
            return Err(PmPublicLaneAdmissionError::RunTerminal { delivery });
        }
        let (authority_id, source, connection, ordering) = {
            let envelope = delivery.envelope();
            (
                delivery.authority_id(),
                envelope.source(),
                envelope.connection_id(),
                envelope.ordering(),
            )
        };
        if authority_id != self.roles.authority_id() {
            return Err(PmPublicLaneAdmissionError::RouteAuthorityMismatch { delivery });
        }
        if !self.okx_lifecycle.accepts_live_input()
            || !self
                .roles
                .okx_delivery_is_current(authority_id, source, connection, ordering)
        {
            return Err(PmPublicLaneAdmissionError::RouteScopeMismatch { delivery });
        }
        let pending_identity = PendingPmRouteFaultIdentity::from_okx_reference(&delivery);
        match self.public_lane.enqueue_okx_reference(delivery) {
            Ok(()) => Ok(()),
            Err(failure) => {
                if failure.is_full() {
                    self.pending_okx_reference_lane_fault = Some(pending_identity);
                } else {
                    let clock = failure.delivery().envelope().received_clock();
                    let (pm_unavailable, okx_unavailable) = self.terminalize_with_receive_evidence(
                        PmPublicCaptureTerminalCause::Lane,
                        reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                        reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                        clock.local_wall_receive_ns(),
                        clock.monotonic_receive_ns(),
                    );
                    self.terminal_pm_unavailable = pm_unavailable;
                    self.terminal_okx_unavailable = okx_unavailable;
                }
                Err(PmPublicLaneAdmissionError::Lane(failure))
            }
        }
    }

    pub(crate) fn enqueue_pm_unavailable(
        &mut self,
        delivery: PmPublicUnavailableDelivery,
    ) -> Result<(), PmPublicLaneAdmissionError<PmPublicUnavailableDelivery>> {
        if !self.pending_pm_book_reductions.is_empty() || self.has_pending_pm_lane_fault() {
            return Err(PmPublicLaneAdmissionError::PendingPmBookAuthority { delivery });
        }
        if self.artifact_terminal() {
            return Err(PmPublicLaneAdmissionError::RunTerminal { delivery });
        }
        let (authority_id, source, connection, ordering, fault) = {
            let envelope = delivery.envelope();
            (
                delivery.authority_id(),
                envelope.source(),
                envelope.connection_id(),
                envelope.ordering(),
                envelope.payload().fault(),
            )
        };
        if authority_id != self.roles.authority_id() {
            return Err(PmPublicLaneAdmissionError::RouteAuthorityMismatch { delivery });
        }
        if !matches!(self.pm_lifecycle, PublicLifecyclePhase::Disconnected)
            || !self.roles.pm_unavailable_delivery_is_current(
                authority_id,
                source,
                connection,
                ordering,
                fault,
            )
        {
            return Err(PmPublicLaneAdmissionError::RouteScopeMismatch { delivery });
        }
        let pending_identity = PendingPmRouteFaultIdentity::from_pm_unavailable(&delivery);
        match self.public_lane.enqueue_pm_unavailable(delivery) {
            Ok(()) => Ok(()),
            Err(failure) => {
                if failure.is_full() {
                    self.pending_pm_unavailable_lane_fault = Some(pending_identity);
                } else {
                    self.terminalize_plain(PmPublicCaptureTerminalCause::Lane);
                }
                Err(PmPublicLaneAdmissionError::Lane(failure))
            }
        }
    }

    pub(crate) fn enqueue_okx_unavailable(
        &mut self,
        delivery: OkxPublicUnavailableDelivery,
    ) -> Result<(), PmPublicLaneAdmissionError<OkxPublicUnavailableDelivery>> {
        if !self.pending_pm_book_reductions.is_empty() || self.has_pending_pm_lane_fault() {
            return Err(PmPublicLaneAdmissionError::PendingPmBookAuthority { delivery });
        }
        if self.artifact_terminal() {
            return Err(PmPublicLaneAdmissionError::RunTerminal { delivery });
        }
        let (authority_id, source, connection, ordering, fault) = {
            let envelope = delivery.envelope();
            (
                delivery.authority_id(),
                envelope.source(),
                envelope.connection_id(),
                envelope.ordering(),
                envelope.payload().fault(),
            )
        };
        if authority_id != self.roles.authority_id() {
            return Err(PmPublicLaneAdmissionError::RouteAuthorityMismatch { delivery });
        }
        if !matches!(self.okx_lifecycle, PublicLifecyclePhase::Disconnected)
            || !self.roles.okx_unavailable_delivery_is_current(
                authority_id,
                source,
                connection,
                ordering,
                fault,
            )
        {
            return Err(PmPublicLaneAdmissionError::RouteScopeMismatch { delivery });
        }
        let pending_identity = PendingPmRouteFaultIdentity::from_okx_unavailable(&delivery);
        match self.public_lane.enqueue_okx_unavailable(delivery) {
            Ok(()) => Ok(()),
            Err(failure) => {
                if failure.is_full() {
                    self.pending_okx_unavailable_lane_fault = Some(pending_identity);
                } else {
                    self.terminalize_plain(PmPublicCaptureTerminalCause::Lane);
                }
                Err(PmPublicLaneAdmissionError::Lane(failure))
            }
        }
    }

    fn admit_pm_unavailable_or_terminal(
        &mut self,
        delivery: PmPublicUnavailableDelivery,
    ) -> Result<reap_polymarket_adapter::PmPublicSessionFault, PmPublicCaptureRunError> {
        let fault = delivery.envelope().payload().fault();
        match self.enqueue_pm_unavailable(delivery) {
            Ok(()) => Ok(fault),
            Err(failure) => {
                let delivery = failure.into_delivery();
                self.clear_pending_pm_unavailable_lane_fault();
                let failure = PmPublicNotificationAdmissionFailure::Polymarket { fault };
                self.terminalize_plain(PmPublicCaptureTerminalCause::Lane);
                self.terminal_notification_admission_failure = Some(failure);
                self.terminal_pm_unavailable = Some(delivery);
                Err(failure.into())
            }
        }
    }

    fn admit_okx_unavailable_or_terminal(
        &mut self,
        delivery: OkxPublicUnavailableDelivery,
    ) -> Result<reap_okx_public_source::OkxPublicSessionFault, PmPublicCaptureRunError> {
        let fault = delivery.envelope().payload().fault();
        match self.enqueue_okx_unavailable(delivery) {
            Ok(()) => Ok(fault),
            Err(failure) => {
                let delivery = failure.into_delivery();
                self.clear_pending_okx_unavailable_lane_fault();
                let failure = PmPublicNotificationAdmissionFailure::Okx { fault };
                self.terminalize_plain(PmPublicCaptureTerminalCause::Lane);
                self.terminal_notification_admission_failure = Some(failure);
                self.terminal_okx_unavailable = Some(delivery);
                Err(failure.into())
            }
        }
    }
}
