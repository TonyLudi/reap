use reap_pm_core::{
    EventEnvelope, PmClientOrderKey, PmFillEvent, PmOrderEvent, PmOrderIdentity, PmOrderProgress,
    SnapshotRevision, U256,
};

use crate::fill_state::PmFillApply;
use crate::order_state::{PmOrderApply, PmOwnedOrderRegistration, PmRemoteOrderKnowledge};
use crate::owned_lifecycle::{
    PmOwnedCancelApply, PmOwnedCancelIntent, PmOwnedCancelOutcome, PmOwnedCancelRequestApply,
    PmOwnedFillApply, PmOwnedFillObservation, PmOwnedFillProjection, PmOwnedLifecycleCounters,
    PmOwnedObservationOccurrence, PmOwnedObservationSource, PmOwnedOrderLifecycle,
    PmOwnedOrderLifecycleError, PmOwnedOrderProgressObservation, PmOwnedOrderProjection,
    PmOwnedProgressApply, PmOwnedQuoteAdmission, PmOwnedQuoteIntent, PmOwnedQuoteSlotProjection,
    PmOwnedRecoveryFill, PmOwnedReductionSequence, PmOwnedRemoteOrderApply, PmOwnedSubmitApply,
    PmOwnedSubmitResult, PmOwnedTerminalCompaction,
};
use crate::private_occurrence::PmPrivateOccurrence;
use crate::refresh::PmRefreshOwnerId;

use super::{PmPrivateState, PmPrivateStateError};

/// Exact owned-lifecycle consequence of one private order observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmOwnedOrderReduction {
    observation: PmOwnedOrderProgressObservation,
    apply: PmOwnedProgressApply,
}

impl PmOwnedOrderReduction {
    #[must_use]
    pub const fn observation(self) -> PmOwnedOrderProgressObservation {
        self.observation
    }

    #[must_use]
    pub const fn apply(self) -> PmOwnedProgressApply {
        self.apply
    }

    #[must_use]
    pub const fn occurrence(self) -> PmOwnedObservationOccurrence {
        self.observation.occurrence()
    }

    #[must_use]
    pub const fn source(self) -> PmOwnedObservationSource {
        self.observation.source()
    }
}

/// One canonical order reduction and its optional exact owned consequence.
///
/// The original envelope and ownership knowledge are retained by value so a
/// coordinator can journal the exact accepted input without consulting a
/// mutable projection after reduction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPrivateOrderReduction {
    envelope: EventEnvelope<PmOrderEvent>,
    knowledge: PmRemoteOrderKnowledge,
    canonical_apply: PmOrderApply,
    owned_remote_apply: PmOwnedRemoteOrderApply,
    owned: Option<PmOwnedOrderReduction>,
}

impl PmPrivateOrderReduction {
    #[must_use]
    pub const fn envelope(self) -> EventEnvelope<PmOrderEvent> {
        self.envelope
    }

    #[must_use]
    pub const fn knowledge(self) -> PmRemoteOrderKnowledge {
        self.knowledge
    }

    #[must_use]
    pub const fn canonical_apply(self) -> PmOrderApply {
        self.canonical_apply
    }

    #[must_use]
    pub const fn owned_remote_apply(self) -> PmOwnedRemoteOrderApply {
        self.owned_remote_apply
    }

    #[must_use]
    pub const fn owned(self) -> Option<PmOwnedOrderReduction> {
        self.owned
    }

    pub(super) const fn new(
        envelope: EventEnvelope<PmOrderEvent>,
        knowledge: PmRemoteOrderKnowledge,
        canonical_apply: PmOrderApply,
        owned_remote_apply: PmOwnedRemoteOrderApply,
        owned: Option<PmOwnedOrderReduction>,
    ) -> Self {
        Self {
            envelope,
            knowledge,
            canonical_apply,
            owned_remote_apply,
            owned,
        }
    }
}

/// Exact owned-lifecycle consequence of one private fill observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmOwnedFillReduction {
    observation: PmOwnedFillObservation,
    apply: PmOwnedFillApply,
}

impl PmOwnedFillReduction {
    #[must_use]
    pub const fn observation(self) -> PmOwnedFillObservation {
        self.observation
    }

    #[must_use]
    pub const fn apply(self) -> PmOwnedFillApply {
        self.apply
    }

    #[must_use]
    pub const fn occurrence(self) -> PmOwnedObservationOccurrence {
        self.observation.occurrence()
    }

    #[must_use]
    pub const fn source(self) -> PmOwnedObservationSource {
        self.observation.source()
    }
}

