use reap_pm_core::{
    ConnectionEpoch, EventEnvelope, IngressSequence, MAX_PM_RECONCILIATION_FILLS,
    MAX_PM_RECONCILIATION_ORDERS, PmAccountHandle, PmAccountScope, PmAggregateError,
    PmCompleteFillQuery, PmCompleteOpenOrdersSnapshot, PmConnectionId, PmExactOrderDetail,
    PmFillEvent, PmFillQueryCursor, PmOrderEvent, PmProductSource, PmReconciliationRequestBoundary,
    PmSnapshotEvidence, PmVenueOrderKey,
};
use reap_polymarket_wire::{
    PmFixtureOpenOrder, PmFixtureUserEvent, PmFixtureUserFrame, parse_open_order_fixture,
    parse_private_user_fixture,
};
use thiserror::Error;

use crate::fixture_delivery::{
    PmFixtureRequestOccurrence, checked_delivery, validate_completion, validate_next_request,
};
use crate::fixture_scope::PmFixtureOwnerId;
use crate::fixture_scope::validate_account_source;
use crate::private_fixture::{normalize_open_order, normalize_order_detail};
use crate::{
    PmFixtureAggregateDelivery, PmFixtureCompletionOccurrence, PmFixtureDeliveryError,
    PmFixtureDeliveryScope, PmFixtureFeeEvidence, PmFixtureInstrumentScope,
    PmFixturePrivateLifecycle, PmFixtureReconciliationRoleGrant, PmFixtureScopeError,
    PmFixtureServicedAggregate, PmPrivateLifecycleObservation, PmPrivateNormalizationError,
};

pub const MAX_PM_FIXTURE_QUERY_PAGES: usize = 128;

pub type PmCompleteOpenOrdersDelivery = PmFixtureAggregateDelivery<PmCompleteOpenOrdersSnapshot>;
pub type PmExactOrderDetailDelivery = PmFixtureAggregateDelivery<PmExactOrderDetail>;
pub type PmCompleteFillQueryDelivery = PmFixtureAggregateDelivery<PmCompleteFillQuery>;

