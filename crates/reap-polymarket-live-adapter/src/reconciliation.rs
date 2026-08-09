use std::fmt;

use reap_pm_core::{
    EvmAddress, MAX_PM_RECONCILIATION_FILLS, MAX_PM_RECONCILIATION_ORDERS, PmBookQuantity,
    PmOrderSide,
};
use reap_polymarket_auth::{FixedOrderId, L2Timestamp};
use reap_polymarket_wire::{
    PmLiveOpenOrderPage, PmLiveOrder, PmLiveTradePage, PmWireScope, parse_live_open_order_page,
    parse_live_order_detail, parse_live_trade_page,
};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

use crate::{
    PM_CLOB_PRODUCTION_ORIGIN, PmLiveAdapterError, PmPrivateReadEdgeClock,
    PmPrivateReadProductClock, PmReadOnlySignatureType, PmReadServerTime,
    config::OriginMode,
    private_credentials::PmHttpCredentialRole,
    private_http::{
        PmPrivateHttpObservation, PmPrivateHttpTransport, PmPrivateRoute, first_page_cursor,
    },
};

/// A deliberate fail-closed ceiling for one account-wide paginated cut.
pub const MAX_PM_AUTHENTICATED_CUT_PAGES: usize = 1_024;
pub const MAX_PM_AUTHENTICATED_ORDER_ROWS: usize = MAX_PM_RECONCILIATION_ORDERS;
pub const MAX_PM_AUTHENTICATED_TRADE_ROWS: usize = MAX_PM_RECONCILIATION_FILLS;

const OPEN_ORDERS_PAGE_SOURCE_COMMITMENT_DOMAIN: &[u8] =
    b"reap.pm.live-adapter.open-orders-page-source.v1\0";
const TRADES_PAGE_SOURCE_COMMITMENT_DOMAIN: &[u8] = b"reap.pm.live-adapter.trades-page-source.v1\0";
const COMPLETE_OPEN_ORDERS_OBSERVATION_COMMITMENT_DOMAIN: &[u8] =
    b"reap.pm.live-adapter.complete-open-orders-observation.v1\0";
const COMPLETE_TRADES_OBSERVATION_COMMITMENT_DOMAIN: &[u8] =
    b"reap.pm.live-adapter.complete-trades-observation.v1\0";
const EXACT_ORDER_SOURCE_COMMITMENT_DOMAIN: &[u8] = b"reap.pm.live-adapter.exact-order-source.v1\0";
const EXACT_ORDER_OBSERVATION_COMMITMENT_DOMAIN: &[u8] =
    b"reap.pm.live-adapter.exact-order-observation.v1\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReconciliationSourceBinding {
    mode: OriginMode,
    authenticated_signer: [u8; 20],
    signature_type: PmReadOnlySignatureType,
    configured_expected_maker: EvmAddress,
    scope: PmWireScope,
}

#[derive(PartialEq, Eq)]
struct AuthenticatedPageSource {
    requested_cursor: Box<str>,
    source_commitment: [u8; 32],
}

#[derive(PartialEq, Eq)]
struct LiveCutSource {
    binding: ReconciliationSourceBinding,
    pages: Vec<AuthenticatedPageSource>,
    terminal_receive_clock: Option<PmPrivateReadEdgeClock>,
}

struct FetchedOpenOrdersPage {
    parsed: PmLiveOpenOrderPage,
    source: AuthenticatedPageSource,
}

struct FetchedTradesPage {
    parsed: PmLiveTradePage,
    source: AuthenticatedPageSource,
}

pub enum PmOpenOrdersCutProgress {
    Incomplete(PmOpenOrdersAssembly),
    Complete(PmCompleteOpenOrdersCut),
}

impl fmt::Debug for PmOpenOrdersCutProgress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Incomplete(value) => value.fmt(formatter),
            Self::Complete(value) => value.fmt(formatter),
        }
    }
}

/// Move-only partial open-order evidence. It deliberately exposes counts, not
/// rows, so a nonterminal cut cannot be mistaken for complete reconciliation.
pub struct PmOpenOrdersAssembly {
    pages: Vec<PmLiveOpenOrderPage>,
    requested_cursors: Vec<String>,
    next_cursor: String,
    row_count: usize,
    live_source: Option<LiveCutSource>,
}

impl PmOpenOrdersAssembly {
    #[must_use]
    pub const fn pages_received(&self) -> usize {
        self.pages.len()
    }

    #[must_use]
    pub const fn rows_received(&self) -> usize {
        self.row_count
    }
}

impl fmt::Debug for PmOpenOrdersAssembly {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmOpenOrdersAssembly(Incomplete)")
            .field("pages_received", &self.pages.len())
            .field("rows_received", &self.row_count)
            .finish()
    }
}

#[derive(PartialEq, Eq)]
pub struct PmCompleteOpenOrdersCut {
    pages: Box<[PmLiveOpenOrderPage]>,
    row_count: usize,
    live_source: Option<LiveCutSource>,
}

impl fmt::Debug for PmCompleteOpenOrdersCut {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmCompleteOpenOrdersCut")
            .field("pages", &self.pages.len())
            .field("row_count", &self.row_count)
            .field("source", &self.live_source.as_ref().map(|_| "<sealed>"))
            .finish()
    }
}

impl PmCompleteOpenOrdersCut {
    #[must_use]
    pub fn pages(&self) -> &[PmLiveOpenOrderPage] {
        &self.pages
    }

    #[must_use]
    pub const fn row_count(&self) -> usize {
        self.row_count
    }

    /// Construct complete typed evidence for downstream contract tests.
    /// Production transport authority never enables this feature.
    #[cfg(feature = "test-support")]
    pub fn test_support_from_pages(pages: Box<[PmLiveOpenOrderPage]>) -> Option<Self> {
        complete_open_orders_for_test(pages)
    }
}

pub enum PmTradesCutProgress {
    Incomplete(PmTradesAssembly),
    Complete(PmCompleteTradesCut),
}

impl fmt::Debug for PmTradesCutProgress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Incomplete(value) => value.fmt(formatter),
            Self::Complete(value) => value.fmt(formatter),
        }
    }
}

/// Move-only partial trade evidence. Rows remain sealed until the exact
/// terminal cursor is observed.
pub struct PmTradesAssembly {
    pages: Vec<PmLiveTradePage>,
    requested_cursors: Vec<String>,
    next_cursor: String,
    row_count: usize,
    live_source: Option<LiveCutSource>,
}

impl PmTradesAssembly {
    #[must_use]
    pub const fn pages_received(&self) -> usize {
        self.pages.len()
    }

    #[must_use]
    pub const fn rows_received(&self) -> usize {
        self.row_count
    }
}

impl fmt::Debug for PmTradesAssembly {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmTradesAssembly(Incomplete)")
            .field("pages_received", &self.pages.len())
            .field("rows_received", &self.row_count)
            .finish()
    }
}

#[derive(PartialEq, Eq)]
pub struct PmCompleteTradesCut {
    pages: Box<[PmLiveTradePage]>,
    row_count: usize,
    live_source: Option<LiveCutSource>,
}

impl fmt::Debug for PmCompleteTradesCut {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmCompleteTradesCut")
            .field("pages", &self.pages.len())
            .field("row_count", &self.row_count)
            .field("source", &self.live_source.as_ref().map(|_| "<sealed>"))
            .finish()
    }
}

impl PmCompleteTradesCut {
    #[must_use]
    pub fn pages(&self) -> &[PmLiveTradePage] {
        &self.pages
    }

    #[must_use]
    pub const fn row_count(&self) -> usize {
        self.row_count
    }

