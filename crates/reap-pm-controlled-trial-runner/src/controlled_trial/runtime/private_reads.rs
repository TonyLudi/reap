//! Runner-private authenticated REST collection for the PM-T2 trial.
//!
//! The production constructor consumes one unsplit credential-authority role
//! bundle and immediately installs its HTTP and user-WebSocket handles in the
//! fixed proxy read-only connectivity owner. The two resulting runtime halves
//! share an opaque allocation identity and one checked user-activity source.
//! Neither half exposes credential material, a generic route, a raw body, or
//! mutation authority.

use std::{fmt, sync::Arc, time::Duration};

use reap_pm_core::{
    ConnectionEpoch, EvmAddress, PmBookQuantity, PmConditionId, PmFillId, PmOrderSide, PmPrice,
    PmQuantity, PmTokenId, PmVenueOrderId, U256,
};
use reap_polymarket_auth::{EoaAddress, FixedOrderId, L2Timestamp};
use reap_polymarket_egress_binding::{PmFixedTlsPeerSelection, PmLocalEgressSelection};
use reap_polymarket_live_adapter::{
    PmAccountAsset, PmAccountBalanceAllowanceObservation,
    PmAccountBalanceAllowanceObservationCommitment, PmAuthenticatedHttpOwner,
    PmAuthenticatedUserWsRole, PmClosedOnlyObservation, PmCompleteOpenOrdersObservationCommitment,
    PmCompleteTradesObservationCommitment, PmExactOrderDetailObservation,
    PmExactOrderDetailObservationCommitment, PmExactOrderObservation,
    PmExternalProxyReadConnectivityOwner, PmLiveAdapterError, PmOpenOrdersCutProgress,
    PmPrivateReadEdgeClock, PmPrivateReadProductClock, PmProductionSelectedPublicWsRole,
    PmProductionSelectedUserWsRole, PmProductionSelectedWsOwner, PmPublicMarketWsRole,
    PmReadServerTime, PmReadServerTimeHttpRole, PmReadServerTimeObservationCommitment,
    PmRestResponseClock, PmTradesCutProgress, PmUserWsActivityView, PmUserWsBounds,
};
#[cfg(test)]
use reap_polymarket_live_adapter::{PmPrivateHttpConfig, PmUserWsConfig};
use reap_polymarket_wire::{PmLiveOrder, PmLiveTrade, PmWireScope};
use thiserror::Error;

#[cfg(test)]
use super::super::authority::FreshStagedObservationAuthorityRoles;
use super::super::authority::{
    CredentialAuthorityShutdownBounds, CredentialAuthorityShutdownOutcome,
    ExactOwnedCancelAuthenticationRole, FixedHttpAuthenticationRole, FixedUserWsAuthenticationRole,
    FreshCredentialAuthorityOwner, FreshCredentialAuthorityRoles,
    FreshCredentialAuthoritySupervisor, FreshPlaceAuthenticationOnce,
    PmObservingFreshCredentialCustody, PmObservingFreshCredentialShutdownError,
    RecoveryCredentialAuthorityRoles, RecoveryCredentialAuthoritySupervisor,
};
use super::public_book::{PmDeferredObservationAssemblyToken, PmPrivateReadClockBundle};

/// Cold production-only profile for the fixed type-1 private read edge.
///
/// There is deliberately no origin, route, method, signature type, or
/// credential slot in this runner-facing value. The live adapter fixes the
/// production endpoints and the proxy signature profile.
pub(super) struct PmPrivateReadRuntimeProfile {
    connect_timeout: Duration,
    request_timeout: Duration,
    scope: PmWireScope,
    user_ws_bounds: PmUserWsBounds,
    signer: EoaAddress,
    proxy_funder: EvmAddress,
}

impl PmPrivateReadRuntimeProfile {
    pub(super) const fn new(
        connect_timeout: Duration,
        request_timeout: Duration,
        scope: PmWireScope,
        user_ws_bounds: PmUserWsBounds,
        signer: EoaAddress,
        proxy_funder: EvmAddress,
    ) -> Self {
        Self {
            connect_timeout,
            request_timeout,
            scope,
            user_ws_bounds,
            signer,
            proxy_funder,
        }
    }
}

impl fmt::Debug for PmPrivateReadRuntimeProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPrivateReadRuntimeProfile([REDACTED; FIXED PROXY READ])")
    }
}

struct SameCredentialAuthorityIdentity {
    scope: PmWireScope,
    signer: EoaAddress,
    proxy_funder: EvmAddress,
    activity: SameCredentialAuthorityActivity,
}

enum SameCredentialAuthorityActivity {
    Live(PmUserWsActivityView),
    #[cfg(test)]
    Test(Arc<std::sync::atomic::AtomicU64>),
}

impl SameCredentialAuthorityActivity {
    fn generation(&self) -> u64 {
        match self {
            Self::Live(activity) => activity.generation(),
            #[cfg(test)]
            Self::Test(activity) => activity.load(std::sync::atomic::Ordering::Acquire),
        }
    }

    fn live_view(&self) -> PmUserWsActivityView {
        match self {
            Self::Live(activity) => activity.clone(),
            #[cfg(test)]
            Self::Test(_) => panic!("test-only preflight marker has no live user-WS role"),
        }
    }
}

/// Opaque, move-only same-credential marker shared by the user runtime and
/// authenticated REST collector. The named fork is intentionally narrower
/// and more reviewable than a blanket `Clone` implementation.
pub(super) struct PmSameCredentialAuthorityMarker {
    identity: Arc<SameCredentialAuthorityIdentity>,
}

impl PmSameCredentialAuthorityMarker {
    fn new(
        scope: PmWireScope,
        signer: EoaAddress,
        proxy_funder: EvmAddress,
        activity: PmUserWsActivityView,
    ) -> Self {
        Self {
            identity: Arc::new(SameCredentialAuthorityIdentity {
                scope,
                signer,
                proxy_funder,
                activity: SameCredentialAuthorityActivity::Live(activity),
            }),
        }
    }

    /// Unit-test fixture for the sibling user-stream state machine. Production
    /// composition can construct a marker only from the live role above.
    #[cfg(test)]
    pub(super) fn test_support_online_preflight_seal(
        scope: PmWireScope,
        signer: EoaAddress,
        proxy_funder: EvmAddress,
        activity: Arc<std::sync::atomic::AtomicU64>,
    ) -> (Self, Arc<PmRestCutIdentity>) {
        (
            Self {
                identity: Arc::new(SameCredentialAuthorityIdentity {
                    scope,
                    signer,
                    proxy_funder,
                    activity: SameCredentialAuthorityActivity::Test(activity),
                }),
            },
            Arc::new(PmRestCutIdentity { _private: () }),
        )
    }

    pub(super) fn same_instance(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
    }

    pub(super) fn fork_for_same_authority(&self) -> Self {
        Self {
            identity: Arc::clone(&self.identity),
        }
    }

    pub(super) fn scope(&self) -> PmWireScope {
        self.identity.scope
    }

    pub(super) fn signer(&self) -> EoaAddress {
        self.identity.signer
    }

    pub(super) fn proxy_funder(&self) -> EvmAddress {
        self.identity.proxy_funder
    }

    /// Called only by the user-stream handle after its source-owned
    /// subscription/PING/correlated-PONG readiness checks. The activity
    /// generation is sampled here rather than accepted from a caller.
    pub(super) fn begin_rest_collection(
        &self,
        stream_revision: u64,
        connection_epoch: ConnectionEpoch,
    ) -> PmUserRestCollectionStart {
        PmUserRestCollectionStart {
            marker: self.fork_for_same_authority(),
            stream_revision,
            connection_epoch,
            activity_generation: self.identity.activity.generation(),
        }
    }

    fn activity_view(&self) -> PmUserWsActivityView {
        self.identity.activity.live_view()
    }
}

impl fmt::Debug for PmSameCredentialAuthorityMarker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmSameCredentialAuthorityMarker(<opaque allocation>)")
    }
}

/// User-WebSocket role joined to the exact credential authority installed in
/// the private HTTP owner. It must be consumed as one value by user runtime
/// assembly.
pub(super) struct PmSameCredentialUserWsInput {
    role: PmAuthenticatedUserWsRole,
    marker: PmSameCredentialAuthorityMarker,
}

impl PmSameCredentialUserWsInput {
    pub(super) fn into_parts(
        self,
    ) -> (
        PmAuthenticatedUserWsRole,
        PmUserWsActivityView,
        PmSameCredentialAuthorityMarker,
    ) {
        let activity = self.marker.activity_view();
        (self.role, activity, self.marker)
    }
}

impl fmt::Debug for PmSameCredentialUserWsInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmSameCredentialUserWsInput([REDACTED; JOINED])")
    }
}

/// Source-sampled user-stream boundary immediately before a REST collection.
///
/// The user runtime supplies only its typed revision and connection epoch;
/// the same-authority marker samples the high-water generation. The value is
/// consumed by REST and returned unchanged inside `PmSameAuthorityRestJoin`.
pub(super) struct PmUserRestCollectionStart {
    marker: PmSameCredentialAuthorityMarker,
    stream_revision: u64,
    connection_epoch: ConnectionEpoch,
    activity_generation: u64,
}

impl PmUserRestCollectionStart {
    pub(super) const fn marker(&self) -> &PmSameCredentialAuthorityMarker {
        &self.marker
    }

    pub(super) const fn stream_revision(&self) -> u64 {
        self.stream_revision
    }

    pub(super) const fn connection_epoch(&self) -> ConnectionEpoch {
        self.connection_epoch
    }

    pub(super) const fn activity_generation(&self) -> u64 {
        self.activity_generation
    }
}

impl fmt::Debug for PmUserRestCollectionStart {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmUserRestCollectionStart")
            .field("stream_revision", &self.stream_revision)
            .field("connection_epoch", &self.connection_epoch)
            .field("activity_generation", &self.activity_generation)
            .finish_non_exhaustive()
    }
}

/// Fresh allocation shared by exactly one final REST cut and the user-stream
/// ticket derived from its accompanying join.
pub(super) struct PmRestCutIdentity {
    _private: (),
}

impl fmt::Debug for PmRestCutIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmRestCutIdentity(<opaque allocation>)")
    }
}

/// REST completion half returned to the user runtime for its immediate
/// source-state recheck. It carries the exact consumed start and the final cut
/// allocation; neither can be reconstructed from scalar fields.
pub(super) struct PmSameAuthorityRestJoin {
    start: PmUserRestCollectionStart,
    rest_cut_identity: Arc<PmRestCutIdentity>,
    open_order_rows: usize,
    trade_rows: usize,
}

impl PmSameAuthorityRestJoin {
    pub(super) const fn open_order_rows(&self) -> usize {
        self.open_order_rows
    }

    pub(super) const fn trade_rows(&self) -> usize {
        self.trade_rows
    }

    pub(super) fn into_parts(self) -> (PmUserRestCollectionStart, Arc<PmRestCutIdentity>) {
        (self.start, self.rest_cut_identity)
    }
}

impl fmt::Debug for PmSameAuthorityRestJoin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmSameAuthorityRestJoin")
            .field("open_order_rows", &self.open_order_rows)
            .field("trade_rows", &self.trade_rows)
            .field("authority", &"<same opaque allocation>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PmPrivateRestObservationPurpose {
    ClosedOnly,
    CollateralBalanceAllowance,
    ConditionalBalanceAllowance,
    OpenOrdersPage { ordinal: usize },
    TradesPage { ordinal: usize },
    RecoveryExactOrder,
}

/// Exact parsed `/time` value and source edge used for one following fixed
/// private GET. Every entry is source-created and consume-once at collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PmPrivateReadServerTimeEvidence {
    purpose: PmPrivateRestObservationPurpose,
    timestamp: L2Timestamp,
    receive_clock: PmRestResponseClock,
    commitment: PmReadServerTimeObservationCommitment,
}

impl PmPrivateReadServerTimeEvidence {
    pub(super) const fn purpose(&self) -> PmPrivateRestObservationPurpose {
        self.purpose
    }

    pub(super) const fn timestamp(&self) -> L2Timestamp {
        self.timestamp
    }

    pub(super) const fn receive_clock(&self) -> PmRestResponseClock {
        self.receive_clock
    }

