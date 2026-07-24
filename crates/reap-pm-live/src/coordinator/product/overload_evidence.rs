use reap_pm_strategy::PmQuoteModel;

use super::*;

/// Narrow copied state used only to prove telemetry coalescing is
/// observational at the integrated ProductRun boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PmTelemetryOverloadState {
    authority_revisions: Option<PmAuthorityRevisions>,
    private_readiness_revision: u64,
    reconciliation_gate: bool,
    reconciliation_recovered: bool,
}

impl PmTelemetryOverloadState {
    pub(crate) const fn mutation_revision_authority_present(self) -> bool {
        self.authority_revisions.is_some()
    }

    pub(crate) const fn reconciliation_gate(self) -> bool {
        self.reconciliation_gate
    }

    pub(crate) const fn reconciliation_recovered(self) -> bool {
        self.reconciliation_recovered
    }
}

impl<M: PmQuoteModel> PmCoordinator<M> {
    pub(crate) const fn telemetry_overload_state(&self) -> PmTelemetryOverloadState {
        PmTelemetryOverloadState {
            authority_revisions: self.mutation.current_revisions_for_overload_evidence(),
            private_readiness_revision: self.private_readiness_revision,
            reconciliation_gate: self.reconciliation_gate,
            reconciliation_recovered: self.reconciliation_recovered,
        }
    }
}