/// One canonical fill reduction and its optional exact owned consequence.
///
/// The accepted envelope is retained by value; no projection lookup or state
/// clone is required to build a durable mutation fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPrivateFillReduction {
    envelope: EventEnvelope<PmFillEvent>,
    canonical_apply: PmFillApply,
    owned_remote_apply: PmOwnedRemoteOrderApply,
    owned: Option<PmOwnedFillReduction>,
}

impl PmPrivateFillReduction {
    #[must_use]
    pub const fn envelope(self) -> EventEnvelope<PmFillEvent> {
        self.envelope
    }

    #[must_use]
    pub const fn canonical_apply(self) -> PmFillApply {
        self.canonical_apply
    }

    #[must_use]
    pub const fn owned_remote_apply(self) -> PmOwnedRemoteOrderApply {
        self.owned_remote_apply
    }

    #[must_use]
    pub const fn owned(self) -> Option<PmOwnedFillReduction> {
        self.owned
    }

    pub(super) const fn new(
        envelope: EventEnvelope<PmFillEvent>,
        canonical_apply: PmFillApply,
        owned_remote_apply: PmOwnedRemoteOrderApply,
        owned: Option<PmOwnedFillReduction>,
    ) -> Self {
        Self {
            envelope,
            canonical_apply,
            owned_remote_apply,
            owned,
        }
    }
}

/// Exact owned-lifecycle disposition of one row in a complete REST fill cut.
///
/// Only [`Self::OwnedApplied`] represents a new principal application that
/// requires one durable fill fact. Delivery duplicates, causally stale rows,
/// and rows that cannot be proven to belong to this strategy are retained as
/// distinct values so a caller cannot accidentally journal them as unique.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmReconciliationFillDisposition {
    OwnedApplied(PmOwnedFillReduction),
    OwnedDuplicate(PmOwnedFillReduction),
    OwnedStale(PmOwnedFillReduction),
    Unowned(PmOwnedRemoteOrderApply),
}

impl PmReconciliationFillDisposition {
    #[must_use]
    pub const fn unique_owned(self) -> Option<PmOwnedFillReduction> {
        match self {
            Self::OwnedApplied(reduction) => Some(reduction),
            Self::OwnedDuplicate(_) | Self::OwnedStale(_) | Self::Unowned(_) => None,
        }
    }
}

/// One exact REST fill row and its canonical owned-lifecycle consequence.
///
/// The envelope is rebuilt from the complete serviced query without changing
/// its source, connection, clock, ordering, or payload. This gives the
/// coordinator every fact needed for a durable record without a projection
/// lookup or a second fill reducer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmReconciliationFillReduction {
    envelope: EventEnvelope<PmFillEvent>,
    disposition: PmReconciliationFillDisposition,
}

impl PmReconciliationFillReduction {
    #[must_use]
    pub const fn envelope(self) -> EventEnvelope<PmFillEvent> {
        self.envelope
    }

    #[must_use]
    pub const fn disposition(self) -> PmReconciliationFillDisposition {
        self.disposition
    }

    pub(super) const fn new(
        envelope: EventEnvelope<PmFillEvent>,
        disposition: PmReconciliationFillDisposition,
    ) -> Self {
        Self {
            envelope,
            disposition,
        }
    }
}

/// Reusable fixed-bound output scratch for one complete reconciliation cut.
///
/// Construction reserves the protocol maximum once. Applying subsequent cuts
/// clears and reuses that allocation; no per-row vector or event clone is
/// created on the owner loop.
#[derive(Debug)]
pub struct PmReconciliationReductions {
    rows: Vec<PmReconciliationFillReduction>,
}

impl PmReconciliationReductions {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rows: Vec::with_capacity(reap_pm_core::MAX_PM_RECONCILIATION_FILLS),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<PmReconciliationFillReduction> {
        self.rows.get(index).copied()
    }

    pub fn iter(
        &self,
    ) -> impl DoubleEndedIterator<Item = PmReconciliationFillReduction> + ExactSizeIterator + '_
    {
        self.rows.iter().copied()
    }

    pub fn unique_owned(&self) -> impl DoubleEndedIterator<Item = PmOwnedFillReduction> + '_ {
        self.rows
            .iter()
            .filter_map(|row| row.disposition.unique_owned())
    }

    #[must_use]
    pub fn reserved_capacity_bytes(&self) -> usize {
        self.rows.capacity() * std::mem::size_of::<PmReconciliationFillReduction>()
    }

    pub(super) fn prepare(&mut self, row_count: usize) {
        debug_assert!(row_count <= reap_pm_core::MAX_PM_RECONCILIATION_FILLS);
        debug_assert!(self.rows.capacity() >= reap_pm_core::MAX_PM_RECONCILIATION_FILLS);
        self.rows.clear();
    }

    pub(super) fn push(&mut self, row: PmReconciliationFillReduction) {
        debug_assert!(self.rows.len() < reap_pm_core::MAX_PM_RECONCILIATION_FILLS);
        self.rows.push(row);
    }
}