    pub(super) const fn commitment(&self) -> PmReadServerTimeObservationCommitment {
        self.commitment
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct PmAuthenticatedOpenOrderProjection {
    id: PmVenueOrderId,
    condition: PmConditionId,
    token: PmTokenId,
    side: PmOrderSide,
    original_size: PmQuantity,
    size_matched: PmBookQuantity,
    price: PmPrice,
    maker: EvmAddress,
    created_at: u64,
    expiration: u64,
}

impl fmt::Debug for PmAuthenticatedOpenOrderProjection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmAuthenticatedOpenOrderProjection([REDACTED])")
    }
}

impl PmAuthenticatedOpenOrderProjection {
    pub(super) const fn id(self) -> PmVenueOrderId {
        self.id
    }

    pub(super) const fn condition(self) -> PmConditionId {
        self.condition
    }

    pub(super) const fn token(self) -> PmTokenId {
        self.token
    }

    pub(super) const fn side(self) -> PmOrderSide {
        self.side
    }

    pub(super) const fn original_size(self) -> PmQuantity {
        self.original_size
    }

    pub(super) const fn size_matched(self) -> PmBookQuantity {
        self.size_matched
    }

    pub(super) const fn price(self) -> PmPrice {
        self.price
    }

    pub(super) const fn maker(self) -> EvmAddress {
        self.maker
    }

    pub(super) const fn created_at(self) -> u64 {
        self.created_at
    }

    pub(super) const fn expiration(self) -> u64 {
        self.expiration
    }
}

/// Variable venue labels are retained as bounded parsed strings, while the
/// credential-owner field is intentionally absent.
#[derive(PartialEq, Eq)]
pub(super) struct PmAuthenticatedOpenOrderTextProjection {
    status: Box<str>,
    outcome: Option<Box<str>>,
    order_type: Option<Box<str>>,
}

impl fmt::Debug for PmAuthenticatedOpenOrderTextProjection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmAuthenticatedOpenOrderTextProjection([REDACTED])")
    }
}

impl PmAuthenticatedOpenOrderTextProjection {
    pub(super) fn status(&self) -> &str {
        &self.status
    }

    pub(super) fn outcome(&self) -> Option<&str> {
        self.outcome.as_deref()
    }

    pub(super) fn order_type(&self) -> Option<&str> {
        self.order_type.as_deref()
    }
}

#[derive(PartialEq, Eq)]
pub(super) struct PmAuthenticatedOpenOrderRow {
    exact: PmAuthenticatedOpenOrderProjection,
    text: PmAuthenticatedOpenOrderTextProjection,
}

impl fmt::Debug for PmAuthenticatedOpenOrderRow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmAuthenticatedOpenOrderRow([REDACTED])")
    }
}

impl PmAuthenticatedOpenOrderRow {
    pub(super) const fn exact(&self) -> PmAuthenticatedOpenOrderProjection {
        self.exact
    }

    pub(super) const fn text(&self) -> &PmAuthenticatedOpenOrderTextProjection {
        &self.text
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct PmAuthenticatedMakerLegProjection {
    order_id: PmVenueOrderId,
    token: PmTokenId,
    side: PmOrderSide,
    price: PmPrice,
    matched_amount: PmQuantity,
    fee_rate_bps: Option<U256>,
    maker: EvmAddress,
}

impl fmt::Debug for PmAuthenticatedMakerLegProjection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmAuthenticatedMakerLegProjection([REDACTED])")
    }
}

impl PmAuthenticatedMakerLegProjection {
    pub(super) const fn order_id(self) -> PmVenueOrderId {
        self.order_id
    }

    pub(super) const fn token(self) -> PmTokenId {
        self.token
    }

    pub(super) const fn side(self) -> PmOrderSide {
        self.side
    }

    pub(super) const fn price(self) -> PmPrice {
        self.price
    }

    pub(super) const fn matched_amount(self) -> PmQuantity {
        self.matched_amount
    }

    pub(super) const fn fee_rate_bps(self) -> Option<U256> {
        self.fee_rate_bps
    }

    pub(super) const fn maker(self) -> EvmAddress {
        self.maker
    }
}

/// Complete owner-free trade row needed for fill/lifecycle reconciliation.
#[derive(PartialEq, Eq)]
pub(super) struct PmAuthenticatedTradeRow {
    id: PmFillId,
    condition: PmConditionId,
    token: PmTokenId,
    side: PmOrderSide,
    size: PmQuantity,
    price: PmPrice,
    status: Box<str>,
    order_id: Option<PmVenueOrderId>,
    taker_order_id: Option<PmVenueOrderId>,
    trader_side: Option<Box<str>>,
    transaction_hash: Option<Box<str>>,
    fee_rate_bps: Option<U256>,
    maker_orders: Box<[PmAuthenticatedMakerLegProjection]>,
    maker: Option<EvmAddress>,
    timestamp: Option<u64>,
    match_time: Option<u64>,
    last_update: Option<u64>,
}

impl fmt::Debug for PmAuthenticatedTradeRow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmAuthenticatedTradeRow([REDACTED])")
    }
}

impl PmAuthenticatedTradeRow {
    pub(super) const fn id(&self) -> PmFillId {
        self.id
    }

    pub(super) const fn condition(&self) -> PmConditionId {
        self.condition
    }

    pub(super) const fn token(&self) -> PmTokenId {
        self.token
    }

    pub(super) const fn side(&self) -> PmOrderSide {
        self.side
    }

    pub(super) const fn size(&self) -> PmQuantity {
        self.size
    }

    pub(super) const fn price(&self) -> PmPrice {
        self.price
    }

    pub(super) fn status(&self) -> &str {
        &self.status
    }

    pub(super) const fn order_id(&self) -> Option<PmVenueOrderId> {
        self.order_id
    }

    pub(super) const fn taker_order_id(&self) -> Option<PmVenueOrderId> {
        self.taker_order_id
    }

    pub(super) fn trader_side(&self) -> Option<&str> {
        self.trader_side.as_deref()
    }

    pub(super) fn transaction_hash(&self) -> Option<&str> {
        self.transaction_hash.as_deref()
    }

    pub(super) const fn fee_rate_bps(&self) -> Option<U256> {
        self.fee_rate_bps
    }

    pub(super) fn maker_orders(&self) -> &[PmAuthenticatedMakerLegProjection] {
        &self.maker_orders
    }

    pub(super) const fn maker(&self) -> Option<EvmAddress> {
        self.maker
    }

    pub(super) const fn timestamp(&self) -> Option<u64> {
        self.timestamp
    }

    pub(super) const fn match_time(&self) -> Option<u64> {
        self.match_time
    }

    pub(super) const fn last_update(&self) -> Option<u64> {
        self.last_update
    }
}

#[derive(PartialEq, Eq)]
pub(super) struct PmCompleteOpenOrdersSummary {
    page_count: usize,
    row_count: usize,
    receive_clock: PmPrivateReadEdgeClock,
    commitment: PmCompleteOpenOrdersObservationCommitment,
    rows: Box<[PmAuthenticatedOpenOrderRow]>,
}

impl fmt::Debug for PmCompleteOpenOrdersSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmCompleteOpenOrdersSummary")
            .field("page_count", &self.page_count)
            .field("row_count", &self.row_count)
            .field("receive_clock", &self.receive_clock)
            .field("commitment", &self.commitment)
            .field("rows", &"[REDACTED]")
            .finish()
    }
}

impl PmCompleteOpenOrdersSummary {
    pub(super) const fn page_count(&self) -> usize {
        self.page_count
    }

    pub(super) const fn row_count(&self) -> usize {
        self.row_count
    }

    pub(super) const fn receive_clock(&self) -> PmPrivateReadEdgeClock {
        self.receive_clock
    }

    pub(super) const fn commitment(&self) -> PmCompleteOpenOrdersObservationCommitment {
        self.commitment
    }

    pub(super) fn rows(&self) -> &[PmAuthenticatedOpenOrderRow] {
        &self.rows
    }
}

#[derive(PartialEq, Eq)]
pub(super) struct PmCompleteTradesSummary {
    page_count: usize,
    row_count: usize,
    receive_clock: PmPrivateReadEdgeClock,
    commitment: PmCompleteTradesObservationCommitment,
    rows: Box<[PmAuthenticatedTradeRow]>,
}

impl fmt::Debug for PmCompleteTradesSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmCompleteTradesSummary")
            .field("page_count", &self.page_count)
            .field("row_count", &self.row_count)
            .field("receive_clock", &self.receive_clock)
            .field("commitment", &self.commitment)
            .field("rows", &"[REDACTED]")
            .finish()
    }
}

impl PmCompleteTradesSummary {
    pub(super) const fn page_count(&self) -> usize {
        self.page_count
    }

    pub(super) const fn row_count(&self) -> usize {
        self.row_count
    }

    pub(super) const fn receive_clock(&self) -> PmPrivateReadEdgeClock {
        self.receive_clock
    }

    pub(super) const fn commitment(&self) -> PmCompleteTradesObservationCommitment {
        self.commitment
    }

    pub(super) fn rows(&self) -> &[PmAuthenticatedTradeRow] {
        &self.rows
    }
}

#[derive(PartialEq, Eq)]
pub(super) enum PmRecoveryExactOrderClassification {
    Absent,
    Present(Box<PmRecoveryExactOrderProjection>),
}

impl fmt::Debug for PmRecoveryExactOrderClassification {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Absent => "PmRecoveryExactOrderClassification::Absent",
            Self::Present(_) => "PmRecoveryExactOrderClassification::Present([REDACTED])",
        })
    }
}

/// Exact-order projection deliberately omitting the credential-owner field.
#[derive(PartialEq, Eq)]
pub(super) struct PmRecoveryExactOrderProjection {
    condition: PmConditionId,
    token: PmTokenId,
    side: PmOrderSide,
    original_size: PmQuantity,
    size_matched: PmBookQuantity,
    price: PmPrice,
    status: Box<str>,
    maker: EvmAddress,
    created_at: u64,
    expiration: u64,
    outcome: Option<Box<str>>,
    order_type: Option<Box<str>>,
}

impl fmt::Debug for PmRecoveryExactOrderProjection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmRecoveryExactOrderProjection([REDACTED])")
    }
}

impl PmRecoveryExactOrderProjection {
    pub(super) const fn condition(&self) -> PmConditionId {
        self.condition
    }

    pub(super) const fn token(&self) -> PmTokenId {
        self.token
    }

    pub(super) const fn side(&self) -> PmOrderSide {
        self.side
    }

    pub(super) const fn original_size(&self) -> PmQuantity {
        self.original_size
    }

    pub(super) const fn size_matched(&self) -> PmBookQuantity {
        self.size_matched
    }

    pub(super) const fn price(&self) -> PmPrice {
        self.price
    }

    pub(super) fn status(&self) -> &str {
        &self.status
    }

    pub(super) const fn maker(&self) -> EvmAddress {
        self.maker
    }

    pub(super) const fn created_at(&self) -> u64 {
        self.created_at
    }

    pub(super) const fn expiration(&self) -> u64 {
        self.expiration
    }

    pub(super) fn outcome(&self) -> Option<&str> {
        self.outcome.as_deref()
    }

    pub(super) fn order_type(&self) -> Option<&str> {
        self.order_type.as_deref()
    }
}

#[derive(PartialEq, Eq)]
pub(super) struct PmRecoveryExactOrderSummary {
    order_id: FixedOrderId,
    classification: PmRecoveryExactOrderClassification,
    receive_clock: PmPrivateReadEdgeClock,
    commitment: PmExactOrderDetailObservationCommitment,
}

/// Runner-owned allowance projection. Exact scalars remain available for
/// in-memory risk recomputation, while aggregate formatting stays redacted.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct PmPrivateAllowanceProjection {
    spender: EvmAddress,
    amount: U256,
}

impl PmPrivateAllowanceProjection {
    pub(super) const fn spender(self) -> EvmAddress {
        self.spender
    }

    pub(super) const fn amount(self) -> U256 {
        self.amount
    }
}

impl fmt::Debug for PmPrivateAllowanceProjection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmPrivateAllowanceProjection([REDACTED])")
    }
}

/// In-memory runner-owned numeric account evidence with deliberately
/// redacted Debug. No externally derived account aggregate escapes this
/// wrapper; callers receive only exact typed scalar and allowance views.
pub(super) struct PmPrivateAccountBalanceAllowanceEvidence {
    asset: PmAccountAsset,
    balance: U256,
    allowances: Box<[PmPrivateAllowanceProjection]>,
    unscoped_scalar_present: bool,
    receive_clock: PmPrivateReadEdgeClock,
    commitment: PmAccountBalanceAllowanceObservationCommitment,
}

