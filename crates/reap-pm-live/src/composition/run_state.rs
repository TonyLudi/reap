use super::*;

pub(super) const MAX_PENDING_PM_BOOK_REDUCTIONS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PublicLifecyclePhase {
    AwaitingConnection,
    AwaitingSubscription,
    Live,
    Disconnected,
}

impl PublicLifecyclePhase {
    pub(super) const fn accepts_connection_start(self) -> bool {
        matches!(self, Self::AwaitingConnection)
    }

    pub(super) const fn accepts_subscription(self) -> bool {
        matches!(self, Self::AwaitingSubscription)
    }

    pub(super) const fn accepts_live_input(self) -> bool {
        matches!(self, Self::Live)
    }

    pub(super) const fn accepts_reconnect(self) -> bool {
        matches!(self, Self::Disconnected)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingPmBookKind {
    Snapshot,
    DeltaBatch,
    TopCheck,
    TickSizeChanged,
}

impl PendingPmBookKind {
    pub(super) const fn from_update(update: &reap_pm_core::PmBookUpdate) -> Self {
        match update {
            reap_pm_core::PmBookUpdate::Snapshot(_) => Self::Snapshot,
            reap_pm_core::PmBookUpdate::DeltaBatch(_) => Self::DeltaBatch,
            reap_pm_core::PmBookUpdate::TopCheck(_) => Self::TopCheck,
            reap_pm_core::PmBookUpdate::TickSizeChanged { .. } => Self::TickSizeChanged,
        }
    }
}

/// Bounded identity of one route-issued PM book capability that still owes an
/// exact reducer transition to this active Run.
///
/// Payloads cannot be forged or cloned outside the crate. The opaque authority
/// plus the route/session ordering and receive clock therefore bind this
/// obligation to one exact delivery without retaining a second book payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PendingPmBookReduction {
    authority_id: PmPublicRouteAuthorityId,
    source: PmProductSource,
    connection: PmConnectionId,
    ordering: EventOrdering,
    received_clock: ReceivedEventClock,
    kind: PendingPmBookKind,
}

impl PendingPmBookReduction {
    pub(super) fn from_delivery(delivery: &PmPublicBookDelivery) -> Self {
        let envelope = delivery.envelope();
        Self {
            authority_id: delivery.authority_id(),
            source: envelope.source(),
            connection: envelope.connection_id(),
            ordering: envelope.ordering(),
            received_clock: envelope.received_clock(),
            kind: PendingPmBookKind::from_update(envelope.payload().update()),
        }
    }

    pub(super) fn matches(
        self,
        delivery: &PmPublicBookDelivery,
        expected_kind: PendingPmBookKind,
    ) -> bool {
        let envelope = delivery.envelope();
        self.authority_id == delivery.authority_id()
            && self.source == envelope.source()
            && self.connection == envelope.connection_id()
            && self.ordering == envelope.ordering()
            && self.received_clock == envelope.received_clock()
            && self.kind == expected_kind
            && self.kind == PendingPmBookKind::from_update(envelope.payload().update())
    }
}

#[derive(Debug)]
pub(super) struct PendingPmBookLaneFault {
    reduction: PendingPmBookReduction,
    reducer_fault_authority: PmPendingExternalBookFaultAuthority,
}

impl PendingPmBookLaneFault {
    pub(super) const fn new(
        reduction: PendingPmBookReduction,
        reducer_fault_authority: PmPendingExternalBookFaultAuthority,
    ) -> Self {
        Self {
            reduction,
            reducer_fault_authority,
        }
    }

    pub(super) fn matches(&self, delivery: &PmPublicBookDelivery) -> bool {
        self.reduction.matches(
            delivery,
            PendingPmBookKind::from_update(delivery.envelope().payload().update()),
        )
    }