    /// Construct complete typed evidence for downstream contract tests.
    /// Production transport authority never enables this feature.
    #[cfg(feature = "test-support")]
    pub fn test_support_from_pages(pages: Box<[PmLiveTradePage]>) -> Option<Self> {
        complete_trades_for_test(pages)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PmExactOrderObservation {
    Present(Box<PmLiveOrder>),
    Absent,
}

struct FetchedExactOrder {
    classification: PmExactOrderObservation,
    source_commitment: [u8; 32],
    response_status: u16,
}

/// Source-only, secret-free SHA-256 commitment to one canonical typed
/// terminal open-order cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmCompleteOpenOrdersObservationCommitment([u8; 32]);

impl PmCompleteOpenOrdersObservationCommitment {
    const fn from_source_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Source-only, secret-free SHA-256 commitment to one canonical typed terminal
/// trade cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmCompleteTradesObservationCommitment([u8; 32]);

impl PmCompleteTradesObservationCommitment {
    const fn from_source_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Source-only, secret-free SHA-256 commitment to one canonical typed
/// exact-order Present/Absent result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PmExactOrderDetailObservationCommitment([u8; 32]);

impl PmExactOrderDetailObservationCommitment {
    const fn from_source_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Move-only terminal observation of a real authenticated open-order cut.
/// Test-support cuts and cuts already sealed once cannot construct it.
pub struct PmCompleteOpenOrdersObservation {
    cut: PmCompleteOpenOrdersCut,
    receive_clock: PmPrivateReadEdgeClock,
    commitment: PmCompleteOpenOrdersObservationCommitment,
}

impl PmCompleteOpenOrdersObservation {
    fn from_source(
        cut: PmCompleteOpenOrdersCut,
        receive_clock: PmPrivateReadEdgeClock,
        commitment: PmCompleteOpenOrdersObservationCommitment,
    ) -> Self {
        Self {
            cut,
            receive_clock,
            commitment,
        }
    }

    #[must_use]
    pub const fn cut(&self) -> &PmCompleteOpenOrdersCut {
        &self.cut
    }

    #[must_use]
    pub const fn receive_clock(&self) -> PmPrivateReadEdgeClock {
        self.receive_clock
    }

    #[must_use]
    pub const fn commitment(&self) -> PmCompleteOpenOrdersObservationCommitment {
        self.commitment
    }

    #[must_use]
    pub fn into_cut(self) -> PmCompleteOpenOrdersCut {
        self.cut
    }
}

impl fmt::Debug for PmCompleteOpenOrdersObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmCompleteOpenOrdersObservation")
            .field("pages", &self.cut.pages.len())
            .field("rows", &self.cut.row_count)
            .field("receive_clock", &self.receive_clock)
            .field("commitment", &self.commitment)
            .finish()
    }
}

/// Move-only terminal observation of a real authenticated trade cut.
/// Test-support cuts and cuts already sealed once cannot construct it.
pub struct PmCompleteTradesObservation {
    cut: PmCompleteTradesCut,
    receive_clock: PmPrivateReadEdgeClock,
    commitment: PmCompleteTradesObservationCommitment,
}

impl PmCompleteTradesObservation {
    fn from_source(
        cut: PmCompleteTradesCut,
        receive_clock: PmPrivateReadEdgeClock,
        commitment: PmCompleteTradesObservationCommitment,
    ) -> Self {
        Self {
            cut,
            receive_clock,
            commitment,
        }
    }

    #[must_use]
    pub const fn cut(&self) -> &PmCompleteTradesCut {
        &self.cut
    }

    #[must_use]
    pub const fn receive_clock(&self) -> PmPrivateReadEdgeClock {
        self.receive_clock
    }

    #[must_use]
    pub const fn commitment(&self) -> PmCompleteTradesObservationCommitment {
        self.commitment
    }

    #[must_use]
    pub fn into_cut(self) -> PmCompleteTradesCut {
        self.cut
    }
}

impl fmt::Debug for PmCompleteTradesObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmCompleteTradesObservation")
            .field("pages", &self.cut.pages.len())
            .field("rows", &self.cut.row_count)
            .field("receive_clock", &self.receive_clock)
            .field("commitment", &self.commitment)
            .finish()
    }
}

/// Move-only terminal observation of one exact authenticated order lookup.
///
/// `Present` proves the configured expected maker and exact scope checks ran.
/// For proxy profiles that maker is only a configured validation expectation;
/// the carrier does not claim a separately echoed funder identity.
pub struct PmExactOrderDetailObservation {
    order_id: FixedOrderId,
    classification: PmExactOrderObservation,
    receive_clock: PmPrivateReadEdgeClock,
    commitment: PmExactOrderDetailObservationCommitment,
}

impl PmExactOrderDetailObservation {
    fn from_source(
        order_id: FixedOrderId,
        classification: PmExactOrderObservation,
        receive_clock: PmPrivateReadEdgeClock,
        commitment: PmExactOrderDetailObservationCommitment,
    ) -> Self {
        Self {
            order_id,
            classification,
            receive_clock,
            commitment,
        }
    }

    #[must_use]
    pub const fn order_id(&self) -> FixedOrderId {
        self.order_id
    }

    #[must_use]
    pub const fn classification(&self) -> &PmExactOrderObservation {
        &self.classification
    }

    #[must_use]
    pub const fn receive_clock(&self) -> PmPrivateReadEdgeClock {
        self.receive_clock
    }

    #[must_use]
    pub const fn commitment(&self) -> PmExactOrderDetailObservationCommitment {
        self.commitment
    }

    #[must_use]
    pub fn into_classification(self) -> PmExactOrderObservation {
        self.classification
    }
}

impl fmt::Debug for PmExactOrderDetailObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let classification = match &self.classification {
            PmExactOrderObservation::Present(_) => "Present",
            PmExactOrderObservation::Absent => "Absent",
        };
        formatter
            .debug_struct("PmExactOrderDetailObservation")
            .field("order_id", &self.order_id)
            .field("classification", &classification)
            .field("receive_clock", &self.receive_clock)
            .field("commitment", &self.commitment)
            .finish()
    }
}

/// Borrowed authenticated capability for complete account cuts and one strict
/// journal-known exact-order lookup.
pub struct PmReconciliationHttpRole<'a> {
    authority: &'a mut PmHttpCredentialRole,
    transport: &'a PmPrivateHttpTransport,
    exact_order_scope: PmWireScope,
    expected_order_maker: EvmAddress,
    signature_type: PmReadOnlySignatureType,
}

impl<'a> PmReconciliationHttpRole<'a> {
    pub(crate) const fn new(
        authority: &'a mut PmHttpCredentialRole,
        transport: &'a PmPrivateHttpTransport,
        exact_order_scope: PmWireScope,
        expected_order_maker: EvmAddress,
        signature_type: PmReadOnlySignatureType,
    ) -> Self {
        Self {
            authority,
            transport,
            exact_order_scope,
            expected_order_maker,
            signature_type,
        }
    }

    fn source_binding(&self) -> ReconciliationSourceBinding {
        ReconciliationSourceBinding {
            mode: self.transport.mode(),
            authenticated_signer: self.transport.configured_signer().bytes(),
            signature_type: self.signature_type,
            configured_expected_maker: self.expected_order_maker,
            scope: self.exact_order_scope,
        }
    }

    /// Start an unfiltered account-wide cut at exact cursor `MA==`.
    pub async fn begin_open_orders(
        &mut self,
        server_time: PmReadServerTime,
    ) -> Result<PmOpenOrdersCutProgress, PmLiveAdapterError> {
        self.begin_open_orders_source(server_time, None).await
    }

    /// Start a source-clocked open-order cut. If this page is terminal, its
    /// receive edge is sampled immediately after parse and credential binding.
    pub async fn begin_open_orders_observation(
        &mut self,
        server_time: PmReadServerTime,
        clock: &mut PmPrivateReadProductClock,
    ) -> Result<PmOpenOrdersCutProgress, PmLiveAdapterError> {
        self.begin_open_orders_source(server_time, Some(clock))
            .await
    }

