//! Immutable least-authority projections over monitor-owned private state.

use reap_pm_core::{PmSignedUnits, PmSpenderId};
use reap_pm_state::{
    PmAccountCounters, PmAccountSnapshotProjection, PmAllowanceKnowledge, PmFillCounters,
    PmFillProjection, PmOrderCounters, PmOrderProjection, PmPrivateConvergence,
    PmPrivateExternalIngressCounters, PmPrivateHaltReason, PmPrivateQuoteRequest,
    PmPrivateReadiness, PmPrivateState, PmProvisionalDeltas, PmRefreshCounters, PmRefreshKey,
    PmRiskCounters, PmUnresolvedFillCounters, PmUnresolvedFillProjection,
};

/// Immutable view over the monitor-owned canonical private state.
///
/// There is intentionally no `Deref`, `AsRef`, mutable accessor, or way to
/// recover the underlying state owner.
pub struct PmReadOnlyPrivateProjection<'a> {
    state: &'a PmPrivateState,
}

impl<'a> PmReadOnlyPrivateProjection<'a> {
    pub(super) const fn new(state: &'a PmPrivateState) -> Self {
        Self { state }
    }

    #[must_use]
    pub const fn account_snapshot(&self) -> PmAccountSnapshotProjection {
        self.state.account_projection()
    }

    pub fn allowance(&self, spender: PmSpenderId) -> PmAllowanceKnowledge {
        self.state.allowance(spender)
    }

    pub fn orders(&self) -> impl Iterator<Item = PmOrderProjection> + '_ {
        self.state.orders()
    }

    pub fn fills(&self) -> impl Iterator<Item = PmFillProjection> + '_ {
        self.state.fills()
    }

    pub fn unresolved_fills(&self) -> impl Iterator<Item = PmUnresolvedFillProjection> + '_ {
        self.state.unresolved_fills()
    }

    #[must_use]
    pub const fn provisional_deltas(&self) -> PmProvisionalDeltas {
        self.state.provisional_deltas()
    }

    /// Last authoritative outcome position plus unique, locally observed
    /// fills not yet covered by reconciliation.
    ///
    /// `None` means the authoritative account snapshot is not complete or no
    /// longer supports a safe effective-position projection.
    #[must_use]
    pub fn effective_position(&self) -> Option<PmSignedUnits> {
        self.state.effective_position()
    }

    #[must_use]
    pub const fn convergence(&self) -> PmPrivateConvergence {
        self.state.convergence()
    }

    #[must_use]
    pub const fn halt(&self) -> Option<PmPrivateHaltReason> {
        self.state.halt()
    }

    #[must_use]
    pub const fn pending_refresh_count(&self) -> usize {
        self.state.pending_refresh_count()
    }

    pub fn pending_refresh_keys(&self) -> impl Iterator<Item = PmRefreshKey> + '_ {
        self.state.pending_refresh_keys()
    }

    #[must_use]
    pub const fn full_reconcile_required(&self) -> bool {
        self.state.full_reconcile_required()
    }

    #[must_use]
    pub const fn account_counters(&self) -> PmAccountCounters {
        self.state.account_counters()
    }

    #[must_use]
    pub const fn order_counters(&self) -> PmOrderCounters {
        self.state.order_counters()
    }

    #[must_use]
    pub const fn fill_counters(&self) -> PmFillCounters {
        self.state.fill_counters()
    }

    #[must_use]
    pub const fn unresolved_fill_counters(&self) -> PmUnresolvedFillCounters {
        self.state.unresolved_fill_counters()
    }

    #[must_use]
    pub const fn refresh_counters(&self) -> PmRefreshCounters {
        self.state.refresh_counters()
    }

    #[must_use]
    pub const fn risk_counters(&self) -> PmRiskCounters {
        self.state.risk_counters()
    }

    #[must_use]
    pub const fn external_ingress_counters(&self) -> PmPrivateExternalIngressCounters {
        self.state.external_ingress_counters()
    }

    #[must_use]
    pub fn quote_readiness(&self, request: PmPrivateQuoteRequest) -> PmPrivateReadiness {
        self.state.quote_readiness(request)
    }
}