impl Default for PmReconciliationReductions {
    fn default() -> Self {
        Self::new()
    }
}

/// Move-only authority for one immediate-acknowledgement reduction.
///
/// The occurrence is assigned by the sole private-state owner. Inspecting it
/// supports exact journaling, but only returning this ticket to the issuing
/// state can mutate the owned lifecycle.
#[derive(Debug)]
pub struct PmOwnedImmediateAckTicket {
    owner: PmRefreshOwnerId,
    occurrence: PmOwnedObservationOccurrence,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PmOwnedObservationRange {
    first_sequence: u64,
    count: usize,
    private_occurrence: PmPrivateOccurrence,
    snapshot_revision: Option<SnapshotRevision>,
}

impl PmOwnedObservationRange {
    pub(super) fn occurrence(self, index: usize) -> PmOwnedObservationOccurrence {
        assert!(index < self.count, "preflighted owned occurrence range");
        let offset = u64::try_from(index).expect("bounded PM fill count fits u64");
        let sequence = self
            .first_sequence
            .checked_add(offset)
            .expect("preflighted owned occurrence sequence");
        PmOwnedObservationOccurrence::new(
            PmOwnedReductionSequence::new(sequence)
                .expect("preflighted owned occurrence is nonzero"),
            Some(self.private_occurrence),
            self.snapshot_revision,
        )
        .expect("preflighted private occurrence evidence")
    }

    pub(super) fn final_sequence(self) -> u64 {
        let offset = u64::try_from(self.count - 1).expect("nonempty bounded range fits u64");
        self.first_sequence
            .checked_add(offset)
            .expect("preflighted owned occurrence range")
    }
}

impl PmOwnedImmediateAckTicket {
    #[must_use]
    pub const fn occurrence(&self) -> PmOwnedObservationOccurrence {
        self.occurrence
    }
}

impl PmPrivateState {
    pub fn admit_owned_quote(
        &mut self,
        intent: PmOwnedQuoteIntent,
    ) -> Result<PmOwnedQuoteAdmission, PmPrivateStateError> {
        let expected = self.owned_lifecycle.preflight_admit_quote(intent)?;
        let registration = if matches!(expected, PmOwnedQuoteAdmission::Admitted(_)) {
            let registration = PmOwnedOrderRegistration::new(
                intent.client_order(),
                intent.slot().instrument(),
                intent.slot().side(),
                intent.price(),
                intent.quantity(),
                intent.reservation(),
            )?;
            self.orders
                .preflight_register_owned(registration, &self.config)?;
            Some(registration)
        } else {
            None
        };
        let outcome = self.owned_lifecycle.admit_quote(intent)?;
        debug_assert_eq!(outcome, expected);
        if let Some(registration) = registration {
            self.orders.register_owned(registration, &self.config)?;
        }
        Ok(outcome)
    }

    pub fn apply_owned_submit_result(
        &mut self,
        client_order: PmClientOrderKey,
        result: PmOwnedSubmitResult,
    ) -> Result<PmOwnedSubmitApply, PmPrivateStateError> {
        let expected = self
            .owned_lifecycle
            .preflight_submit_result(client_order, result)?;
        if let PmOwnedSubmitResult::Accepted(venue_order) = result {
            self.orders
                .preflight_bind_owned_venue(client_order, venue_order, &self.config)?;
        }
        let outcome = self
            .owned_lifecycle
            .apply_submit_result(client_order, result)?;
        debug_assert_eq!(outcome, expected);
        if let PmOwnedSubmitResult::Accepted(venue_order) = result {
            self.orders
                .bind_owned_venue(client_order, venue_order, &self.config)?;
        }
        Ok(outcome)
    }

    pub fn issue_owned_immediate_ack_ticket(
        &mut self,
    ) -> Result<PmOwnedImmediateAckTicket, PmPrivateStateError> {
        if self.outstanding_owned_immediate.is_some() {
            return Err(PmPrivateStateError::OwnedImmediateAckPending);
        }
        let occurrence = self.mint_owned_occurrence(None, None)?;
        self.outstanding_owned_immediate = Some(occurrence.reduction_sequence());
        Ok(PmOwnedImmediateAckTicket {
            owner: self.owner,
            occurrence,
        })
    }