    async fn begin_open_orders_source(
        &mut self,
        server_time: PmReadServerTime,
        clock: Option<&mut PmPrivateReadProductClock>,
    ) -> Result<PmOpenOrdersCutProgress, PmLiveAdapterError> {
        let fetched = self
            .open_orders_page(
                server_time
                    .into_l2_timestamp()
                    .map_err(|_| PmLiveAdapterError::ProductClock)?,
                first_page_cursor(),
            )
            .await?;
        let terminal_receive_clock = observe_terminal_page(&fetched.parsed, clock)?;
        let live_source = LiveCutSource {
            binding: self.source_binding(),
            pages: vec![fetched.source],
            terminal_receive_clock,
        };
        advance_open_orders(
            vec![fetched.parsed],
            vec![first_page_cursor().to_owned()],
            0,
            Some(live_source),
        )
    }

    /// Consume the only authority to continue this open-order cut.
    pub async fn continue_open_orders(
        &mut self,
        server_time: PmReadServerTime,
        assembly: PmOpenOrdersAssembly,
    ) -> Result<PmOpenOrdersCutProgress, PmLiveAdapterError> {
        self.continue_open_orders_source(server_time, assembly, None)
            .await
    }

    /// Continue a source-clocked open-order cut. The receive edge is sampled
    /// only when this exact fetched page is terminal.
    pub async fn continue_open_orders_observation(
        &mut self,
        server_time: PmReadServerTime,
        assembly: PmOpenOrdersAssembly,
        clock: &mut PmPrivateReadProductClock,
    ) -> Result<PmOpenOrdersCutProgress, PmLiveAdapterError> {
        self.continue_open_orders_source(server_time, assembly, Some(clock))
            .await
    }

    async fn continue_open_orders_source(
        &mut self,
        server_time: PmReadServerTime,
        mut assembly: PmOpenOrdersAssembly,
        clock: Option<&mut PmPrivateReadProductClock>,
    ) -> Result<PmOpenOrdersCutProgress, PmLiveAdapterError> {
        if assembly.pages.len() >= MAX_PM_AUTHENTICATED_CUT_PAGES {
            return Err(PmLiveAdapterError::PaginationPageLimit);
        }
        ensure_live_source_binding(assembly.live_source.as_ref(), self.source_binding())?;
        let cursor = std::mem::take(&mut assembly.next_cursor);
        assembly.requested_cursors.push(cursor.clone());
        let fetched = self
            .open_orders_page(
                server_time
                    .into_l2_timestamp()
                    .map_err(|_| PmLiveAdapterError::ProductClock)?,
                &cursor,
            )
            .await?;
        let terminal_receive_clock = observe_terminal_page(&fetched.parsed, clock)?;
        assembly.pages.push(fetched.parsed);
        let live_source = assembly
            .live_source
            .as_mut()
            .expect("validated live source");
        live_source.pages.push(fetched.source);
        live_source.terminal_receive_clock = terminal_receive_clock;
        advance_open_orders(
            assembly.pages,
            assembly.requested_cursors,
            assembly.row_count,
            assembly.live_source,
        )
    }

    /// Start an unfiltered full-account trade cut.
    pub async fn begin_trades(
        &mut self,
        server_time: PmReadServerTime,
    ) -> Result<PmTradesCutProgress, PmLiveAdapterError> {
        self.begin_trades_source(server_time, None).await
    }

    /// Start a source-clocked trade cut. A terminal receive edge is sampled
    /// immediately after the terminal page is parsed and credential-bound.
    pub async fn begin_trades_observation(
        &mut self,
        server_time: PmReadServerTime,
        clock: &mut PmPrivateReadProductClock,
    ) -> Result<PmTradesCutProgress, PmLiveAdapterError> {
        self.begin_trades_source(server_time, Some(clock)).await
    }

    async fn begin_trades_source(
        &mut self,
        server_time: PmReadServerTime,
        clock: Option<&mut PmPrivateReadProductClock>,
    ) -> Result<PmTradesCutProgress, PmLiveAdapterError> {
        let fetched = self
            .trades_page(
                server_time
                    .into_l2_timestamp()
                    .map_err(|_| PmLiveAdapterError::ProductClock)?,
                first_page_cursor(),
            )
            .await?;
        let terminal_receive_clock = observe_terminal_page(&fetched.parsed, clock)?;
        let live_source = LiveCutSource {
            binding: self.source_binding(),
            pages: vec![fetched.source],
            terminal_receive_clock,
        };
        advance_trades(
            vec![fetched.parsed],
            vec![first_page_cursor().to_owned()],
            0,
            Some(live_source),
        )
    }

    /// Consume the only authority to continue this trade cut.
    pub async fn continue_trades(
        &mut self,
        server_time: PmReadServerTime,
        assembly: PmTradesAssembly,
    ) -> Result<PmTradesCutProgress, PmLiveAdapterError> {
        self.continue_trades_source(server_time, assembly, None)
            .await
    }

    /// Continue a source-clocked trade cut, sampling only after a terminal
    /// page has completed parse and credential-owner binding.
    pub async fn continue_trades_observation(
        &mut self,
        server_time: PmReadServerTime,
        assembly: PmTradesAssembly,
        clock: &mut PmPrivateReadProductClock,
    ) -> Result<PmTradesCutProgress, PmLiveAdapterError> {
        self.continue_trades_source(server_time, assembly, Some(clock))
            .await
    }

    async fn continue_trades_source(
        &mut self,
        server_time: PmReadServerTime,
        mut assembly: PmTradesAssembly,
        clock: Option<&mut PmPrivateReadProductClock>,
    ) -> Result<PmTradesCutProgress, PmLiveAdapterError> {
        if assembly.pages.len() >= MAX_PM_AUTHENTICATED_CUT_PAGES {
            return Err(PmLiveAdapterError::PaginationPageLimit);
        }
        ensure_live_source_binding(assembly.live_source.as_ref(), self.source_binding())?;
        let cursor = std::mem::take(&mut assembly.next_cursor);
        assembly.requested_cursors.push(cursor.clone());
        let fetched = self
            .trades_page(
                server_time
                    .into_l2_timestamp()
                    .map_err(|_| PmLiveAdapterError::ProductClock)?,
                &cursor,
            )
            .await?;
        let terminal_receive_clock = observe_terminal_page(&fetched.parsed, clock)?;
        assembly.pages.push(fetched.parsed);
        let live_source = assembly
            .live_source
            .as_mut()
            .expect("validated live source");
        live_source.pages.push(fetched.source);
        live_source.terminal_receive_clock = terminal_receive_clock;
        advance_trades(
            assembly.pages,
            assembly.requested_cursors,
            assembly.row_count,
            assembly.live_source,
        )
    }

    pub async fn exact_local_order_detail(
        &mut self,
        server_time: PmReadServerTime,
        order_id: FixedOrderId,
    ) -> Result<PmExactOrderObservation, PmLiveAdapterError> {
        Ok(self
            .exact_order_source(server_time, order_id)
            .await?
            .classification)
    }

    /// Consume a real complete cut and its private live-chain token into one
    /// terminal source-clocked observation. A test-support or previously
    /// sealed cut has no token and fails closed.
    pub fn seal_complete_open_orders(
        &self,
        mut cut: PmCompleteOpenOrdersCut,
    ) -> Result<PmCompleteOpenOrdersObservation, PmLiveAdapterError> {
        let source = cut
            .live_source
            .take()
            .ok_or(PmLiveAdapterError::InvalidConfiguration(
                "only a real unsealed open-order cut can produce live provenance",
            ))?;
        ensure_complete_source(&source, self.source_binding(), &cut.pages)?;
        let receive_clock =
            source
                .terminal_receive_clock
                .ok_or(PmLiveAdapterError::InvalidConfiguration(
                    "open-order cut was not terminally source-clocked",
                ))?;
        let commitment = complete_open_orders_observation_commitment(&source, &cut, receive_clock);
        Ok(PmCompleteOpenOrdersObservation::from_source(
            cut,
            receive_clock,
            commitment,
        ))
    }