impl PmPrivateAccountBalanceAllowanceEvidence {
    fn new(observation: PmAccountBalanceAllowanceObservation) -> Self {
        let balance_allowance = observation.balance_allowance();
        Self {
            asset: balance_allowance.asset(),
            balance: balance_allowance.value().balance(),
            allowances: balance_allowance
                .value()
                .allowances()
                .iter()
                .map(|allowance| PmPrivateAllowanceProjection {
                    spender: allowance.spender(),
                    amount: allowance.amount(),
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            unscoped_scalar_present: balance_allowance.value().unscoped_scalar_present(),
            receive_clock: observation.receive_clock(),
            commitment: observation.commitment(),
        }
    }

    pub(super) const fn asset(&self) -> PmAccountAsset {
        self.asset
    }

    pub(super) const fn balance(&self) -> U256 {
        self.balance
    }

    pub(super) fn allowances(&self) -> &[PmPrivateAllowanceProjection] {
        &self.allowances
    }

    pub(super) fn exact_allowance(&self, spender: EvmAddress) -> Option<U256> {
        self.allowances
            .iter()
            .find(|allowance| allowance.spender == spender)
            .map(|allowance| allowance.amount)
    }

    pub(super) const fn unscoped_scalar_present(&self) -> bool {
        self.unscoped_scalar_present
    }

    pub(super) const fn receive_clock(&self) -> PmPrivateReadEdgeClock {
        self.receive_clock
    }

    pub(super) const fn commitment(&self) -> PmAccountBalanceAllowanceObservationCommitment {
        self.commitment
    }
}

impl fmt::Debug for PmPrivateAccountBalanceAllowanceEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmPrivateAccountBalanceAllowanceEvidence")
            .field("asset", &self.asset)
            .field("receive_clock", &self.receive_clock)
            .field("commitment", &self.commitment)
            .field("numeric_values", &"[REDACTED]")
            .finish()
    }
}

impl fmt::Debug for PmRecoveryExactOrderSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let classification = match &self.classification {
            PmRecoveryExactOrderClassification::Absent => "Absent",
            PmRecoveryExactOrderClassification::Present(_) => "Present([REDACTED])",
        };
        formatter
            .debug_struct("PmRecoveryExactOrderSummary")
            .field("order_id", &"[REDACTED]")
            .field("classification", &classification)
            .field("receive_clock", &self.receive_clock)
            .field("commitment", &self.commitment)
            .finish()
    }
}

impl PmRecoveryExactOrderSummary {
    pub(super) const fn order_id(&self) -> FixedOrderId {
        self.order_id
    }

    pub(super) const fn classification(&self) -> &PmRecoveryExactOrderClassification {
        &self.classification
    }

    pub(super) const fn receive_clock(&self) -> PmPrivateReadEdgeClock {
        self.receive_clock
    }

    pub(super) const fn commitment(&self) -> PmExactOrderDetailObservationCommitment {
        self.commitment
    }
}

/// Shared private evidence storage. It is never returned directly: mode-
/// specific wrappers below prevent a recovery cut from entering fresh-place
/// permit assembly.
struct PmAuthenticatedRestCutCore {
    rest_cut_identity: Arc<PmRestCutIdentity>,
    activity_generation: u64,
    server_times: Box<[PmPrivateReadServerTimeEvidence]>,
    closed_only: PmClosedOnlyObservation,
    collateral: PmPrivateAccountBalanceAllowanceEvidence,
    conditional: PmPrivateAccountBalanceAllowanceEvidence,
    open_orders: PmCompleteOpenOrdersSummary,
    trades: PmCompleteTradesSummary,
    exact_order: Option<PmRecoveryExactOrderSummary>,
}

impl PmAuthenticatedRestCutCore {
    fn matches_rest_cut_identity(&self, identity: &Arc<PmRestCutIdentity>) -> bool {
        Arc::ptr_eq(&self.rest_cut_identity, identity)
    }

    const fn activity_generation(&self) -> u64 {
        self.activity_generation
    }

    fn server_times(&self) -> &[PmPrivateReadServerTimeEvidence] {
        &self.server_times
    }

    const fn closed_only(&self) -> &PmClosedOnlyObservation {
        &self.closed_only
    }

    const fn collateral(&self) -> &PmPrivateAccountBalanceAllowanceEvidence {
        &self.collateral
    }

    const fn conditional(&self) -> &PmPrivateAccountBalanceAllowanceEvidence {
        &self.conditional
    }

    const fn open_orders(&self) -> &PmCompleteOpenOrdersSummary {
        &self.open_orders
    }

    const fn trades(&self) -> &PmCompleteTradesSummary {
        &self.trades
    }

    const fn exact_order(&self) -> Option<&PmRecoveryExactOrderSummary> {
        self.exact_order.as_ref()
    }
}

impl fmt::Debug for PmAuthenticatedRestCutCore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmAuthenticatedRestCutCore")
            .field("activity_generation", &self.activity_generation)
            .field("server_time_observations", &self.server_times.len())
            .field("closed_only", &"<typed source observation>")
            .field("account_balances", &"[REDACTED]")
            .field("open_orders", &self.open_orders)
            .field("trades", &self.trades)
            .field("exact_order", &self.exact_order.as_ref().map(|_| "<typed>"))
            .finish()
    }
}

macro_rules! impl_common_cut_view {
    ($cut:ident) => {
        impl $cut {
            pub(super) fn matches_rest_cut_identity(
                &self,
                identity: &Arc<PmRestCutIdentity>,
            ) -> bool {
                self.core.matches_rest_cut_identity(identity)
            }

            pub(super) const fn activity_generation(&self) -> u64 {
                self.core.activity_generation()
            }

            pub(super) fn server_times(&self) -> &[PmPrivateReadServerTimeEvidence] {
                self.core.server_times()
            }

            pub(super) const fn closed_only(&self) -> &PmClosedOnlyObservation {
                self.core.closed_only()
            }

            pub(super) const fn collateral(&self) -> &PmPrivateAccountBalanceAllowanceEvidence {
                self.core.collateral()
            }

            pub(super) const fn conditional(&self) -> &PmPrivateAccountBalanceAllowanceEvidence {
                self.core.conditional()
            }

            pub(super) const fn open_orders(&self) -> &PmCompleteOpenOrdersSummary {
                self.core.open_orders()
            }

            pub(super) const fn trades(&self) -> &PmCompleteTradesSummary {
                self.core.trades()
            }
        }

        impl fmt::Debug for $cut {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_tuple(stringify!($cut))
                    .field(&self.core)
                    .finish()
            }
        }
    };
}

/// Only this cut can be consumed by later fresh-place admission.
pub(super) struct PmFreshAuthenticatedRestCut {
    core: PmAuthenticatedRestCutCore,
}

impl_common_cut_view!(PmFreshAuthenticatedRestCut);

/// L2-only recovery account cut without an expected-order lookup.
pub(super) struct PmRecoveryAccountAuthenticatedRestCut {
    core: PmAuthenticatedRestCutCore,
}

impl_common_cut_view!(PmRecoveryAccountAuthenticatedRestCut);

/// L2-only recovery cut that additionally observes one expected order ID.
/// A later integration slice must bind that candidate to A3 journal
/// provenance before this path becomes constructible by runner siblings.
pub(super) struct PmRecoveryExactAuthenticatedRestCut {
    core: PmAuthenticatedRestCutCore,
}

impl_common_cut_view!(PmRecoveryExactAuthenticatedRestCut);

impl PmRecoveryExactAuthenticatedRestCut {
    pub(super) fn exact_order(&self) -> &PmRecoveryExactOrderSummary {
        self.core
            .exact_order()
            .expect("recovery-exact constructor always installs exact evidence")
    }
}

#[derive(Debug, Error)]
pub(super) enum PmPrivateReadRuntimeError {
    #[error("fixed authenticated REST source failed: {0}")]
    Live(#[from] PmLiveAdapterError),
    #[error("user and REST roles do not share the same credential authority")]
    SameCredentialAuthorityMismatch,
    #[error("user-stream activity changed; discard the torn REST cut and retry")]
    UserActivityChangedRetryRequired,
    #[error("staged observation credentials do not match the fixed private-read signer")]
    StagedObservationSignerBindingMismatch,
    #[error("staged observation credential authority could not be started")]
    StagedObservationStartFailed,
    #[error(
        "staged observation build failed ({build}) and bounded credential cleanup failed ({cleanup})"
    )]
    StagedObservationCleanupFailed {
        build: Box<PmPrivateReadRuntimeError>,
        cleanup: PmObservingFreshCredentialShutdownError,
    },
    #[error(
        "staged observation build failed ({build}) after credential custody reached an abnormal but terminal shutdown"
    )]
    StagedObservationCleanupAbnormal {
        build: Box<PmPrivateReadRuntimeError>,
        abort_requested: bool,
        task_completed_cleanly: bool,
    },
}

struct PmAuthenticatedRestRuntimeCore {
    http: PmAuthenticatedHttpOwner,
    server_time: PmReadServerTimeHttpRole,
    private_clock: PmPrivateReadProductClock,
    marker: PmSameCredentialAuthorityMarker,
}

impl PmAuthenticatedRestRuntimeCore {
    async fn collect(
        &mut self,
        start: PmUserRestCollectionStart,
        exact_order_id: Option<FixedOrderId>,
    ) -> Result<(PmAuthenticatedRestCutCore, PmSameAuthorityRestJoin), PmPrivateReadRuntimeError>
    {
        if !self.marker.same_instance(start.marker()) {
            return Err(PmPrivateReadRuntimeError::SameCredentialAuthorityMismatch);
        }
        let generation = start.activity_generation();
        self.ensure_activity_unchanged(generation)?;

        let mut server_times = Vec::new();

        let time = self
            .fresh_server_time(
                PmPrivateRestObservationPurpose::ClosedOnly,
                generation,
                &mut server_times,
            )
            .await?;
        let closed_only = self
            .http
            .preflight()
            .closed_only_observation(time, &mut self.private_clock)
            .await?;
        self.ensure_activity_unchanged(generation)?;

        let time = self
            .fresh_server_time(
                PmPrivateRestObservationPurpose::CollateralBalanceAllowance,
                generation,
                &mut server_times,
            )
            .await?;
        let collateral = self
            .http
            .account()
            .collateral_balance_allowance_observation(time, &mut self.private_clock)
            .await?;
        self.ensure_activity_unchanged(generation)?;

        let time = self
            .fresh_server_time(
                PmPrivateRestObservationPurpose::ConditionalBalanceAllowance,
                generation,
                &mut server_times,
            )
            .await?;
        let conditional = self
            .http
            .account()
            .conditional_balance_allowance_observation(time, &mut self.private_clock)
            .await?;
        self.ensure_activity_unchanged(generation)?;

        let open_orders = self
            .collect_open_orders(generation, &mut server_times)
            .await?;
        let trades = self.collect_trades(generation, &mut server_times).await?;

        let exact_order = match exact_order_id {
            Some(order_id) => Some(
                self.collect_exact_order(order_id, generation, &mut server_times)
                    .await?,
            ),
            None => None,
        };

        self.ensure_activity_unchanged(generation)?;
        let rest_cut_identity = Arc::new(PmRestCutIdentity { _private: () });
        let join = PmSameAuthorityRestJoin {
            start,
            rest_cut_identity: Arc::clone(&rest_cut_identity),
            open_order_rows: open_orders.row_count,
            trade_rows: trades.row_count,
        };
        let cut = PmAuthenticatedRestCutCore {
            rest_cut_identity,
            activity_generation: generation,
            server_times: server_times.into_boxed_slice(),
            closed_only,
            collateral: PmPrivateAccountBalanceAllowanceEvidence::new(collateral),
            conditional: PmPrivateAccountBalanceAllowanceEvidence::new(conditional),
            open_orders,
            trades,
            exact_order,
        };
        Ok((cut, join))
    }

    fn ensure_activity_unchanged(&self, expected: u64) -> Result<(), PmPrivateReadRuntimeError> {
        if self.marker.identity.activity.generation() == expected {
            Ok(())
        } else {
            Err(PmPrivateReadRuntimeError::UserActivityChangedRetryRequired)
        }
    }

    async fn fresh_server_time(
        &self,
        purpose: PmPrivateRestObservationPurpose,
        generation: u64,
        evidence: &mut Vec<PmPrivateReadServerTimeEvidence>,
    ) -> Result<PmReadServerTime, PmPrivateReadRuntimeError> {
        self.ensure_activity_unchanged(generation)?;
        let observation = self
            .server_time
            .fresh_read_server_time_observation()
            .await?;
        self.ensure_activity_unchanged(generation)?;
        evidence.push(PmPrivateReadServerTimeEvidence {
            purpose,
            timestamp: observation.parsed_l2_timestamp(),
            receive_clock: observation.receive_clock(),
            commitment: observation.commitment(),
        });
        Ok(observation.into_read_server_time())
    }

