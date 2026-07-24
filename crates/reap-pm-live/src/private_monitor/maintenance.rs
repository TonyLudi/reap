use reap_pm_state::{
    PmFillCompaction, PmOwnedOrderProjection, PmPreparedFillCompaction, PmPrivateStateError,
    PmRefreshAdmission, PmRefreshCompletion, PmRefreshCounters, PmRefreshReason, PmRefreshRequired,
    PmRefreshTicket,
};

use super::PmPrivateMonitorRuntime;

impl PmPrivateMonitorRuntime {
    pub(crate) const fn fill_watermark_compaction_pending(&self) -> bool {
        self.state.fill_watermark_compaction_pending()
    }

    pub(crate) fn prepare_fill_watermark_compaction(
        &mut self,
    ) -> Result<PmPreparedFillCompaction, PmPrivateStateError> {
        self.state.prepare_fill_watermark_compaction()
    }

    pub(crate) fn commit_fill_watermark_compaction(
        &mut self,
        ticket: PmPreparedFillCompaction,
    ) -> Result<PmFillCompaction, PmPrivateStateError> {
        self.state.commit_fill_watermark_compaction(ticket)
    }

    pub(crate) fn abort_fill_watermark_compaction(
        &mut self,
        ticket: PmPreparedFillCompaction,
    ) -> Result<(), PmPrivateStateError> {
        self.state.abort_fill_watermark_compaction(ticket)
    }

    pub(crate) fn pending_refresh(&self, reason: PmRefreshReason) -> Option<PmRefreshTicket> {
        self.state
            .pending_refreshes()
            .find(|ticket| ticket.key().reason() == reason)
    }

    pub(crate) fn next_pending_refresh(&self) -> Option<PmRefreshTicket> {
        self.state.pending_refreshes().next()
    }

    pub(crate) fn pending_refresh_count_for(&self, reason: PmRefreshReason) -> usize {
        self.state
            .pending_refresh_keys()
            .filter(|key| key.reason() == reason)
            .count()
    }

    pub(crate) const fn refresh_obligation_count(&self) -> usize {
        self.state.pending_refresh_count()
    }

    pub(crate) const fn refresh_counters(&self) -> PmRefreshCounters {
        self.state.refresh_counters()
    }

    pub(crate) fn require_refresh(
        &mut self,
        reason: PmRefreshReason,
    ) -> Result<PmRefreshRequired, PmPrivateStateError> {
        self.state.require_refresh(reason)
    }

    pub(crate) fn mark_refresh_admitted(
        &mut self,
        ticket: PmRefreshTicket,
    ) -> Result<PmRefreshAdmission, PmPrivateStateError> {
        self.state.mark_refresh_admitted(ticket)
    }

    pub(crate) fn complete_refresh(
        &mut self,
        ticket: PmRefreshTicket,
    ) -> Result<PmRefreshCompletion, PmPrivateStateError> {
        self.state.complete_refresh(ticket)
    }

    pub(crate) fn owned_orders(&self) -> impl Iterator<Item = PmOwnedOrderProjection> + '_ {
        self.state.owned_orders()
    }
}