    /// Consume a real complete trade cut and its private live-chain token into
    /// one terminal source-clocked observation.
    pub fn seal_complete_trades(
        &self,
        mut cut: PmCompleteTradesCut,
    ) -> Result<PmCompleteTradesObservation, PmLiveAdapterError> {
        let source = cut
            .live_source
            .take()
            .ok_or(PmLiveAdapterError::InvalidConfiguration(
                "only a real unsealed trade cut can produce live provenance",
            ))?;
        ensure_complete_source(&source, self.source_binding(), &cut.pages)?;
        let receive_clock =
            source
                .terminal_receive_clock
                .ok_or(PmLiveAdapterError::InvalidConfiguration(
                    "trade cut was not terminally source-clocked",
                ))?;
        let commitment = complete_trades_observation_commitment(&source, &cut, receive_clock);
        Ok(PmCompleteTradesObservation::from_source(
            cut,
            receive_clock,
            commitment,
        ))
    }

    /// Fetch and seal one exact authenticated Present/Absent classification.
    /// The clock is sampled only after credential-owner binding and all exact
    /// identity/maker/scope checks complete for a present row. An authenticated
    /// 404 is committed as a distinct absent classification.
    pub async fn exact_local_order_detail_observation(
        &mut self,
        server_time: PmReadServerTime,
        order_id: FixedOrderId,
        clock: &mut PmPrivateReadProductClock,
    ) -> Result<PmExactOrderDetailObservation, PmLiveAdapterError> {
        let fetched = self.exact_order_source(server_time, order_id).await?;
        let receive_clock = clock
            .observe_authenticated_read_complete()
            .map_err(|_| PmLiveAdapterError::ProductClock)?;
        let commitment = exact_order_observation_commitment(
            self.source_binding(),
            self.expected_order_maker,
            order_id,
            &fetched,
            receive_clock,
        );
        Ok(PmExactOrderDetailObservation::from_source(
            order_id,
            fetched.classification,
            receive_clock,
            commitment,
        ))
    }

    async fn exact_order_source(
        &mut self,
        server_time: PmReadServerTime,
        order_id: FixedOrderId,
    ) -> Result<FetchedExactOrder, PmLiveAdapterError> {
        let headers = self
            .authority
            .authenticate_exact_order(
                server_time
                    .into_l2_timestamp()
                    .map_err(|_| PmLiveAdapterError::ProductClock)?,
                order_id,
            )
            .await?;
        match self
            .transport
            .get(PmPrivateRoute::ExactOrder(order_id), headers)
            .await?
        {
            PmPrivateHttpObservation::NotFound => {
                let classification = PmExactOrderObservation::Absent;
                Ok(FetchedExactOrder {
                    source_commitment: exact_order_source_commitment(
                        self.source_binding(),
                        order_id,
                        404,
                        &classification,
                    ),
                    classification,
                    response_status: 404,
                })
            }
            PmPrivateHttpObservation::Found(body) => {
                let order = self
                    .authority
                    .bind_exact_order(parse_live_order_detail(&body)?)
                    .await?;
                if order.id().as_str() != order_id.to_string() {
                    return Err(PmLiveAdapterError::ExactOrderIdentityMismatch);
                }
                if order.maker() != self.expected_order_maker {
                    return Err(PmLiveAdapterError::ExactOrderMakerMismatch);
                }
                if order.condition() != self.exact_order_scope.condition()
                    || order.token() != self.exact_order_scope.token()
                {
                    return Err(PmLiveAdapterError::ExactOrderScopeMismatch);
                }
                let classification = PmExactOrderObservation::Present(Box::new(order));
                Ok(FetchedExactOrder {
                    source_commitment: exact_order_source_commitment(
                        self.source_binding(),
                        order_id,
                        200,
                        &classification,
                    ),
                    classification,
                    response_status: 200,
                })
            }
        }
    }

    async fn open_orders_page(
        &mut self,
        timestamp: L2Timestamp,
        cursor: &str,
    ) -> Result<FetchedOpenOrdersPage, PmLiveAdapterError> {
        let headers = self.authority.authenticate_open_orders(timestamp).await?;
        let body = found(
            self.transport
                .get(PmPrivateRoute::OpenOrders { cursor }, headers)
                .await?,
        )?;
        let parsed = self
            .authority
            .bind_open_orders(parse_live_open_order_page(&body)?)
            .await?;
        let source = authenticated_page_source(
            OPEN_ORDERS_PAGE_SOURCE_COMMITMENT_DOMAIN,
            b"/data/orders",
            self.source_binding(),
            cursor,
            &parsed,
        );
        Ok(FetchedOpenOrdersPage { parsed, source })
    }

    async fn trades_page(
        &mut self,
        timestamp: L2Timestamp,
        cursor: &str,
    ) -> Result<FetchedTradesPage, PmLiveAdapterError> {
        let headers = self.authority.authenticate_trades(timestamp).await?;
        let body = found(
            self.transport
                .get(PmPrivateRoute::Trades { cursor }, headers)
                .await?,
        )?;
        let parsed = self
            .authority
            .bind_trades(parse_live_trade_page(&body)?)
            .await?;
        let source = authenticated_page_source(
            TRADES_PAGE_SOURCE_COMMITMENT_DOMAIN,
            b"/data/trades",
            self.source_binding(),
            cursor,
            &parsed,
        );
        Ok(FetchedTradesPage { parsed, source })
    }
}

trait ReconciliationPage {
    fn row_count(&self) -> usize;
    fn next_cursor(&self) -> Option<&str>;
    fn terminal(&self) -> bool;
    fn declared_limit(&self) -> usize;
    fn declared_count(&self) -> usize;
    fn encode_typed_rows(&self, digest: &mut Sha256);
}

impl ReconciliationPage for PmLiveOpenOrderPage {
    fn row_count(&self) -> usize {
        self.orders().len()
    }

    fn next_cursor(&self) -> Option<&str> {
        self.next_cursor().map(|cursor| cursor.as_str())
    }

    fn terminal(&self) -> bool {
        self.terminal()
    }

    fn declared_limit(&self) -> usize {
        self.declared_limit()
    }

    fn declared_count(&self) -> usize {
        self.declared_count()
    }

    fn encode_typed_rows(&self, digest: &mut Sha256) {
        for order in self.orders() {
            encode_live_order(digest, order);
        }
    }
}

impl ReconciliationPage for PmLiveTradePage {
    fn row_count(&self) -> usize {
        self.trades().len()
    }

    fn next_cursor(&self) -> Option<&str> {
        self.next_cursor().map(|cursor| cursor.as_str())
    }

    fn terminal(&self) -> bool {
        self.terminal()
    }

    fn declared_limit(&self) -> usize {
        self.declared_limit()
    }

    fn declared_count(&self) -> usize {
        self.declared_count()
    }

