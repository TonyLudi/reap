#![allow(
    clippy::result_large_err,
    reason = "capture-role failures retain exact move-only routed and unavailable evidence on the bounded fail-closed path without heap allocation"
)]

mod reducer_freshness;

use std::time::Duration;

use reap_okx_public_source::{
    OkxPublicControlEvidence, OkxPublicEventEvidence, OkxPublicSession, OkxPublicSessionError,
    OkxPublicSessionEvent, OkxPublicSessionFault,
};
use reap_pm_core::{
    EventOrdering, IngressSequence, OkxReferenceHandle, PmBookUpdate, PmConnectionId,
    PmInstrumentHandle, PmProductSource, ReceivedEventClock,
};
use reap_pm_live_contracts::{
    ConstructedRoleBinding, PmConnectionRoute, PmPublicConnectivityConfig,
};
use reap_pm_state::{
    PmBookBatchEvidence, PmBookReducer, PmBookTransition, PmDomainFingerprint, PmExternalBookFault,
    PmMetadataContract, PmMetadataFingerprint, PmPendingExternalBookFaultAuthority,
    PmPublicReadinessReason,
};
use reap_polymarket_adapter::{
    PmAuthoritativeMetadata, PmPublicHeartbeatAction, PmPublicHeartbeatEvidence, PmPublicRole,
    PmPublicRoleError, PmPublicSession, PmPublicSessionError, PmPublicSessionFault,
    PmSnapshotFlowToken,
};
use thiserror::Error;

use crate::capture::PmCaptureSessionPolicy;
use crate::public_routes::{
    OkxPublicReferenceDelivery, OkxPublicUnavailableDelivery, PmPublicBookDelivery,
    PmPublicMetadataDelivery, PmPublicRouteAuthorityId, PmPublicRouteError, PmPublicRoutes,
    PmPublicUnavailableDelivery,
};

/// Checked static public-role blueprint retained by a composition plan.
///
/// Active venue sessions are deliberately absent until the capture root is
/// started with its authoritative metadata and explicit session policy.
#[derive(Debug)]
pub(crate) struct PmCaptureBlueprint {
    polymarket: PmPublicRole,
}

impl PmCaptureBlueprint {
    pub(crate) fn new(config: &PmPublicConnectivityConfig) -> Result<Self, PmPublicRoleError> {
        let route = config.polymarket_route();
        Ok(Self {
            polymarket: PmPublicRole::from_expected_metadata(
                config.observation_grant(),
                config.expected_metadata(),
                route.source(),
                route.connection(),
            )?,
        })
    }

    pub(crate) fn bindings(
        &self,
        config: &PmPublicConnectivityConfig,
    ) -> Vec<ConstructedRoleBinding> {
        let mut bindings = Vec::with_capacity(5);
        bindings.push(ConstructedRoleBinding::okx_public(
            config.okx_reference(),
            config.okx_route(),
        ));
        bindings.extend(ConstructedRoleBinding::pm_public(
            self.polymarket.instrument(),
            PmConnectionRoute::new(self.polymarket.source(), self.polymarket.connection()),
        ));
        bindings
    }

    pub(crate) fn reference(&self, config: &PmPublicConnectivityConfig) -> OkxReferenceHandle {
        config.okx_reference()
    }

    pub(crate) const fn instrument(&self) -> PmInstrumentHandle {
        self.polymarket.instrument()
    }
}

/// Active public capture authority owned by one running composition root.
///
/// The route authority and both venue sessions are constructed together and
/// never exposed independently to callers.
#[derive(Debug)]
pub(crate) struct PmCaptureRoles {
    pm: PmPublicSession,
    okx: OkxPublicSession,
    routes: PmPublicRoutes,
    expected_metadata_fingerprint: PmMetadataFingerprint,
    expected_metadata_contract: PmMetadataContract,
    okx_source: PmProductSource,
    okx_connection: PmConnectionId,
}

impl PmCaptureRoles {
    pub(crate) fn start(
        blueprint: PmCaptureBlueprint,
        config: &PmPublicConnectivityConfig,
        authoritative: PmAuthoritativeMetadata,
        policy: PmCaptureSessionPolicy,
    ) -> Result<Self, PmCaptureRoleStartError> {
        let pm = PmPublicSession::new(
            blueprint.polymarket,
            authoritative,
            policy.pm_initial_epoch(),
            policy.pm_last_snapshot_revision(),
            policy.pm_reconnect().as_transport(),
            policy.pm_heartbeat()?,
        )?;
        let okx = OkxPublicSession::new_configured_capture(
            config.okx_reference_instrument().instrument_id().as_str(),
            config.okx_route().connection().as_str(),
            policy.okx_initial_epoch(),
            policy.okx_reconnect().as_transport(),
        )?;
        let expected_metadata_fingerprint =
            PmMetadataFingerprint::new(authoritative.metadata_fingerprint())?;
        let expected_metadata_contract = PmMetadataContract::goal_f_clob_v2(
            config.expected_metadata(),
            PmDomainFingerprint::new(authoritative.domain_fingerprint())?,
        );
        let routes = PmPublicRoutes::new(config, &pm, &okx)?;
        Ok(Self {
            pm,
            okx,
            routes,
            expected_metadata_fingerprint,
            expected_metadata_contract,
            okx_source: config.okx_route().source(),
            okx_connection: config.okx_route().connection(),
        })
    }

    pub(crate) fn pm_subscription_bytes(&self) -> &[u8] {
        self.pm.subscription_bytes()
    }

    pub(crate) fn okx_subscription_bytes(&self) -> &[u8] {
        self.okx.subscription_bytes()
    }

    pub(crate) const fn pm_epoch(&self) -> u64 {
        self.pm.connection_epoch().value()
    }

    pub(crate) const fn okx_epoch(&self) -> u64 {
        self.okx.connection_epoch()
    }

    pub(crate) const fn authority_id(&self) -> PmPublicRouteAuthorityId {
        self.routes.authority_id()
    }

    pub(crate) fn pm_delivery_is_current(
        &self,
        authority_id: PmPublicRouteAuthorityId,
        source: PmProductSource,
        connection: PmConnectionId,
        ordering: EventOrdering,
    ) -> bool {
        authority_id == self.routes.authority_id()
            && source == self.pm.role().source()
            && connection == self.pm.role().connection()
            && ordering.connection_epoch() == self.pm.connection_epoch()
            && !self.pm.requires_reconnect()
    }

    pub(crate) fn pm_terminal_tick_delivery_is_current(
        &self,
        authority_id: PmPublicRouteAuthorityId,
        source: PmProductSource,
        connection: PmConnectionId,
        ordering: EventOrdering,
    ) -> bool {
        authority_id == self.routes.authority_id()
            && source == self.pm.role().source()
            && connection == self.pm.role().connection()
            && ordering.connection_epoch() == self.pm.connection_epoch()
            && self.pm.requires_reconnect()
            && self.pm.last_fault() == Some(PmPublicSessionFault::TickSizeChanged)
    }