    pub fn observe_owned_immediate_fill(
        &mut self,
        ticket: PmOwnedImmediateAckTicket,
        event: PmFillEvent,
        reported_cumulative: Option<U256>,
    ) -> Result<PmOwnedFillApply, PmPrivateStateError> {
        self.validate_immediate_ticket(&ticket)?;
        if !matches!(
            self.owned_lifecycle.match_remote_order(event.order()),
            PmOwnedRemoteOrderApply::Matched(_)
        ) {
            return Err(PmOwnedOrderLifecycleError::UnboundVenueOrder.into());
        }
        preflight_owned_fill_event(
            &self.owned_lifecycle,
            event,
            ticket.occurrence,
            PmOwnedObservationSource::ImmediateAcknowledgement,
            reported_cumulative,
        )?;
        self.outstanding_owned_immediate = None;
        let canonical =
            self.fills
                .observe_owned_immediate(event, ticket.occurrence, &self.config)?;
        let owned = bridge_owned_fill_event(
            &mut self.owned_lifecycle,
            event,
            ticket.occurrence,
            PmOwnedObservationSource::ImmediateAcknowledgement,
            reported_cumulative,
        )?
        .ok_or(PmOwnedOrderLifecycleError::UnboundVenueOrder)?;
        if matches!(
            canonical,
            crate::fill_state::PmFillApply::PrincipalApplied { .. }
        ) {
            self.convergence = crate::private_readiness::PmPrivateConvergence::Divergent {
                uncovered_fills: self.uncovered_fill_count(),
            };
            self.require_refresh(crate::refresh::PmRefreshReason::FillObserved)?;
        }
        match canonical {
            crate::fill_state::PmFillApply::PrincipalApplied { fee, settlement }
            | crate::fill_state::PmFillApply::Enriched { fee, settlement } => {
                self.require_for_fee(fee)?;
                self.require_for_settlement(settlement)?;
            }
            crate::fill_state::PmFillApply::Duplicate
            | crate::fill_state::PmFillApply::IgnoredStale => {}
        }
        Ok(owned)
    }

    pub fn observe_owned_immediate_progress(
        &mut self,
        ticket: PmOwnedImmediateAckTicket,
        client_order: PmClientOrderKey,
        venue_order: reap_pm_core::PmVenueOrderKey,
        progress: PmOrderProgress,
    ) -> Result<PmOwnedProgressApply, PmPrivateStateError> {
        self.consume_immediate_ticket(&ticket)?;
        Ok(self
            .owned_lifecycle
            .observe_progress(PmOwnedOrderProgressObservation::new(
                client_order,
                venue_order,
                progress,
                ticket.occurrence,
                PmOwnedObservationSource::ImmediateAcknowledgement,
            ))?)
    }

    /// Deterministic journal-recovery seam. Runtime WS, REST, and immediate
    /// observations must use their aggregate-owned paths instead.
    pub fn observe_owned_fill(
        &mut self,
        observation: PmOwnedFillObservation,
    ) -> Result<PmOwnedFillApply, PmPrivateStateError> {
        self.preflight_recovery_occurrence(observation.occurrence())?;
        self.owned_lifecycle.preflight_fill(observation)?;
        let outcome = self.owned_lifecycle.observe_fill(observation)?;
        self.owned_reduction_high_watermark = observation.occurrence().reduction_sequence().value();
        Ok(outcome)
    }

    /// Deterministic journal-recovery seam. Runtime WS, REST, and immediate
    /// observations must use their aggregate-owned paths instead.
    pub fn observe_owned_progress(
        &mut self,
        observation: PmOwnedOrderProgressObservation,
    ) -> Result<PmOwnedProgressApply, PmPrivateStateError> {
        self.preflight_recovery_occurrence(observation.occurrence())?;
        self.owned_lifecycle.preflight_progress(observation)?;
        let outcome = self.owned_lifecycle.observe_progress(observation)?;
        self.owned_reduction_high_watermark = observation.occurrence().reduction_sequence().value();
        Ok(outcome)
    }

