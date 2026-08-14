//! Capability-narrow live Polymarket transport roles.
//!
//! The reached surface is deliberately small: current public CLOB `/time`,
//! exact-token `/book`, the atomic `/markets` + `/clob-markets` metadata pair,
//! fixed authenticated reads for complete account cuts, exact local order
//! detail and balance/allowance, one scoped public market-WebSocket role, and
//! feature-gated literal-loopback evidence roles and reviewed-fixed-peer
//! production transports for fixed place/exact-owned cancel. It has no
//! generic mutation transport; transport construction alone grants no order
//! authority.

#![forbid(unsafe_code)]

mod account;
mod clob_health_http;
mod config;
mod deferred_mutation_time;
mod error;
mod geoblock_http;
mod http_transport;
#[cfg(any(test, feature = "loopback-evidence"))]
mod loopback_mutation_credentials;
mod metadata_http;
mod mutation;
mod observation_clock;
mod private_credentials;
mod private_http;
mod product_clock;
mod public_connectivity;
mod public_http;
mod public_ws;
mod public_ws_config;
mod read_authority;
mod read_only_private;
mod reconciliation;
mod selected_ws;
mod status_announcement_http;
mod task_guard;
mod user_ws;
mod user_ws_config;
mod ws_transport;

pub use account::{
    PmAccountAsset, PmAccountBalanceAllowance, PmAccountBalanceAllowanceObservation,
    PmAccountBalanceAllowanceObservationCommitment, PmAccountHttpRole, PmReadOnlyAccountHttpOwner,
    PmReadOnlySignatureType,
};
pub use clob_health_http::{
    PmClobLivenessHealthError, PmClobLivenessHealthHttpRole, PmClobLivenessHealthObservation,
    PmClobLivenessHealthObservationCommitment, PmProductionClobLivenessHealthObservation,
};
pub use config::{
    PM_CLOB_PRODUCTION_ORIGIN, PM_GEOBLOCK_PRODUCTION_ORIGIN, PM_STATUS_PRODUCTION_ORIGIN,
    PmGeoblockHttpConfig, PmPrivateHttpConfig, PmPublicHttpConfig,
};
pub use deferred_mutation_time::{
    PmDeferredMutationClockCapsule, PmProductionSelectedPlaceCancelTimeOwner,
    PmPublicObservationWithDeferredMutationClockOwner,
};
pub use error::{PmLiveAdapterError, PmPublicMetadataDeliveryError, PmRestBookDeliveryError};
pub use geoblock_http::{
    PmGeoblockHttpRole, PmGeoblockObservation, PmGeoblockObservationCommitment,
    PmProductionGeoblockObservation,
};
#[cfg(any(test, feature = "loopback-evidence"))]
pub use loopback_mutation_credentials::{
    PmLoopbackCancelAuthenticationFailure, PmLoopbackCancelAuthenticationRole,
    PmLoopbackMutationAuthError, PmLoopbackMutationConnectivityBinding,
    PmLoopbackMutationConnectivityOwner, PmLoopbackMutationConnectivityRoles,
    PmLoopbackPlaceAuthenticationFailure, PmLoopbackPlaceAuthenticationRole,
};
pub use metadata_http::{
    PmLiveAuthoritativeMetadataError, PmLiveAuthoritativeMetadataObservation,
    PmLiveMetadataObservation, PmLiveMetadataObservationCommitment, PmLiveMetadataPair,
    PmLiveMetadataPairSink, PmPublicMetadataHttpRole, PmTypedLiveMarketDetails,
};
pub use mutation::{
    PmCancelMutationOutcome, PmExactOwnedCancelLoopbackRole, PmExactOwnedCancelProductionRole,
    PmFixedGtcFillPlaceProductionRole, PmFixedPlaceLoopbackRole, PmFixedPlaceProductionRole,
    PmLoopbackMutationConfig, PmMutationClassification, PmMutationDiagnostic,
    PmMutationDiagnosticKind, PmMutationEdgeError, PmPlaceMutationOutcome,
    PmProductionMutationConfig, PmRetainedGtcFillPlaceRequest, PmRetainedOwnedCancelRequest,
    PmRetainedPlaceRequest,
};
pub use observation_clock::PmHttpReceiveClock;
pub use private_credentials::{
    PM_CREDENTIAL_AUTHORITY_DEFAULT_SHUTDOWN_BOUNDS,
    PM_CREDENTIAL_AUTHORITY_READ_ONLY_SHUTDOWN_BOUNDS, PmCredentialAuthorityShutdownBounds,
    PmCredentialAuthorityShutdownOutcome, PmCredentialAuthoritySupervisor,
    PmPrivateConnectivityOwner, PmPrivateConnectivityRoles,
};
pub use private_http::{
    PmAuthenticatedHttpOwner, PmClosedOnlyObservation, PmClosedOnlyObservationCommitment,
    PmPrivatePreflightHttpRole,
};
#[cfg(any(test, feature = "loopback-evidence"))]
pub use product_clock::PmLoopbackServerTimeScript;
pub use product_clock::{
    PM_MUTATION_SERVER_TIME_MAX_AGE, PmActorClockObservation, PmActorProductClock,
    PmCancelMutationAuthenticationError, PmCancelMutationTimeFinalizer, PmCancelMutationTimeProof,
    PmCancelMutationTimeProvider, PmFinalCancelMutationTime, PmFinalPlaceMutationTime,
    PmMutationTimeConsumeError, PmMutationTimeProviderError, PmOkxProductClock,
    PmPlaceMutationAuthenticationError, PmPlaceMutationTimeFinalizer, PmPlaceMutationTimeProof,
    PmPlaceMutationTimeProvider, PmPrivateReadEdgeClock, PmPrivateReadProductClock,
    PmProductClockError, PmProductClockOwner, PmPublicHttpProductClock, PmPublicWsProductClock,
    PmReadServerTime, PmReadServerTimeProductClock, PmRestResponseClock, PmUserWsProductClock,
};
#[cfg(any(test, feature = "loopback-evidence"))]
pub use product_clock::{
    PmAuthorizedMutationServerTime, PmMutationServerTimeProductClock,
    PmMutationServerTimeValidator, PmPendingMutationServerTime,
};
pub use public_connectivity::{
    PmPublicConnectivityOwner, PmPublicConnectivityRoles, PmPublicObservationConnectivityOwner,
    PmPublicObservationConnectivityRoles,
};
pub use public_http::{
    PmCancelMutationTimeOwner, PmCancelServerTimeHttpRole, PmCancelServerTimeObservation,
    PmCancelServerTimeObservationCommitment, PmPlaceMutationTimeOwner, PmPlaceServerTimeHttpRole,
    PmPlaceServerTimeObservation, PmPlaceServerTimeObservationCommitment, PmPublicHttpRole,
    PmReadServerTimeHttpRole, PmReadServerTimeObservation, PmReadServerTimeObservationCommitment,
    PmRestBookPurpose, PmRestBookSnapshotSink,
};
#[cfg(any(test, feature = "loopback-evidence"))]
pub use public_http::{
    PmMutationServerTimeHttpRole, PmMutationServerTimeObservation,
    PmMutationServerTimeObservationCommitment,
};
pub use public_ws::{
    PmProductionSelectedPublicWsRole, PmPublicMarketWsRole, PmPublicWsActivityView,
    PmPublicWsClockError, PmPublicWsClockSource, PmPublicWsConnection, PmPublicWsDisconnectReason,
    PmPublicWsEdgeClock, PmPublicWsEvent, PmPublicWsEventSink, PmPublicWsObservation,
    PmPublicWsRawData, PmPublicWsReconnect, PmPublicWsReconnectDirective, PmPublicWsRetirement,
    PmPublicWsRunError, PmPublicWsShutdownHandle, PmPublicWsShutdownSignal,
    PmPublicWsTransportError, pm_public_ws_shutdown_channel,
};
pub use public_ws_config::{
    PM_PUBLIC_MARKET_WS_ENDPOINT, PM_PUBLIC_WS_HEARTBEAT_INTERVAL, PmPublicWsBounds,
    PmPublicWsConfig, PmPublicWsTransportPolicy,
};
pub use read_authority::{
    PmExternalProxyReadConnectivityOwner, PmHttpReadAuthorityProvider,
    PmUserWsReadAuthorityProvider,
};
pub use read_only_private::{
    PmReadOnlyAccountConnectivityOwner, PmReadOnlyAccountConnectivityRoles,
    PmReadOnlyCredentialInput, PmReadOnlyPrivateConnectivityOwner,
    PmReadOnlyPrivateConnectivityRoles,
};
pub use reconciliation::{
    MAX_PM_AUTHENTICATED_CUT_PAGES, MAX_PM_AUTHENTICATED_ORDER_ROWS,
    MAX_PM_AUTHENTICATED_TRADE_ROWS, PmCompleteOpenOrdersCut, PmCompleteOpenOrdersObservation,
    PmCompleteOpenOrdersObservationCommitment, PmCompleteTradesCut, PmCompleteTradesObservation,
    PmCompleteTradesObservationCommitment, PmExactOrderDetailObservation,
    PmExactOrderDetailObservationCommitment, PmExactOrderObservation, PmOpenOrdersAssembly,
    PmOpenOrdersCutProgress, PmReconciliationHttpRole, PmTradesAssembly, PmTradesCutProgress,
};
pub use selected_ws::{PmProductionSelectedWsOwner, PmSelectedWsSocketFacts};
pub use status_announcement_http::{
    MAX_PM_STATUS_ACTIVE_INCIDENTS, MAX_PM_STATUS_ACTIVE_MAINTENANCES, MAX_PM_STATUS_COMPONENTS,
    MAX_PM_STATUS_COMPONENTS_BODY_BYTES, MAX_PM_STATUS_SUMMARY_BODY_BYTES,
    PmProductionStatusAnnouncementObservation, PmStatusActiveIncident, PmStatusActiveMaintenance,
    PmStatusAnnouncementError, PmStatusAnnouncementHttpRole, PmStatusAnnouncementObservation,
    PmStatusAnnouncementObservationCommitment, PmStatusComponentAnnouncement,
    PmStatusComponentGroup, PmStatusComponentState, PmStatusIncidentImpact, PmStatusIncidentState,
    PmStatusMaintenanceState, PmStatusPageAnnouncement, PmStatusPageState,
};
pub use user_ws::{
    PmAuthenticatedUserWsRole, PmProductionSelectedUserWsRole, PmUserWsActivityView,
    PmUserWsBoundFrame, PmUserWsClockError, PmUserWsClockSource, PmUserWsConnection,
    PmUserWsDisconnectReason, PmUserWsEdgeClock, PmUserWsEvent, PmUserWsEventSink,
    PmUserWsObservation, PmUserWsReconnect, PmUserWsRetirement, PmUserWsRunError,
    PmUserWsShutdownHandle, PmUserWsShutdownSignal, PmUserWsTransportError,
    pm_user_ws_shutdown_channel,
};
pub use user_ws_config::{
    PM_USER_WS_ENDPOINT, PM_USER_WS_HEARTBEAT_INTERVAL, PmUserWsBounds, PmUserWsConfig,
};

/// This tranche has no production order-entry authority.
pub const PRODUCTION_ORDER_ENTRY_AUTHORIZED: bool = false;