    pub(crate) fn okx_delivery_is_current(
        &self,
        authority_id: PmPublicRouteAuthorityId,
        source: PmProductSource,
        connection: PmConnectionId,
        ordering: EventOrdering,
    ) -> bool {
        authority_id == self.routes.authority_id()
            && source == self.okx_source
            && connection == self.okx_connection
            && ordering.connection_epoch().value() == self.okx.connection_epoch()
            && !self.okx.requires_reconnect()
    }

    pub(crate) fn pm_unavailable_delivery_is_current(
        &self,
        authority_id: PmPublicRouteAuthorityId,
        source: PmProductSource,
        connection: PmConnectionId,
        ordering: EventOrdering,
        fault: PmPublicSessionFault,
    ) -> bool {
        authority_id == self.routes.authority_id()
            && source == self.pm.role().source()
            && connection == self.pm.role().connection()
            && ordering.connection_epoch() == self.pm.connection_epoch()
            && self.pm.requires_reconnect()
            && self.pm.last_fault() == Some(fault)
    }

    pub(crate) fn okx_unavailable_delivery_is_current(
        &self,
        authority_id: PmPublicRouteAuthorityId,
        source: PmProductSource,
        connection: PmConnectionId,
        ordering: EventOrdering,
        fault: OkxPublicSessionFault,
    ) -> bool {
        authority_id == self.routes.authority_id()
            && source == self.okx_source
            && connection == self.okx_connection
            && ordering.connection_epoch().value() == self.okx.connection_epoch()
            && self.okx.requires_reconnect()
            && self.okx.last_fault() == Some(fault)
    }

    pub(crate) fn terminalize_plain(
        &mut self,
        pm_fault: PmPublicSessionFault,
        okx_fault: OkxPublicSessionFault,
    ) {
        if !self.pm.requires_reconnect() {
            self.pm.invalidate(pm_fault);
        }
        if !self.okx.requires_reconnect() {
            self.okx.invalidate(okx_fault);
        }
    }