    pub fn recover_owned_fill(
        &mut self,
        recovery: PmOwnedRecoveryFill,
    ) -> Result<PmOwnedFillApply, PmPrivateStateError> {
        let event = recovery.event();
        let reported_cumulative = recovery.reported_cumulative();
        let occurrence = recovery.occurrence();
        let source = recovery.source();
        self.prepare_owned_recovery_epoch(occurrence)?;
        self.preflight_recovery_occurrence(occurrence)?;
        preflight_owned_fill_event(
            &self.owned_lifecycle,
            event,
            occurrence,
            source,
            reported_cumulative,
        )?;
        let canonical = self
            .fills
            .observe_owned_recovery(event, occurrence, &self.config)?;
        let owned = bridge_owned_fill_event(
            &mut self.owned_lifecycle,
            event,
            occurrence,
            source,
            reported_cumulative,
        )?
        .ok_or(PmOwnedOrderLifecycleError::UnboundVenueOrder)?;
        self.owned_reduction_high_watermark = occurrence.reduction_sequence().value();
        if matches!(
            canonical,
            crate::fill_state::PmFillApply::PrincipalApplied { .. }
        ) {
            self.convergence = crate::private_readiness::PmPrivateConvergence::Divergent {
                uncovered_fills: self.uncovered_fill_count(),
            };
            self.require_refresh(crate::refresh::PmRefreshReason::FillObserved)?;
        }
        match canonical {
            crate::fill_state::PmFillApply::PrincipalApplied { fee, settlement }
            | crate::fill_state::PmFillApply::Enriched { fee, settlement } => {
                self.require_for_fee(fee)?;
                self.require_for_settlement(settlement)?;
            }
            crate::fill_state::PmFillApply::Duplicate
            | crate::fill_state::PmFillApply::IgnoredStale => {}
        }
        Ok(owned)
    }

    /// Replays one exact durable order-progress observation.
    ///
    /// Like fill recovery, this restores only owner chronology. It never
    /// grants live private availability or freshness.
    pub fn recover_owned_progress(
        &mut self,
        observation: PmOwnedOrderProgressObservation,
    ) -> Result<PmOwnedProgressApply, PmPrivateStateError> {
        self.prepare_owned_recovery_epoch(observation.occurrence())?;
        self.preflight_recovery_occurrence(observation.occurrence())?;
        self.owned_lifecycle.preflight_progress(observation)?;
        let outcome = self.owned_lifecycle.observe_progress(observation)?;
        self.owned_reduction_high_watermark = observation.occurrence().reduction_sequence().value();
        Ok(outcome)
    }

    fn prepare_owned_recovery_epoch(
        &mut self,
        occurrence: PmOwnedObservationOccurrence,
    ) -> Result<(), PmPrivateStateError> {
        let Some(private) = occurrence.private_occurrence() else {
            return Ok(());
        };
        match self.current_epoch {
            Some(current) if private.epoch() < current => {
                return Err(PmPrivateStateError::OldConnectionEpoch);
            }
            Some(current) if private.epoch() == current => return Ok(()),
            Some(_) | None => {}
        }
        self.owned_lifecycle
            .restore_epoch_for_recovery(private.epoch())?;
        self.current_epoch = Some(private.epoch());
        // Journal chronology is not a live reconnect. Do not grant private
        // availability/freshness or increment reconnect counters here.
        Ok(())
    }

    /// Advances the owner-local sequence after replaying non-observation
    /// journal rows. It grants no private availability or quote readiness.
    pub fn finish_owned_recovery(
        &mut self,
        high_watermark: PmOwnedReductionSequence,
    ) -> Result<(), PmPrivateStateError> {
        if self.outstanding_owned_immediate.is_some() {
            return Err(PmPrivateStateError::OwnedImmediateAckPending);
        }
        if high_watermark.value() < self.owned_reduction_high_watermark {
            return Err(PmPrivateStateError::OwnedRecoverySequenceDidNotAdvance);
        }
        self.owned_reduction_high_watermark = high_watermark.value();
        Ok(())
    }

    pub fn observe_owned_remote_order(
        &mut self,
        identity: PmOrderIdentity,
    ) -> Result<PmOwnedRemoteOrderApply, PmPrivateStateError> {
        Ok(self.owned_lifecycle.observe_remote_order(identity)?)
    }

    pub fn request_owned_cancel(
        &mut self,
        client_order: PmClientOrderKey,
    ) -> Result<PmOwnedCancelRequestApply, PmPrivateStateError> {
        Ok(self.owned_lifecycle.request_cancel(client_order)?)
    }

    pub fn apply_owned_cancel_result(
        &mut self,
        intent: PmOwnedCancelIntent,
        outcome: PmOwnedCancelOutcome,
    ) -> Result<PmOwnedCancelApply, PmPrivateStateError> {
        Ok(self.owned_lifecycle.apply_cancel_result(intent, outcome)?)
    }