    fn encode_typed_rows(&self, digest: &mut Sha256) {
        for trade in self.trades() {
            encode_reconciliation_bytes(digest, trade.id().as_str().as_bytes());
            digest.update(trade.condition().bytes());
            digest.update(trade.token().units().to_be_bytes());
            encode_side(digest, trade.side());
            digest.update(trade.size().protocol_units().to_be_bytes());
            digest.update(trade.price().units().to_be_bytes());
            encode_reconciliation_bytes(digest, trade.status().as_bytes());
            encode_optional_order_id(digest, trade.order_id());
            encode_optional_order_id(digest, trade.taker_order_id());
            encode_optional_reconciliation_ascii(digest, trade.trader_side());
            encode_optional_reconciliation_ascii(digest, trade.transaction_hash());
            match trade.fee_rate_bps() {
                Some(fee_rate_bps) => {
                    digest.update([1]);
                    digest.update(fee_rate_bps.to_be_bytes());
                }
                None => digest.update([0]),
            }
            digest.update(
                u32::try_from(trade.maker_orders().len())
                    .expect("bounded maker-order count fits u32")
                    .to_be_bytes(),
            );
            for maker_order in trade.maker_orders() {
                encode_reconciliation_bytes(digest, maker_order.order_id().as_str().as_bytes());
                digest.update(maker_order.token().units().to_be_bytes());
                encode_side(digest, maker_order.side());
                digest.update(maker_order.price().units().to_be_bytes());
                digest.update(maker_order.matched_amount().protocol_units().to_be_bytes());
                match maker_order.fee_rate_bps() {
                    Some(fee_rate_bps) => {
                        digest.update([1]);
                        digest.update(fee_rate_bps.to_be_bytes());
                    }
                    None => digest.update([0]),
                }
                digest.update(maker_order.maker().bytes());
            }
            match trade.maker() {
                Some(maker) => {
                    digest.update([1]);
                    digest.update(maker.bytes());
                }
                None => digest.update([0]),
            }
            encode_optional_u64(digest, trade.timestamp());
            encode_optional_u64(digest, trade.match_time());
            encode_optional_u64(digest, trade.last_update());
        }
    }
}

fn observe_terminal_page<P: ReconciliationPage>(
    page: &P,
    clock: Option<&mut PmPrivateReadProductClock>,
) -> Result<Option<PmPrivateReadEdgeClock>, PmLiveAdapterError> {
    if !page.terminal() {
        return Ok(None);
    }
    clock
        .map(PmPrivateReadProductClock::observe_authenticated_read_complete)
        .transpose()
        .map_err(|_| PmLiveAdapterError::ProductClock)
}

fn authenticated_page_source<P: ReconciliationPage>(
    domain: &'static [u8],
    route: &'static [u8],
    binding: ReconciliationSourceBinding,
    requested_cursor: &str,
    parsed: &P,
) -> AuthenticatedPageSource {
    let mut digest = Sha256::new();
    encode_reconciliation_bytes(&mut digest, domain);
    encode_reconciliation_binding(&mut digest, binding);
    encode_reconciliation_bytes(&mut digest, b"GET");
    encode_reconciliation_bytes(&mut digest, route);
    encode_reconciliation_bytes(&mut digest, b"next_cursor");
    encode_reconciliation_bytes(&mut digest, requested_cursor.as_bytes());
    encode_page_projection(&mut digest, parsed);
    AuthenticatedPageSource {
        requested_cursor: requested_cursor.into(),
        source_commitment: digest.finalize().into(),
    }
}

fn exact_order_source_commitment(
    binding: ReconciliationSourceBinding,
    order_id: FixedOrderId,
    response_status: u16,
    classification: &PmExactOrderObservation,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    encode_reconciliation_bytes(&mut digest, EXACT_ORDER_SOURCE_COMMITMENT_DOMAIN);
    encode_reconciliation_binding(&mut digest, binding);
    encode_reconciliation_bytes(&mut digest, b"GET");
    encode_reconciliation_bytes(&mut digest, b"/data/order/");
    digest.update(order_id.bytes());
    encode_reconciliation_bytes(&mut digest, b"");
    digest.update(response_status.to_be_bytes());
    encode_exact_order_classification(&mut digest, classification);
    digest.finalize().into()
}

fn complete_open_orders_observation_commitment(
    source: &LiveCutSource,
    cut: &PmCompleteOpenOrdersCut,
    receive_clock: PmPrivateReadEdgeClock,
) -> PmCompleteOpenOrdersObservationCommitment {
    PmCompleteOpenOrdersObservationCommitment::from_source_bytes(
        complete_cut_observation_commitment(
            COMPLETE_OPEN_ORDERS_OBSERVATION_COMMITMENT_DOMAIN,
            b"/data/orders",
            source,
            &cut.pages,
            cut.row_count,
            receive_clock,
        ),
    )
}

fn complete_trades_observation_commitment(
    source: &LiveCutSource,
    cut: &PmCompleteTradesCut,
    receive_clock: PmPrivateReadEdgeClock,
) -> PmCompleteTradesObservationCommitment {
    PmCompleteTradesObservationCommitment::from_source_bytes(complete_cut_observation_commitment(
        COMPLETE_TRADES_OBSERVATION_COMMITMENT_DOMAIN,
        b"/data/trades",
        source,
        &cut.pages,
        cut.row_count,
        receive_clock,
    ))
}

fn complete_cut_observation_commitment<P: ReconciliationPage>(
    domain: &'static [u8],
    route: &'static [u8],
    source: &LiveCutSource,
    pages: &[P],
    row_count: usize,
    receive_clock: PmPrivateReadEdgeClock,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    encode_reconciliation_bytes(&mut digest, domain);
    encode_reconciliation_binding(&mut digest, source.binding);
    encode_reconciliation_bytes(&mut digest, b"GET");
    encode_reconciliation_bytes(&mut digest, route);
    encode_reconciliation_bytes(&mut digest, b"next_cursor");
    digest.update(
        u32::try_from(pages.len())
            .expect("bounded authenticated page count fits u32")
            .to_be_bytes(),
    );
    digest.update(
        u32::try_from(row_count)
            .expect("bounded authenticated row count fits u32")
            .to_be_bytes(),
    );
    for (page, page_source) in pages.iter().zip(&source.pages) {
        encode_reconciliation_bytes(&mut digest, page_source.requested_cursor.as_bytes());
        digest.update(page_source.source_commitment);
        encode_page_projection(&mut digest, page);
    }
    digest.update(receive_clock.local_wall_receive_ns().to_be_bytes());
    digest.update(receive_clock.monotonic_receive_ns().to_be_bytes());
    digest.finalize().into()
}

fn exact_order_observation_commitment(
    binding: ReconciliationSourceBinding,
    expected_order_maker: EvmAddress,
    order_id: FixedOrderId,
    fetched: &FetchedExactOrder,
    receive_clock: PmPrivateReadEdgeClock,
) -> PmExactOrderDetailObservationCommitment {
    let mut digest = Sha256::new();
    encode_reconciliation_bytes(&mut digest, EXACT_ORDER_OBSERVATION_COMMITMENT_DOMAIN);
    encode_reconciliation_binding(&mut digest, binding);
    encode_reconciliation_bytes(&mut digest, b"GET");
    encode_reconciliation_bytes(&mut digest, b"/data/order/");
    digest.update(order_id.bytes());
    encode_reconciliation_bytes(&mut digest, b"");
    digest.update(expected_order_maker.bytes());
    digest.update(fetched.response_status.to_be_bytes());
    digest.update(fetched.source_commitment);
    encode_exact_order_classification(&mut digest, &fetched.classification);
    digest.update(receive_clock.local_wall_receive_ns().to_be_bytes());
    digest.update(receive_clock.monotonic_receive_ns().to_be_bytes());
    PmExactOrderDetailObservationCommitment::from_source_bytes(digest.finalize().into())
}

fn encode_reconciliation_binding(digest: &mut Sha256, binding: ReconciliationSourceBinding) {
    encode_reconciliation_bytes(digest, origin_mode_name(binding.mode));
    encode_reconciliation_bytes(digest, PM_CLOB_PRODUCTION_ORIGIN.as_bytes());
    digest.update(binding.authenticated_signer);
    digest.update([binding.signature_type.value()]);
    digest.update(binding.configured_expected_maker.bytes());
    digest.update(binding.scope.condition().bytes());
    digest.update(binding.scope.market().bytes());
    digest.update(binding.scope.token().units().to_be_bytes());
}

