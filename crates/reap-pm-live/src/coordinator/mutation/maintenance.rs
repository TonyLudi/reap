use reap_pm_core::PmOrderSide;
use reap_pm_state::{
    PmFillCompaction, PmOwnedCancelState, PmOwnedOrderProjection, PmOwnedSubmitState,
    PmPreparedFillCompaction, PmRefreshAdmission, PmRefreshCompletion, PmRefreshCounters,
    PmRefreshReason, PmRefreshRequired, PmRefreshTicket,
};

use super::{
    PmFakeEffectPermit, PmJournalImmediateFillsV1, PmJournalPlaceOutcomeV1,
    PmJournalPlaceRejectReasonV1, PmJournalPlaceResultV1, PmJournalRecordV1, PmMutationHalt,
    PmPersistenceIntentIdentity, PmPersistenceService,
};
use super::{PmMutationError, PmMutationOwner};
use reap_pm_core::PmClientOrderKey;
use reap_pm_state::{PmOwnedSubmitApply, PmOwnedSubmitResult};

impl PmMutationOwner {
    pub(super) fn invalidate_durable_quote(
        &mut self,
        identity: PmPersistenceIntentIdentity,
        effect_permit: PmFakeEffectPermit,
        monotonic_service_ns: u64,
    ) -> Result<PmPersistenceService, PmMutationError> {
        let PmPersistenceIntentIdentity::Quote { client_order, .. } = identity else {
            self.halt = Some(PmMutationHalt::InternalInvariant);
            return Err(PmMutationError::InvalidPersistenceIdentity);
        };
        self.ensure_fact_capacity(1)?;
        self.reject_durable_never_dispatched_quote(client_order, monotonic_service_ns)?;
        self.effects
            .invalidate_after_durability(effect_permit)
            .map_err(|error| {
                self.halt = Some(PmMutationHalt::InternalInvariant);
                PmMutationError::EffectQueue(error)
            })?;
        Ok(PmPersistenceService::QuoteInvalidated { identity })
    }

    pub(super) fn reject_durable_never_dispatched_quote(
        &mut self,
        client_order: PmClientOrderKey,
        monotonic_service_ns: u64,
    ) -> Result<(), PmMutationError> {
        let apply = self
            .private
            .apply_owned_submit_result(client_order, PmOwnedSubmitResult::Rejected)?;
        if apply != PmOwnedSubmitApply::Rejected {
            self.halt = Some(PmMutationHalt::InternalInvariant);
            return Err(PmMutationError::InvalidLocalInvalidation);
        }
        self.record_fact(
            PmJournalRecordV1::PlaceResult(PmJournalPlaceResultV1 {
                client_order,
                outcome: PmJournalPlaceOutcomeV1::Rejected,
                reject_reason: Some(
                    PmJournalPlaceRejectReasonV1::AuthorityInvalidatedBeforeDispatch,
                ),
                venue_order: None,
                immediate_fills: PmJournalImmediateFillsV1::empty(),
            }),
            monotonic_service_ns,
        )
    }

    pub(super) fn reject_never_dispatched_quote(&mut self, client_order: PmClientOrderKey) {
        if self
            .private
            .apply_owned_submit_result(client_order, PmOwnedSubmitResult::Rejected)
            .is_ok()
        {
            // This path failed before any durable identity existed, so local
            // cleanup cannot diverge from journal replay.
            let _ = self.private.compact_proven_owned_terminal(client_order);
        }
    }

    pub(crate) fn ensure_fill_watermark_compaction_available(&self) -> Result<(), PmMutationError> {
        if self.private.fill_watermark_compaction_pending() {
            Err(PmMutationError::State(
                reap_pm_state::PmPrivateStateError::FillCompactionPending,
            ))
        } else {
            Ok(())
        }
    }

    pub(crate) fn owned_cancel_candidate(
        &self,
        side: PmOrderSide,
    ) -> Option<PmOwnedOrderProjection> {
        self.private.owned_orders().find(|order| {
            order.slot().side() == side
                && order.venue_order().is_some()
                && order.submit() == PmOwnedSubmitState::Accepted
                && !order.is_terminal()
                && matches!(
                    order.cancel(),
                    PmOwnedCancelState::None | PmOwnedCancelState::Rejected
                )
        })
    }

    pub(crate) fn next_pending_refresh(&self) -> Option<PmRefreshTicket> {
        self.private.next_pending_refresh()
    }

    pub(crate) fn pending_refresh(&self, reason: PmRefreshReason) -> Option<PmRefreshTicket> {
        self.private.pending_refresh(reason)
    }

    pub(crate) fn pending_refresh_count_for(&self, reason: PmRefreshReason) -> usize {
        self.private.pending_refresh_count_for(reason)
    }

    pub(crate) const fn refresh_obligation_count(&self) -> usize {
        self.private.refresh_obligation_count()
    }

    pub(crate) const fn refresh_counters(&self) -> PmRefreshCounters {
        self.private.refresh_counters()
    }

    pub(crate) fn require_refresh(
        &mut self,
        reason: PmRefreshReason,
    ) -> Result<PmRefreshRequired, PmMutationError> {
        Ok(self.private.require_refresh(reason)?)
    }

    pub(crate) fn mark_refresh_admitted(
        &mut self,
        ticket: PmRefreshTicket,
    ) -> Result<PmRefreshAdmission, PmMutationError> {
        Ok(self.private.mark_refresh_admitted(ticket)?)
    }

    pub(crate) fn complete_refresh(
        &mut self,
        ticket: PmRefreshTicket,
    ) -> Result<PmRefreshCompletion, PmMutationError> {
        Ok(self.private.complete_refresh(ticket)?)
    }

    pub(crate) fn prepare_fill_watermark_compaction(
        &mut self,
    ) -> Result<PmPreparedFillCompaction, PmMutationError> {
        Ok(self.private.prepare_fill_watermark_compaction()?)
    }

    pub(super) fn count_compaction(&mut self, compacted: PmFillCompaction) {
        self.counters.fill_watermark_compactions =
            self.counters.fill_watermark_compactions.saturating_add(1);
        self.counters.owned_lifecycle_rows_compacted =
            self.counters.owned_lifecycle_rows_compacted.saturating_add(
                u64::try_from(compacted.owned_lifecycle_rows())
                    .expect("bounded PM owned rows fit u64"),
            );
        self.counters.canonical_order_rows_compacted =
            self.counters.canonical_order_rows_compacted.saturating_add(
                u64::try_from(compacted.canonical_order_rows())
                    .expect("bounded PM canonical orders fit u64"),
            );
        self.counters.owned_fill_keys_compacted =
            self.counters.owned_fill_keys_compacted.saturating_add(
                u64::try_from(compacted.owned_fill_keys()).expect("bounded PM fill keys fit u64"),
            );
        self.counters.canonical_fill_rows_compacted =
            self.counters.canonical_fill_rows_compacted.saturating_add(
                u64::try_from(compacted.canonical_fill_rows())
                    .expect("bounded PM canonical fills fit u64"),
            );
    }
}