    pub fn compact_proven_owned_terminal(
        &mut self,
        client_order: PmClientOrderKey,
    ) -> Result<PmOwnedTerminalCompaction, PmPrivateStateError> {
        self.owned_lifecycle
            .preflight_compact_proven_terminal(client_order)?;
        self.orders.preflight_compact_proven_owned(client_order)?;
        let compacted = self.owned_lifecycle.compact_proven_terminal(client_order)?;
        self.orders.compact_proven_owned(client_order)?;
        Ok(compacted)
    }

    pub fn owned_orders(&self) -> impl Iterator<Item = PmOwnedOrderProjection> + '_ {
        self.owned_lifecycle.orders()
    }

    #[must_use]
    pub fn owned_order(&self, client_order: PmClientOrderKey) -> Option<PmOwnedOrderProjection> {
        self.owned_lifecycle.order(client_order)
    }

    pub fn owned_fills(&self) -> impl Iterator<Item = PmOwnedFillProjection> + '_ {
        self.owned_lifecycle.fills()
    }

    pub fn owned_quote_slots(&self) -> impl Iterator<Item = PmOwnedQuoteSlotProjection> + '_ {
        self.owned_lifecycle.slots()
    }

    #[must_use]
    pub const fn owned_lifecycle_counters(&self) -> PmOwnedLifecycleCounters {
        self.owned_lifecycle.counters()
    }

    pub(super) fn mint_owned_private_occurrence(
        &mut self,
        private_occurrence: PmPrivateOccurrence,
        snapshot_revision: Option<SnapshotRevision>,
    ) -> Result<PmOwnedObservationOccurrence, PmPrivateStateError> {
        self.mint_owned_occurrence(Some(private_occurrence), snapshot_revision)
    }

    pub(super) fn preflight_owned_private_range(
        &self,
        count: usize,
        private_occurrence: PmPrivateOccurrence,
        snapshot_revision: Option<SnapshotRevision>,
    ) -> Result<PmOwnedObservationRange, PmPrivateStateError> {
        if self.outstanding_owned_immediate.is_some() {
            return Err(PmPrivateStateError::OwnedImmediateAckPending);
        }
        let count = count.max(1);
        let count_u64 = u64::try_from(count)
            .map_err(|_| PmPrivateStateError::OwnedReductionSequenceExhausted)?;
        let first_sequence = self
            .owned_reduction_high_watermark
            .checked_add(1)
            .ok_or(PmPrivateStateError::OwnedReductionSequenceExhausted)?;
        self.owned_reduction_high_watermark
            .checked_add(count_u64)
            .ok_or(PmPrivateStateError::OwnedReductionSequenceExhausted)?;
        let range = PmOwnedObservationRange {
            first_sequence,
            count,
            private_occurrence,
            snapshot_revision,
        };
        let _first = range.occurrence(0);
        let _last = range.occurrence(count - 1);
        Ok(range)
    }

    pub(super) fn commit_owned_private_range(&mut self, range: PmOwnedObservationRange) {
        debug_assert_eq!(
            range.first_sequence,
            self.owned_reduction_high_watermark + 1
        );
        self.owned_reduction_high_watermark = range.final_sequence();
    }

    fn mint_owned_occurrence(
        &mut self,
        private_occurrence: Option<PmPrivateOccurrence>,
        snapshot_revision: Option<SnapshotRevision>,
    ) -> Result<PmOwnedObservationOccurrence, PmPrivateStateError> {
        if self.outstanding_owned_immediate.is_some() {
            return Err(PmPrivateStateError::OwnedImmediateAckPending);
        }
        let next = self
            .owned_reduction_high_watermark
            .checked_add(1)
            .ok_or(PmPrivateStateError::OwnedReductionSequenceExhausted)?;
        let sequence = PmOwnedReductionSequence::new(next)?;
        let occurrence =
            PmOwnedObservationOccurrence::new(sequence, private_occurrence, snapshot_revision)?;
        self.owned_reduction_high_watermark = next;
        Ok(occurrence)
    }

    fn consume_immediate_ticket(
        &mut self,
        ticket: &PmOwnedImmediateAckTicket,
    ) -> Result<(), PmPrivateStateError> {
        self.validate_immediate_ticket(ticket)?;
        self.outstanding_owned_immediate = None;
        Ok(())
    }

    fn validate_immediate_ticket(
        &self,
        ticket: &PmOwnedImmediateAckTicket,
    ) -> Result<(), PmPrivateStateError> {
        if ticket.owner != self.owner
            || self.outstanding_owned_immediate != Some(ticket.occurrence.reduction_sequence())
        {
            return Err(PmPrivateStateError::OwnedImmediateAckTicketMismatch);
        }
        Ok(())
    }

    fn preflight_recovery_occurrence(
        &self,
        occurrence: PmOwnedObservationOccurrence,
    ) -> Result<(), PmPrivateStateError> {
        if self.outstanding_owned_immediate.is_some() {
            return Err(PmPrivateStateError::OwnedImmediateAckPending);
        }
        let sequence = occurrence.reduction_sequence().value();
        if sequence <= self.owned_reduction_high_watermark {
            return Err(PmPrivateStateError::OwnedRecoverySequenceDidNotAdvance);
        }
        Ok(())
    }
}