mod sealed {
    pub trait Sealed {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PmReconciliationContractError {
    #[error("reconciliation role requires a Polymarket account source")]
    WrongSource,
    #[error("reconciliation role source belongs to another account")]
    SourceAccountMismatch,
    #[error("reconciliation request ticket is invalid: {0}")]
    Delivery(#[from] PmFixtureDeliveryError),
    #[error("private reconciliation normalization failed: {0}")]
    Normalization(#[from] PmPrivateNormalizationError),
    #[error("complete aggregate contract failed: {0}")]
    Aggregate(#[from] PmAggregateError),
    #[error("requested order belongs to another exact account scope")]
    RequestedOrderAccountMismatch,
    #[error("fill cursor belongs to another exact account scope")]
    CursorAccountScopeMismatch,
    #[error("fixture page belongs to another causal request")]
    PageRequestMismatch,
    #[error("fixture page carries another snapshot revision")]
    PageSnapshotMismatch,
    #[error("fixture page cursor chain is broken")]
    BrokenCursorChain,
    #[error("fixture page cursor chain contains a cycle")]
    CursorCycle,
    #[error("fixture page arrived after the terminal page")]
    PageAfterTerminal,
    #[error("fixture query has no page and cannot prove explicit emptiness")]
    MissingPage,
    #[error("fixture query did not reach a terminal cursor")]
    MissingTerminalPage,
    #[error("terminal fill page lacks a resulting watermark")]
    MissingResultingWatermark,
    #[error("nonterminal fill page carries a resulting watermark")]
    PrematureResultingWatermark,
    #[error("fixture query exceeds its fixed page bound")]
    TooManyPages,
    #[error("fixture reconciliation row belongs to another configured instrument")]
    InstrumentMismatch,
    #[error("fill query contained a non-fill private observation")]
    NonFillObservation,
    #[error("fill query contained an unresolved private trade")]
    UnresolvedTrade,
}

/// Fixture-only, read-only order reconciliation capability.
pub trait PmReconciliationRole: sealed::Sealed {
    type CompleteOpenOrders;
    type ExactOrderDetail;
    type CompleteFillQuery;

    fn account_scope(&self) -> PmAccountScope;
    fn account(&self) -> PmAccountHandle;
    fn instrument_scope(&self) -> PmFixtureInstrumentScope;
    fn source(&self) -> PmProductSource;
    fn connection(&self) -> PmConnectionId;
}

#[derive(Debug)]
pub struct PmFixtureReconciliation {
    binding: ReconciliationBinding,
    last_request: Option<(ConnectionEpoch, IngressSequence)>,
}

impl PmFixtureReconciliation {
    pub fn new(
        grant: PmFixtureReconciliationRoleGrant,
        account_scope: PmAccountScope,
        instrument: PmFixtureInstrumentScope,
        source: PmProductSource,
        connection: PmConnectionId,
    ) -> Result<Self, PmReconciliationContractError> {
        validate_role_source(account_scope, source)?;
        Ok(Self {
            binding: ReconciliationBinding {
                owner_id: grant.into_owner_id(),
                account_scope,
                instrument,
                source,
                connection,
            },
            last_request: None,
        })
    }

    #[must_use]
    pub const fn account_scope(&self) -> PmAccountScope {
        self.binding.account_scope
    }

    #[must_use]
    pub const fn account(&self) -> PmAccountHandle {
        self.binding.account_scope.handle()
    }

    #[must_use]
    pub const fn instrument_scope(&self) -> PmFixtureInstrumentScope {
        self.binding.instrument
    }

    #[must_use]
    pub const fn source(&self) -> PmProductSource {
        self.binding.source
    }

    #[must_use]
    pub const fn connection(&self) -> PmConnectionId {
        self.binding.connection
    }

    pub fn request_open_orders(
        &mut self,
        connection_epoch: ConnectionEpoch,
        request_sequence: IngressSequence,
    ) -> Result<PmFixtureOpenOrdersRequest, PmReconciliationContractError> {
        let request = self.issue_request(connection_epoch, request_sequence)?;
        Ok(PmFixtureOpenOrdersRequest {
            binding: self.binding,
            request,
        })
    }

    pub fn request_order_detail(
        &mut self,
        connection_epoch: ConnectionEpoch,
        request_sequence: IngressSequence,
        requested_order: PmVenueOrderKey,
    ) -> Result<PmFixtureOrderDetailRequest, PmReconciliationContractError> {
        if requested_order.account() != self.account() {
            return Err(PmReconciliationContractError::RequestedOrderAccountMismatch);
        }
        let request = self.issue_request(connection_epoch, request_sequence)?;
        Ok(PmFixtureOrderDetailRequest {
            binding: self.binding,
            request,
            requested_order,
        })
    }

    pub fn request_fills(
        &mut self,
        connection_epoch: ConnectionEpoch,
        request_sequence: IngressSequence,
        requested_after: Option<PmFillQueryCursor>,
    ) -> Result<PmFixtureFillQueryRequest, PmReconciliationContractError> {
        if requested_after.is_some_and(|cursor| cursor.account_scope() != self.account_scope()) {
            return Err(PmReconciliationContractError::CursorAccountScopeMismatch);
        }
        let request = self.issue_request(connection_epoch, request_sequence)?;
        Ok(PmFixtureFillQueryRequest {
            binding: self.binding,
            request,
            requested_after,
        })
    }

    fn issue_request(
        &mut self,
        connection_epoch: ConnectionEpoch,
        request_sequence: IngressSequence,
    ) -> Result<PmFixtureRequestOccurrence, PmReconciliationContractError> {
        let request = validate_next_request(self.last_request, connection_epoch, request_sequence)?;
        self.last_request = Some((connection_epoch, request_sequence));
        Ok(request)
    }

    pub fn reduce_open_orders_delivery<R>(
        &self,
        delivery: PmFixtureServicedAggregate<PmCompleteOpenOrdersSnapshot>,
        reduce: impl FnOnce(PmFixtureDeliveryScope, EventEnvelope<PmCompleteOpenOrdersSnapshot>) -> R,
    ) -> Result<R, Box<PmFixtureServicedAggregate<PmCompleteOpenOrdersSnapshot>>> {
        delivery.reduce_with_owner(self.binding.owner_id, reduce)
    }

    pub fn reduce_order_detail_delivery<R>(
        &self,
        delivery: PmFixtureServicedAggregate<PmExactOrderDetail>,
        reduce: impl FnOnce(PmFixtureDeliveryScope, EventEnvelope<PmExactOrderDetail>) -> R,
    ) -> Result<R, Box<PmFixtureServicedAggregate<PmExactOrderDetail>>> {
        delivery.reduce_with_owner(self.binding.owner_id, reduce)
    }

    pub fn reduce_fill_query_delivery<R>(
        &self,
        delivery: PmFixtureServicedAggregate<PmCompleteFillQuery>,
        reduce: impl FnOnce(PmFixtureDeliveryScope, EventEnvelope<PmCompleteFillQuery>) -> R,
    ) -> Result<R, Box<PmFixtureServicedAggregate<PmCompleteFillQuery>>> {
        delivery.reduce_with_owner(self.binding.owner_id, reduce)
    }
}

impl sealed::Sealed for PmFixtureReconciliation {}

impl PmReconciliationRole for PmFixtureReconciliation {
    type CompleteOpenOrders = PmCompleteOpenOrdersDelivery;
    type ExactOrderDetail = PmExactOrderDetailDelivery;
    type CompleteFillQuery = PmCompleteFillQueryDelivery;

    fn account_scope(&self) -> PmAccountScope {
        self.binding.account_scope
    }

    fn account(&self) -> PmAccountHandle {
        self.binding.account_scope.handle()
    }

    fn instrument_scope(&self) -> PmFixtureInstrumentScope {
        self.binding.instrument
    }

    fn source(&self) -> PmProductSource {
        self.binding.source
    }

    fn connection(&self) -> PmConnectionId {
        self.binding.connection
    }
}

#[derive(Debug)]
pub struct PmFixtureOpenOrdersRequest {
    binding: ReconciliationBinding,
    request: PmFixtureRequestOccurrence,
}

impl PmFixtureOpenOrdersRequest {
    #[must_use]
    pub fn begin(self, snapshot: PmSnapshotEvidence) -> PmFixtureOpenOrdersAssembly {
        PmFixtureOpenOrdersAssembly {
            binding: self.binding,
            assembler: OpenOrdersAssembler::new(self.binding, self.request, snapshot),
        }
    }

    pub fn complete(
        self,
        completion: PmFixtureCompletionOccurrence,
        snapshot: PmSnapshotEvidence,
        orders: &[PmFixtureOpenOrder],
    ) -> Result<PmCompleteOpenOrdersDelivery, PmReconciliationContractError> {
        let mut assembly = self.begin(snapshot);
        assembly.push_page(None, None, orders)?;
        assembly.finish(completion)
    }

    pub fn complete_json_objects(
        self,
        completion: PmFixtureCompletionOccurrence,
        snapshot: PmSnapshotEvidence,
        orders: &[&[u8]],
    ) -> Result<PmCompleteOpenOrdersDelivery, PmReconciliationContractError> {
        let mut assembly = self.begin(snapshot);
        assembly.push_json_page(None, None, orders)?;
        assembly.finish(completion)
    }
}

/// Move-only bounded assembly of one complete open-order cursor chain.
pub struct PmFixtureOpenOrdersAssembly {
    binding: ReconciliationBinding,
    assembler: OpenOrdersAssembler,
}

impl PmFixtureOpenOrdersAssembly {
    pub fn push_page(
        &mut self,
        requested_cursor: Option<[u8; 32]>,
        next_cursor: Option<[u8; 32]>,
        orders: &[PmFixtureOpenOrder],
    ) -> Result<(), PmReconciliationContractError> {
        self.assembler
            .preflight_page(requested_cursor, next_cursor, orders.len())?;
        let normalizer = self.binding.normalizer();
        let orders = orders
            .iter()
            .map(|order| normalize_open_order(&normalizer, order))
            .collect::<Result<Vec<_>, _>>()?;
        self.assembler.push_page(
            self.assembler.request_sequence(),
            self.assembler.snapshot,
            requested_cursor,
            next_cursor,
            orders,
        )
    }

    pub fn push_json_page(
        &mut self,
        requested_cursor: Option<[u8; 32]>,
        next_cursor: Option<[u8; 32]>,
        orders: &[&[u8]],
    ) -> Result<(), PmReconciliationContractError> {
        self.assembler
            .preflight_page(requested_cursor, next_cursor, orders.len())?;
        let parsed = orders
            .iter()
            .map(|raw| parse_open_order_fixture(raw))
            .collect::<Result<Vec<_>, _>>()
            .map_err(PmPrivateNormalizationError::from)?;
        self.push_page(requested_cursor, next_cursor, &parsed)
    }

    pub fn finish(
        self,
        completion: PmFixtureCompletionOccurrence,
    ) -> Result<PmCompleteOpenOrdersDelivery, PmReconciliationContractError> {
        let payload = self.assembler.finish(&completion)?;
        self.binding.delivery(completion, payload)
    }
}

#[derive(Debug)]
pub struct PmFixtureOrderDetailRequest {
    binding: ReconciliationBinding,
    request: PmFixtureRequestOccurrence,
    requested_order: PmVenueOrderKey,
}

impl PmFixtureOrderDetailRequest {
    pub fn complete_json_object(
        self,
        completion: PmFixtureCompletionOccurrence,
        snapshot: PmSnapshotEvidence,
        order: Option<&[u8]>,
    ) -> Result<PmExactOrderDetailDelivery, PmReconciliationContractError> {
        let parsed = order
            .map(parse_open_order_fixture)
            .transpose()
            .map_err(PmPrivateNormalizationError::from)?;
        self.complete(completion, snapshot, parsed.as_ref())
    }

    pub fn complete(
        self,
        completion: PmFixtureCompletionOccurrence,
        snapshot: PmSnapshotEvidence,
        order: Option<&PmFixtureOpenOrder>,
    ) -> Result<PmExactOrderDetailDelivery, PmReconciliationContractError> {
        let normalizer = self.binding.normalizer();
        let order = order
            .map(|order| normalize_order_detail(&normalizer, order))
            .transpose()?;
        let assembler =
            OrderDetailAssembler::new(self.binding, self.request, snapshot, self.requested_order);
        let payload = assembler.finish(&completion, order)?;
        self.binding.delivery(completion, payload)
    }
}

#[derive(Debug)]
pub struct PmFixtureFillQueryRequest {
    binding: ReconciliationBinding,
    request: PmFixtureRequestOccurrence,
    requested_after: Option<PmFillQueryCursor>,
}

impl PmFixtureFillQueryRequest {
    #[must_use]
    pub fn begin(self, snapshot: PmSnapshotEvidence) -> PmFixtureFillQueryAssembly {
        PmFixtureFillQueryAssembly {
            binding: self.binding,
            assembler: FillQueryAssembler::new(
                self.binding,
                self.request,
                snapshot,
                self.requested_after,
            ),
        }
    }

    pub fn complete_user_frames(
        self,
        completion: PmFixtureCompletionOccurrence,
        snapshot: PmSnapshotEvidence,
        resulting_watermark: PmFillQueryCursor,
        raw_frames: &[&[u8]],
        fee: PmFixtureFeeEvidence,
    ) -> Result<PmCompleteFillQueryDelivery, PmReconciliationContractError> {
        let requested_after = self.requested_after;
        let mut assembly = self.begin(snapshot);
        assembly.push_user_frame_page(
            requested_after,
            None,
            Some(resulting_watermark),
            raw_frames,
            fee,
        )?;
        assembly.finish(completion)
    }
}

/// Move-only bounded assembly of one complete fill-query cursor chain.
pub struct PmFixtureFillQueryAssembly {
    binding: ReconciliationBinding,
    assembler: FillQueryAssembler,
}

impl PmFixtureFillQueryAssembly {
    pub fn push_user_frame_page(
        &mut self,
        requested_after: Option<PmFillQueryCursor>,
        next_after: Option<PmFillQueryCursor>,
        resulting_watermark: Option<PmFillQueryCursor>,
        raw_frames: &[&[u8]],
        fee: PmFixtureFeeEvidence,
    ) -> Result<(), PmReconciliationContractError> {
        self.assembler.preflight_page(
            requested_after,
            next_after,
            resulting_watermark,
            raw_frames.len(),
        )?;
        let remaining_fills = self.assembler.remaining_fills();
        let normalizer = self.binding.normalizer();
        let mut fills = Vec::with_capacity(raw_frames.len().min(remaining_fills));
        for raw in raw_frames {
            let frame =
                parse_private_user_fixture(raw).map_err(PmPrivateNormalizationError::from)?;
            let frame_observations = fixture_observation_count(&frame);
            if fills.len().saturating_add(frame_observations) > remaining_fills {
                return Err(PmAggregateError::TooManyFills.into());
            }
            let batch = normalizer.normalize_user_frame(&frame, fee)?;
            for observation in batch.observations() {
                match observation {
                    PmPrivateLifecycleObservation::Fill(fill) => fills.push(*fill),
                    PmPrivateLifecycleObservation::Order(_) => {
                        return Err(PmReconciliationContractError::NonFillObservation);
                    }
                    PmPrivateLifecycleObservation::UnresolvedTrade(_) => {
                        return Err(PmReconciliationContractError::UnresolvedTrade);
                    }
                }
            }
        }
        self.assembler.push_page(FillPage {
            request_sequence: self.assembler.request_sequence(),
            snapshot: self.assembler.snapshot,
            requested_after,
            next_after,
            resulting_watermark,
            fills,
        })
    }

    pub fn finish(
        self,
        completion: PmFixtureCompletionOccurrence,
    ) -> Result<PmCompleteFillQueryDelivery, PmReconciliationContractError> {
        let payload = self.assembler.finish(&completion)?;
        self.binding.delivery(completion, payload)
    }
}

#[derive(Debug, Clone, Copy)]
struct ReconciliationBinding {
    owner_id: PmFixtureOwnerId,
    account_scope: PmAccountScope,
    instrument: PmFixtureInstrumentScope,
    source: PmProductSource,
    connection: PmConnectionId,
}

impl ReconciliationBinding {
    fn normalizer(self) -> PmFixturePrivateLifecycle {
        PmFixturePrivateLifecycle::new_bound(
            self.owner_id,
            self.account_scope,
            self.instrument,
            self.source,
            self.connection,
        )
        .expect("reconciliation binding was validated at role construction")
    }

    fn delivery<P: reap_pm_core::PmSourceBound>(
        self,
        completion: PmFixtureCompletionOccurrence,
        payload: P,
    ) -> Result<PmFixtureAggregateDelivery<P>, PmReconciliationContractError> {
        Ok(checked_delivery(
            self.owner_id,
            self.account_scope,
            self.instrument,
            self.source,
            self.connection,
            completion,
            payload,
        )?)
    }
}

struct OpenOrdersAssembler {
    binding: ReconciliationBinding,
    request: PmFixtureRequestOccurrence,
    snapshot: PmSnapshotEvidence,
    chain: PageChain<[u8; 32]>,
    orders: Vec<PmOrderEvent>,
}

impl OpenOrdersAssembler {
    fn new(
        binding: ReconciliationBinding,
        request: PmFixtureRequestOccurrence,
        snapshot: PmSnapshotEvidence,
    ) -> Self {
        Self {
            binding,
            request,
            snapshot,
            chain: PageChain::new(None),
            orders: Vec::new(),
        }
    }

    fn request_sequence(&self) -> IngressSequence {
        self.request.sequence()
    }

    fn remaining_orders(&self) -> usize {
        MAX_PM_RECONCILIATION_ORDERS.saturating_sub(self.orders.len())
    }

    fn preflight_page(
        &self,
        requested_cursor: Option<[u8; 32]>,
        next_cursor: Option<[u8; 32]>,
        additional_orders: usize,
    ) -> Result<(), PmReconciliationContractError> {
        self.chain.preflight(requested_cursor, next_cursor)?;
        if additional_orders > self.remaining_orders() {
            return Err(PmAggregateError::TooManyOrders.into());
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn push_page(
        &mut self,
        request_sequence: IngressSequence,
        snapshot: PmSnapshotEvidence,
        requested_cursor: Option<[u8; 32]>,
        next_cursor: Option<[u8; 32]>,
        orders: Vec<PmOrderEvent>,
    ) -> Result<(), PmReconciliationContractError> {
        self.validate_page(request_sequence, snapshot)?;
        self.preflight_page(requested_cursor, next_cursor, orders.len())?;
        if orders
            .iter()
            .any(|order| order.instrument() != self.binding.instrument.handle())
        {
            return Err(PmReconciliationContractError::InstrumentMismatch);
        }
        self.chain.accept(requested_cursor, next_cursor)?;
        self.orders.extend(orders);
        Ok(())
    }

    fn finish(
        self,
        completion: &PmFixtureCompletionOccurrence,
    ) -> Result<PmCompleteOpenOrdersSnapshot, PmReconciliationContractError> {
        self.chain.validate_terminal()?;
        let boundary = self.boundary(completion)?;
        Ok(PmCompleteOpenOrdersSnapshot::new(
            self.binding.source,
            self.binding.account_scope,
            self.snapshot,
            boundary,
            self.orders.into_boxed_slice(),
        )?)
    }

    fn validate_page(
        &self,
        request_sequence: IngressSequence,
        snapshot: PmSnapshotEvidence,
    ) -> Result<(), PmReconciliationContractError> {
        validate_page_identity(&self.request, self.snapshot, request_sequence, snapshot)
    }

    fn boundary(
        &self,
        completion: &PmFixtureCompletionOccurrence,
    ) -> Result<PmReconciliationRequestBoundary, PmReconciliationContractError> {
        let completion_sequence =
            validate_completion(&self.request, completion, self.snapshot.revision())?;
        Ok(PmReconciliationRequestBoundary::new(
            self.request.sequence(),
            completion_sequence,
        )?)
    }
}

struct OrderDetailAssembler {
    binding: ReconciliationBinding,
    request: PmFixtureRequestOccurrence,
    snapshot: PmSnapshotEvidence,
    requested_order: PmVenueOrderKey,
}

impl OrderDetailAssembler {
    fn new(
        binding: ReconciliationBinding,
        request: PmFixtureRequestOccurrence,
        snapshot: PmSnapshotEvidence,
        requested_order: PmVenueOrderKey,
    ) -> Self {
        Self {
            binding,
            request,
            snapshot,
            requested_order,
        }
    }

    fn finish(
        self,
        completion: &PmFixtureCompletionOccurrence,
        order: Option<PmOrderEvent>,
    ) -> Result<PmExactOrderDetail, PmReconciliationContractError> {
        if order.is_some_and(|order| order.instrument() != self.binding.instrument.handle()) {
            return Err(PmReconciliationContractError::InstrumentMismatch);
        }
        let completion_sequence =
            validate_completion(&self.request, completion, self.snapshot.revision())?;
        let boundary =
            PmReconciliationRequestBoundary::new(self.request.sequence(), completion_sequence)?;
        Ok(PmExactOrderDetail::new(
            self.binding.source,
            self.binding.account_scope,
            self.snapshot,
            boundary,
            self.requested_order,
            order,
        )?)
    }
}

struct FillQueryAssembler {
    binding: ReconciliationBinding,
    request: PmFixtureRequestOccurrence,
    snapshot: PmSnapshotEvidence,
    requested_after: Option<PmFillQueryCursor>,
    chain: PageChain<PmFillQueryCursor>,
    resulting_watermark: Option<PmFillQueryCursor>,
    fills: Vec<PmFillEvent>,
}

impl FillQueryAssembler {
    fn new(
        binding: ReconciliationBinding,
        request: PmFixtureRequestOccurrence,
        snapshot: PmSnapshotEvidence,
        requested_after: Option<PmFillQueryCursor>,
    ) -> Self {
        Self {
            binding,
            request,
            snapshot,
            requested_after,
            chain: PageChain::new(requested_after),
            resulting_watermark: None,
            fills: Vec::new(),
        }
    }

    fn request_sequence(&self) -> IngressSequence {
        self.request.sequence()
    }

    fn remaining_fills(&self) -> usize {
        MAX_PM_RECONCILIATION_FILLS.saturating_sub(self.fills.len())
    }

    fn preflight_page(
        &self,
        requested_after: Option<PmFillQueryCursor>,
        next_after: Option<PmFillQueryCursor>,
        resulting_watermark: Option<PmFillQueryCursor>,
        additional_fills: usize,
    ) -> Result<(), PmReconciliationContractError> {
        for cursor in requested_after
            .into_iter()
            .chain(next_after)
            .chain(resulting_watermark)
        {
            if cursor.account_scope() != self.binding.account_scope {
                return Err(PmReconciliationContractError::CursorAccountScopeMismatch);
            }
        }
        if next_after.is_some() && resulting_watermark.is_some() {
            return Err(PmReconciliationContractError::PrematureResultingWatermark);
        }
        if next_after.is_none() && resulting_watermark.is_none() {
            return Err(PmReconciliationContractError::MissingResultingWatermark);
        }
        self.chain.preflight(requested_after, next_after)?;
        if additional_fills > self.remaining_fills() {
            return Err(PmAggregateError::TooManyFills.into());
        }
        Ok(())
    }

    fn push_page(&mut self, page: FillPage) -> Result<(), PmReconciliationContractError> {
        validate_page_identity(
            &self.request,
            self.snapshot,
            page.request_sequence,
            page.snapshot,
        )?;
        self.preflight_page(
            page.requested_after,
            page.next_after,
            page.resulting_watermark,
            page.fills.len(),
        )?;
        if page
            .fills
            .iter()
            .any(|fill| fill.instrument() != self.binding.instrument.handle())
        {
            return Err(PmReconciliationContractError::InstrumentMismatch);
        }
        self.chain.accept(page.requested_after, page.next_after)?;
        if let Some(watermark) = page.resulting_watermark {
            self.resulting_watermark = Some(watermark);
        }
        self.fills.extend(page.fills);
        Ok(())
    }

    fn finish(
        self,
        completion: &PmFixtureCompletionOccurrence,
    ) -> Result<PmCompleteFillQuery, PmReconciliationContractError> {
        self.chain.validate_terminal()?;
        let resulting_watermark = self
            .resulting_watermark
            .ok_or(PmReconciliationContractError::MissingResultingWatermark)?;
        let completion_sequence =
            validate_completion(&self.request, completion, self.snapshot.revision())?;
        let boundary =
            PmReconciliationRequestBoundary::new(self.request.sequence(), completion_sequence)?;
        Ok(PmCompleteFillQuery::new(
            self.binding.source,
            self.binding.account_scope,
            self.snapshot,
            boundary,
            self.requested_after,
            resulting_watermark,
            self.fills.into_boxed_slice(),
        )?)
    }
}

struct FillPage {
    request_sequence: IngressSequence,
    snapshot: PmSnapshotEvidence,
    requested_after: Option<PmFillQueryCursor>,
    next_after: Option<PmFillQueryCursor>,
    resulting_watermark: Option<PmFillQueryCursor>,
    fills: Vec<PmFillEvent>,
}

struct PageChain<C> {
    expected_cursor: Option<C>,
    seen_requested: Vec<C>,
    started: bool,
    terminal: bool,
    page_count: usize,
}

impl<C: Copy + Eq> PageChain<C> {
    fn new(expected_cursor: Option<C>) -> Self {
        Self {
            expected_cursor,
            seen_requested: Vec::new(),
            started: false,
            terminal: false,
            page_count: 0,
        }
    }

    fn preflight(
        &self,
        requested_cursor: Option<C>,
        next_cursor: Option<C>,
    ) -> Result<(), PmReconciliationContractError> {
        if self.terminal {
            return Err(PmReconciliationContractError::PageAfterTerminal);
        }
        if requested_cursor != self.expected_cursor {
            return Err(PmReconciliationContractError::BrokenCursorChain);
        }
        if self.page_count == MAX_PM_FIXTURE_QUERY_PAGES {
            return Err(PmReconciliationContractError::TooManyPages);
        }
        if let Some(cursor) = requested_cursor
            && self.seen_requested.contains(&cursor)
        {
            return Err(PmReconciliationContractError::CursorCycle);
        }
        if next_cursor.is_some_and(|cursor| {
            self.seen_requested.contains(&cursor) || requested_cursor == Some(cursor)
        }) {
            return Err(PmReconciliationContractError::CursorCycle);
        }
        Ok(())
    }

    fn accept(
        &mut self,
        requested_cursor: Option<C>,
        next_cursor: Option<C>,
    ) -> Result<(), PmReconciliationContractError> {
        self.preflight(requested_cursor, next_cursor)?;
        if let Some(cursor) = requested_cursor {
            self.seen_requested.push(cursor);
        }
        self.started = true;
        self.page_count += 1;
        self.expected_cursor = next_cursor;
        self.terminal = next_cursor.is_none();
        Ok(())
    }

    fn validate_terminal(&self) -> Result<(), PmReconciliationContractError> {
        if !self.started {
            Err(PmReconciliationContractError::MissingPage)
        } else if !self.terminal {
            Err(PmReconciliationContractError::MissingTerminalPage)
        } else {
            Ok(())
        }
    }
}

fn fixture_observation_count(frame: &PmFixtureUserFrame) -> usize {
    frame
        .events()
        .iter()
        .map(|event| match event {
            PmFixtureUserEvent::Order(_) => 1,
            PmFixtureUserEvent::Trade(trade) => {
                if trade.trader_side() == Some("MAKER") {
                    trade
                        .maker_orders()
                        .filter(|orders| !orders.is_empty())
                        .map_or(1, <[_]>::len)
                } else {
                    1
                }
            }
        })
        .fold(0_usize, usize::saturating_add)
}

fn validate_page_identity(
    request: &PmFixtureRequestOccurrence,
    snapshot: PmSnapshotEvidence,
    page_request_sequence: IngressSequence,
    page_snapshot: PmSnapshotEvidence,
) -> Result<(), PmReconciliationContractError> {
    if page_request_sequence != request.sequence() {
        return Err(PmReconciliationContractError::PageRequestMismatch);
    }
    if page_snapshot != snapshot {
        return Err(PmReconciliationContractError::PageSnapshotMismatch);
    }
    Ok(())
}

fn validate_role_source(
    account_scope: PmAccountScope,
    source: PmProductSource,
) -> Result<(), PmReconciliationContractError> {
    match validate_account_source(account_scope, source) {
        Ok(()) => Ok(()),
        Err(PmFixtureScopeError::WrongSource) => Err(PmReconciliationContractError::WrongSource),
        Err(PmFixtureScopeError::SourceAccountMismatch) => {
            Err(PmReconciliationContractError::SourceAccountMismatch)
        }
    }
}

#[cfg(test)]
mod tests {
    use reap_pm_core::{ConnectionEpoch, IngressSequence, PmSnapshotEvidence, SnapshotRevision};

    use super::{PmReconciliationContractError, validate_page_identity};
    use crate::fixture_delivery::PmFixtureRequestOccurrence;

    #[test]
    fn private_page_identity_rejects_another_request_or_snapshot() {
        let request =
            PmFixtureRequestOccurrence::new(ConnectionEpoch::new(1), IngressSequence::new(10))
                .unwrap();
        let snapshot = PmSnapshotEvidence::new(SnapshotRevision::new(7)).unwrap();

        assert_eq!(
            validate_page_identity(&request, snapshot, IngressSequence::new(11), snapshot,),
            Err(PmReconciliationContractError::PageRequestMismatch)
        );
        assert_eq!(
            validate_page_identity(
                &request,
                snapshot,
                IngressSequence::new(10),
                PmSnapshotEvidence::new(SnapshotRevision::new(8)).unwrap(),
            ),
            Err(PmReconciliationContractError::PageSnapshotMismatch)
        );
    }
}
