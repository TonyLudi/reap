#![forbid(unsafe_code)]

mod account_fixture;
mod fake_execution;
mod private_fixture;
mod public;
mod public_metadata;
mod public_session;
mod reconcile_fixture;

pub use reap_polymarket_wire::MAX_PUBLIC_WS_FRAME_BYTES as MAX_PM_PUBLIC_RAW_FRAME_BYTES;

pub use account_fixture::{
    PmAccountPositionRoleError, PmAccountPositionSnapshotRole, PmFixtureAccountPositionSnapshot,
};
pub use fake_execution::{
    PmCancelOwnedPurpose, PmFixtureOwnedExecution, PmGtcPostOnlyProfile, PmOwnedExecutionRole,
};
pub use private_fixture::PmPrivateLifecycleRoleError;
pub use private_fixture::{PmFixturePrivateLifecycle, PmPrivateLifecycleRole};
pub use public::{PmPublicObservationRole, PmPublicRole, PmPublicRoleError};
pub use public_metadata::{PmAuthoritativeMetadata, PmMetadataJoinError, PmMetadataRevisionInput};
pub use public_session::{
    PM_PUBLIC_PING_BYTES, PM_PUBLIC_PONG_BYTES, PmPublicBookDelivery, PmPublicHeartbeatAction,
    PmPublicHeartbeatConfig, PmPublicHeartbeatEvidence, PmPublicMetadataOccurrence,
    PmPublicSession, PmPublicSessionBatch, PmPublicSessionError, PmPublicSessionFault,
    PmPublicSessionIgnored, PmPublicUnavailableOccurrence, PmSnapshotFlowToken,
};
pub use reconcile_fixture::{
    MAX_PM_RECONCILIATION_FILLS, MAX_PM_RECONCILIATION_ORDERS, PmCompleteFillPage,
    PmCompleteOpenOrdersSnapshot, PmExactOrderDetail, PmFixtureFillWatermark,
    PmFixtureReconciliation, PmReconciliationContractError, PmReconciliationRole,
};