pub(super) fn reduce_owned_order_event(
    lifecycle: &mut PmOwnedOrderLifecycle,
    event: PmOrderEvent,
    occurrence: PmOwnedObservationOccurrence,
    source: PmOwnedObservationSource,
) -> Result<(PmOwnedRemoteOrderApply, Option<PmOwnedOrderReduction>), PmOwnedOrderLifecycleError> {
    let matched = lifecycle.observe_remote_order(event.order())?;
    let PmOwnedRemoteOrderApply::Matched(client_order) = matched else {
        return Ok((matched, None));
    };
    let Some(owned) = lifecycle.order(client_order) else {
        return Ok((PmOwnedRemoteOrderApply::AmbiguousRemote, None));
    };
    let Some(venue_order) = event.order().venue_order_key().or(owned.venue_order()) else {
        return Ok((PmOwnedRemoteOrderApply::AmbiguousRemote, None));
    };
    if owned.venue_order() != Some(venue_order)
        || owned.slot().side() != event.side()
        || owned.price() != event.price()
        || owned.quantity() != event.progress().original_quantity()
    {
        return Err(PmOwnedOrderLifecycleError::VenueBindingConflict);
    }
    let observation = PmOwnedOrderProgressObservation::new(
        client_order,
        venue_order,
        event.progress(),
        occurrence,
        source,
    );
    let apply = lifecycle.observe_progress(observation)?;
    Ok((matched, Some(PmOwnedOrderReduction { observation, apply })))
}

pub(super) fn bridge_owned_order_event(
    lifecycle: &mut PmOwnedOrderLifecycle,
    event: PmOrderEvent,
    occurrence: PmOwnedObservationOccurrence,
    source: PmOwnedObservationSource,
) -> Result<PmOwnedRemoteOrderApply, PmOwnedOrderLifecycleError> {
    Ok(reduce_owned_order_event(lifecycle, event, occurrence, source)?.0)
}

pub(super) fn preflight_owned_order_event(
    lifecycle: &PmOwnedOrderLifecycle,
    event: PmOrderEvent,
    occurrence: PmOwnedObservationOccurrence,
    source: PmOwnedObservationSource,
) -> Result<PmOwnedRemoteOrderApply, PmOwnedOrderLifecycleError> {
    let matched = lifecycle.match_remote_order(event.order());
    let PmOwnedRemoteOrderApply::Matched(client_order) = matched else {
        return Ok(matched);
    };
    let Some(owned) = lifecycle.order(client_order) else {
        return Ok(PmOwnedRemoteOrderApply::AmbiguousRemote);
    };
    let Some(venue_order) = event.order().venue_order_key().or(owned.venue_order()) else {
        return Ok(PmOwnedRemoteOrderApply::AmbiguousRemote);
    };
    if owned.venue_order() != Some(venue_order)
        || owned.slot().side() != event.side()
        || owned.price() != event.price()
        || owned.quantity() != event.progress().original_quantity()
    {
        return Err(PmOwnedOrderLifecycleError::VenueBindingConflict);
    }
    lifecycle.preflight_progress(PmOwnedOrderProgressObservation::new(
        client_order,
        venue_order,
        event.progress(),
        occurrence,
        source,
    ))?;
    Ok(matched)
}

pub(super) fn reduce_owned_fill_event(
    lifecycle: &mut PmOwnedOrderLifecycle,
    event: PmFillEvent,
    occurrence: PmOwnedObservationOccurrence,
    source: PmOwnedObservationSource,
    reported_cumulative: Option<U256>,
) -> Result<(PmOwnedRemoteOrderApply, Option<PmOwnedFillReduction>), PmOwnedOrderLifecycleError> {
    let matched = lifecycle.observe_remote_order(event.order())?;
    let PmOwnedRemoteOrderApply::Matched(client_order) = matched else {
        return Ok((matched, None));
    };
    let Some(owned) = lifecycle.order(client_order) else {
        return Ok((PmOwnedRemoteOrderApply::AmbiguousRemote, None));
    };
    let execution = event.execution();
    if owned.venue_order() != Some(event.fill_key().venue_order())
        || owned.slot().side() != execution.side()
        || !fill_is_inside_limit(owned.slot().side(), owned.price(), execution.price())
    {
        return Err(PmOwnedOrderLifecycleError::FillConflict);
    }
    let observation = PmOwnedFillObservation::new(
        event.fill_key(),
        execution.quantity(),
        reported_cumulative,
        occurrence,
        source,
    )?;
    let apply = lifecycle.observe_fill(observation)?;
    Ok((matched, Some(PmOwnedFillReduction { observation, apply })))
}