    async fn collect_open_orders(
        &mut self,
        generation: u64,
        server_times: &mut Vec<PmPrivateReadServerTimeEvidence>,
    ) -> Result<PmCompleteOpenOrdersSummary, PmPrivateReadRuntimeError> {
        let mut ordinal = 0usize;
        let time = self
            .fresh_server_time(
                PmPrivateRestObservationPurpose::OpenOrdersPage { ordinal },
                generation,
                server_times,
            )
            .await?;
        let mut progress = self
            .http
            .reconciliation()
            .begin_open_orders_observation(time, &mut self.private_clock)
            .await?;
        self.ensure_activity_unchanged(generation)?;
        loop {
            match progress {
                PmOpenOrdersCutProgress::Complete(cut) => {
                    let observation = self.http.reconciliation().seal_complete_open_orders(cut)?;
                    let rows = observation
                        .cut()
                        .pages()
                        .iter()
                        .flat_map(|page| page.orders())
                        .map(project_open_order)
                        .collect::<Vec<_>>()
                        .into_boxed_slice();
                    return Ok(PmCompleteOpenOrdersSummary {
                        page_count: observation.cut().pages().len(),
                        row_count: observation.cut().row_count(),
                        receive_clock: observation.receive_clock(),
                        commitment: observation.commitment(),
                        rows,
                    });
                }
                PmOpenOrdersCutProgress::Incomplete(assembly) => {
                    ordinal = ordinal
                        .checked_add(1)
                        .ok_or(PmLiveAdapterError::PaginationPageLimit)?;
                    let time = self
                        .fresh_server_time(
                            PmPrivateRestObservationPurpose::OpenOrdersPage { ordinal },
                            generation,
                            server_times,
                        )
                        .await?;
                    progress = self
                        .http
                        .reconciliation()
                        .continue_open_orders_observation(time, assembly, &mut self.private_clock)
                        .await?;
                    self.ensure_activity_unchanged(generation)?;
                }
            }
        }
    }

    async fn collect_trades(
        &mut self,
        generation: u64,
        server_times: &mut Vec<PmPrivateReadServerTimeEvidence>,
    ) -> Result<PmCompleteTradesSummary, PmPrivateReadRuntimeError> {
        let mut ordinal = 0usize;
        let time = self
            .fresh_server_time(
                PmPrivateRestObservationPurpose::TradesPage { ordinal },
                generation,
                server_times,
            )
            .await?;
        let mut progress = self
            .http
            .reconciliation()
            .begin_trades_observation(time, &mut self.private_clock)
            .await?;
        self.ensure_activity_unchanged(generation)?;
        loop {
            match progress {
                PmTradesCutProgress::Complete(cut) => {
                    let observation = self.http.reconciliation().seal_complete_trades(cut)?;
                    let rows = observation
                        .cut()
                        .pages()
                        .iter()
                        .flat_map(|page| page.trades())
                        .map(project_trade)
                        .collect::<Vec<_>>()
                        .into_boxed_slice();
                    return Ok(PmCompleteTradesSummary {
                        page_count: observation.cut().pages().len(),
                        row_count: observation.cut().row_count(),
                        receive_clock: observation.receive_clock(),
                        commitment: observation.commitment(),
                        rows,
                    });
                }
                PmTradesCutProgress::Incomplete(assembly) => {
                    ordinal = ordinal
                        .checked_add(1)
                        .ok_or(PmLiveAdapterError::PaginationPageLimit)?;
                    let time = self
                        .fresh_server_time(
                            PmPrivateRestObservationPurpose::TradesPage { ordinal },
                            generation,
                            server_times,
                        )
                        .await?;
                    progress = self
                        .http
                        .reconciliation()
                        .continue_trades_observation(time, assembly, &mut self.private_clock)
                        .await?;
                    self.ensure_activity_unchanged(generation)?;
                }
            }
        }
    }

    async fn collect_exact_order(
        &mut self,
        order_id: FixedOrderId,
        generation: u64,
        server_times: &mut Vec<PmPrivateReadServerTimeEvidence>,
    ) -> Result<PmRecoveryExactOrderSummary, PmPrivateReadRuntimeError> {
        let time = self
            .fresh_server_time(
                PmPrivateRestObservationPurpose::RecoveryExactOrder,
                generation,
                server_times,
            )
            .await?;
        let observation = self
            .http
            .reconciliation()
            .exact_local_order_detail_observation(time, order_id, &mut self.private_clock)
            .await?;
        self.ensure_activity_unchanged(generation)?;
        Ok(project_exact_order(observation))
    }
}

fn project_exact_order(observation: PmExactOrderDetailObservation) -> PmRecoveryExactOrderSummary {
    let order_id = observation.order_id();
    let receive_clock = observation.receive_clock();
    let commitment = observation.commitment();
    let classification = match observation.into_classification() {
        PmExactOrderObservation::Absent => PmRecoveryExactOrderClassification::Absent,
        PmExactOrderObservation::Present(order) => {
            PmRecoveryExactOrderClassification::Present(Box::new(PmRecoveryExactOrderProjection {
                condition: order.condition(),
                token: order.token(),
                side: order.side(),
                original_size: order.original_size(),
                size_matched: order.size_matched(),
                price: order.price(),
                status: order.status().into(),
                maker: order.maker(),
                created_at: order.created_at(),
                expiration: order.expiration(),
                outcome: order.outcome().map(Into::into),
                order_type: order.order_type().map(Into::into),
            }))
        }
    };
    PmRecoveryExactOrderSummary {
        order_id,
        classification,
        receive_clock,
        commitment,
    }
}

fn project_open_order(order: &PmLiveOrder) -> PmAuthenticatedOpenOrderRow {
    PmAuthenticatedOpenOrderRow {
        exact: PmAuthenticatedOpenOrderProjection {
            id: order.id(),
            condition: order.condition(),
            token: order.token(),
            side: order.side(),
            original_size: order.original_size(),
            size_matched: order.size_matched(),
            price: order.price(),
            maker: order.maker(),
            created_at: order.created_at(),
            expiration: order.expiration(),
        },
        text: PmAuthenticatedOpenOrderTextProjection {
            status: order.status().into(),
            outcome: order.outcome().map(Into::into),
            order_type: order.order_type().map(Into::into),
        },
    }
}

fn project_trade(trade: &PmLiveTrade) -> PmAuthenticatedTradeRow {
    PmAuthenticatedTradeRow {
        id: trade.id(),
        condition: trade.condition(),
        token: trade.token(),
        side: trade.side(),
        size: trade.size(),
        price: trade.price(),
        status: trade.status().into(),
        order_id: trade.order_id(),
        taker_order_id: trade.taker_order_id(),
        trader_side: trade.trader_side().map(Into::into),
        transaction_hash: trade.transaction_hash().map(Into::into),
        fee_rate_bps: trade.fee_rate_bps(),
        maker_orders: trade
            .maker_orders()
            .iter()
            .map(|order| PmAuthenticatedMakerLegProjection {
                order_id: order.order_id(),
                token: order.token(),
                side: order.side(),
                price: order.price(),
                matched_amount: order.matched_amount(),
                fee_rate_bps: order.fee_rate_bps(),
                maker: order.maker(),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        maker: trade.maker(),
        timestamp: trade.timestamp(),
        match_time: trade.match_time(),
        last_update: trade.last_update(),
    }
}

/// Fresh mode retains the consume-once place handle. Its REST surface has no
/// expected-order method because no venue order exists yet.
pub(super) struct PmFreshAuthenticatedRestRuntime {
    core: PmAuthenticatedRestRuntimeCore,
}

impl PmFreshAuthenticatedRestRuntime {
    pub(super) async fn collect(
        &mut self,
        start: PmUserRestCollectionStart,
    ) -> Result<(PmFreshAuthenticatedRestCut, PmSameAuthorityRestJoin), PmPrivateReadRuntimeError>
    {
        let (core, join) = self.core.collect(start, None).await?;
        Ok((PmFreshAuthenticatedRestCut { core }, join))
    }
}

/// Opaque expected-order candidate for recovery detail collection.
///
/// Production runner siblings cannot construct this placeholder. A later A3
/// integration must replace its private provenance marker with a journal-
/// issued exact-owned carrier before enabling the recovery detail path.
pub(super) struct PmRecoveryExpectedOrderCandidate {
    order_id: FixedOrderId,
    _a3_provenance_pending: (),
}

impl PmRecoveryExpectedOrderCandidate {
    const fn into_order_id(self) -> FixedOrderId {
        self.order_id
    }

    #[cfg(test)]
    const fn test_support_unproven(order_id: FixedOrderId) -> Self {
        Self {
            order_id,
            _a3_provenance_pending: (),
        }
    }
}

impl fmt::Debug for PmRecoveryExpectedOrderCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmRecoveryExpectedOrderCandidate([REDACTED; A3 PENDING])")
    }
}

/// Recovery mode has L2 read/cancel authority only. Candidate exact detail is
/// exposed solely here and remains bounded to one opaque, consume-once input.
pub(super) struct PmRecoveryAuthenticatedRestRuntime {
    core: PmAuthenticatedRestRuntimeCore,
}

impl PmRecoveryAuthenticatedRestRuntime {
    pub(super) async fn collect_account_cut(
        &mut self,
        start: PmUserRestCollectionStart,
    ) -> Result<
        (
            PmRecoveryAccountAuthenticatedRestCut,
            PmSameAuthorityRestJoin,
        ),
        PmPrivateReadRuntimeError,
    > {
        let (core, join) = self.core.collect(start, None).await?;
        Ok((PmRecoveryAccountAuthenticatedRestCut { core }, join))
    }

    pub(super) async fn collect_with_expected_order_candidate(
        &mut self,
        start: PmUserRestCollectionStart,
        candidate_expected_order_id: PmRecoveryExpectedOrderCandidate,
    ) -> Result<
        (PmRecoveryExactAuthenticatedRestCut, PmSameAuthorityRestJoin),
        PmPrivateReadRuntimeError,
    > {
        let (core, join) = self
            .core
            .collect(start, Some(candidate_expected_order_id.into_order_id()))
            .await?;
        Ok((PmRecoveryExactAuthenticatedRestCut { core }, join))
    }
}

// BEGIN STAGED_OBSERVATION_PRIVATE_READ_ROLES
/// Inseparable selected observation roles and the exact fresh credential
/// custody behind them.
///
/// All fields are private and this tranche exposes no source or transport
/// projection. Construction installs selected fixed-peer HTTP, immediately
/// pairs the public and same-credential user WebSockets, and retains both
/// forks of the same-authority marker. The only production operation is
/// consuming bounded shutdown.
pub(super) struct PmFreshStagedSelectedObservationRoles {
    rest: PmFreshAuthenticatedRestRuntime,
    public_ws: PmProductionSelectedPublicWsRole,
    user_ws: PmProductionSelectedUserWsRole,
    user_ws_activity: PmUserWsActivityView,
    same_authority: PmSameCredentialAuthorityMarker,
    custody: PmObservingFreshCredentialCustody,
    _configured_scope: PmWireScope,
}