fn encode_page_projection<P: ReconciliationPage>(digest: &mut Sha256, page: &P) {
    digest.update(
        u32::try_from(page.row_count())
            .expect("bounded authenticated page rows fit u32")
            .to_be_bytes(),
    );
    digest.update(
        u32::try_from(page.declared_limit())
            .expect("bounded declared page limit fits u32")
            .to_be_bytes(),
    );
    digest.update(
        u32::try_from(page.declared_count())
            .expect("bounded declared page count fits u32")
            .to_be_bytes(),
    );
    digest.update([u8::from(page.terminal())]);
    match page.next_cursor() {
        Some(cursor) => {
            digest.update([1]);
            encode_reconciliation_bytes(digest, cursor.as_bytes());
        }
        None => digest.update([0]),
    }
    page.encode_typed_rows(digest);
}

fn encode_exact_order_classification(
    digest: &mut Sha256,
    classification: &PmExactOrderObservation,
) {
    match classification {
        PmExactOrderObservation::Absent => digest.update([0]),
        PmExactOrderObservation::Present(order) => {
            digest.update([1]);
            encode_live_order(digest, order);
        }
    }
}

fn encode_reconciliation_bytes(digest: &mut Sha256, value: &[u8]) {
    digest.update(
        u64::try_from(value.len())
            .expect("bounded reconciliation commitment field length fits u64")
            .to_be_bytes(),
    );
    digest.update(value);
}

fn encode_optional_reconciliation_ascii(digest: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            digest.update([1]);
            encode_reconciliation_bytes(digest, value.as_bytes());
        }
        None => digest.update([0]),
    }
}

fn encode_live_order(digest: &mut Sha256, order: &PmLiveOrder) {
    encode_reconciliation_bytes(digest, order.id().as_str().as_bytes());
    digest.update(order.condition().bytes());
    digest.update(order.token().units().to_be_bytes());
    encode_side(digest, order.side());
    digest.update(order.original_size().protocol_units().to_be_bytes());
    match order.size_matched() {
        PmBookQuantity::Delete => digest.update([0]),
        PmBookQuantity::Quantity(quantity) => {
            digest.update([1]);
            digest.update(quantity.protocol_units().to_be_bytes());
        }
    }
    digest.update(order.price().units().to_be_bytes());
    encode_reconciliation_bytes(digest, order.status().as_bytes());
    digest.update(order.maker().bytes());
    digest.update(order.created_at().to_be_bytes());
    digest.update(order.expiration().to_be_bytes());
    encode_optional_reconciliation_ascii(digest, order.outcome());
    encode_optional_reconciliation_ascii(digest, order.order_type());
}

fn encode_side(digest: &mut Sha256, side: PmOrderSide) {
    digest.update([match side {
        PmOrderSide::Buy => 0,
        PmOrderSide::Sell => 1,
    }]);
}

fn encode_optional_order_id(digest: &mut Sha256, order_id: Option<reap_pm_core::PmVenueOrderId>) {
    match order_id {
        Some(order_id) => {
            digest.update([1]);
            encode_reconciliation_bytes(digest, order_id.as_str().as_bytes());
        }
        None => digest.update([0]),
    }
}

fn encode_optional_u64(digest: &mut Sha256, value: Option<u64>) {
    match value {
        Some(value) => {
            digest.update([1]);
            digest.update(value.to_be_bytes());
        }
        None => digest.update([0]),
    }
}

const fn origin_mode_name(mode: OriginMode) -> &'static [u8] {
    match mode {
        OriginMode::Production => b"production",
        #[cfg(any(test, feature = "loopback-evidence", feature = "read-only-evidence"))]
        OriginMode::LocalEvidence => b"local-evidence",
    }
}

fn ensure_live_source_binding(
    source: Option<&LiveCutSource>,
    expected: ReconciliationSourceBinding,
) -> Result<(), PmLiveAdapterError> {
    if source.is_some_and(|source| source.binding == expected) {
        Ok(())
    } else {
        Err(PmLiveAdapterError::InvalidConfiguration(
            "authenticated reconciliation assembly belongs to another source",
        ))
    }
}

fn ensure_live_source_shape(
    source: Option<&LiveCutSource>,
    page_count: usize,
    requested_cursors: &[String],
) -> Result<(), PmLiveAdapterError> {
    let Some(source) = source else {
        return Ok(());
    };
    if source.pages.len() != page_count
        || requested_cursors.len() != page_count
        || source
            .pages
            .iter()
            .zip(requested_cursors)
            .any(|(page, cursor)| page.requested_cursor.as_ref() != cursor.as_str())
    {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "authenticated reconciliation source chain is inconsistent",
        ));
    }
    Ok(())
}

fn ensure_complete_source<P: ReconciliationPage>(
    source: &LiveCutSource,
    expected: ReconciliationSourceBinding,
    pages: &[P],
) -> Result<(), PmLiveAdapterError> {
    ensure_live_source_binding(Some(source), expected)?;
    if pages.is_empty()
        || source.pages.len() != pages.len()
        || source.pages[0].requested_cursor.as_ref() != first_page_cursor()
        || !pages.last().is_some_and(|page| page.terminal())
    {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "authenticated reconciliation source chain is not terminal",
        ));
    }
    for index in 1..pages.len() {
        if pages[index - 1].next_cursor() != Some(source.pages[index].requested_cursor.as_ref()) {
            return Err(PmLiveAdapterError::InvalidConfiguration(
                "authenticated reconciliation source cursor order is inconsistent",
            ));
        }
    }
    Ok(())
}

fn advance_open_orders(
    pages: Vec<PmLiveOpenOrderPage>,
    requested_cursors: Vec<String>,
    prior_rows: usize,
    live_source: Option<LiveCutSource>,
) -> Result<PmOpenOrdersCutProgress, PmLiveAdapterError> {
    ensure_live_source_shape(live_source.as_ref(), pages.len(), &requested_cursors)?;
    let page = pages.last().expect("advance always has a page");
    ensure_nonterminal_clock_state(live_source.as_ref(), page.terminal())?;
    let row_count = checked_rows(
        prior_rows,
        page.orders().len(),
        MAX_PM_AUTHENTICATED_ORDER_ROWS,
    )?;
    let next_cursor = page.next_cursor().map(|cursor| cursor.as_str().to_owned());
    match next_cursor {
        None => Ok(PmOpenOrdersCutProgress::Complete(PmCompleteOpenOrdersCut {
            pages: pages.into_boxed_slice(),
            row_count,
            live_source,
        })),
        Some(next_cursor) => {
            enforce_continuation(&pages, &requested_cursors, &next_cursor)?;
            Ok(PmOpenOrdersCutProgress::Incomplete(PmOpenOrdersAssembly {
                pages,
                requested_cursors,
                next_cursor,
                row_count,
                live_source,
            }))
        }
    }
}

fn advance_trades(
    pages: Vec<PmLiveTradePage>,
    requested_cursors: Vec<String>,
    prior_rows: usize,
    live_source: Option<LiveCutSource>,
) -> Result<PmTradesCutProgress, PmLiveAdapterError> {
    ensure_live_source_shape(live_source.as_ref(), pages.len(), &requested_cursors)?;
    let page = pages.last().expect("advance always has a page");
    ensure_nonterminal_clock_state(live_source.as_ref(), page.terminal())?;
    let row_count = checked_rows(
        prior_rows,
        page.trades().len(),
        MAX_PM_AUTHENTICATED_TRADE_ROWS,
    )?;
    let next_cursor = page.next_cursor().map(|cursor| cursor.as_str().to_owned());
    match next_cursor {
        None => Ok(PmTradesCutProgress::Complete(PmCompleteTradesCut {
            pages: pages.into_boxed_slice(),
            row_count,
            live_source,
        })),
        Some(next_cursor) => {
            enforce_continuation(&pages, &requested_cursors, &next_cursor)?;
            Ok(PmTradesCutProgress::Incomplete(PmTradesAssembly {
                pages,
                requested_cursors,
                next_cursor,
                row_count,
                live_source,
            }))
        }
    }
}

