#![forbid(unsafe_code)]

mod account_fixture;
mod fake_execution;
mod fill_cut;
mod fixture_delivery;
mod fixture_scope;
mod live_account;
mod live_diagnostics;
mod live_normalization;
mod live_private;
mod live_reconciliation;
mod private_fixture;
mod public;
mod public_metadata;
mod public_session;
mod reconcile_fixture;
mod roles;

pub use reap_polymarket_wire::MAX_PUBLIC_WS_FRAME_BYTES as MAX_PM_PUBLIC_RAW_FRAME_BYTES;

pub use account_fixture::{
    PmAccountPositionRoleError, PmAccountPositionSnapshotRole, PmCompleteAccountSnapshotDelivery,
    PmFixtureAccountPositionSnapshot, PmFixtureAccountSnapshotAssembly,
    PmFixtureAccountSnapshotRequest, PmFixtureAllowanceRow, PmFixtureBalanceRow,
    PmFixturePositionRow,
};
pub use fake_execution::{
    MAX_PM_FAKE_ACK_FILL_LEGS, PmCancelOwnedPurpose, PmExactOwnedCancelRequest,
    PmFakeAckImmediateFillLeg, PmFakeCancelCommand, PmFakeCancelOutcome, PmFakeCancelRejectReason,
    PmFakeCancelResult, PmFakeCancelScript, PmFakeExecutionError, PmFakeImmediateFill,
    PmFakeOrderType, PmFakePlaceAck, PmFakePlaceCommand, PmFakePlaceOutcome,
    PmFakePlaceRejectReason, PmFakePlaceResult, PmFakePlaceScript, PmFixedMutationPreparation,
    PmFixedOrderType, PmFixtureOwnedExecution, PmGtcPostOnlyPlaceRequest, PmGtcPostOnlyProfile,
    PmOwnedExecutionRole,
};
pub use fill_cut::{
    MAX_PM_FULL_ACCOUNT_FILL_LEGS, PmAccountFillLeg, PmAccountFillLegKey,
    PmCanonicalFullAccountFillSnapshot, PmCompleteFillCutEvidence, PmFillCutError,
    PmFullAccountFillCutScope, PmFullAccountFillSnapshotDigest,
};
pub use fixture_delivery::{
    PmFixtureAggregateDelivery, PmFixtureCompletionOccurrence, PmFixtureDeliveryError,
    PmFixtureDeliveryScope, PmFixtureServicedAggregate,
};
pub use fixture_scope::{
    PmFixtureAccountRoleGrant, PmFixtureInstrumentScope, PmFixturePrivateRoleGrant,
    PmFixtureReadOwnerGrant, PmFixtureReconciliationRoleGrant, PmFixtureScopeError,
};
pub use live_account::PmLiveAccountSnapshotCompletion;
pub use live_diagnostics::{
    MAX_PM_LIVE_FOREIGN_DIAGNOSTIC_ROWS, PmForeignDiagnosticError, PmForeignRowDiagnostics,
};
pub use live_normalization::PmLiveNormalizationError;
pub use live_private::PmLivePrivateCompletion;
pub use live_reconciliation::{PmLiveFillQueryCompletion, PmLiveOpenOrdersCompletion};
pub use private_fixture::{
    MAX_PM_PRIVATE_NORMALIZED_OBSERVATIONS, PmFixtureFeeEvidence, PmFixturePrivateBatch,
    PmFixturePrivateDelivery, PmFixturePrivateLifecycle, PmFixtureUnresolvedTrade,
    PmPrivateLifecycleObservation, PmPrivateLifecycleRole, PmPrivateLifecycleRoleError,
    PmPrivateNormalizationError, PmUnresolvedTradeReason,
};
pub use public::{PmPublicObservationRole, PmPublicRole, PmPublicRoleError};
pub use public_metadata::{
    PmAuthoritativeMetadata, PmMetadataJoinError, PmMetadataRevisionInput,
    PmRecordedMetadataEvidence,
};
pub use public_session::{
    PM_PUBLIC_PING_BYTES, PM_PUBLIC_PONG_BYTES, PmPublicBookDelivery, PmPublicHeartbeatAction,
    PmPublicHeartbeatConfig, PmPublicHeartbeatEvidence, PmPublicMetadataOccurrence,
    PmPublicReconnectTransition, PmPublicSession, PmPublicSessionBatch, PmPublicSessionError,
    PmPublicSessionFault, PmPublicSessionIgnored, PmPublicUnavailableOccurrence,
    PmSnapshotFlowToken,
};
pub use reconcile_fixture::{
    MAX_PM_FIXTURE_QUERY_PAGES, PmCompleteFillQueryDelivery, PmCompleteOpenOrdersDelivery,
    PmExactOrderDetailDelivery, PmFixtureFillQueryAssembly, PmFixtureFillQueryRequest,
    PmFixtureOpenOrdersAssembly, PmFixtureOpenOrdersRequest, PmFixtureOrderDetailRequest,
    PmFixtureReconciliation, PmReconciliationContractError, PmReconciliationRole,
};
pub use roles::{
    PmAccountPositionSnapshot, PmAccountRoleGrant, PmAccountSnapshotRequest, PmAggregateDelivery,
    PmCompletionOccurrence, PmDeliveryScope, PmFeeEvidence, PmFillQueryRequest, PmInstrumentScope,
    PmOpenOrdersRequest, PmOrderDetailRequest, PmPrivateBatch, PmPrivateDelivery,
    PmPrivateLifecycle, PmPrivateRoleGrant, PmReadOwnerGrant, PmReconciliation,
    PmReconciliationRoleGrant, PmServicedAggregate,
};