    pub(crate) fn terminalize_with_receive_evidence(
        &mut self,
        pm_fault: PmPublicSessionFault,
        okx_fault: OkxPublicSessionFault,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> (
        Option<PmPublicUnavailableDelivery>,
        Option<OkxPublicUnavailableDelivery>,
    ) {
        let mut pm_unavailable = self.take_pm_unavailable().ok().flatten();
        if pm_unavailable.is_none() && !self.pm.requires_reconnect() {
            pm_unavailable = self
                .invalidate_and_route_pm(pm_fault, local_wall_receive_ns, monotonic_receive_ns)
                .ok()
                .flatten();
        }
        let mut okx_unavailable = self.take_okx_unavailable().ok().flatten();
        if okx_unavailable.is_none() && !self.okx.requires_reconnect() {
            okx_unavailable = self
                .invalidate_and_route_okx(okx_fault, local_wall_receive_ns, monotonic_receive_ns)
                .ok()
                .flatten();
        }
        self.terminalize_plain(pm_fault, okx_fault);
        (pm_unavailable, okx_unavailable)
    }

    pub(crate) fn pm_metadata(
        &mut self,
        local_wall_receive_ns: u64,
    ) -> Result<PmPublicMetadataDelivery, PmPublicRouteError> {
        self.routes.pm_metadata(&mut self.pm, local_wall_receive_ns)
    }

    pub(crate) fn mark_pm_subscription_sent(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<(), PmPublicSessionError> {
        self.pm.mark_subscription_sent(monotonic_ns)
    }

    pub(crate) fn preflight_pm_subscription_sent(
        &self,
        monotonic_ns: u64,
    ) -> Result<(), PmPublicSessionError> {
        self.pm.preflight_mark_subscription_sent(monotonic_ns)
    }

    pub(crate) fn poll_pm_heartbeat(
        &mut self,
        local_wall_now_ns: u64,
        monotonic_now_ns: u64,
    ) -> Result<PmPublicHeartbeatAction, PmPublicSessionError> {
        self.pm
            .poll_heartbeat_with_receive_evidence(local_wall_now_ns, monotonic_now_ns)
    }

    pub(crate) fn preview_pm_heartbeat(
        &self,
        monotonic_now_ns: u64,
    ) -> Result<PmPublicHeartbeatAction, PmPublicSessionError> {
        self.pm.preview_heartbeat(monotonic_now_ns)
    }

    pub(crate) fn commit_pm_snapshot(
        &mut self,
        delivery: PmPublicBookDelivery,
        flow: PmPublicSnapshotFlow,
        reducer: &mut PmBookReducer,
    ) -> Result<PmPublicBookDelivery, PmCaptureSnapshotCommitFailure> {
        let (delivery_authority_id, envelope) = delivery.into_parts();
        let clock = envelope.received_clock();
        let result = self.try_commit_pm_snapshot(delivery_authority_id, &envelope, &flow, reducer);
        if let Err(source) = result {
            let _ = reducer.apply_external_fault(
                envelope.ordering().connection_epoch(),
                PmExternalBookFault::InvalidTransition,
            );
            let fallback_fault = match &source {
                PmPublicSnapshotCommitError::Reducer(_) => PmPublicSessionFault::ReducerRejected,
                _ => PmPublicSessionFault::InvalidTransition,
            };
            if self
                .pm
                .reject_snapshot_flow_with_receive_evidence(
                    flow.token,
                    clock.local_wall_receive_ns(),
                    clock.monotonic_receive_ns(),
                )
                .is_err()
            {
                self.pm.invalidate(fallback_fault);
            }
            let unavailable = self
                .take_pm_unavailable()
                .map_err(PmCaptureSnapshotCommitFailure::Route)?;
            return Err(PmCaptureSnapshotCommitFailure::Commit {
                source,
                unavailable,
            });
        }
        match self.pm.open_protocol_flow_after_snapshot(flow.token) {
            Ok(()) => Ok(PmPublicBookDelivery::from_parts(
                delivery_authority_id,
                envelope,
            )),
            Err(source) => {
                let _ = reducer.apply_external_fault(
                    envelope.ordering().connection_epoch(),
                    PmExternalBookFault::InvalidTransition,
                );
                let _ = self.pm.invalidate_with_receive_evidence(
                    PmPublicSessionFault::InvalidTransition,
                    clock.local_wall_receive_ns(),
                    clock.monotonic_receive_ns(),
                );
                let unavailable = self
                    .take_pm_unavailable()
                    .map_err(PmCaptureSnapshotCommitFailure::Route)?;
                Err(PmCaptureSnapshotCommitFailure::Commit {
                    source: PmPublicSnapshotCommitError::Protocol(source),
                    unavailable,
                })
            }
        }
    }

    fn try_commit_pm_snapshot(
        &self,
        delivery_authority_id: PmPublicRouteAuthorityId,
        envelope: &reap_pm_core::ReceivedEventEnvelope<reap_pm_core::PmBookEvent>,
        flow: &PmPublicSnapshotFlow,
        reducer: &mut PmBookReducer,
    ) -> Result<(), PmPublicSnapshotCommitError> {
        if delivery_authority_id != self.routes.authority_id()
            || flow.authority_id != self.routes.authority_id()
        {
            return Err(PmPublicSnapshotCommitError::RouteAuthorityMismatch);
        }
        if reducer.instrument() != self.pm.role().instrument()
            || reducer.expected_metadata_fingerprint() != self.expected_metadata_fingerprint
            || reducer.expected_metadata_contract() != self.expected_metadata_contract
        {
            return Err(PmPublicSnapshotCommitError::ReducerConfigurationMismatch);
        }
        let payload = envelope.payload();
        let PmBookUpdate::Snapshot(snapshot) = payload.update() else {
            return Err(PmPublicSnapshotCommitError::DeliveryIsNotSnapshot);
        };
        let ordering = envelope.ordering();
        let snapshot_revision = ordering
            .snapshot_revision()
            .ok_or(PmPublicSnapshotCommitError::MissingSnapshotRevision)?;
        if flow.token.connection_epoch() != ordering.connection_epoch()
            || flow.token.snapshot_revision() != snapshot_revision
            || flow.token.local_ingress_sequence() != ordering.local_ingress_sequence()
            || Some(flow.token.venue_hash()) != ordering.venue_hash()
        {
            return Err(PmPublicSnapshotCommitError::DeliveryFlowMismatch);
        }
        if self.pm.requires_reconnect()
            || self.pm.connection_epoch() != flow.token.connection_epoch()
            || self.pm.current_snapshot_revision() != Some(flow.token.snapshot_revision())
            || self.pm.metadata_revision() != payload.metadata_revision()
            || payload.instrument() != self.pm.role().instrument()
        {
            return Err(PmPublicSnapshotCommitError::ActiveSessionMismatch);
        }
        let evidence = PmBookBatchEvidence::new(
            payload.instrument(),
            ordering.connection_epoch(),
            payload.metadata_revision(),
            snapshot_revision,
            ordering.local_ingress_sequence(),
            envelope.received_clock().monotonic_receive_ns(),
            ordering.venue_hash(),
        )?;
        let transition = reducer.apply_snapshot(evidence, snapshot)?;
        let PmBookTransition::SnapshotCommitted {
            revision,
            levels,
            proof,
        } = transition
        else {
            return Err(PmPublicSnapshotCommitError::ReducerDidNotCommitSnapshot);
        };
        if revision != flow.token.snapshot_revision()
            || usize::from(levels) != snapshot.levels().len()
            || proof.instrument() != self.pm.role().instrument()
            || proof.metadata_fingerprint() != self.expected_metadata_fingerprint
            || proof.connection_epoch() != flow.token.connection_epoch()
            || proof.metadata_revision() != self.pm.metadata_revision()
            || proof.snapshot_revision() != flow.token.snapshot_revision()
            || proof.local_ingress_sequence() != flow.token.local_ingress_sequence()
            || proof.venue_hash() != flow.token.venue_hash()
            || reducer.connection_epoch() != Some(flow.token.connection_epoch())
            || !reducer.readiness().is_ready()
            || reducer.readiness().metadata_revision() != Some(self.pm.metadata_revision())
            || reducer.readiness().snapshot_revision() != Some(flow.token.snapshot_revision())
            || reducer.last_verified_snapshot_hash() != Some(flow.token.venue_hash())
        {
            return Err(PmPublicSnapshotCommitError::ReducerProofMismatch);
        }
        Ok(())
    }

    pub(crate) fn reduce_pm_book_update(
        &mut self,
        delivery: PmPublicBookDelivery,
        reducer: &mut PmBookReducer,
    ) -> Result<(PmBookTransition, PmPublicBookDelivery), PmCaptureBookReduceFailure> {
        let (delivery_authority_id, envelope) = delivery.into_parts();
        let clock = envelope.received_clock();
        match self.try_reduce_pm_book_update(delivery_authority_id, &envelope, reducer) {
            Ok(transition) => Ok((
                transition,
                PmPublicBookDelivery::from_parts(delivery_authority_id, envelope),
            )),
            Err(source) => {
                if matches!(source, PmPublicBookReduceError::ReducerTransitionMismatch) {
                    let _ = reducer.apply_external_fault(
                        envelope.ordering().connection_epoch(),
                        PmExternalBookFault::InvalidTransition,
                    );
                }
                let fault = if matches!(source, PmPublicBookReduceError::Reducer(_)) {
                    PmPublicSessionFault::ReducerRejected
                } else {
                    PmPublicSessionFault::InvalidTransition
                };
                let unavailable = self
                    .invalidate_and_route_pm(
                        fault,
                        clock.local_wall_receive_ns(),
                        clock.monotonic_receive_ns(),
                    )
                    .map_err(PmCaptureBookReduceFailure::Route)?;
                Err(PmCaptureBookReduceFailure::Reduce {
                    source,
                    unavailable,
                })
            }
        }
    }

    fn try_reduce_pm_book_update(
        &self,
        delivery_authority_id: PmPublicRouteAuthorityId,
        envelope: &reap_pm_core::ReceivedEventEnvelope<reap_pm_core::PmBookEvent>,
        reducer: &mut PmBookReducer,
    ) -> Result<PmBookTransition, PmPublicBookReduceError> {
        if delivery_authority_id != self.routes.authority_id() {
            return Err(PmPublicBookReduceError::RouteAuthorityMismatch);
        }
        if reducer.instrument() != self.pm.role().instrument()
            || reducer.expected_metadata_fingerprint() != self.expected_metadata_fingerprint
            || reducer.expected_metadata_contract() != self.expected_metadata_contract
        {
            return Err(PmPublicBookReduceError::ReducerConfigurationMismatch);
        }
        let payload = envelope.payload();
        let expected = match payload.update() {
            PmBookUpdate::DeltaBatch(batch) => {
                ExpectedPmBookTransition::DeltaBatch(batch.changes().len())
            }
            PmBookUpdate::TopCheck(_) => ExpectedPmBookTransition::TopCheck,
            PmBookUpdate::Snapshot(_) | PmBookUpdate::TickSizeChanged { .. } => {
                return Err(PmPublicBookReduceError::DeliveryIsNotFlowUpdate);
            }
        };
        let ordering = envelope.ordering();
        let snapshot_revision = ordering
            .snapshot_revision()
            .ok_or(PmPublicBookReduceError::MissingSnapshotRevision)?;
        let readiness = reducer.readiness();
        if !self.pm_delivery_is_current(
            delivery_authority_id,
            envelope.source(),
            envelope.connection_id(),
            ordering,
        ) || !self.pm.protocol_flow_open()
            || self.pm.current_snapshot_revision() != Some(snapshot_revision)
            || self.pm.metadata_revision() != payload.metadata_revision()
            || payload.instrument() != self.pm.role().instrument()
            || ordering.venue_hash().is_some()
        {
            return Err(PmPublicBookReduceError::ActiveSessionMismatch);
        }
        if reducer.connection_epoch() != Some(ordering.connection_epoch())
            || !readiness.is_ready()
            || readiness.metadata_revision() != Some(payload.metadata_revision())
            || readiness.snapshot_revision() != Some(snapshot_revision)
        {
            return Err(PmPublicBookReduceError::ReducerStateMismatch);
        }
        let prior_counters = reducer.counters();
        let verified_snapshot_hash = reducer.last_verified_snapshot_hash();
        let evidence = PmBookBatchEvidence::new(
            payload.instrument(),
            ordering.connection_epoch(),
            payload.metadata_revision(),
            snapshot_revision,
            ordering.local_ingress_sequence(),
            envelope.received_clock().monotonic_receive_ns(),
            ordering.venue_hash(),
        )?;
        let transition = reducer.apply_update(evidence, payload.update())?;
        if !expected.matches(&transition, snapshot_revision)
            || !expected.matches_counters(prior_counters, reducer.counters())
            || reducer.connection_epoch() != Some(ordering.connection_epoch())
            || reducer.last_ingress_sequence() != Some(ordering.local_ingress_sequence())
            || reducer.last_ingress_receive_ns()
                != Some(envelope.received_clock().monotonic_receive_ns())
            || reducer.last_verified_snapshot_hash() != verified_snapshot_hash
            || !reducer.readiness().is_ready()
            || reducer.readiness().metadata_revision() != Some(payload.metadata_revision())
            || reducer.readiness().snapshot_revision() != Some(snapshot_revision)
        {
            return Err(PmPublicBookReduceError::ReducerTransitionMismatch);
        }
        Ok(transition)
    }

    pub(crate) fn preflight_pm_reducer_external_fault(
        &self,
        reducer: &PmBookReducer,
    ) -> Result<(), PmPublicReducerSyncError> {
        self.validate_pm_reducer_identity(reducer)?;
        if self.pm.requires_reconnect()
            || reducer.connection_epoch() != Some(self.pm.connection_epoch())
        {
            return Err(PmPublicReducerSyncError::SessionReducerStateMismatch);
        }
        Ok(())
    }

    pub(crate) fn apply_pm_reducer_external_fault(
        &self,
        reducer: &mut PmBookReducer,
        fault: PmExternalBookFault,
        expected: PmPublicReadinessReason,
    ) -> Result<PmPublicReadinessReason, PmPublicReducerSyncError> {
        self.validate_pm_reducer_identity(reducer)?;
        if reducer.connection_epoch() != Some(self.pm.connection_epoch()) {
            return Err(PmPublicReducerSyncError::SessionReducerStateMismatch);
        }
        let actual = reducer
            .apply_external_fault(self.pm.connection_epoch(), fault)
            .err()
            .ok_or(PmPublicReducerSyncError::ReducerTransitionMismatch)?;
        if actual != expected {
            return Err(PmPublicReducerSyncError::ReducerReasonMismatch { expected, actual });
        }
        Ok(actual)
    }

    pub(crate) fn preflight_pm_reducer_reconnect(
        &self,
        reducer: &PmBookReducer,
    ) -> Result<(), PmPublicReducerSyncError> {
        self.validate_pm_reducer_identity(reducer)?;
        if !self.pm.requires_reconnect()
            || reducer.connection_epoch() != Some(self.pm.connection_epoch())
        {
            return Err(PmPublicReducerSyncError::SessionReducerStateMismatch);
        }
        let expected_reason = match self.pm.last_fault() {
            Some(PmPublicSessionFault::Disconnect) => PmPublicReadinessReason::Disconnected,
            Some(PmPublicSessionFault::HeartbeatTimeout) => {
                PmPublicReadinessReason::HeartbeatTimeout
            }
            Some(PmPublicSessionFault::Overflow) => PmPublicReadinessReason::Overflow,
            Some(PmPublicSessionFault::Stale) => PmPublicReadinessReason::BookStale,
            _ => return Err(PmPublicReducerSyncError::SessionReducerStateMismatch),
        };
        if reducer.readiness().reason() != Some(expected_reason) {
            return Err(PmPublicReducerSyncError::SessionReducerStateMismatch);
        }
        Ok(())
    }

    pub(crate) fn synchronize_pm_reducer_epoch(
        &self,
        prior_epoch: u64,
        next_epoch: u64,
        reducer: &mut PmBookReducer,
    ) -> Result<(), PmPublicReducerSyncError> {
        self.validate_pm_reducer_identity(reducer)?;
        if self.pm_epoch() != next_epoch
            || reducer.connection_epoch() != Some(reap_pm_core::ConnectionEpoch::new(prior_epoch))
        {
            return Err(PmPublicReducerSyncError::SessionReducerStateMismatch);
        }
        let next_epoch = reap_pm_core::ConnectionEpoch::new(next_epoch);
        let transition = reducer
            .begin_epoch(next_epoch)
            .map_err(PmPublicReducerSyncError::Reducer)?;
        if transition != (PmBookTransition::EpochStarted { epoch: next_epoch })
            || reducer.connection_epoch() != Some(next_epoch)
            || reducer.readiness().is_ready()
        {
            return Err(PmPublicReducerSyncError::ReducerTransitionMismatch);
        }
        Ok(())
    }

    pub(crate) fn validate_pm_reducer_identity(
        &self,
        reducer: &PmBookReducer,
    ) -> Result<(), PmPublicReducerSyncError> {
        if reducer.instrument() != self.pm.role().instrument()
            || reducer.expected_metadata_fingerprint() != self.expected_metadata_fingerprint
            || reducer.expected_metadata_contract() != self.expected_metadata_contract
        {
            return Err(PmPublicReducerSyncError::ReducerConfigurationMismatch);
        }
        Ok(())
    }

    pub(crate) fn validate_pm_reducer_start(
        &self,
        reducer: &PmBookReducer,
    ) -> Result<(), PmPublicReducerSyncError> {
        self.validate_pm_reducer_identity(reducer)?;
        if reducer.connection_epoch() != Some(self.pm.connection_epoch())
            || reducer.readiness().reason() != Some(PmPublicReadinessReason::SnapshotMissing)
            || reducer.pending_external_fault().is_some()
            || !reducer.is_pristine_pre_snapshot()
        {
            return Err(PmPublicReducerSyncError::SessionReducerStateMismatch);
        }
        Ok(())
    }

    pub(crate) fn classify_and_route_pm(
        &mut self,
        raw: &[u8],
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<PmPublicCaptureBatch, PmCaptureRoleIngressError> {
        let batch = self
            .pm
            .classify(raw, local_wall_receive_ns, monotonic_receive_ns)?;
        let ignored_public_trades =
            u8::try_from(batch.ignored().len()).expect("wire frame event count is bounded at 64");
        let snapshot_flow = batch
            .snapshot_flow_token()
            .map(|token| PmPublicSnapshotFlow {
                token,
                authority_id: self.routes.authority_id(),
            });
        let heartbeat = batch.heartbeat();
        let mut books = Vec::with_capacity(batch.events().len());
        for delivery in batch.into_events() {
            books.push(self.routes.pm_book(&self.pm, delivery)?);
        }
        let unavailable = self.take_pm_unavailable()?;
        Ok(PmPublicCaptureBatch {
            books,
            ignored_public_trades,
            snapshot_flow,
            heartbeat,
            unavailable,
        })
    }

    pub(crate) fn classify_and_route_okx(
        &mut self,
        raw: &[u8],
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
        raw_hash: u64,
    ) -> Result<OkxCaptureRoleEvent, PmCaptureRoleIngressError> {
        let payload = match std::str::from_utf8(raw) {
            Ok(payload) => payload,
            Err(_) => {
                let unavailable = self.invalidate_and_route_okx(
                    OkxPublicSessionFault::InvalidTransition,
                    local_wall_receive_ns,
                    monotonic_receive_ns,
                )?;
                return Err(PmCaptureRoleIngressError::OkxRawNotUtf8 { unavailable });
            }
        };
        let delivery = self.okx.classify_captured_payload(
            payload,
            local_wall_receive_ns,
            monotonic_receive_ns,
            raw_hash,
        )?;
        match delivery.payload() {
            OkxPublicSessionEvent::SubscriptionAcknowledged(evidence) => {
                Ok(OkxCaptureRoleEvent::SubscriptionAcknowledged(*evidence))
            }
            OkxPublicSessionEvent::Heartbeat(evidence) => {
                Ok(OkxCaptureRoleEvent::Heartbeat(*evidence))
            }
            OkxPublicSessionEvent::Control(control) => {
                Ok(OkxCaptureRoleEvent::Control(control.clone()))
            }
            OkxPublicSessionEvent::Reference(_) => Ok(OkxCaptureRoleEvent::Reference(
                self.routes.okx_reference(&self.okx, delivery)?,
            )),
        }
    }

    pub(crate) fn invalidate_and_route_pm(
        &mut self,
        fault: PmPublicSessionFault,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<Option<PmPublicUnavailableDelivery>, PmPublicRouteError> {
        if self
            .pm
            .invalidate_with_receive_evidence(fault, local_wall_receive_ns, monotonic_receive_ns)
            .is_err()
        {
            self.pm.invalidate(fault);
        }
        self.take_pm_unavailable()
    }

    pub(crate) fn invalidate_and_route_okx(
        &mut self,
        fault: OkxPublicSessionFault,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<Option<OkxPublicUnavailableDelivery>, PmPublicRouteError> {
        if self
            .okx
            .invalidate_with_receive_evidence(fault, local_wall_receive_ns, monotonic_receive_ns)
            .is_err()
        {
            self.okx.invalidate(fault);
        }
        self.take_okx_unavailable()
    }

    pub(crate) fn take_pm_unavailable(
        &mut self,
    ) -> Result<Option<PmPublicUnavailableDelivery>, PmPublicRouteError> {
        self.pm
            .take_unavailable()
            .map(|occurrence| self.routes.pm_unavailable(&self.pm, occurrence))
            .transpose()
    }

    pub(crate) fn take_okx_unavailable(
        &mut self,
    ) -> Result<Option<OkxPublicUnavailableDelivery>, PmPublicRouteError> {
        self.okx
            .take_unavailable()
            .map(|occurrence| self.routes.okx_unavailable(&self.okx, occurrence))
            .transpose()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn preflight_pm_lane_fault(
        &self,
        source: PmProductSource,
        authority_id: PmPublicRouteAuthorityId,
        connection: PmConnectionId,
        ordering: EventOrdering,
        received_clock: ReceivedEventClock,
        local_wall_now_ns: u64,
        monotonic_now_ns: u64,
        reducer: &PmBookReducer,
    ) -> Result<(), PmPublicLaneFaultError> {
        if authority_id != self.routes.authority_id()
            || source != self.pm.role().source()
            || connection != self.pm.role().connection()
            || ordering.connection_epoch() != self.pm.connection_epoch()
            || self.pm.requires_reconnect()
            || monotonic_now_ns < received_clock.monotonic_receive_ns()
        {
            return Err(PmPublicLaneFaultError::EvidenceMismatch);
        }
        self.pm
            .preflight_invalidate_with_receive_evidence(local_wall_now_ns, monotonic_now_ns)?;
        if reducer.instrument() != self.pm.role().instrument()
            || reducer.expected_metadata_fingerprint() != self.expected_metadata_fingerprint
            || reducer.expected_metadata_contract() != self.expected_metadata_contract
            || reducer.connection_epoch() != Some(self.pm.connection_epoch())
        {
            return Err(PmPublicLaneFaultError::ReducerConfigurationMismatchWithoutOccurrence);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn preflight_okx_lane_fault(
        &self,
        source: PmProductSource,
        authority_id: PmPublicRouteAuthorityId,
        connection: PmConnectionId,
        ordering: EventOrdering,
        received_clock: ReceivedEventClock,
        local_wall_now_ns: u64,
        monotonic_now_ns: u64,
    ) -> Result<(), PmPublicLaneFaultError> {
        if authority_id != self.routes.authority_id()
            || source != self.okx_source
            || connection != self.okx_connection
            || ordering.connection_epoch().value() != self.okx.connection_epoch()
            || self.okx.requires_reconnect()
            || monotonic_now_ns < received_clock.monotonic_receive_ns()
        {
            return Err(PmPublicLaneFaultError::EvidenceMismatch);
        }
        self.okx
            .preflight_invalidate_with_receive_evidence(local_wall_now_ns, monotonic_now_ns)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn enact_pm_lane_fault(
        &mut self,
        source: PmProductSource,
        authority_id: PmPublicRouteAuthorityId,
        connection: PmConnectionId,
        ordering: EventOrdering,
        received_clock: ReceivedEventClock,
        local_wall_now_ns: u64,
        monotonic_now_ns: u64,
        session_fault: PmPublicSessionFault,
        reducer_fault: PmExternalBookFault,
        expected_reason: PmPublicReadinessReason,
        reducer: &mut PmBookReducer,
        pending_fault: Option<&PmPendingExternalBookFaultAuthority>,
    ) -> Result<(PmPublicUnavailableDelivery, PmPublicReadinessReason), PmPublicLaneFaultError>
    {
        self.preflight_pm_lane_fault(
            source,
            authority_id,
            connection,
            ordering,
            received_clock,
            local_wall_now_ns,
            monotonic_now_ns,
            reducer,
        )?;
        let unavailable = self
            .invalidate_and_route_pm(session_fault, local_wall_now_ns, monotonic_now_ns)?
            .ok_or(PmPublicLaneFaultError::MissingUnavailableOccurrence)?;
        let reducer_reason = match pending_fault {
            Some(authority) => reducer
                .finalize_pending_external_fault(authority, reducer_fault)
                .err(),
            None => reducer
                .apply_external_fault(ordering.connection_epoch(), reducer_fault)
                .err(),
        };
        let Some(reducer_reason) = reducer_reason else {
            return Err(PmPublicLaneFaultError::ReducerDidNotFailClosed { unavailable });
        };
        if reducer_reason != expected_reason {
            return Err(PmPublicLaneFaultError::ReducerFaultMismatch {
                expected: expected_reason,
                actual: reducer_reason,
                unavailable,
            });
        }
        Ok((unavailable, reducer_reason))
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the internal authority boundary names every independently authenticated route and receive-evidence component"
    )]
    pub(crate) fn enact_okx_lane_fault(
        &mut self,
        source: PmProductSource,
        authority_id: PmPublicRouteAuthorityId,
        connection: PmConnectionId,
        ordering: EventOrdering,
        received_clock: ReceivedEventClock,
        local_wall_now_ns: u64,
        monotonic_now_ns: u64,
        fault: OkxPublicSessionFault,
    ) -> Result<OkxPublicUnavailableDelivery, PmPublicLaneFaultError> {
        self.preflight_okx_lane_fault(
            source,
            authority_id,
            connection,
            ordering,
            received_clock,
            local_wall_now_ns,
            monotonic_now_ns,
        )?;
        self.invalidate_and_route_okx(fault, local_wall_now_ns, monotonic_now_ns)?
            .ok_or(PmPublicLaneFaultError::MissingUnavailableOccurrence)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn preflight_already_unavailable_pm_lane_fault(
        &self,
        source: PmProductSource,
        authority_id: PmPublicRouteAuthorityId,
        connection: PmConnectionId,
        ordering: EventOrdering,
        received_clock: ReceivedEventClock,
        monotonic_now_ns: u64,
        existing_fault: PmPublicSessionFault,
        reducer: &PmBookReducer,
    ) -> Result<(), PmPublicLaneFaultError> {
        if authority_id != self.routes.authority_id()
            || source != self.pm.role().source()
            || connection != self.pm.role().connection()
            || ordering.connection_epoch() != self.pm.connection_epoch()
            || !self.pm.requires_reconnect()
            || self.pm.last_fault() != Some(existing_fault)
            || monotonic_now_ns < received_clock.monotonic_receive_ns()
        {
            return Err(PmPublicLaneFaultError::EvidenceMismatch);
        }
        if reducer.instrument() != self.pm.role().instrument()
            || reducer.expected_metadata_fingerprint() != self.expected_metadata_fingerprint
            || reducer.expected_metadata_contract() != self.expected_metadata_contract
            || reducer.connection_epoch() != Some(self.pm.connection_epoch())
        {
            return Err(PmPublicLaneFaultError::ReducerConfigurationMismatchWithoutOccurrence);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn enact_already_unavailable_pm_lane_fault(
        &mut self,
        source: PmProductSource,
        authority_id: PmPublicRouteAuthorityId,
        connection: PmConnectionId,
        ordering: EventOrdering,
        received_clock: ReceivedEventClock,
        monotonic_now_ns: u64,
        existing_fault: PmPublicSessionFault,
        reducer_fault: PmExternalBookFault,
        expected_reason: PmPublicReadinessReason,
        reducer: &mut PmBookReducer,
    ) -> Result<PmPublicReadinessReason, PmPublicLaneFaultError> {
        self.preflight_already_unavailable_pm_lane_fault(
            source,
            authority_id,
            connection,
            ordering,
            received_clock,
            monotonic_now_ns,
            existing_fault,
            reducer,
        )?;
        let reason = reducer
            .readiness()
            .reason()
            .ok_or(PmPublicLaneFaultError::ReducerDidNotFailClosedWithoutOccurrence)?;
        if reason != expected_reason {
            return Err(
                PmPublicLaneFaultError::ReducerFaultMismatchWithoutOccurrence {
                    expected: expected_reason,
                    actual: reason,
                },
            );
        }
        let counters = reducer.counters();
        let has_exact_fault_evidence = counters.external_faults > 0
            && match reducer_fault {
                PmExternalBookFault::Disconnect => counters.disconnects > 0,
                PmExternalBookFault::HeartbeatTimeout => counters.heartbeat_timeouts > 0,
                PmExternalBookFault::Gap => counters.gaps > 0,
                PmExternalBookFault::Overflow => counters.overflows > 0,
                PmExternalBookFault::BacklogAged => counters.backlog_aged_faults > 0,
                PmExternalBookFault::InvalidTransition => counters.invalid_transitions > 0,
                PmExternalBookFault::HashMismatch => counters.hash_mismatches > 0,
            };
        if !has_exact_fault_evidence {
            return Err(PmPublicLaneFaultError::ReducerDidNotFailClosedWithoutOccurrence);
        }
        Ok(reason)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn validate_already_unavailable_okx_lane_fault(
        &self,
        source: PmProductSource,
        authority_id: PmPublicRouteAuthorityId,
        connection: PmConnectionId,
        ordering: EventOrdering,
        received_clock: ReceivedEventClock,
        monotonic_now_ns: u64,
        existing_fault: OkxPublicSessionFault,
    ) -> Result<(), PmPublicLaneFaultError> {
        if authority_id != self.routes.authority_id()
            || source != self.okx_source
            || connection != self.okx_connection
            || ordering.connection_epoch().value() != self.okx.connection_epoch()
            || !self.okx.requires_reconnect()
            || self.okx.last_fault() != Some(existing_fault)
            || monotonic_now_ns < received_clock.monotonic_receive_ns()
        {
            return Err(PmPublicLaneFaultError::EvidenceMismatch);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn preflight_terminal_tick_size_aged(
        &self,
        source: PmProductSource,
        authority_id: PmPublicRouteAuthorityId,
        connection: PmConnectionId,
        ordering: EventOrdering,
        received_clock: ReceivedEventClock,
        monotonic_now_ns: u64,
        reducer: &PmBookReducer,
    ) -> Result<PmBookBatchEvidence, PmPublicLaneFaultError> {
        if !self.pm_terminal_tick_delivery_is_current(authority_id, source, connection, ordering)
            || monotonic_now_ns < received_clock.monotonic_receive_ns()
        {
            return Err(PmPublicLaneFaultError::EvidenceMismatch);
        }
        if reducer.instrument() != self.pm.role().instrument()
            || reducer.expected_metadata_fingerprint() != self.expected_metadata_fingerprint
            || reducer.expected_metadata_contract() != self.expected_metadata_contract
            || reducer.connection_epoch() != Some(self.pm.connection_epoch())
        {
            return Err(PmPublicLaneFaultError::ReducerConfigurationMismatchWithoutOccurrence);
        }
        let snapshot_revision = ordering
            .snapshot_revision()
            .ok_or(PmPublicLaneFaultError::EvidenceMismatch)?;
        PmBookBatchEvidence::new(
            self.pm.role().instrument(),
            ordering.connection_epoch(),
            self.pm.metadata_revision(),
            snapshot_revision,
            ordering.local_ingress_sequence(),
            received_clock.monotonic_receive_ns(),
            ordering.venue_hash(),
        )
        .map_err(|_| PmPublicLaneFaultError::EvidenceMismatch)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn enact_terminal_tick_size_aged(
        &self,
        source: PmProductSource,
        authority_id: PmPublicRouteAuthorityId,
        connection: PmConnectionId,
        ordering: EventOrdering,
        received_clock: ReceivedEventClock,
        monotonic_now_ns: u64,
        old: reap_pm_core::PmTick,
        new: reap_pm_core::PmTick,
        reducer: &mut PmBookReducer,
    ) -> Result<PmPublicReadinessReason, PmPublicLaneFaultError> {
        let evidence = self.preflight_terminal_tick_size_aged(
            source,
            authority_id,
            connection,
            ordering,
            received_clock,
            monotonic_now_ns,
            reducer,
        )?;
        let actual = reducer
            .tick_size_changed(evidence, old, new)
            .err()
            .ok_or(PmPublicLaneFaultError::ReducerDidNotFailClosedWithoutOccurrence)?;
        let expected = PmPublicReadinessReason::MetadataDrift(reap_pm_state::PmMetadataDrift::Grid);
        if actual != expected {
            return Err(
                PmPublicLaneFaultError::ReducerFaultMismatchWithoutOccurrence { expected, actual },
            );
        }
        Ok(actual)
    }

    pub(crate) fn after_pm_failure(
        &mut self,
    ) -> Result<(u64, u64, Duration), PmPublicSessionError> {
        let prior_epoch = self.pm_epoch();
        let delay = self.pm.after_failure()?;
        Ok((prior_epoch, self.pm_epoch(), delay))
    }

    pub(crate) fn preview_pm_failure(&self) -> Result<(u64, u64, Duration), PmPublicSessionError> {
        let prior_epoch = self.pm_epoch();
        let (next_epoch, delay) = self.pm.preview_after_failure()?;
        Ok((prior_epoch, next_epoch.value(), delay))
    }

    pub(crate) fn after_okx_failure(
        &mut self,
    ) -> Result<(u64, u64, Duration), OkxPublicSessionError> {
        let prior_epoch = self.okx_epoch();
        let delay = self.okx.after_failure()?;
        Ok((prior_epoch, self.okx_epoch(), delay))
    }

    pub(crate) fn preview_okx_failure(
        &self,
    ) -> Result<(u64, u64, Duration), OkxPublicSessionError> {
        let prior_epoch = self.okx_epoch();
        let (next_epoch, delay) = self.okx.preview_after_failure()?;
        Ok((prior_epoch, next_epoch, delay))
    }

    pub(crate) fn preflight_pm_invalidation(
        &self,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<(), PmPublicSessionError> {
        self.pm
            .preflight_invalidate_with_receive_evidence(local_wall_receive_ns, monotonic_receive_ns)
    }

    pub(crate) fn preflight_okx_invalidation(
        &self,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<(), OkxPublicSessionError> {
        self.okx
            .preflight_invalidate_with_receive_evidence(local_wall_receive_ns, monotonic_receive_ns)
    }
}

/// Final route-issued outputs from one captured PM public frame.
#[derive(Debug)]
#[must_use = "active PM deliveries must be consumed by their authorized reducer or lane operation"]
pub struct PmPublicCaptureBatch {
    books: Vec<PmPublicBookDelivery>,
    ignored_public_trades: u8,
    snapshot_flow: Option<PmPublicSnapshotFlow>,
    heartbeat: Option<PmPublicHeartbeatEvidence>,
    unavailable: Option<PmPublicUnavailableDelivery>,
}

impl PmPublicCaptureBatch {
    pub fn books(&self) -> &[PmPublicBookDelivery] {
        &self.books
    }

    #[must_use]
    pub fn into_books(self) -> Vec<PmPublicBookDelivery> {
        self.books
    }

    #[must_use]
    pub const fn ignored_public_trades(&self) -> u8 {
        self.ignored_public_trades
    }

    #[must_use]
    pub fn take_snapshot_flow(&mut self) -> Option<PmPublicSnapshotFlow> {
        self.snapshot_flow.take()
    }

    #[must_use]
    pub const fn heartbeat(&self) -> Option<PmPublicHeartbeatEvidence> {
        self.heartbeat
    }

    #[must_use]
    pub fn take_unavailable(&mut self) -> Option<PmPublicUnavailableDelivery> {
        self.unavailable.take()
    }
}

/// Opaque flow token issued only alongside this run's routed snapshot.
#[derive(Debug)]
pub struct PmPublicSnapshotFlow {
    token: PmSnapshotFlowToken,
    authority_id: PmPublicRouteAuthorityId,
}

impl PmPublicSnapshotFlow {
    #[must_use]
    pub const fn connection_epoch(&self) -> reap_pm_core::ConnectionEpoch {
        self.token.connection_epoch()
    }

    #[must_use]
    pub const fn snapshot_revision(&self) -> reap_pm_core::SnapshotRevision {
        self.token.snapshot_revision()
    }

    #[must_use]
    pub const fn local_ingress_sequence(&self) -> IngressSequence {
        self.token.local_ingress_sequence()
    }
}

/// Crate-private routed result before the active Run performs atomic lane
/// admission.
#[allow(
    clippy::large_enum_variant,
    reason = "the active Run must retain exact routed reference evidence without allocation"
)]
#[derive(Debug)]
pub(crate) enum OkxCaptureRoleEvent {
    SubscriptionAcknowledged(OkxPublicEventEvidence),
    Heartbeat(OkxPublicEventEvidence),
    Control(OkxPublicControlEvidence),
    Reference(OkxPublicReferenceDelivery),
}

/// Final public fact from one atomically captured and admitted OKX frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OkxPublicCaptureEvent {
    SubscriptionAcknowledged(OkxPublicEventEvidence),
    Heartbeat(OkxPublicEventEvidence),
    Control(OkxPublicControlEvidence),
    ReferenceEnqueued,
}

#[derive(Debug, Error)]
pub enum PmPublicSnapshotCommitError {
    #[error("routed PM snapshot or flow belongs to a sibling capture authority")]
    RouteAuthorityMismatch,
    #[error("routed PM delivery is not a snapshot")]
    DeliveryIsNotSnapshot,
    #[error("routed PM snapshot has no snapshot revision")]
    MissingSnapshotRevision,
    #[error("routed PM snapshot does not match its move-only flow evidence")]
    DeliveryFlowMismatch,
    #[error("routed PM snapshot does not match the active owned session")]
    ActiveSessionMismatch,
    #[error("PM reducer configuration does not match authoritative capture configuration")]
    ReducerConfigurationMismatch,
    #[error("PM reducer did not report a committed snapshot")]
    ReducerDidNotCommitSnapshot,
    #[error("PM reducer commit proof does not match the exact routed snapshot")]
    ReducerProofMismatch,
    #[error("PM reducer rejected the exact routed snapshot: {0}")]
    Reducer(#[from] PmPublicReadinessReason),
    #[error("PM protocol flow could not be opened after the exact snapshot commit: {0}")]
    Protocol(#[from] PmPublicSessionError),
}

#[derive(Debug, Error)]
pub enum PmPublicBookReduceError {
    #[error("routed PM book update belongs to a sibling capture authority")]
    RouteAuthorityMismatch,
    #[error("routed PM book delivery is not a delta batch or top check")]
    DeliveryIsNotFlowUpdate,
    #[error("routed PM book update has no snapshot revision")]
    MissingSnapshotRevision,
    #[error("routed PM book update does not match the active owned session and flow")]
    ActiveSessionMismatch,
    #[error("PM reducer configuration does not match authoritative capture configuration")]
    ReducerConfigurationMismatch,
    #[error("PM reducer state does not match the routed update epoch and revisions")]
    ReducerStateMismatch,
    #[error("PM reducer rejected the exact routed update: {0}")]
    Reducer(#[from] PmPublicReadinessReason),
    #[error("PM reducer transition did not match the exact routed update")]
    ReducerTransitionMismatch,
}

#[derive(Debug, Error)]
pub enum PmPublicReducerSyncError {
    #[error("PM reducer configuration does not match authoritative capture configuration")]
    ReducerConfigurationMismatch,
    #[error("PM session and reducer epoch/readiness state do not match")]
    SessionReducerStateMismatch,
    #[error("PM reducer rejected the synchronized lifecycle transition: {0}")]
    Reducer(#[from] PmPublicReadinessReason),
    #[error("PM reducer lifecycle transition did not match the enacted session transition")]
    ReducerTransitionMismatch,
    #[error("PM reducer lifecycle reason mismatch: expected {expected}, observed {actual}")]
    ReducerReasonMismatch {
        expected: PmPublicReadinessReason,
        actual: PmPublicReadinessReason,
    },
}

/// Exact product-state result of one durably recorded freshness timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmPublicFreshnessTimerOutcome {
    Confirmed,
    Unavailable { reason: PmPublicReadinessReason },
}

impl PmPublicFreshnessTimerOutcome {
    #[must_use]
    pub const fn unavailable_reason(self) -> Option<PmPublicReadinessReason> {
        match self {
            Self::Confirmed => None,
            Self::Unavailable { reason } => Some(reason),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum ExpectedPmBookTransition {
    DeltaBatch(usize),
    TopCheck,
}

impl ExpectedPmBookTransition {
    fn matches(
        self,
        transition: &PmBookTransition,
        snapshot_revision: reap_pm_core::SnapshotRevision,
    ) -> bool {
        match (self, transition) {
            (
                Self::DeltaBatch(expected_changes),
                PmBookTransition::DeltaBatchCommitted { revision, changes },
            ) => *revision == snapshot_revision && usize::from(*changes) == expected_changes,
            (Self::TopCheck, PmBookTransition::TopConfirmed) => true,
            _ => false,
        }
    }

    fn matches_counters(
        self,
        before: reap_pm_state::PmBookCounters,
        after: reap_pm_state::PmBookCounters,
    ) -> bool {
        match self {
            Self::DeltaBatch(changes) => {
                after.delta_batch_attempts == before.delta_batch_attempts.saturating_add(1)
                    && after.delta_batches_committed
                        == before.delta_batches_committed.saturating_add(1)
                    && after.delta_changes_committed
                        == before
                            .delta_changes_committed
                            .saturating_add(u64::try_from(changes).expect("bounded PM changes"))
                    && after.delta_top_checks == before.delta_top_checks.saturating_add(1)
                    && after.delta_top_checks_confirmed
                        == before.delta_top_checks_confirmed.saturating_add(1)
            }
            Self::TopCheck => {
                after.top_checks == before.top_checks.saturating_add(1)
                    && after.top_checks_confirmed == before.top_checks_confirmed.saturating_add(1)
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum PmPublicLaneFaultError {
    #[error("public lane fault evidence does not match the active owned venue session")]
    EvidenceMismatch,
    #[error("PM reducer identity/configuration does not match the active owned venue session")]
    ReducerConfigurationMismatch {
        unavailable: PmPublicUnavailableDelivery,
    },
    #[error("PM reducer identity/configuration does not match an already-unavailable stream")]
    ReducerConfigurationMismatchWithoutOccurrence,
    #[error("PM reducer unexpectedly remained available after a public lane fault")]
    ReducerDidNotFailClosed {
        unavailable: PmPublicUnavailableDelivery,
    },
    #[error("PM reducer unexpectedly remained available for an already-unavailable stream")]
    ReducerDidNotFailClosedWithoutOccurrence,
    #[error("PM reducer lane-fault reason mismatch: expected {expected}, observed {actual}")]
    ReducerFaultMismatch {
        expected: PmPublicReadinessReason,
        actual: PmPublicReadinessReason,
        unavailable: PmPublicUnavailableDelivery,
    },
    #[error(
        "PM reducer already-unavailable reason mismatch: expected {expected}, observed {actual}"
    )]
    ReducerFaultMismatchWithoutOccurrence {
        expected: PmPublicReadinessReason,
        actual: PmPublicReadinessReason,
    },
    #[error("owned venue session did not issue the required unavailable occurrence")]
    MissingUnavailableOccurrence,
    #[error(transparent)]
    PmSession(#[from] PmPublicSessionError),
    #[error(transparent)]
    OkxSession(#[from] OkxPublicSessionError),
    #[error(transparent)]
    Route(#[from] PmPublicRouteError),
}

#[derive(Debug, Error)]
#[allow(
    clippy::large_enum_variant,
    reason = "snapshot failure retains exact move-only unavailable evidence on the bounded fail-closed path"
)]
pub(crate) enum PmCaptureSnapshotCommitFailure {
    #[error("PM snapshot commit failed: {source}")]
    Commit {
        #[source]
        source: PmPublicSnapshotCommitError,
        unavailable: Option<PmPublicUnavailableDelivery>,
    },
    #[error(transparent)]
    Route(#[from] PmPublicRouteError),
}

#[derive(Debug, Error)]
#[allow(
    clippy::large_enum_variant,
    reason = "book-reduce failure retains exact move-only unavailable evidence on the bounded fail-closed path"
)]
pub(crate) enum PmCaptureBookReduceFailure {
    #[error("PM routed book update reduction failed: {source}")]
    Reduce {
        #[source]
        source: PmPublicBookReduceError,
        unavailable: Option<PmPublicUnavailableDelivery>,
    },
    #[error(transparent)]
    Route(#[from] PmPublicRouteError),
}

#[derive(Debug, Error)]
#[allow(
    clippy::large_enum_variant,
    reason = "classification failure retains exact move-only unavailable evidence on the bounded fail-closed path"
)]
pub(crate) enum PmCaptureRoleIngressError {
    #[error(transparent)]
    PmSession(#[from] PmPublicSessionError),
    #[error(transparent)]
    OkxSession(#[from] OkxPublicSessionError),
    #[error("captured OKX public payload is not UTF-8 text")]
    OkxRawNotUtf8 {
        unavailable: Option<OkxPublicUnavailableDelivery>,
    },
    #[error(transparent)]
    Route(#[from] PmPublicRouteError),
}

#[derive(Debug, Error)]
pub(crate) enum PmCaptureRoleStartError {
    #[error(transparent)]
    Header(#[from] crate::capture::PmCaptureVerifyError),
    #[error(transparent)]
    PmSession(#[from] PmPublicSessionError),
    #[error(transparent)]
    OkxSession(#[from] OkxPublicSessionError),
    #[error(transparent)]
    MetadataContract(#[from] reap_pm_state::PmMetadataContractError),
    #[error(transparent)]
    Route(#[from] PmPublicRouteError),
}