fn ensure_nonterminal_clock_state(
    source: Option<&LiveCutSource>,
    terminal: bool,
) -> Result<(), PmLiveAdapterError> {
    if !terminal && source.is_some_and(|source| source.terminal_receive_clock.is_some()) {
        return Err(PmLiveAdapterError::InvalidConfiguration(
            "nonterminal reconciliation source cannot carry a terminal clock",
        ));
    }
    Ok(())
}

fn enforce_continuation<T>(
    pages: &[T],
    requested_cursors: &[String],
    next_cursor: &str,
) -> Result<(), PmLiveAdapterError> {
    if pages.len() >= MAX_PM_AUTHENTICATED_CUT_PAGES {
        return Err(PmLiveAdapterError::PaginationPageLimit);
    }
    if requested_cursors.iter().any(|cursor| cursor == next_cursor) {
        return Err(PmLiveAdapterError::PaginationCursorCycle);
    }
    Ok(())
}

fn checked_rows(
    prior: usize,
    additional: usize,
    maximum: usize,
) -> Result<usize, PmLiveAdapterError> {
    prior
        .checked_add(additional)
        .filter(|total| *total <= maximum)
        .ok_or(PmLiveAdapterError::PaginationRowLimit)
}

#[cfg(feature = "test-support")]
fn complete_open_orders_for_test(
    pages: Box<[PmLiveOpenOrderPage]>,
) -> Option<PmCompleteOpenOrdersCut> {
    if pages.is_empty()
        || pages.len() > MAX_PM_AUTHENTICATED_CUT_PAGES
        || !pages.last().is_some_and(PmLiveOpenOrderPage::terminal)
        || pages[..pages.len() - 1]
            .iter()
            .any(PmLiveOpenOrderPage::terminal)
    {
        return None;
    }
    let row_count = pages.iter().try_fold(0_usize, |total, page| {
        checked_rows(total, page.orders().len(), MAX_PM_AUTHENTICATED_ORDER_ROWS).ok()
    })?;
    Some(PmCompleteOpenOrdersCut {
        pages,
        row_count,
        live_source: None,
    })
}

#[cfg(feature = "test-support")]
fn complete_trades_for_test(pages: Box<[PmLiveTradePage]>) -> Option<PmCompleteTradesCut> {
    if pages.is_empty()
        || pages.len() > MAX_PM_AUTHENTICATED_CUT_PAGES
        || !pages.last().is_some_and(PmLiveTradePage::terminal)
        || pages[..pages.len() - 1]
            .iter()
            .any(PmLiveTradePage::terminal)
    {
        return None;
    }
    let row_count = pages.iter().try_fold(0_usize, |total, page| {
        checked_rows(total, page.trades().len(), MAX_PM_AUTHENTICATED_TRADE_ROWS).ok()
    })?;
    Some(PmCompleteTradesCut {
        pages,
        row_count,
        live_source: None,
    })
}

fn found(observation: PmPrivateHttpObservation) -> Result<Zeroizing<Vec<u8>>, PmLiveAdapterError> {
    match observation {
        PmPrivateHttpObservation::Found(body) => Ok(body),
        PmPrivateHttpObservation::NotFound => {
            Err(PmLiveAdapterError::UnexpectedStatus { status: 404 })
        }
    }
}

#[cfg(test)]
mod tests {
    use reap_pm_core::{PmConditionId, PmMarketId, PmTokenId, U256};
    use reap_polymarket_auth::FixedOrderId;
    use reap_polymarket_wire::{parse_live_open_order_page, parse_live_trade_page};

    use super::*;