pub(super) fn bridge_owned_fill_event(
    lifecycle: &mut PmOwnedOrderLifecycle,
    event: PmFillEvent,
    occurrence: PmOwnedObservationOccurrence,
    source: PmOwnedObservationSource,
    reported_cumulative: Option<U256>,
) -> Result<Option<PmOwnedFillApply>, PmOwnedOrderLifecycleError> {
    Ok(
        reduce_owned_fill_event(lifecycle, event, occurrence, source, reported_cumulative)?
            .1
            .map(PmOwnedFillReduction::apply),
    )
}

pub(super) fn preflight_owned_fill_event(
    lifecycle: &PmOwnedOrderLifecycle,
    event: PmFillEvent,
    occurrence: PmOwnedObservationOccurrence,
    source: PmOwnedObservationSource,
    reported_cumulative: Option<U256>,
) -> Result<(), PmOwnedOrderLifecycleError> {
    let PmOwnedRemoteOrderApply::Matched(client_order) =
        lifecycle.match_remote_order(event.order())
    else {
        return Ok(());
    };
    let Some(owned) = lifecycle.order(client_order) else {
        return Ok(());
    };
    let execution = event.execution();
    if owned.venue_order() != Some(event.fill_key().venue_order())
        || owned.slot().side() != execution.side()
        || !fill_is_inside_limit(owned.slot().side(), owned.price(), execution.price())
    {
        return Err(PmOwnedOrderLifecycleError::FillConflict);
    }
    lifecycle.preflight_fill(PmOwnedFillObservation::new(
        event.fill_key(),
        execution.quantity(),
        reported_cumulative,
        occurrence,
        source,
    )?)
}

pub(super) fn preflight_owned_fill_batch(
    lifecycle: &PmOwnedOrderLifecycle,
    events: &[PmFillEvent],
    occurrences: PmOwnedObservationRange,
    source: PmOwnedObservationSource,
) -> Result<(), PmOwnedOrderLifecycleError> {
    let mut new_owned_fills = 0_usize;
    for (index, event) in events.iter().copied().enumerate() {
        let occurrence = occurrences.occurrence(index);
        preflight_owned_fill_event(lifecycle, event, occurrence, source, None)?;
        let PmOwnedRemoteOrderApply::Matched(client_order) =
            lifecycle.match_remote_order(event.order())
        else {
            continue;
        };
        if lifecycle.fills().any(|fill| fill.key() == event.fill_key()) {
            continue;
        }
        new_owned_fills = new_owned_fills.saturating_add(1);
        let owned = lifecycle
            .order(client_order)
            .ok_or(PmOwnedOrderLifecycleError::UnboundVenueOrder)?;
        let mut batch_total = U256::ZERO;
        for candidate in events.iter().copied() {
            if candidate.fill_key().venue_order() != event.fill_key().venue_order()
                || lifecycle
                    .fills()
                    .any(|fill| fill.key() == candidate.fill_key())
            {
                continue;
            }
            batch_total = batch_total
                .checked_add(candidate.execution().quantity().protocol_units())
                .map_err(|_| PmOwnedOrderLifecycleError::ArithmeticOverflow)?;
        }
        let total = owned
            .known_fill_total()
            .checked_add(batch_total)
            .map_err(|_| PmOwnedOrderLifecycleError::ArithmeticOverflow)?;
        if total > owned.quantity().protocol_units() {
            return Err(PmOwnedOrderLifecycleError::Overfill);
        }
    }
    if lifecycle.fills().count().saturating_add(new_owned_fills)
        > crate::owned_lifecycle::MAX_PM_OWNED_FILL_KEYS
    {
        return Err(PmOwnedOrderLifecycleError::FillCapacity);
    }
    Ok(())
}

fn fill_is_inside_limit(
    side: reap_pm_core::PmOrderSide,
    limit: reap_pm_core::PmPrice,
    execution: reap_pm_core::PmPrice,
) -> bool {
    match side {
        reap_pm_core::PmOrderSide::Buy => execution <= limit,
        reap_pm_core::PmOrderSide::Sell => execution >= limit,
    }
}
