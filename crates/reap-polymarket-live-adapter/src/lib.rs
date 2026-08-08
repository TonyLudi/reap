//! Capability-narrow live Polymarket transport roles.
//!
//! The reached surface is deliberately small: current public CLOB `/time`,
//! exact-token `/book`, the atomic `/markets` + `/clob-markets` metadata pair,
//! fixed authenticated reads for complete account cuts, exact local order
//! detail and balance/allowance, one scoped public market-WebSocket role, and
//! feature-gated literal-loopback evidence roles for fixed place/exact-owned
//! cancel. It has no generic transport or production order-entry authority.

#![forbid(unsafe_code)]

mod account;
mod config;
mod error;
mod http_transport;
#[cfg(any(test, feature = "loopback-evidence"))]
mod loopback_mutation_credentials;
mod metadata_http;
mod mutation;
mod private_credentials;
mod private_http;
mod product_clock;
mod public_connectivity;
mod public_http;
mod public_ws;
mod public_ws_config;
mod reconciliation;
mod task_guard;
mod user_ws;
mod user_ws_config;

pub use account::{PmAccountAsset, PmAccountBalanceAllowance, PmAccountHttpRole};
pub use config::{PM_CLOB_PRODUCTION_ORIGIN, PmPrivateHttpConfig, PmPublicHttpConfig};
pub use error::{PmLiveAdapterError, PmPublicMetadataDeliveryError, PmRestBookDeliveryError};
#[cfg(any(test, feature = "loopback-evidence"))]
pub use loopback_mutation_credentials::{
    PmLoopbackCancelAuthenticationFailure, PmLoopbackCancelAuthenticationRole,
    PmLoopbackMutationAuthError, PmLoopbackMutationConnectivityBinding,
    PmLoopbackMutationConnectivityOwner, PmLoopbackMutationConnectivityRoles,
    PmLoopbackPlaceAuthenticationFailure, PmLoopbackPlaceAuthenticationRole,
};
pub use metadata_http::{PmLiveMetadataPair, PmLiveMetadataPairSink, PmPublicMetadataHttpRole};
pub use mutation::{
    PmCancelMutationOutcome, PmExactOwnedCancelLoopbackRole, PmFixedPlaceLoopbackRole,
    PmLoopbackMutationConfig, PmMutationClassification, PmMutationDiagnostic,
    PmMutationDiagnosticKind, PmMutationEdgeError, PmPlaceMutationOutcome,
    PmRetainedOwnedCancelRequest, PmRetainedPlaceRequest,
};
pub use private_credentials::{
    PmCredentialAuthoritySupervisor, PmPrivateConnectivityOwner, PmPrivateConnectivityRoles,
};
pub use private_http::PmAuthenticatedHttpOwner;
#[cfg(any(test, feature = "loopback-evidence"))]
pub use product_clock::PmLoopbackServerTimeScript;
pub use product_clock::{
    PM_MUTATION_SERVER_TIME_MAX_AGE, PmActorClockObservation, PmActorProductClock,
    PmAuthorizedMutationServerTime, PmMutationServerTimeProductClock,
    PmMutationServerTimeValidator, PmOkxProductClock, PmPendingMutationServerTime,
    PmPrivateReadEdgeClock, PmPrivateReadProductClock, PmProductClockError, PmProductClockOwner,
    PmProductClockViews, PmPublicHttpProductClock, PmPublicWsProductClock, PmReadServerTime,
    PmReadServerTimeProductClock, PmRestResponseClock, PmUserWsProductClock,
};
pub use public_connectivity::{PmPublicConnectivityOwner, PmPublicConnectivityRoles};
pub use public_http::{
    PmMutationServerTimeHttpRole, PmPublicHttpRole, PmReadServerTimeHttpRole, PmRestBookPurpose,
    PmRestBookSnapshotSink,
};
pub use public_ws::{
    PmPublicMarketWsRole, PmPublicWsClockError, PmPublicWsClockSource, PmPublicWsConnection,
    PmPublicWsDisconnectReason, PmPublicWsEdgeClock, PmPublicWsEvent, PmPublicWsEventSink,
    PmPublicWsObservation, PmPublicWsRawData, PmPublicWsReconnect, PmPublicWsReconnectDirective,
    PmPublicWsRetirement, PmPublicWsRunError, PmPublicWsShutdownHandle, PmPublicWsShutdownSignal,
    PmPublicWsTransportError, pm_public_ws_shutdown_channel,
};
pub use public_ws_config::{
    PM_PUBLIC_MARKET_WS_ENDPOINT, PM_PUBLIC_WS_HEARTBEAT_INTERVAL, PmPublicWsBounds,
    PmPublicWsConfig, PmPublicWsTransportPolicy,
};
pub use reconciliation::{
    MAX_PM_AUTHENTICATED_CUT_PAGES, MAX_PM_AUTHENTICATED_ORDER_ROWS,
    MAX_PM_AUTHENTICATED_TRADE_ROWS, PmCompleteOpenOrdersCut, PmCompleteTradesCut,
    PmExactOrderObservation, PmOpenOrdersAssembly, PmOpenOrdersCutProgress,
    PmReconciliationHttpRole, PmTradesAssembly, PmTradesCutProgress,
};
pub use user_ws::{
    PmAuthenticatedUserWsRole, PmUserWsBoundFrame, PmUserWsClockError, PmUserWsClockSource,
    PmUserWsConnection, PmUserWsDisconnectReason, PmUserWsEdgeClock, PmUserWsEvent,
    PmUserWsEventSink, PmUserWsObservation, PmUserWsReconnect, PmUserWsRetirement,
    PmUserWsRunError, PmUserWsShutdownHandle, PmUserWsShutdownSignal, PmUserWsTransportError,
    pm_user_ws_shutdown_channel,
};
pub use user_ws_config::{
    PM_USER_WS_ENDPOINT, PM_USER_WS_HEARTBEAT_INTERVAL, PmUserWsBounds, PmUserWsConfig,
};

/// This tranche has no production order-entry authority.
pub const PRODUCTION_ORDER_ENTRY_AUTHORIZED: bool = false;
