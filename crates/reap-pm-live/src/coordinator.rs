//! Single-owner PM product coordination.

#[cfg(any(test, feature = "loopback-evidence"))]
mod authenticated_execution;
mod authenticated_recovery;
mod authenticated_reduction;
pub mod authority;
mod dispatch;
mod effect_queue;
mod effects;
mod input;
mod live_completion;
mod mutation;
mod mutation_recovery;
mod persistence;
mod private_reduction;
mod product;
mod reduction;

#[cfg(any(test, feature = "loopback-evidence"))]
pub(crate) use authenticated_execution::{
    PmAuthenticatedCancelTaskFinish, PmAuthenticatedCancelTaskOutcome,
    PmAuthenticatedExecutionError, PmAuthenticatedPlaceTaskFinish, PmAuthenticatedPlaceTaskOutcome,
    PmAuthenticatedTaskPreparationError, PmLoopbackAuthenticatedExecutionShutdown,
    PmLoopbackAuthenticatedExecutionStage, PmLoopbackAuthenticatedMutationWorkers,
};
#[cfg(any(test, feature = "loopback-evidence"))]
pub(crate) use live_completion::{PmLiveCancelCompletion, PmLivePlaceCompletion};
pub(crate) use mutation::PmPendingFakeCancelResult;
#[cfg(any(test, feature = "loopback-evidence"))]
pub(crate) use mutation::{PmAuthenticatedBridgeFailure, PmGoalFWriterFailure, PmMutationError};
#[cfg(test)]
pub(crate) use persistence::Phase6StorageAllocationProbe;
pub(crate) use persistence::PmPersistencePoll;
#[cfg(any(test, feature = "loopback-evidence"))]
pub(crate) use product::PmCoordinatorAssemblyError;
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
pub use dispatch::{PmPreparedCancelDispatch, PmPreparedPlaceDispatch};
#[cfg(test)]
pub(crate) use effect_queue::Phase6FakeEffectAllocationProbe;
#[cfg(any(test, feature = "loopback-evidence"))]
pub(crate) use effect_queue::PmPreparedMutationKind;
pub use effect_queue::{PmEffectDispatchMetrics, PmFakeEffectMetrics};
pub use effects::{
    MAX_PM_EFFECTS_PER_INPUT, PmCancelDispatchEffect, PmCancelIntentReason, PmDurableRecordEffect,
    PmDurableRecordKind, PmEffectDispatchStage, PmFailClosedEffect, PmFakeCancelEffect,
    PmFakeEffectStage, PmFakeQuoteEffect, PmHealthMetricEffect, PmHealthMetricKind,
    PmPlaceDispatchEffect, PmProductEffect, PmProductEffectBatch, PmProductEffectMetrics,
    PmRefreshEffect, PmRefreshEffectKind,
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