    pub(super) const fn reducer_fault_authority(&self) -> &PmPendingExternalBookFaultAuthority {
        &self.reducer_fault_authority
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PendingPmRouteFaultIdentity {
    authority_id: PmPublicRouteAuthorityId,
    source: PmProductSource,
    connection: PmConnectionId,
    ordering: EventOrdering,
    received_clock: ReceivedEventClock,
}

impl PendingPmRouteFaultIdentity {
    pub(super) fn from_metadata(delivery: &PmPublicMetadataDelivery) -> Self {
        let envelope = delivery.envelope();
        Self {
            authority_id: delivery.authority_id(),
            source: envelope.source(),
            connection: envelope.connection_id(),
            ordering: envelope.ordering(),
            received_clock: envelope.received_clock(),
        }
    }

    pub(super) fn matches_metadata(self, delivery: &PmPublicMetadataDelivery) -> bool {
        self == Self::from_metadata(delivery)
    }

    pub(super) fn from_okx_reference(delivery: &OkxPublicReferenceDelivery) -> Self {
        let envelope = delivery.envelope();
        Self {
            authority_id: delivery.authority_id(),
            source: envelope.source(),
            connection: envelope.connection_id(),
            ordering: envelope.ordering(),
            received_clock: envelope.received_clock(),
        }
    }

    pub(super) fn matches_okx_reference(self, delivery: &OkxPublicReferenceDelivery) -> bool {
        self == Self::from_okx_reference(delivery)
    }

    pub(super) fn from_pm_unavailable(delivery: &PmPublicUnavailableDelivery) -> Self {
        let envelope = delivery.envelope();
        Self {
            authority_id: delivery.authority_id(),
            source: envelope.source(),
            connection: envelope.connection_id(),
            ordering: envelope.ordering(),
            received_clock: envelope.received_clock(),
        }
    }

    pub(super) fn matches_pm_unavailable(self, delivery: &PmPublicUnavailableDelivery) -> bool {
        self == Self::from_pm_unavailable(delivery)
    }

    pub(super) fn from_okx_unavailable(delivery: &OkxPublicUnavailableDelivery) -> Self {
        let envelope = delivery.envelope();
        Self {
            authority_id: delivery.authority_id(),
            source: envelope.source(),
            connection: envelope.connection_id(),
            ordering: envelope.ordering(),
            received_clock: envelope.received_clock(),
        }
    }

    pub(super) fn matches_okx_unavailable(self, delivery: &OkxPublicUnavailableDelivery) -> bool {
        self == Self::from_okx_unavailable(delivery)
    }

    pub(super) fn from_aged(evidence: &PmAgedDeliveryEvidence) -> Self {
        Self {
            authority_id: evidence.public_authority_id(),
            source: evidence.public_source(),
            connection: evidence.connection(),
            ordering: evidence.ordering(),
            received_clock: evidence.received_clock(),
        }
    }

    pub(super) fn matches_aged(self, evidence: &PmAgedDeliveryEvidence) -> bool {
        Self::from_aged(evidence) == self
    }
}

#[derive(Debug)]
pub(super) struct PendingPmMetadataLaneFault {
    identity: PendingPmRouteFaultIdentity,
    reducer_fault_authority: PmPendingExternalBookFaultAuthority,
}

impl PendingPmMetadataLaneFault {
    pub(super) const fn new(
        identity: PendingPmRouteFaultIdentity,
        reducer_fault_authority: PmPendingExternalBookFaultAuthority,
    ) -> Self {
        Self {
            identity,
            reducer_fault_authority,
        }
    }

    pub(super) fn matches(&self, delivery: &PmPublicMetadataDelivery) -> bool {
        self.identity.matches_metadata(delivery)
    }

    pub(super) const fn reducer_fault_authority(&self) -> &PmPendingExternalBookFaultAuthority {
        &self.reducer_fault_authority
    }
}

#[derive(Debug)]
pub(super) struct PendingPmAgedLaneFault {
    identity: PendingPmRouteFaultIdentity,
    observed_now_ns: u64,
    reducer_fault_authority: PmPendingExternalBookFaultAuthority,
}

impl PendingPmAgedLaneFault {
    pub(super) fn new(
        evidence: &PmAgedDeliveryEvidence,
        reducer_fault_authority: PmPendingExternalBookFaultAuthority,
    ) -> Self {
        Self {
            identity: PendingPmRouteFaultIdentity::from_aged(evidence),
            observed_now_ns: evidence.observed_now_ns(),
            reducer_fault_authority,
        }
    }

    pub(super) fn matches(&self, evidence: &PmAgedDeliveryEvidence) -> bool {
        self.observed_now_ns == evidence.observed_now_ns() && self.identity.matches_aged(evidence)
    }

    pub(super) const fn reducer_fault_authority(&self) -> &PmPendingExternalBookFaultAuthority {
        &self.reducer_fault_authority
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PendingOtherAgedLaneFault {
    identity: PendingPmRouteFaultIdentity,
    observed_now_ns: u64,
    head: PmPublicAgedHead,
}

impl PendingOtherAgedLaneFault {
    pub(super) fn new(evidence: &PmAgedDeliveryEvidence) -> Self {
        Self {
            identity: PendingPmRouteFaultIdentity::from_aged(evidence),
            observed_now_ns: evidence.observed_now_ns(),
            head: evidence.public_head(),
        }
    }

    pub(super) fn matches(&self, evidence: &PmAgedDeliveryEvidence) -> bool {
        self.observed_now_ns == evidence.observed_now_ns()
            && evidence.public_head() == self.head
            && self.identity.matches_aged(evidence)
    }
}

impl PmPublicCaptureRun {
    #[must_use]
    pub fn pm_book_readiness(&self) -> PmPublicBookReadiness {
        let reason = if self.artifact_terminal() {
            Some(PmPublicBookReadinessReason::ArtifactTerminal)
        } else if self.public_lane.consumer_transfer_poisoned() {
            Some(PmPublicBookReadinessReason::ConsumerTransferPoisoned)
        } else if !self.pm_lifecycle.accepts_live_input() {
            Some(PmPublicBookReadinessReason::LifecycleUnavailable)
        } else if self.has_pending_pm_book_reductions()
            || self.terminal_tick_cleanup != PmPublicTerminalTickCleanupStatus::NotRequired
        {
            Some(PmPublicBookReadinessReason::PendingReduction)
        } else if self.has_pending_pm_lane_fault() {
            Some(PmPublicBookReadinessReason::PendingLaneFault)
        } else if let Some(reason) = self.pm_reducer.readiness().reason() {
            Some(PmPublicBookReadinessReason::Reducer(reason))
        } else if self
            .roles
            .preflight_pm_reducer_freshness(&self.pm_reducer)
            .is_err()
        {
            Some(PmPublicBookReadinessReason::SessionReducerMismatch)
        } else {
            None
        };

        match reason {
            Some(reason) => PmPublicBookReadiness::unavailable(&self.pm_reducer, reason),
            None => PmPublicBookReadiness::ready(&self.pm_reducer),
        }
    }

    #[must_use]
    pub fn ready_pm_book_view(&self) -> Option<PmPublicReadyBookView<'_>> {
        let readiness = self.pm_book_readiness();
        PmPublicReadyBookView::new(&self.pm_reducer, readiness)
    }

    /// Copied diagnostics only; quote consumers must use
    /// [`Self::ready_pm_book_view`] for book authority.
    #[must_use]
    pub const fn pm_book_counters(&self) -> PmBookCounters {
        self.pm_reducer.counters()
    }

    #[must_use]
    pub const fn pm_book_last_ingress_sequence(&self) -> Option<IngressSequence> {
        self.pm_reducer.last_ingress_sequence()
    }

    #[must_use]
    pub const fn pm_book_last_verified_snapshot_hash(
        &self,
    ) -> Option<reap_pm_core::VenueEventHash> {
        self.pm_reducer.last_verified_snapshot_hash()
    }

    #[must_use]
    pub const fn pm_book_pending_external_fault(&self) -> Option<PmExternalBookFault> {
        self.pm_reducer.pending_external_fault()
    }

    pub(super) fn ensure_no_pending_pm_book_reductions(
        &self,
    ) -> Result<(), PmPublicCaptureRunError> {
        if !self.pending_pm_book_reductions.is_empty() {
            return Err(PmPublicCaptureRunError::PendingPmBookReductions {
                pending: self.pending_pm_book_reductions.len(),
            });
        }
        if self.has_pending_pm_lane_fault() {
            return Err(PmPublicCaptureRunError::PendingPmBookLaneFault);
        }
        Ok(())
    }

    pub(super) fn register_pm_book_reductions(
        &mut self,
        batch: &PmPublicCaptureBatch,
    ) -> Result<(), PmPublicCaptureRunError> {
        debug_assert!(self.pending_pm_book_reductions.is_empty());
        if batch.books().len() > MAX_PENDING_PM_BOOK_REDUCTIONS {
            self.terminalize_plain(PmPublicCaptureTerminalCause::InternalInvariant);
            return Err(PmPublicCaptureRunError::PendingPmBookReductionOverflow);
        }
        for delivery in batch.books() {
            let obligation = PendingPmBookReduction::from_delivery(delivery);
            if obligation.kind == PendingPmBookKind::TickSizeChanged {
                if self.terminal_tick_cleanup != PmPublicTerminalTickCleanupStatus::NotRequired {
                    self.terminalize_plain(PmPublicCaptureTerminalCause::InternalInvariant);
                    return Err(PmPublicCaptureRunError::PendingPmBookReductionOverflow);
                }
                self.terminal_tick_cleanup = PmPublicTerminalTickCleanupStatus::Pending;
            }
            self.pending_pm_book_reductions.push_back(obligation);
        }
        Ok(())
    }

    pub(super) fn pending_pm_book_matches(
        &self,
        delivery: &PmPublicBookDelivery,
        expected_kind: PendingPmBookKind,
    ) -> bool {
        self.pending_pm_book_reductions
            .front()
            .is_some_and(|pending| pending.matches(delivery, expected_kind))
    }

    pub(super) fn consume_pending_pm_book(&mut self) {
        let removed = self.pending_pm_book_reductions.pop_front();
        debug_assert!(removed.is_some());
    }

    pub(super) fn clear_pending_pm_book_reductions(&mut self) {
        self.pending_pm_book_reductions.clear();
    }

    pub(super) fn register_pending_pm_book_lane_fault(
        &mut self,
        pending: PendingPmBookLaneFault,
    ) -> Result<(), PendingPmBookLaneFault> {
        if self.pending_pm_book_lane_fault.is_some() {
            Err(pending)
        } else {
            self.pending_pm_book_lane_fault = Some(pending);
            Ok(())
        }
    }

    pub(super) fn ensure_no_pending_pm_book_lane_fault(
        &self,
    ) -> Result<(), PmPublicCaptureRunError> {
        if self.has_pending_pm_lane_fault() {
            Err(PmPublicCaptureRunError::PendingPmBookLaneFault)
        } else {
            Ok(())
        }
    }

    pub(super) fn pending_pm_book_lane_fault_matches(
        &self,
        delivery: &PmPublicBookDelivery,
    ) -> bool {
        self.pending_pm_book_lane_fault
            .as_ref()
            .is_some_and(|pending| pending.matches(delivery))
    }

    pub(super) fn clear_pending_pm_book_lane_fault(&mut self) {
        self.pending_pm_book_lane_fault = None;
    }

    pub(super) fn register_pending_pm_metadata_lane_fault(
        &mut self,
        pending: PendingPmMetadataLaneFault,
    ) -> Result<(), PendingPmMetadataLaneFault> {
        if self.pending_pm_metadata_lane_fault.is_some() {
            Err(pending)
        } else {
            self.pending_pm_metadata_lane_fault = Some(pending);
            Ok(())
        }
    }

    pub(super) fn pending_pm_metadata_lane_fault_matches(
        &self,
        delivery: &PmPublicMetadataDelivery,
    ) -> bool {
        self.pending_pm_metadata_lane_fault
            .as_ref()
            .is_some_and(|pending| pending.matches(delivery))
    }

    pub(super) fn clear_pending_pm_metadata_lane_fault(&mut self) {
        self.pending_pm_metadata_lane_fault = None;
    }

    pub(super) fn pending_okx_reference_lane_fault_matches(
        &self,
        delivery: &OkxPublicReferenceDelivery,
    ) -> bool {
        self.pending_okx_reference_lane_fault
            .is_some_and(|pending| pending.matches_okx_reference(delivery))
    }

    pub(super) fn clear_pending_okx_reference_lane_fault(&mut self) {
        self.pending_okx_reference_lane_fault = None;
    }

    pub(super) fn pending_pm_unavailable_lane_fault_matches(
        &self,
        delivery: &PmPublicUnavailableDelivery,
    ) -> bool {
        self.pending_pm_unavailable_lane_fault
            .is_some_and(|pending| pending.matches_pm_unavailable(delivery))
    }

    pub(super) fn clear_pending_pm_unavailable_lane_fault(&mut self) {
        self.pending_pm_unavailable_lane_fault = None;
    }

    pub(super) fn pending_okx_unavailable_lane_fault_matches(
        &self,
        delivery: &OkxPublicUnavailableDelivery,
    ) -> bool {
        self.pending_okx_unavailable_lane_fault
            .is_some_and(|pending| pending.matches_okx_unavailable(delivery))
    }

    pub(super) fn clear_pending_okx_unavailable_lane_fault(&mut self) {
        self.pending_okx_unavailable_lane_fault = None;
    }

    pub(super) const fn has_pending_pm_lane_fault(&self) -> bool {
        self.pending_pm_book_lane_fault.is_some()
            || self.pending_pm_metadata_lane_fault.is_some()
            || self.pending_pm_aged_lane_fault.is_some()
            || self.pending_okx_reference_lane_fault.is_some()
            || self.pending_pm_unavailable_lane_fault.is_some()
            || self.pending_okx_unavailable_lane_fault.is_some()
            || self.pending_other_aged_lane_fault.is_some()
    }

    pub(super) fn register_pending_pm_aged_lane_fault(
        &mut self,
        pending: PendingPmAgedLaneFault,
    ) -> Result<(), PendingPmAgedLaneFault> {
        if self.pending_pm_aged_lane_fault.is_some() {
            Err(pending)
        } else {
            self.pending_pm_aged_lane_fault = Some(pending);
            Ok(())
        }
    }

    pub(super) fn pending_pm_aged_lane_fault_matches(
        &self,
        evidence: &PmAgedDeliveryEvidence,
    ) -> bool {
        self.pending_pm_aged_lane_fault
            .as_ref()
            .is_some_and(|pending| pending.matches(evidence))
    }

    pub(super) fn clear_pending_pm_aged_lane_fault(&mut self) {
        self.pending_pm_aged_lane_fault = None;
    }

    pub(super) fn pending_other_aged_lane_fault_matches(
        &self,
        evidence: &PmAgedDeliveryEvidence,
    ) -> bool {
        self.pending_other_aged_lane_fault
            .as_ref()
            .is_some_and(|pending| pending.matches(evidence))
    }

    pub(super) fn clear_pending_other_aged_lane_fault(&mut self) {
        self.pending_other_aged_lane_fault = None;
    }

    #[must_use]
    pub fn pending_pm_book_reduction_count(&self) -> usize {
        self.pending_pm_book_reductions.len()
    }

    pub(super) fn has_pending_pm_book_reductions(&self) -> bool {
        !self.pending_pm_book_reductions.is_empty()
    }

    #[must_use]
    pub const fn has_pending_pm_book_lane_fault(&self) -> bool {
        self.pending_pm_book_lane_fault.is_some()
    }
}
