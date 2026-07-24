//! Single-owner PM product coordination.

pub mod authority;
mod effect_queue;
mod effects;
mod input;
mod mutation;
mod mutation_recovery;
mod persistence;
mod private_reduction;
mod product;
mod reduction;

pub(crate) use mutation::PmPendingFakeCancelResult;
#[cfg(test)]
pub(crate) use persistence::Phase6StorageAllocationProbe;
pub(crate) use persistence::PmPersistencePoll;
pub(crate) use product::{
    MAX_COPIED_EFFECT_CORRELATIONS, PmCoordinator, PmCoordinatorError, PmCoordinatorShutdownError,
    PmCoordinatorStartError, PmEvidenceTerminalLengths,
};
#[cfg(test)]
pub(crate) use product::{Phase6RefreshAllocationProbe, PmTelemetryOverloadState};

pub use authority::{
    ApprovedPmCancel, ApprovedPmQuote, PmAuthorityError, PmAuthorityRevisions, PreparedPmCancel,
    PreparedPmQuote, ReservedPmCancel, ReservedPmQuote,
};
#[cfg(test)]
pub(crate) use effect_queue::Phase6FakeEffectAllocationProbe;
pub use effect_queue::PmFakeEffectMetrics;
pub use effects::{
    MAX_PM_EFFECTS_PER_INPUT, PmCancelIntentReason, PmDurableRecordEffect, PmDurableRecordKind,
    PmFailClosedEffect, PmFakeCancelEffect, PmFakeEffectStage, PmFakeQuoteEffect,
    PmHealthMetricEffect, PmHealthMetricKind, PmProductEffect, PmProductEffectBatch,
    PmProductEffectMetrics, PmRefreshEffect, PmRefreshEffectKind,
};
pub use input::{
    PmBookDecisionProjection, PmBookInput, PmControlReason, PmMarketInput, PmOkxReferenceInput,
    PmProductInputError, PmTimerInput,
};
pub use mutation::{PmMutationCounters, PmMutationHalt};
pub use persistence::PmPersistenceMetrics;
pub use product::{
    PmCoordinatorCounters, PmCoordinatorPolicy, PmCoordinatorPolicyError, PmQuoteSuppression,
    PmRefreshObligationMetrics,
};