    fn open_page(cursor: &str) -> PmLiveOpenOrderPage {
        parse_live_open_order_page(
            format!(r#"{{"data":[],"next_cursor":"{cursor}","limit":128,"count":0}}"#).as_bytes(),
        )
        .unwrap()
    }

    fn trade_page(cursor: &str) -> PmLiveTradePage {
        parse_live_trade_page(
            format!(r#"{{"data":[],"next_cursor":"{cursor}","limit":128,"count":0}}"#).as_bytes(),
        )
        .unwrap()
    }

    fn source_binding() -> ReconciliationSourceBinding {
        ReconciliationSourceBinding {
            mode: OriginMode::LocalEvidence,
            authenticated_signer: EvmAddress::parse("0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266")
                .unwrap()
                .bytes(),
            signature_type: PmReadOnlySignatureType::Proxy,
            configured_expected_maker: EvmAddress::parse(
                "0x2222222222222222222222222222222222222222",
            )
            .unwrap(),
            scope: PmWireScope::new(
                PmConditionId::parse(
                    "0x1111111111111111111111111111111111111111111111111111111111111111",
                )
                .unwrap(),
                PmMarketId::parse(
                    "0x9999999999999999999999999999999999999999999999999999999999999999",
                )
                .unwrap(),
                PmTokenId::new(U256::from_u64(123)).unwrap(),
            ),
        }
    }

    fn private_read_edges(readings: &[(u64, u64)]) -> Vec<PmPrivateReadEdgeClock> {
        let owner = crate::PmProductClockOwner::test_support_scripted(readings).unwrap();
        let (_, _, _, _, mut clock, _, _, _, _, _) = owner.split().into_views();
        readings
            .iter()
            .map(|_| clock.observe_authenticated_read_complete().unwrap())
            .collect()
    }

    #[test]
    fn cursor_cycles_and_page_caps_cannot_produce_incomplete_authority() {
        assert!(matches!(
            advance_open_orders(vec![open_page("MA==")], vec!["MA==".into()], 0, None),
            Err(PmLiveAdapterError::PaginationCursorCycle)
        ));
        assert!(matches!(
            advance_trades(
                vec![trade_page("cursor-1")],
                vec!["MA==".into(), "cursor-1".into()],
                0,
                None,
            ),
            Err(PmLiveAdapterError::PaginationCursorCycle)
        ));

        let pages = (0..MAX_PM_AUTHENTICATED_CUT_PAGES)
            .map(|_| open_page("next"))
            .collect();
        assert!(matches!(
            advance_open_orders(pages, vec!["MA==".into()], 0, None),
            Err(PmLiveAdapterError::PaginationPageLimit)
        ));
    }

    #[test]
    fn only_exact_terminal_cursor_constructs_a_complete_cut() {
        let progress = advance_open_orders(vec![open_page("LTE=")], vec!["MA==".into()], 0, None)
            .expect("terminal cut");
        let PmOpenOrdersCutProgress::Complete(complete) = progress else {
            panic!("terminal cursor must complete")
        };
        assert_eq!(complete.pages().len(), 1);
        assert_eq!(complete.row_count(), 0);

        let progress = advance_trades(vec![trade_page("next")], vec!["MA==".into()], 0, None)
            .expect("nonterminal cut");
        let PmTradesCutProgress::Incomplete(incomplete) = progress else {
            panic!("nonterminal cursor must remain incomplete")
        };
        assert_eq!(incomplete.pages_received(), 1);
        assert_eq!(incomplete.rows_received(), 0);

        assert_eq!(
            checked_rows(
                MAX_PM_AUTHENTICATED_ORDER_ROWS,
                1,
                MAX_PM_AUTHENTICATED_ORDER_ROWS
            ),
            Err(PmLiveAdapterError::PaginationRowLimit)
        );
        assert_eq!(
            checked_rows(
                MAX_PM_AUTHENTICATED_TRADE_ROWS,
                1,
                MAX_PM_AUTHENTICATED_TRADE_ROWS
            ),
            Err(PmLiveAdapterError::PaginationRowLimit)
        );
    }

    #[test]
    fn page_sources_bind_only_canonical_typed_projection_and_public_source_facts() {
        let binding = source_binding();
        let canonical = parse_live_open_order_page(
            br#"{"data":[],"next_cursor":"LTE=","limit":128,"count":0}"#,
        )
        .unwrap();
        let whitespace_variant = parse_live_open_order_page(
            br#"{ "data": [], "next_cursor": "LTE=", "limit": 128, "count": 0 }"#,
        )
        .unwrap();
        let baseline = authenticated_page_source(
            OPEN_ORDERS_PAGE_SOURCE_COMMITMENT_DOMAIN,
            b"/data/orders",
            binding,
            first_page_cursor(),
            &canonical,
        );
        let whitespace_variant = authenticated_page_source(
            OPEN_ORDERS_PAGE_SOURCE_COMMITMENT_DOMAIN,
            b"/data/orders",
            binding,
            first_page_cursor(),
            &whitespace_variant,
        );
        assert_eq!(
            baseline.source_commitment,
            whitespace_variant.source_commitment
        );
        let cursor_mutation = authenticated_page_source(
            OPEN_ORDERS_PAGE_SOURCE_COMMITMENT_DOMAIN,
            b"/data/orders",
            binding,
            "cursor-1",
            &canonical,
        );
        let route_mutation = authenticated_page_source(
            OPEN_ORDERS_PAGE_SOURCE_COMMITMENT_DOMAIN,
            b"/data/trades",
            binding,
            first_page_cursor(),
            &canonical,
        );
        let mut profile_mutation = binding;
        profile_mutation.signature_type = PmReadOnlySignatureType::Eoa;
        let profile_mutation = authenticated_page_source(
            OPEN_ORDERS_PAGE_SOURCE_COMMITMENT_DOMAIN,
            b"/data/orders",
            profile_mutation,
            first_page_cursor(),
            &canonical,
        );

        for mutation in [cursor_mutation, route_mutation, profile_mutation] {
            assert_ne!(baseline.source_commitment, mutation.source_commitment);
        }
        assert_ne!(
            baseline.source_commitment,
            authenticated_page_source(
                TRADES_PAGE_SOURCE_COMMITMENT_DOMAIN,
                b"/data/orders",
                binding,
                first_page_cursor(),
                &canonical,
            )
            .source_commitment
        );
    }

    #[test]
    fn complete_cut_commitment_binds_ordered_pages_typed_shape_and_terminal_clock() {
        let binding = source_binding();
        let pages = vec![open_page("cursor-1"), open_page("LTE=")];
        let [receive, later] = private_read_edges(&[(1_000, 10), (1_001, 11)])
            .try_into()
            .unwrap();
        let source = LiveCutSource {
            binding,
            pages: vec![
                authenticated_page_source(
                    OPEN_ORDERS_PAGE_SOURCE_COMMITMENT_DOMAIN,
                    b"/data/orders",
                    binding,
                    first_page_cursor(),
                    &pages[0],
                ),
                authenticated_page_source(
                    OPEN_ORDERS_PAGE_SOURCE_COMMITMENT_DOMAIN,
                    b"/data/orders",
                    binding,
                    "cursor-1",
                    &pages[1],
                ),
            ],
            terminal_receive_clock: Some(receive),
        };
        ensure_complete_source(&source, binding, &pages).unwrap();
        let cut = PmCompleteOpenOrdersCut {
            pages: pages.into_boxed_slice(),
            row_count: 0,
            live_source: None,
        };
        let baseline = complete_open_orders_observation_commitment(&source, &cut, receive);
        assert_ne!(
            baseline,
            complete_open_orders_observation_commitment(&source, &cut, later)
        );

        let typed_shape_mutation = PmCompleteOpenOrdersCut {
            pages: vec![
                parse_live_open_order_page(
                    br#"{"data":[],"next_cursor":"cursor-1","limit":127,"count":0}"#,
                )
                .unwrap(),
                open_page("LTE="),
            ]
            .into_boxed_slice(),
            row_count: 0,
            live_source: None,
        };
        assert_ne!(
            baseline,
            complete_open_orders_observation_commitment(&source, &typed_shape_mutation, receive,)
        );

        let mut reordered_source = source;
        reordered_source.pages.swap(0, 1);
        assert!(matches!(
            ensure_complete_source(&reordered_source, binding, &cut.pages),
            Err(PmLiveAdapterError::InvalidConfiguration(_))
        ));
    }

    #[test]
    fn exact_order_commitment_binds_status_id_classification_and_terminal_clock() {
        let binding = source_binding();
        let id = FixedOrderId::parse(
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap();
        let other_id = FixedOrderId::parse(
            "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .unwrap();
        let maker = binding.configured_expected_maker;
        let absent_classification = PmExactOrderObservation::Absent;
        let absent = FetchedExactOrder {
            classification: PmExactOrderObservation::Absent,
            source_commitment: exact_order_source_commitment(
                binding,
                id,
                404,
                &absent_classification,
            ),
            response_status: 404,
        };
        let status_mutation = FetchedExactOrder {
            classification: PmExactOrderObservation::Absent,
            source_commitment: exact_order_source_commitment(
                binding,
                id,
                410,
                &absent_classification,
            ),
            response_status: 410,
        };
        let order = parse_live_order_detail(
            format!(
                r#"{{"id":"{id}","market":"{}","asset_id":"123","side":"BUY","original_size":"10.000000","size_matched":"0","price":"0.420000","status":"LIVE","maker_address":"{maker}","owner":"00000000-0000-4000-8000-000000000001","expiration":"0","created_at":1700000000}}"#,
                binding.scope.condition()
            )
            .as_bytes(),
        )
        .unwrap();
        let present_classification = PmExactOrderObservation::Present(Box::new(order));
        let present = FetchedExactOrder {
            source_commitment: exact_order_source_commitment(
                binding,
                id,
                200,
                &present_classification,
            ),
            classification: present_classification,
            response_status: 200,
        };
        let [receive, later] = private_read_edges(&[(1_000, 10), (1_001, 11)])
            .try_into()
            .unwrap();
        let baseline = exact_order_observation_commitment(binding, maker, id, &absent, receive);

        assert_ne!(
            baseline,
            exact_order_observation_commitment(binding, maker, id, &status_mutation, receive)
        );
        assert_ne!(
            baseline,
            exact_order_observation_commitment(binding, maker, other_id, &absent, receive)
        );
        assert_ne!(
            baseline,
            exact_order_observation_commitment(binding, maker, id, &present, receive)
        );
        assert_ne!(
            baseline,
            exact_order_observation_commitment(binding, maker, id, &absent, later)
        );
    }

    #[test]
    fn source_less_and_nonterminal_clocked_cuts_fail_closed() {
        let progress =
            advance_open_orders(vec![open_page("LTE=")], vec!["MA==".into()], 0, None).unwrap();
        let PmOpenOrdersCutProgress::Complete(cut) = progress else {
            panic!("terminal page completes")
        };
        assert!(cut.live_source.is_none());

        let binding = source_binding();
        let receive = private_read_edges(&[(1_000, 10)])[0];
        let source = LiveCutSource {
            binding,
            pages: vec![authenticated_page_source(
                OPEN_ORDERS_PAGE_SOURCE_COMMITMENT_DOMAIN,
                b"/data/orders",
                binding,
                first_page_cursor(),
                &open_page("next"),
            )],
            terminal_receive_clock: Some(receive),
        };
        assert!(matches!(
            advance_open_orders(
                vec![open_page("next")],
                vec![first_page_cursor().into()],
                0,
                Some(source),
            ),
            Err(PmLiveAdapterError::InvalidConfiguration(_))
        ));
    }
}