impl PmFreshStagedSelectedObservationRoles {
    /// Start cold credential custody on the supplied actor LocalSet, construct
    /// fixed-peer authenticated HTTP, and immediately pair both production
    /// WebSockets to the one supplied selected local route.
    ///
    /// Every error after the credential task is armed explicitly joins and
    /// tears down custody before returning. This function makes no HTTP or
    /// WebSocket source call. The unforgeable assembly token arrives only from
    /// the whole deferred-observation owner, whose selected constructor has
    /// already rejected a non-production public WebSocket configuration; the
    /// exact full scope is nevertheless rechecked here before arming secrets.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn production_selected_internal(
        _assembly: PmDeferredObservationAssemblyToken,
        credential_owner: FreshCredentialAuthorityOwner,
        local_set: &tokio::task::LocalSet,
        profile: PmPrivateReadRuntimeProfile,
        clocks: PmPrivateReadClockBundle,
        public_ws: PmPublicMarketWsRole,
        fixed_clob_http_peer: PmFixedTlsPeerSelection,
        fixed_clob_ws_peer: PmFixedTlsPeerSelection,
        selected_local_egress: PmLocalEgressSelection,
        shutdown_bounds: CredentialAuthorityShutdownBounds,
    ) -> Result<Self, PmPrivateReadRuntimeError> {
        if public_ws.scope() != profile.scope {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "selected public and private observation scopes diverged",
            )
            .into());
        }
        let authority = match credential_owner.spawn_staged_observation(local_set) {
            Ok(authority) => authority,
            Err(_) => return Err(PmPrivateReadRuntimeError::StagedObservationStartFailed),
        };
        let (http_authority, user_authority, loaded_signer, custody) =
            authority.into_private_read_runtime_parts();
        if loaded_signer != profile.signer {
            drop(http_authority);
            drop(user_authority);
            drop(public_ws);
            drop(clocks);
            return Err(cleanup_staged_observation_after_assembly_failure(
                custody,
                shutdown_bounds,
                PmPrivateReadRuntimeError::StagedObservationSignerBindingMismatch,
            )
            .await);
        }

        let configured_scope = profile.scope;
        let selected_http_local = selected_local_egress.clone();
        let (core, user_ws) =
            match production_runtime_core_on_fixed_tls_peer_and_selected_local_egress(
                profile,
                clocks,
                fixed_clob_http_peer,
                selected_http_local,
                http_authority,
                user_authority,
            ) {
                Ok(roles) => roles,
                Err(build) => {
                    drop(public_ws);
                    return Err(cleanup_staged_observation_after_assembly_failure(
                        custody,
                        shutdown_bounds,
                        build,
                    )
                    .await);
                }
            };
        let rest = PmFreshAuthenticatedRestRuntime { core };
        let (user_ws, user_ws_activity, same_authority) = user_ws.into_parts();
        let selected_ws = match PmProductionSelectedWsOwner::new(
            public_ws,
            user_ws,
            fixed_clob_ws_peer,
            selected_local_egress,
        ) {
            Ok(selected_ws) => selected_ws,
            Err(build) => {
                drop(rest);
                drop(user_ws_activity);
                drop(same_authority);
                return Err(cleanup_staged_observation_after_assembly_failure(
                    custody,
                    shutdown_bounds,
                    build.into(),
                )
                .await);
            }
        };
        let (public_ws, user_ws) = selected_ws.into_roles();
        Ok(Self {
            rest,
            public_ws,
            user_ws,
            user_ws_activity,
            same_authority,
            custody,
            _configured_scope: configured_scope,
        })
    }

    /// Destroy all role/provider senders before asking the credential task to
    /// stop. The returned outcome retains whether bounded shutdown required an
    /// abort; a safe abnormal join is evidence, not silently rewritten as a
    /// clean completion.
    pub(super) async fn shutdown_bounded(
        self,
        bounds: CredentialAuthorityShutdownBounds,
    ) -> Result<CredentialAuthorityShutdownOutcome, PmObservingFreshCredentialShutdownError> {
        let Self {
            rest,
            public_ws,
            user_ws,
            user_ws_activity,
            same_authority,
            custody,
            _configured_scope: _,
        } = self;
        drop(rest);
        drop(public_ws);
        drop(user_ws);
        drop(user_ws_activity);
        drop(same_authority);
        custody.shutdown_bounded(bounds).await
    }
}

impl fmt::Debug for PmFreshStagedSelectedObservationRoles {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmFreshStagedSelectedObservationRoles([REDACTED; SELECTED; ARMED])")
    }
}

/// Test-only legacy observation projection. Production selected composition
/// cannot obtain this armed intermediate or its raw user role.
#[cfg(test)]
struct PmFreshStagedObservationPrivateReadRuntimeRoles {
    rest: PmFreshAuthenticatedRestRuntime,
    user_ws: PmSameCredentialUserWsInput,
    custody: PmObservingFreshCredentialCustody,
}

#[cfg(test)]
impl PmFreshStagedObservationPrivateReadRuntimeRoles {
    async fn production(
        authority: FreshStagedObservationAuthorityRoles,
        profile: PmPrivateReadRuntimeProfile,
        clocks: PmPrivateReadClockBundle,
        shutdown_bounds: CredentialAuthorityShutdownBounds,
    ) -> Result<Self, PmPrivateReadRuntimeError> {
        let (http_authority, user_authority, loaded_signer, custody) =
            authority.into_private_read_runtime_parts();
        if loaded_signer != profile.signer {
            drop((http_authority, user_authority));
            return Err(cleanup_staged_observation_after_assembly_failure(
                custody,
                shutdown_bounds,
                PmPrivateReadRuntimeError::StagedObservationSignerBindingMismatch,
            )
            .await);
        }
        match production_runtime_core(profile, clocks, http_authority, user_authority) {
            Ok((core, user_ws)) => Ok(Self {
                rest: PmFreshAuthenticatedRestRuntime { core },
                user_ws,
                custody,
            }),
            Err(build) => Err(cleanup_staged_observation_after_assembly_failure(
                custody,
                shutdown_bounds,
                build,
            )
            .await),
        }
    }

    fn into_parts(
        self,
    ) -> (
        PmFreshAuthenticatedRestRuntime,
        PmSameCredentialUserWsInput,
        PmObservingFreshCredentialCustody,
    ) {
        (self.rest, self.user_ws, self.custody)
    }
}

async fn cleanup_staged_observation_after_assembly_failure(
    custody: PmObservingFreshCredentialCustody,
    shutdown_bounds: CredentialAuthorityShutdownBounds,
    build: PmPrivateReadRuntimeError,
) -> PmPrivateReadRuntimeError {
    match custody.shutdown_bounded(shutdown_bounds).await {
        Ok(outcome)
            if outcome.shutdown_requested()
                && !outcome.abort_requested()
                && outcome.task_joined()
                && outcome.task_completed_cleanly()
                && outcome.credentials_dropped()
                && outcome.staged_l2_files_removed() =>
        {
            build
        }
        Ok(outcome) => PmPrivateReadRuntimeError::StagedObservationCleanupAbnormal {
            build: Box::new(build),
            abort_requested: outcome.abort_requested(),
            task_completed_cleanly: outcome.task_completed_cleanly(),
        },
        Err(cleanup) => PmPrivateReadRuntimeError::StagedObservationCleanupFailed {
            build: Box::new(build),
            cleanup,
        },
    }
}
// END STAGED_OBSERVATION_PRIVATE_READ_ROLES

/// Mode-specific result after the fresh authority bundle has already been
/// consumed and its read roles installed in fixed connectivity.
pub(super) struct PmFreshPrivateReadRuntimeRoles {
    place: FreshPlaceAuthenticationOnce,
    cancel: ExactOwnedCancelAuthenticationRole,
    rest: PmFreshAuthenticatedRestRuntime,
    user_ws: PmSameCredentialUserWsInput,
    supervisor: FreshCredentialAuthoritySupervisor,
}

impl PmFreshPrivateReadRuntimeRoles {
    pub(super) fn production(
        authority: FreshCredentialAuthorityRoles,
        profile: PmPrivateReadRuntimeProfile,
        clocks: PmPrivateReadClockBundle,
    ) -> Result<Self, PmPrivateReadRuntimeError> {
        let (place, cancel, http_authority, user_authority, supervisor) =
            authority.into_private_read_runtime_parts();
        let (core, user_ws) =
            production_runtime_core(profile, clocks, http_authority, user_authority)?;
        Ok(Self {
            place,
            cancel,
            rest: PmFreshAuthenticatedRestRuntime { core },
            user_ws,
            supervisor,
        })
    }

    pub(super) fn into_parts(
        self,
    ) -> (
        FreshPlaceAuthenticationOnce,
        ExactOwnedCancelAuthenticationRole,
        PmFreshAuthenticatedRestRuntime,
        PmSameCredentialUserWsInput,
        FreshCredentialAuthoritySupervisor,
    ) {
        (
            self.place,
            self.cancel,
            self.rest,
            self.user_ws,
            self.supervisor,
        )
    }
}

/// Recovery output intentionally has no signer, place handle, or place
/// method. Both HTTP and WS nevertheless remain joined to its sole L2 task.
pub(super) struct PmRecoveryPrivateReadRuntimeRoles {
    cancel: ExactOwnedCancelAuthenticationRole,
    rest: PmRecoveryAuthenticatedRestRuntime,
    user_ws: PmSameCredentialUserWsInput,
    supervisor: RecoveryCredentialAuthoritySupervisor,
}

impl PmRecoveryPrivateReadRuntimeRoles {
    pub(super) fn production(
        authority: RecoveryCredentialAuthorityRoles,
        profile: PmPrivateReadRuntimeProfile,
        clocks: PmPrivateReadClockBundle,
    ) -> Result<Self, PmPrivateReadRuntimeError> {
        let (cancel, http_authority, user_authority, supervisor) =
            authority.into_private_read_runtime_parts();
        let (core, user_ws) =
            production_runtime_core(profile, clocks, http_authority, user_authority)?;
        Ok(Self {
            cancel,
            rest: PmRecoveryAuthenticatedRestRuntime { core },
            user_ws,
            supervisor,
        })
    }

    pub(super) fn into_parts(
        self,
    ) -> (
        ExactOwnedCancelAuthenticationRole,
        PmRecoveryAuthenticatedRestRuntime,
        PmSameCredentialUserWsInput,
        RecoveryCredentialAuthoritySupervisor,
    ) {
        (self.cancel, self.rest, self.user_ws, self.supervisor)
    }
}

fn production_runtime_core(
    profile: PmPrivateReadRuntimeProfile,
    clocks: PmPrivateReadClockBundle,
    http_authority: FixedHttpAuthenticationRole,
    user_authority: FixedUserWsAuthenticationRole,
) -> Result<(PmAuthenticatedRestRuntimeCore, PmSameCredentialUserWsInput), PmPrivateReadRuntimeError>
{
    let owner = PmExternalProxyReadConnectivityOwner::production(
        profile.connect_timeout,
        profile.request_timeout,
        profile.scope,
        profile.user_ws_bounds,
        profile.signer,
        profile.proxy_funder,
        http_authority,
        user_authority,
    )?;
    finish_runtime_core(
        owner,
        clocks,
        profile.scope,
        profile.signer,
        profile.proxy_funder,
    )
}

fn production_runtime_core_on_fixed_tls_peer_and_selected_local_egress(
    profile: PmPrivateReadRuntimeProfile,
    clocks: PmPrivateReadClockBundle,
    fixed_tls_peer: PmFixedTlsPeerSelection,
    selected_local_egress: PmLocalEgressSelection,
    http_authority: FixedHttpAuthenticationRole,
    user_authority: FixedUserWsAuthenticationRole,
) -> Result<(PmAuthenticatedRestRuntimeCore, PmSameCredentialUserWsInput), PmPrivateReadRuntimeError>
{
    let owner = PmExternalProxyReadConnectivityOwner::
        production_on_fixed_tls_peer_and_selected_local_egress(
            profile.connect_timeout,
            profile.request_timeout,
            profile.scope,
            profile.user_ws_bounds,
            fixed_tls_peer,
            selected_local_egress,
            profile.signer,
            profile.proxy_funder,
            http_authority,
            user_authority,
        )?;
    finish_runtime_core(
        owner,
        clocks,
        profile.scope,
        profile.signer,
        profile.proxy_funder,
    )
}

fn finish_runtime_core(
    owner: PmExternalProxyReadConnectivityOwner,
    clocks: PmPrivateReadClockBundle,
    scope: PmWireScope,
    signer: EoaAddress,
    proxy_funder: EvmAddress,
) -> Result<(PmAuthenticatedRestRuntimeCore, PmSameCredentialUserWsInput), PmPrivateReadRuntimeError>
{
    let (server_time, private_clock, user_ws_clock) = clocks.into_parts();
    let (http, user_ws) = owner.into_read_roles();
    let user_ws = user_ws.with_clock_source(user_ws_clock);
    if user_ws.condition() != scope.condition() {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "fixed private read and user stream scopes diverged",
        )
        .into());
    }
    let activity = user_ws.activity_view();
    let marker =
        PmSameCredentialAuthorityMarker::new(scope, signer, proxy_funder, activity.clone());
    let user_marker = marker.fork_for_same_authority();
    Ok((
        PmAuthenticatedRestRuntimeCore {
            http,
            server_time,
            private_clock,
            marker,
        },
        PmSameCredentialUserWsInput {
            role: user_ws,
            marker: user_marker,
        },
    ))
}

