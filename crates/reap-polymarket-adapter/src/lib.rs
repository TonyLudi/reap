#![forbid(unsafe_code)]

mod account_fixture;
mod fake_execution;
mod fixture_delivery;
mod fixture_scope;
mod private_fixture;
mod public;
mod public_metadata;
mod public_session;
mod reconcile_fixture;

pub use reap_polymarket_wire::MAX_PUBLIC_WS_FRAME_BYTES as MAX_PM_PUBLIC_RAW_FRAME_BYTES;

pub use account_fixture::{
    PmAccountPositionRoleError, PmAccountPositionSnapshotRole, PmCompleteAccountSnapshotDelivery,
    PmFixtureAccountPositionSnapshot, PmFixtureAccountSnapshotAssembly,
    PmFixtureAccountSnapshotRequest, PmFixtureAllowanceRow, PmFixtureBalanceRow,
    PmFixturePositionRow,
};
pub use fake_execution::{
    PmCancelOwnedPurpose, PmFixtureOwnedExecution, PmGtcPostOnlyProfile, PmOwnedExecutionRole,
};
pub use fixture_delivery::{
    PmFixtureAggregateDelivery, PmFixtureCompletionOccurrence, PmFixtureDeliveryError,
    PmFixtureDeliveryScope, PmFixtureServicedAggregate,
};
pub use fixture_scope::{
    PmFixtureAccountRoleGrant, PmFixtureInstrumentScope, PmFixturePrivateRoleGrant,
    PmFixtureReadOwnerGrant, PmFixtureReconciliationRoleGrant, PmFixtureScopeError,
};
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
    PmPublicSession, PmPublicSessionBatch, PmPublicSessionError, PmPublicSessionFault,
    PmPublicSessionIgnored, PmPublicUnavailableOccurrence, PmSnapshotFlowToken,
};
pub use reconcile_fixture::{
    MAX_PM_FIXTURE_QUERY_PAGES, PmCompleteFillQueryDelivery, PmCompleteOpenOrdersDelivery,
    PmExactOrderDetailDelivery, PmFixtureFillQueryAssembly, PmFixtureFillQueryRequest,
    PmFixtureOpenOrdersAssembly, PmFixtureOpenOrdersRequest, PmFixtureOrderDetailRequest,
    PmFixtureReconciliation, PmReconciliationContractError, PmReconciliationRole,
};