#[cfg(test)]
fn loopback_runtime_core(
    private_config: PmPrivateHttpConfig,
    user_config: PmUserWsConfig,
    signer: EoaAddress,
    proxy_funder: EvmAddress,
    clocks: PmPrivateReadClockBundle,
    http_authority: FixedHttpAuthenticationRole,
    user_authority: FixedUserWsAuthenticationRole,
) -> Result<(PmAuthenticatedRestRuntimeCore, PmSameCredentialUserWsInput), PmPrivateReadRuntimeError>
{
    let scope = private_config.exact_order_scope();
    let owner = PmExternalProxyReadConnectivityOwner::loopback_evidence(
        private_config,
        user_config,
        signer,
        proxy_funder,
        http_authority,
        user_authority,
    )?;
    finish_runtime_core(owner, clocks, scope, signer, proxy_funder)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt as _,
        path::Path,
        sync::{Arc, Mutex},
    };

    use reap_pm_core::{PmMarketId, PmTick, U256};
    use reap_polymarket_live_adapter::{
        PmProductClockOwner, PmPublicConnectivityOwner, PmPublicHttpConfig,
        PmPublicObservationConnectivityOwner, PmPublicWsBounds, PmPublicWsConfig,
    };
    use reap_polymarket_wire::PmBookParserConfig;
    use tempfile::TempDir;
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::TcpListener,
        sync::oneshot,
        task::JoinHandle,
    };

    use super::*;
    use crate::controlled_trial::authority::{
        CredentialAuthorityShutdownBounds, FreshCredentialAuthorityOwner,
        FreshStagedObservationAuthorityRoles, RecoveryCredentialAuthorityOwner,
    };

    const KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const SIGNER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    const FOREIGN_SIGNER: &str = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC";
    const PROXY: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    const API_KEY: &str = "00000000-0000-4000-8000-000000000001";
    const FOREIGN_API_KEY: &str = "00000000-0000-4000-8000-000000000002";
    const L2_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    const PASSPHRASE: &str = "synthetic-passphrase";
    const CONDITION: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
    const MARKET: &str = "0x9999999999999999999999999999999999999999999999999999999999999999";
    const ORDER_ID: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const MAKER_ORDER_ID: &str =
        "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const SPENDER: &str = "0x3333333333333333333333333333333333333333";
    const AUTH_SECONDS: u64 = 1_780_449_126;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum OrdersPlan {
        Empty,
        OneRow,
        ForeignOwner,
        CursorCycle,
        IncompleteThenError,
        PageCap,
    }

    struct MockPlan {
        orders: OrdersPlan,
        nonempty_trades: bool,
        exact_present: bool,
        time_count: u64,
        order_page_count: usize,
    }

    impl MockPlan {
        const fn new(orders: OrdersPlan) -> Self {
            Self {
                orders,
                nonempty_trades: false,
                exact_present: false,
                time_count: 0,
                order_page_count: 0,
            }
        }

        fn response(&mut self, target: &str) -> MockResponse {
            if target == "/time" {
                self.time_count += 1;
                return MockResponse::ok((AUTH_SECONDS + self.time_count).to_string());
            }
            if target == "/auth/ban-status/closed-only" {
                return MockResponse::ok(r#"{"closed_only":false}"#);
            }
            if target.starts_with("/balance-allowance?") {
                return MockResponse::ok(format!(
                    r#"{{"balance":"123456789012345678901234567890","allowances":{{"{SPENDER}":"987654321098765432109876543210"}}}}"#
                ));
            }
            if target.starts_with("/data/orders?") {
                self.order_page_count += 1;
                return match self.orders {
                    OrdersPlan::Empty => MockResponse::ok(empty_page()),
                    OrdersPlan::OneRow => MockResponse::ok(order_page(API_KEY)),
                    OrdersPlan::ForeignOwner => MockResponse::ok(order_page(FOREIGN_API_KEY)),
                    OrdersPlan::CursorCycle => MockResponse::ok(page_with_cursor("MA==")),
                    OrdersPlan::IncompleteThenError if self.order_page_count == 1 => {
                        MockResponse::ok(page_with_cursor("cursor-1"))
                    }
                    OrdersPlan::IncompleteThenError => MockResponse::status(503),
                    OrdersPlan::PageCap => MockResponse::ok(page_with_cursor(&format!(
                        "cursor-{}",
                        self.order_page_count
                    ))),
                };
            }
            if target.starts_with("/data/trades?") {
                return if self.nonempty_trades {
                    MockResponse::ok(trade_page(API_KEY))
                } else {
                    MockResponse::ok(empty_page())
                };
            }
            if target.starts_with("/data/order/") {
                return if self.exact_present {
                    MockResponse::ok(order_json(API_KEY))
                } else {
                    MockResponse::status(404)
                };
            }
            MockResponse::status(500)
        }
    }

    struct MockResponse {
        status: u16,
        body: String,
    }

    impl MockResponse {
        fn ok(body: impl Into<String>) -> Self {
            Self {
                status: 200,
                body: body.into(),
            }
        }

        fn status(status: u16) -> Self {
            Self {
                status,
                body: "{}".into(),
            }
        }
    }

    struct LoopbackHttpServer {
        origin: String,
        targets: Arc<Mutex<Vec<String>>>,
        shutdown: oneshot::Sender<()>,
        task: JoinHandle<()>,
    }

    impl LoopbackHttpServer {
        async fn start(plan: MockPlan) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let targets = Arc::new(Mutex::new(Vec::new()));
            let task_targets = Arc::clone(&targets);
            let plan = Arc::new(Mutex::new(plan));
            let (shutdown, mut shutdown_receiver) = oneshot::channel();
            let task = tokio::spawn(async move {
                loop {
                    let accepted = tokio::select! {
                        _ = &mut shutdown_receiver => break,
                        accepted = listener.accept() => accepted,
                    };
                    let (mut stream, _) = accepted.unwrap();
                    let mut raw = Vec::new();
                    let mut chunk = [0_u8; 1024];
                    loop {
                        let read = stream.read(&mut chunk).await.unwrap();
                        if read == 0 {
                            break;
                        }
                        raw.extend_from_slice(&chunk[..read]);
                        if raw.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let request = String::from_utf8(raw).unwrap();
                    let target = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_ascii_whitespace().nth(1))
                        .unwrap()
                        .to_owned();
                    task_targets.lock().unwrap().push(target.clone());
                    let response = plan.lock().unwrap().response(&target);
                    let reason = match response.status {
                        200 => "OK",
                        404 => "Not Found",
                        500 => "Internal Server Error",
                        503 => "Service Unavailable",
                        _ => "Mock",
                    };
                    let headers = format!(
                        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        response.status,
                        reason,
                        response.body.len(),
                    );
                    if stream.write_all(headers.as_bytes()).await.is_ok() {
                        let _ = stream.write_all(response.body.as_bytes()).await;
                    }
                }
            });
            Self {
                origin: format!("http://{address}"),
                targets,
                shutdown,
                task,
            }
        }

        fn targets(&self) -> Vec<String> {
            self.targets.lock().unwrap().clone()
        }

        async fn finish(self) {
            let _ = self.shutdown.send(());
            self.task.await.unwrap();
        }
    }

    struct RecoveryTestRuntime {
        core: PmAuthenticatedRestRuntimeCore,
        input: Option<PmSameCredentialUserWsInput>,
        cancel: ExactOwnedCancelAuthenticationRole,
        supervisor: RecoveryCredentialAuthoritySupervisor,
        directory: TempDir,
        server: LoopbackHttpServer,
    }

    impl RecoveryTestRuntime {
        fn take_user_parts(
            &mut self,
        ) -> (
            PmAuthenticatedUserWsRole,
            PmUserWsActivityView,
            PmSameCredentialAuthorityMarker,
        ) {
            self.input.take().unwrap().into_parts()
        }

        async fn finish(self) {
            let Self {
                core,
                input,
                cancel,
                supervisor,
                directory,
                server,
            } = self;
            drop(core);
            drop(input);
            drop(cancel);
            server.finish().await;
            let outcome = supervisor
                .shutdown_bounded(
                    CredentialAuthorityShutdownBounds::new(
                        Duration::from_secs(2),
                        Duration::from_secs(2),
                    )
                    .unwrap(),
                )
                .await
                .unwrap();
            assert!(outcome.credentials_dropped());
            assert!(outcome.staged_l2_files_removed());
            for entry in ["api-key", "l2-secret", "passphrase"] {
                assert!(!directory.path().join(entry).exists());
            }
        }
    }

    fn scope() -> PmWireScope {
        PmWireScope::new(
            PmConditionId::parse(CONDITION).unwrap(),
            PmMarketId::parse(MARKET).unwrap(),
            PmTokenId::new(U256::from_u64(1_234)).unwrap(),
        )
    }

    fn signer() -> EoaAddress {
        EoaAddress::parse(SIGNER).unwrap()
    }

    fn proxy() -> EvmAddress {
        EvmAddress::parse(PROXY).unwrap()
    }

    fn write_secret(directory: &Path, name: &str, value: &str) {
        let path = directory.join(name);
        fs::write(&path, value).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn recovery_authority() -> (TempDir, RecoveryCredentialAuthorityRoles) {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        write_secret(directory.path(), "api-key", API_KEY);
        write_secret(directory.path(), "l2-secret", L2_SECRET);
        write_secret(directory.path(), "passphrase", PASSPHRASE);
        let roles = RecoveryCredentialAuthorityOwner::load_from_protected_files(
            directory.path().to_owned(),
            "api-key".into(),
            "l2-secret".into(),
            "passphrase".into(),
            signer(),
        )
        .unwrap()
        .spawn()
        .unwrap();
        (directory, roles)
    }

    fn fresh_staged_authority(
        local_set: &tokio::task::LocalSet,
    ) -> (TempDir, FreshStagedObservationAuthorityRoles) {
        let (directory, owner) = fresh_credential_owner();
        let roles = owner.spawn_staged_observation(local_set).unwrap();
        (directory, roles)
    }

    fn fresh_credential_owner() -> (TempDir, FreshCredentialAuthorityOwner) {
        fn assert_send<T: Send>() {}
        assert_send::<FreshCredentialAuthorityOwner>();

        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        write_secret(directory.path(), "private-key", KEY);
        write_secret(directory.path(), "api-key", API_KEY);
        write_secret(directory.path(), "l2-secret", L2_SECRET);
        write_secret(directory.path(), "passphrase", PASSPHRASE);
        let owner = FreshCredentialAuthorityOwner::load_from_protected_files(
            directory.path().to_owned(),
            "private-key".into(),
            "api-key".into(),
            "l2-secret".into(),
            "passphrase".into(),
            signer(),
        )
        .unwrap();
        (directory, owner)
    }

    fn user_ws_bounds() -> PmUserWsBounds {
        PmUserWsBounds::new(
            Duration::from_secs(1),
            Duration::from_secs(20),
            Duration::from_secs(2),
            64 * 1_024,
            1,
            Duration::from_millis(1),
            8,
            ConnectionEpoch::new(1),
        )
        .unwrap()
    }

    fn clock_bundle(origin: &str) -> PmPrivateReadClockBundle {
        let http = PmPublicHttpConfig::loopback_evidence(
            origin,
            Duration::from_secs(1),
            Duration::from_secs(2),
        )
        .unwrap();
        let parser = PmBookParserConfig::new_condition_bound(
            scope(),
            PmTick::parse_decimal("0.01").unwrap(),
            PmQuantity::parse_decimal("5").unwrap(),
            false,
        );
        let public_ws = PmPublicWsConfig::loopback_evidence(
            "ws://127.0.0.1:9/ws/market",
            scope(),
            Duration::from_secs(1),
            Duration::from_secs(20),
            Duration::from_secs(10),
            Duration::from_secs(2),
            64 * 1_024,
            1,
            Duration::from_millis(1),
            8,
            ConnectionEpoch::new(1),
        )
        .unwrap();
        let (
            _metadata,
            _book,
            server_time,
            private_read,
            _place_time,
            _cancel_time,
            _public_ws,
            user_ws,
            _actor,
            _okx,
        ) = PmPublicConnectivityOwner::new(http, parser, public_ws, PmProductClockOwner::system())
            .unwrap()
            .into_roles()
            .into_roles();
        PmPrivateReadClockBundle::test_support_from_roles(server_time, private_read, user_ws)
    }

    fn selected_public_and_clocks() -> (PmPublicMarketWsRole, PmPrivateReadClockBundle) {
        let http_peer =
            PmFixedTlsPeerSelection::production("clob.polymarket.com", "8.8.8.8").unwrap();
        let local =
            PmLocalEgressSelection::production("pm0", "192.0.2.10".parse().unwrap()).unwrap();
        let public_bounds = PmPublicWsBounds::new(
            Duration::from_secs(1),
            Duration::from_secs(20),
            Duration::from_secs(2),
            64 * 1_024,
            0,
            Duration::from_millis(1),
            8,
            ConnectionEpoch::new(1),
        )
        .unwrap();
        let parser = PmBookParserConfig::new_condition_bound(
            scope(),
            PmTick::parse_decimal("0.01").unwrap(),
            PmQuantity::parse_decimal("5").unwrap(),
            false,
        );
        let roles = PmPublicObservationConnectivityOwner::
            production_on_fixed_tls_peer_and_selected_local_egress(
                Duration::from_secs(1),
                Duration::from_secs(2),
                parser,
                PmPublicWsConfig::production(scope(), public_bounds).unwrap(),
                http_peer,
                local,
                PmProductClockOwner::system(),
            )
            .unwrap()
            .into_roles();
        let (metadata, book, server_time, private_read, public_ws, user_ws, actor, okx) =
            roles.into_roles();
        drop((metadata, book, actor, okx));
        (
            public_ws,
            PmPrivateReadClockBundle::test_support_from_roles(server_time, private_read, user_ws),
        )
    }

    fn selected_http_peer() -> PmFixedTlsPeerSelection {
        PmFixedTlsPeerSelection::production("clob.polymarket.com", "8.8.8.8").unwrap()
    }

    fn selected_ws_peer() -> PmFixedTlsPeerSelection {
        PmFixedTlsPeerSelection::production("ws-subscriptions-clob.polymarket.com", "8.8.8.8")
            .unwrap()
    }

    fn selected_local() -> PmLocalEgressSelection {
        PmLocalEgressSelection::production("pm0", "192.0.2.10".parse().unwrap()).unwrap()
    }

    async fn recovery_runtime(mut plan: MockPlan) -> RecoveryTestRuntime {
        // A nonempty trade belongs to the same plan as a nonempty order unless
        // a test explicitly changes the field before this call.
        if plan.orders == OrdersPlan::OneRow {
            plan.nonempty_trades = true;
        }
        let server = LoopbackHttpServer::start(plan).await;
        let (directory, authority) = recovery_authority();
        let (cancel, http_authority, user_authority, supervisor) =
            authority.into_private_read_runtime_parts();
        let private_config = PmPrivateHttpConfig::loopback_evidence(
            &server.origin,
            Duration::from_secs(1),
            Duration::from_secs(2),
            scope(),
        )
        .unwrap();
        let user_config = PmUserWsConfig::loopback_evidence(
            "ws://127.0.0.1:9/ws/user",
            scope().condition(),
            Duration::from_secs(1),
            Duration::from_secs(20),
            Duration::from_secs(10),
            Duration::from_secs(2),
            64 * 1_024,
            1,
            Duration::from_millis(1),
            8,
            ConnectionEpoch::new(1),
        )
        .unwrap();
        let (core, input) = loopback_runtime_core(
            private_config,
            user_config,
            signer(),
            proxy(),
            clock_bundle(&server.origin),
            http_authority,
            user_authority,
        )
        .unwrap();
        RecoveryTestRuntime {
            core,
            input: Some(input),
            cancel,
            supervisor,
            directory,
            server,
        }
    }

    fn empty_page() -> String {
        r#"{"data":[],"next_cursor":"LTE=","limit":128,"count":0}"#.into()
    }

    fn page_with_cursor(cursor: &str) -> String {
        format!(r#"{{"data":[],"next_cursor":"{cursor}","limit":128,"count":0}}"#)
    }

    fn order_json(owner: &str) -> String {
        format!(
            r#"{{"id":"{ORDER_ID}","market":"{CONDITION}","asset_id":"1234","side":"BUY","original_size":"10.000000","size_matched":"2.500000","price":"0.400000","status":"LIVE","maker_address":"{PROXY}","owner":"{owner}","expiration":"0","created_at":1780449126,"outcome":"YES","order_type":"GTC"}}"#
        )
    }

    fn order_page(owner: &str) -> String {
        format!(
            r#"{{"data":[{}],"next_cursor":"LTE=","limit":128,"count":1}}"#,
            order_json(owner)
        )
    }

    fn trade_json(owner: &str) -> String {
        format!(
            r#"{{"id":"trade-1","market":"{CONDITION}","asset_id":"1234","side":"SELL","size":"2.500000","price":"0.400000","status":"CONFIRMED","match_time":"1780449126002","last_update":"1780449126003","timestamp":"1780449126001","order_id":"{ORDER_ID}","taker_order_id":"{ORDER_ID}","trader_side":"TAKER","transaction_hash":"0xfeed","fee_rate_bps":"30","maker_orders":[{{"order_id":"{MAKER_ORDER_ID}","asset_id":"1234","side":"BUY","price":"0.400000","matched_amount":"2.500000","fee_rate_bps":"20","owner":"{owner}","maker_address":"{PROXY}"}}],"maker_address":"{PROXY}","owner":"{owner}"}}"#
        )
    }

    fn trade_page(owner: &str) -> String {
        format!(
            r#"{{"data":[{}],"next_cursor":"LTE=","limit":128,"count":1}}"#,
            trade_json(owner)
        )
    }

    fn collection_start(marker: &PmSameCredentialAuthorityMarker) -> PmUserRestCollectionStart {
        marker.begin_rest_collection(7, ConnectionEpoch::new(3))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn staged_production_builds_real_same_credential_rest_and_user_roles() {
        let local_set = tokio::task::LocalSet::new();
        local_set
            .run_until(async {
                let (directory, authority) = fresh_staged_authority(&local_set);
                let profile = PmPrivateReadRuntimeProfile::new(
                    Duration::from_secs(1),
                    Duration::from_secs(2),
                    scope(),
                    user_ws_bounds(),
                    signer(),
                    proxy(),
                );
                let bounds = CredentialAuthorityShutdownBounds::new(
                    Duration::from_secs(2),
                    Duration::from_secs(2),
                )
                .unwrap();

                let runtime = PmFreshStagedObservationPrivateReadRuntimeRoles::production(
                    authority,
                    profile,
                    clock_bundle("http://127.0.0.1:9"),
                    bounds,
                )
                .await
                .unwrap();
                let (rest, user_ws, custody) = runtime.into_parts();

                assert!(rest.core.marker.same_instance(&user_ws.marker));
                assert!(
                    ["private-key", "api-key", "l2-secret", "passphrase"]
                        .iter()
                        .all(|entry| directory.path().join(entry).exists())
                );
                drop((rest, user_ws));
                let outcome = custody.shutdown_bounded(bounds).await.unwrap();
                assert!(outcome.task_joined());
                assert!(outcome.credentials_dropped());
                assert!(outcome.staged_l2_files_removed());
                for entry in ["private-key", "api-key", "l2-secret", "passphrase"] {
                    assert!(!directory.path().join(entry).exists());
                }
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn selected_staged_observation_pairs_both_ws_roles_and_cleans_immediate_shutdown() {
        let local_set = tokio::task::LocalSet::new();
        local_set
            .run_until(async {
                let (directory, credential_owner) = fresh_credential_owner();
                let (public_ws, clocks) = selected_public_and_clocks();
                let bounds = CredentialAuthorityShutdownBounds::new(
                    Duration::from_secs(2),
                    Duration::from_secs(2),
                )
                .unwrap();
                let profile = PmPrivateReadRuntimeProfile::new(
                    Duration::from_secs(1),
                    Duration::from_secs(2),
                    scope(),
                    user_ws_bounds(),
                    signer(),
                    proxy(),
                );

                let selected = PmFreshStagedSelectedObservationRoles::production_selected_internal(
                    PmDeferredObservationAssemblyToken::test_support(),
                    credential_owner,
                    &local_set,
                    profile,
                    clocks,
                    public_ws,
                    selected_http_peer(),
                    selected_ws_peer(),
                    selected_local(),
                    bounds,
                )
                .await
                .unwrap();
                assert!(
                    selected
                        .rest
                        .core
                        .marker
                        .same_instance(&selected.same_authority)
                );
                let outcome = selected.shutdown_bounded(bounds).await.unwrap();
                assert!(outcome.shutdown_requested());
                assert!(!outcome.abort_requested());
                assert!(outcome.task_joined());
                assert!(outcome.task_completed_cleanly());
                assert!(outcome.credentials_dropped());
                assert!(outcome.staged_l2_files_removed());
                for entry in ["private-key", "api-key", "l2-secret", "passphrase"] {
                    assert!(!directory.path().join(entry).exists());
                }
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn selected_scope_mismatch_is_prearm_and_leaves_protected_files_for_retry() {
        let local_set = tokio::task::LocalSet::new();
        local_set
            .run_until(async {
                let (directory, credential_owner) = fresh_credential_owner();
                let (public_ws, clocks) = selected_public_and_clocks();
                let mismatched_scope = PmWireScope::new(
                    scope().condition(),
                    PmMarketId::parse(
                        "0x8888888888888888888888888888888888888888888888888888888888888888",
                    )
                    .unwrap(),
                    PmTokenId::new(U256::from_u64(4_321)).unwrap(),
                );
                let profile = PmPrivateReadRuntimeProfile::new(
                    Duration::from_secs(1),
                    Duration::from_secs(2),
                    mismatched_scope,
                    user_ws_bounds(),
                    signer(),
                    proxy(),
                );
                let bounds = CredentialAuthorityShutdownBounds::new(
                    Duration::from_secs(2),
                    Duration::from_secs(2),
                )
                .unwrap();

                let result = PmFreshStagedSelectedObservationRoles::production_selected_internal(
                    PmDeferredObservationAssemblyToken::test_support(),
                    credential_owner,
                    &local_set,
                    profile,
                    clocks,
                    public_ws,
                    selected_http_peer(),
                    selected_ws_peer(),
                    selected_local(),
                    bounds,
                )
                .await;
                assert!(matches!(
                    result,
                    Err(PmPrivateReadRuntimeError::Live(
                        PmLiveAdapterError::InvalidConfiguration(
                            "selected public and private observation scopes diverged"
                        )
                    ))
                ));
                for entry in ["private-key", "api-key", "l2-secret", "passphrase"] {
                    assert!(
                        directory.path().join(entry).exists(),
                        "pre-arm rejection must leave protected {entry} available for retry"
                    );
                }
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn failed_staged_production_build_joins_and_removes_all_four_files() {
        let local_set = tokio::task::LocalSet::new();
        local_set
            .run_until(async {
                let (directory, authority) = fresh_staged_authority(&local_set);
                let profile = PmPrivateReadRuntimeProfile::new(
                    Duration::ZERO,
                    Duration::from_secs(2),
                    scope(),
                    user_ws_bounds(),
                    signer(),
                    proxy(),
                );
                let bounds = CredentialAuthorityShutdownBounds::new(
                    Duration::from_secs(2),
                    Duration::from_secs(2),
                )
                .unwrap();

                let result = PmFreshStagedObservationPrivateReadRuntimeRoles::production(
                    authority,
                    profile,
                    clock_bundle("http://127.0.0.1:9"),
                    bounds,
                )
                .await;

                assert!(matches!(
                    result,
                    Err(PmPrivateReadRuntimeError::Live(
                        PmLiveAdapterError::InvalidConfiguration(
                            "connect and request timeouts must be positive"
                        )
                    ))
                ));
                for entry in ["private-key", "api-key", "l2-secret", "passphrase"] {
                    assert!(
                        !directory.path().join(entry).exists(),
                        "failed staged build retained {entry}"
                    );
                }
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn staged_production_rejects_profile_signer_mismatch_then_cleans_up() {
        let local_set = tokio::task::LocalSet::new();
        local_set
            .run_until(async {
                let (directory, authority) = fresh_staged_authority(&local_set);
                let profile = PmPrivateReadRuntimeProfile::new(
                    Duration::from_secs(1),
                    Duration::from_secs(2),
                    scope(),
                    user_ws_bounds(),
                    EoaAddress::parse(FOREIGN_SIGNER).unwrap(),
                    proxy(),
                );
                let bounds = CredentialAuthorityShutdownBounds::new(
                    Duration::from_secs(2),
                    Duration::from_secs(2),
                )
                .unwrap();

                let result = PmFreshStagedObservationPrivateReadRuntimeRoles::production(
                    authority,
                    profile,
                    clock_bundle("http://127.0.0.1:9"),
                    bounds,
                )
                .await;

                assert!(matches!(
                    result,
                    Err(PmPrivateReadRuntimeError::StagedObservationSignerBindingMismatch)
                ));
                for entry in ["private-key", "api-key", "l2-secret", "passphrase"] {
                    assert!(
                        !directory.path().join(entry).exists(),
                        "signer mismatch retained staged file {entry}"
                    );
                }
            })
            .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn empty_complete_cuts_use_one_fresh_server_time_per_fixed_request() {
        let mut runtime = recovery_runtime(MockPlan::new(OrdersPlan::Empty)).await;
        let (user_role, _activity, marker) = runtime.take_user_parts();
        drop(user_role);
        let (core, join) = runtime
            .core
            .collect(collection_start(&marker), None)
            .await
            .unwrap();
        let cut = PmRecoveryAccountAuthenticatedRestCut { core };
        assert_eq!(cut.open_orders().page_count(), 1);
        assert_eq!(cut.open_orders().row_count(), 0);
        assert!(cut.open_orders().rows().is_empty());
        assert_eq!(cut.trades().page_count(), 1);
        assert_eq!(cut.trades().row_count(), 0);
        assert!(cut.trades().rows().is_empty());
        assert_eq!(cut.server_times().len(), 5);
        assert_eq!(
            cut.server_times()
                .iter()
                .map(PmPrivateReadServerTimeEvidence::purpose)
                .collect::<Vec<_>>(),
            [
                PmPrivateRestObservationPurpose::ClosedOnly,
                PmPrivateRestObservationPurpose::CollateralBalanceAllowance,
                PmPrivateRestObservationPurpose::ConditionalBalanceAllowance,
                PmPrivateRestObservationPurpose::OpenOrdersPage { ordinal: 0 },
                PmPrivateRestObservationPurpose::TradesPage { ordinal: 0 },
            ]
        );
        assert!(
            cut.server_times()
                .windows(2)
                .all(|pair| pair[0].timestamp() != pair[1].timestamp())
        );
        let (returned_start, identity) = join.into_parts();
        assert!(marker.same_instance(returned_start.marker()));
        assert!(cut.matches_rest_cut_identity(&identity));
        assert_eq!(returned_start.stream_revision(), 7);
        assert_eq!(returned_start.connection_epoch(), ConnectionEpoch::new(3));
        assert_eq!(
            returned_start.activity_generation(),
            cut.activity_generation()
        );

        let targets = runtime.server.targets();
        assert_eq!(targets.len(), 10);
        assert!(targets.chunks_exact(2).all(|pair| pair[0] == "/time"));
        assert_eq!(targets[1], "/auth/ban-status/closed-only");
        assert!(targets[3].contains("asset_type=COLLATERAL"));
        assert!(targets[5].contains("asset_type=CONDITIONAL"));
        assert!(targets[7].starts_with("/data/orders?"));
        assert!(targets[9].starts_with("/data/trades?"));
        drop(returned_start);
        drop(identity);
        drop(cut);
        drop(marker);
        runtime.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn nonempty_owner_free_rows_and_recovery_exact_detail_survive_typed() {
        let mut plan = MockPlan::new(OrdersPlan::OneRow);
        plan.exact_present = true;
        let mut runtime = recovery_runtime(plan).await;
        let (user_role, _activity, marker) = runtime.take_user_parts();
        drop(user_role);
        let order_id = FixedOrderId::parse(ORDER_ID).unwrap();
        let candidate = PmRecoveryExpectedOrderCandidate::test_support_unproven(order_id);
        let (core, join) = runtime
            .core
            .collect(collection_start(&marker), Some(candidate.into_order_id()))
            .await
            .unwrap();
        let cut = PmRecoveryExactAuthenticatedRestCut { core };
        assert_eq!(cut.open_orders().rows().len(), 1);
        let order = &cut.open_orders().rows()[0];
        assert_eq!(order.exact().id().to_string(), ORDER_ID);
        assert_eq!(order.exact().token(), scope().token());
        assert_eq!(order.exact().side(), PmOrderSide::Buy);
        assert_eq!(order.exact().original_size().to_string(), "10");
        let PmBookQuantity::Quantity(size_matched) = order.exact().size_matched() else {
            panic!("nonzero matched quantity")
        };
        assert_eq!(size_matched.to_string(), "2.5");
        assert_eq!(order.text().status(), "LIVE");
        assert_eq!(order.text().outcome(), Some("YES"));
        assert_eq!(order.text().order_type(), Some("GTC"));

        assert_eq!(cut.collateral().asset(), PmAccountAsset::Collateral);
        assert_eq!(
            cut.collateral().balance().to_string(),
            "123456789012345678901234567890"
        );
        assert_eq!(cut.collateral().allowances().len(), 1);
        let spender = EvmAddress::parse(SPENDER).unwrap();
        assert_eq!(cut.collateral().allowances()[0].spender(), spender);
        assert_eq!(
            cut.collateral().allowances()[0].amount().to_string(),
            "987654321098765432109876543210"
        );
        assert_eq!(
            cut.collateral()
                .exact_allowance(spender)
                .map(|amount| amount.to_string())
                .as_deref(),
            Some("987654321098765432109876543210")
        );
        assert!(!cut.collateral().unscoped_scalar_present());

        assert_eq!(cut.trades().rows().len(), 1);
        let trade = &cut.trades().rows()[0];
        assert_eq!(trade.id().to_string(), "trade-1");
        assert_eq!(trade.side(), PmOrderSide::Sell);
        assert_eq!(trade.size().to_string(), "2.5");
        assert_eq!(trade.status(), "CONFIRMED");
        assert_eq!(trade.transaction_hash(), Some("0xfeed"));
        assert_eq!(trade.fee_rate_bps(), Some(U256::from_u64(30)));
        assert_eq!(trade.maker_orders().len(), 1);
        assert_eq!(
            trade.maker_orders()[0].fee_rate_bps(),
            Some(U256::from_u64(20))
        );

        let exact = cut.exact_order();
        assert_eq!(exact.order_id(), order_id);
        let PmRecoveryExactOrderClassification::Present(exact) = exact.classification() else {
            panic!("present exact order")
        };
        assert_eq!(exact.condition(), scope().condition());
        assert_eq!(exact.token(), scope().token());
        assert_eq!(exact.status(), "LIVE");
        assert_eq!(cut.server_times().len(), 6);
        let debug_values = [
            format!("{cut:?}"),
            format!("{:?}", cut.collateral()),
            format!("{:?}", cut.collateral().allowances()[0]),
            format!("{:?}", cut.conditional()),
            format!("{:?}", cut.open_orders()),
            format!("{:?}", cut.open_orders().rows()[0]),
            format!("{:?}", cut.open_orders().rows()[0].exact()),
            format!("{:?}", cut.open_orders().rows()[0].text()),
            format!("{:?}", cut.trades()),
            format!("{:?}", cut.trades().rows()[0]),
            format!("{:?}", cut.trades().rows()[0].maker_orders()[0]),
            format!("{:?}", cut.exact_order()),
            format!("{:?}", cut.exact_order().classification()),
            format!("{exact:?}"),
        ];
        for debug in debug_values {
            for canary in [
                API_KEY,
                L2_SECRET,
                PASSPHRASE,
                ORDER_ID,
                MAKER_ORDER_ID,
                "trade-1",
                "0xfeed",
                "2.5",
                "LIVE",
                "CONFIRMED",
                "123456789012345678901234567890",
                "987654321098765432109876543210",
            ] {
                assert!(!debug.contains(canary), "debug leaked canary `{canary}`");
            }
        }

        let (returned_start, identity) = join.into_parts();
        assert!(cut.matches_rest_cut_identity(&identity));
        drop(returned_start);
        drop(identity);
        drop(cut);
        drop(marker);
        runtime.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wrong_same_credential_marker_and_stale_activity_generation_fail_before_http() {
        let mut runtime = recovery_runtime(MockPlan::new(OrdersPlan::Empty)).await;
        let (user_role, activity, marker) = runtime.take_user_parts();
        drop(user_role);

        let foreign = PmSameCredentialAuthorityMarker::new(scope(), signer(), proxy(), activity);
        assert!(matches!(
            runtime.core.collect(collection_start(&foreign), None).await,
            Err(PmPrivateReadRuntimeError::SameCredentialAuthorityMismatch)
        ));

        let mut stale = collection_start(&marker);
        stale.activity_generation = stale.activity_generation.saturating_add(1);
        assert!(matches!(
            runtime.core.collect(stale, None).await,
            Err(PmPrivateReadRuntimeError::UserActivityChangedRetryRequired)
        ));
        assert!(runtime.server.targets().is_empty());
        drop(foreign);
        drop(marker);
        runtime.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn foreign_response_owner_fails_before_a_complete_cut_exists() {
        let mut runtime = recovery_runtime(MockPlan::new(OrdersPlan::ForeignOwner)).await;
        let (user_role, _activity, marker) = runtime.take_user_parts();
        drop(user_role);
        assert!(matches!(
            runtime.core.collect(collection_start(&marker), None).await,
            Err(PmPrivateReadRuntimeError::Live(
                PmLiveAdapterError::CredentialOwnerMismatch
            ))
        ));
        drop(marker);
        runtime.finish().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cursor_cycle_and_nonterminal_transport_failure_never_mint_a_join() {
        for (orders, expected) in [
            (OrdersPlan::CursorCycle, "cycle"),
            (OrdersPlan::IncompleteThenError, "incomplete"),
        ] {
            let mut runtime = recovery_runtime(MockPlan::new(orders)).await;
            let (user_role, _activity, marker) = runtime.take_user_parts();
            drop(user_role);
            let error = runtime
                .core
                .collect(collection_start(&marker), None)
                .await
                .unwrap_err();
            match expected {
                "cycle" => assert!(matches!(
                    error,
                    PmPrivateReadRuntimeError::Live(PmLiveAdapterError::PaginationCursorCycle)
                )),
                "incomplete" => assert!(matches!(
                    error,
                    PmPrivateReadRuntimeError::Live(PmLiveAdapterError::UnexpectedStatus {
                        status: 503
                    })
                )),
                _ => unreachable!(),
            }
            drop(marker);
            runtime.finish().await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authenticated_page_cap_fails_closed_without_starting_trades() {
        let mut runtime = recovery_runtime(MockPlan::new(OrdersPlan::PageCap)).await;
        let (user_role, _activity, marker) = runtime.take_user_parts();
        drop(user_role);
        assert!(matches!(
            runtime.core.collect(collection_start(&marker), None).await,
            Err(PmPrivateReadRuntimeError::Live(
                PmLiveAdapterError::PaginationPageLimit
            ))
        ));
        let targets = runtime.server.targets();
        assert!(
            !targets
                .iter()
                .any(|target| target.starts_with("/data/trades"))
        );
        assert_eq!(
            targets
                .iter()
                .filter(|target| target.starts_with("/data/orders"))
                .count(),
            reap_polymarket_live_adapter::MAX_PM_AUTHENTICATED_CUT_PAGES
        );
        drop(marker);
        runtime.finish().await;
    }

    #[test]
    fn fresh_and_recovery_surfaces_are_source_separated() {
        const SOURCE: &str = include_str!("private_reads.rs");
        let fresh = SOURCE
            .split("pub(super) struct PmFreshAuthenticatedRestRuntime")
            .nth(1)
            .unwrap()
            .split("pub(super) struct PmRecoveryAuthenticatedRestRuntime")
            .next()
            .unwrap();
        assert!(fresh.contains("PmFreshAuthenticatedRestCut"));
        assert!(!fresh.contains("expected_order_candidate"));

        let recovery = SOURCE
            .split("pub(super) struct PmRecoveryAuthenticatedRestRuntime")
            .nth(1)
            .unwrap()
            .split("pub(super) struct PmFreshPrivateReadRuntimeRoles")
            .next()
            .unwrap();
        assert!(recovery.contains("PmRecoveryExpectedOrderCandidate"));
        assert!(!recovery.contains("exact_journal_order_id: FixedOrderId"));

        let recovery_roles = SOURCE
            .split("pub(super) struct PmRecoveryPrivateReadRuntimeRoles")
            .nth(1)
            .unwrap()
            .split("fn production_runtime_core")
            .next()
            .unwrap();
        assert!(!recovery_roles.contains("FreshPlaceAuthenticationOnce"));
        assert!(!recovery_roles.contains("authenticate_place"));
        assert!(recovery_roles.contains("RecoveryCredentialAuthorityRoles"));
    }
}
