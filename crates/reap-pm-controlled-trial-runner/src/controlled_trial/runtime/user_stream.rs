//! Runner-private authenticated user-stream authority for the PM-T2 trial.
//!
//! The WebSocket sink and coordinator handle share one state machine behind
//! a short synchronous lock. No lock guard crosses an `.await`. Source-owned
//! activity is checked around every REST/ticket command, so queued or
//! in-flight socket activity prevents a final cut even before the sink admits
//! the corresponding event.

use std::{
    fmt,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use reap_pm_core::{
    ConnectionEpoch, EvmAddress, PmBookQuantity, PmConditionId, PmFillId, PmOrderSide, PmPrice,
    PmQuantity, PmTokenId, PmVenueOrderId, U256,
};
use reap_polymarket_live_adapter::{
    PmAuthenticatedUserWsRole, PmUserWsActivityView, PmUserWsConnection, PmUserWsDisconnectReason,
    PmUserWsEvent, PmUserWsEventSink, PmUserWsRunError, PmUserWsShutdownSignal,
};
use reap_polymarket_wire::{PmLiveUserEvent, PmWireScope};
use thiserror::Error;

use super::private_reads::{
    PmRestCutIdentity, PmSameAuthorityRestJoin, PmSameCredentialAuthorityMarker,
    PmSameCredentialUserWsInput, PmUserRestCollectionStart,
};

/// A controlled trial fails closed instead of retaining an unbounded user
/// event history. Each accepted wire frame is already independently bounded;
/// this cap bounds the complete task-lifetime reconciliation log.
const MAX_RETAINED_USER_BUSINESS_EVENTS: usize = 256;

/// Move-only authenticated user-stream runtime split.
///
/// `sink` stays with `PmAuthenticatedUserWsRole::run`; `handle` remains with
/// the coordinator and can issue/recheck REST collection boundaries while the
/// socket task is still alive.
pub(super) struct PmUserStreamRuntime {
    role: PmAuthenticatedUserWsRole,
    sink: PmUserStreamSink,
    handle: PmUserStreamHandle,
}

impl PmUserStreamRuntime {
    pub(super) fn new(
        input: PmSameCredentialUserWsInput,
        expected_scope: PmWireScope,
        expected_proxy_maker: EvmAddress,
    ) -> Result<Self, PmUserStreamRuntimeError> {
        let (role, activity, marker) = input.into_parts();
        if marker.scope() != expected_scope
            || marker.proxy_funder() != expected_proxy_maker
            || role.condition() != expected_scope.condition()
        {
            return Err(PmUserStreamRuntimeError::ConfigurationMismatch);
        }
        if activity.generation() != 0 {
            return Err(PmUserStreamRuntimeError::ActivityAlreadyStarted);
        }

        let shared = Arc::new(PmUserStreamShared {
            state: Mutex::new(PmUserStreamState::new(expected_scope, expected_proxy_maker)),
            activity: RuntimeUserActivityView::Live(activity),
        });
        if shared.activity.generation() != 0 {
            return Err(PmUserStreamRuntimeError::ActivityAlreadyStarted);
        }

        Ok(Self {
            role,
            sink: PmUserStreamSink {
                shared: Arc::clone(&shared),
                task_exit_recorded: false,
            },
            handle: PmUserStreamHandle { shared, marker },
        })
    }

    #[must_use]
    pub(super) fn into_parts(self) -> (PmUserStreamWsTask, PmUserStreamHandle) {
        (
            PmUserStreamWsTask {
                role: self.role,
                sink: self.sink,
            },
            self.handle,
        )
    }
}

impl fmt::Debug for PmUserStreamRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmUserStreamRuntime(<move-only; same credential authority>)")
    }
}

struct PmUserStreamShared {
    state: Mutex<PmUserStreamState>,
    activity: RuntimeUserActivityView,
}

enum RuntimeUserActivityView {
    Live(PmUserWsActivityView),
    #[cfg(test)]
    Test(Arc<std::sync::atomic::AtomicU64>),
}

impl RuntimeUserActivityView {
    fn generation(&self) -> u64 {
        match self {
            Self::Live(view) => view.generation(),
            #[cfg(test)]
            Self::Test(value) => value.load(std::sync::atomic::Ordering::Acquire),
        }
    }
}

/// Cancellation-safe owner of the authenticated role and its only event sink.
/// The raw pair is never released to orchestration, so dropping the run future
/// necessarily drops the sink and terminalizes the shared state.
pub(super) struct PmUserStreamWsTask {
    role: PmAuthenticatedUserWsRole,
    sink: PmUserStreamSink,
}

impl PmUserStreamWsTask {
    pub(super) async fn run_to_exit(
        self,
        shutdown: PmUserWsShutdownSignal,
    ) -> Result<(), PmUserStreamWsTaskError> {
        let Self { role, mut sink } = self;
        let run_result = role.run(shutdown, &mut sink).await;
        sink.invalidate_task_exit()
            .map_err(PmUserStreamWsTaskError::ExitInvalidation)?;
        run_result.map_err(PmUserStreamWsTaskError::Run)
    }
}

impl fmt::Debug for PmUserStreamWsTask {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmUserStreamWsTask(<role-and-armed-sink>)")
    }
}

/// Long-lived WebSocket task side. It carries no REST or ticket authority and
/// never leaves `PmUserStreamWsTask` in production composition.
struct PmUserStreamSink {
    shared: Arc<PmUserStreamShared>,
    task_exit_recorded: bool,
}

impl PmUserStreamSink {
    /// Explicitly records task completion. Drop provides the same invalidation
    /// for cancellation/unwind paths.
    pub(super) fn invalidate_task_exit(&mut self) -> Result<(), PmUserStreamRuntimeError> {
        if self.task_exit_recorded {
            return Ok(());
        }
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| PmUserStreamRuntimeError::StatePoisoned)?;
        state.invalidate_task_exit();
        self.task_exit_recorded = true;
        Ok(())
    }
}

impl Drop for PmUserStreamSink {
    fn drop(&mut self) {
        if self.task_exit_recorded {
            return;
        }
        // Poison is itself terminal. Never recover the inner state: every
        // handle operation will continue to reject the poisoned lock.
        if let Ok(mut state) = self.shared.state.lock() {
            state.invalidate_task_exit();
            self.task_exit_recorded = true;
        }
    }
}

impl fmt::Debug for PmUserStreamSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmUserStreamSink(<serialized; task-owned>)")
    }
}

#[async_trait]
impl PmUserWsEventSink for PmUserStreamSink {
    type Error = PmUserStreamRuntimeError;

    async fn deliver_user_ws_event(&mut self, event: PmUserWsEvent) -> Result<(), Self::Error> {
        let source_high_water = self.shared.activity.generation();
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| PmUserStreamRuntimeError::StatePoisoned)?;
        state.admit_ws_event(event, source_high_water)
    }
}

/// Non-cloneable coordinator side of the active user-stream runtime.
pub(super) struct PmUserStreamHandle {
    shared: Arc<PmUserStreamShared>,
    marker: PmSameCredentialAuthorityMarker,
}

impl PmUserStreamHandle {
    /// Start mandatory authenticated REST collection from a source-sampled
    /// current-epoch subscription/PING/correlated-PONG boundary.
    pub(super) fn begin_rest_collection(
        &mut self,
    ) -> Result<PmUserRestCollectionStart, PmUserStreamRuntimeError> {
        let before = self.shared.activity.generation();
        let boundary = {
            let state = self
                .shared
                .state
                .lock()
                .map_err(|_| PmUserStreamRuntimeError::StatePoisoned)?;
            state.collection_boundary(before)?
        };
        let start = self
            .marker
            .begin_rest_collection(boundary.state_generation, boundary.connection_epoch);
        let after = self.shared.activity.generation();
        if before != boundary.activity_generation
            || start.activity_generation() != boundary.activity_generation
            || after != boundary.activity_generation
            || !start.marker().same_instance(&self.marker)
        {
            return Err(PmUserStreamRuntimeError::ConcurrentActivity);
        }
        {
            let state = self
                .shared
                .state
                .lock()
                .map_err(|_| PmUserStreamRuntimeError::StatePoisoned)?;
            state.validate_boundary(boundary, after)?;
        }
        if self.shared.activity.generation() != boundary.activity_generation {
            return Err(PmUserStreamRuntimeError::ConcurrentActivity);
        }
        Ok(start)
    }

    /// Consume the same-authority REST join and mint a move-only final ticket
    /// only if the stream remained exactly at the collection start boundary.
    pub(super) fn finish_rest_collection(
        &mut self,
        join: PmSameAuthorityRestJoin,
    ) -> Result<FinalCutTicket, PmUserStreamRuntimeError> {
        let open_order_rows = join.open_order_rows();
        let trade_rows = join.trade_rows();
        let (start, rest_cut_identity) = join.into_parts();
        if !start.marker().same_instance(&self.marker) {
            return Err(PmUserStreamRuntimeError::SameAuthorityMismatch);
        }
        let ticket_marker = start.marker().fork_for_same_authority();
        let boundary = CollectionBoundary {
            state_generation: start.stream_revision(),
            connection_epoch: start.connection_epoch(),
            activity_generation: start.activity_generation(),
        };
        let before = self.shared.activity.generation();
        let core = {
            let state = self
                .shared
                .state
                .lock()
                .map_err(|_| PmUserStreamRuntimeError::StatePoisoned)?;
            state.finish_rest_collection(boundary, before, open_order_rows, trade_rows)?
        };
        let after = self.shared.activity.generation();
        if before != boundary.activity_generation || after != boundary.activity_generation {
            return Err(PmUserStreamRuntimeError::ConcurrentActivity);
        }
        {
            let state = self
                .shared
                .state
                .lock()
                .map_err(|_| PmUserStreamRuntimeError::StatePoisoned)?;
            state.validate_final_core(&core, after)?;
        }
        if self.shared.activity.generation() != boundary.activity_generation {
            return Err(PmUserStreamRuntimeError::ConcurrentActivity);
        }
        Ok(FinalCutTicket {
            core,
            marker: ticket_marker,
            rest_cut_identity,
        })
    }

    /// Final dispatch-side recheck. The ticket is consumed regardless of the
    /// result and cannot be replayed after a state or activity transition.
    pub(super) fn consume_final_cut_ticket(
        &mut self,
        ticket: FinalCutTicket,
    ) -> Result<FinalCutJoinFields, PmUserStreamRuntimeError> {
        if !ticket.marker.same_instance(&self.marker) {
            return Err(PmUserStreamRuntimeError::SameAuthorityMismatch);
        }
        let before = self.shared.activity.generation();
        {
            let state = self
                .shared
                .state
                .lock()
                .map_err(|_| PmUserStreamRuntimeError::StatePoisoned)?;
            state.validate_final_core(&ticket.core, before)?;
        }
        let after = self.shared.activity.generation();
        if before != ticket.core.activity_generation || after != ticket.core.activity_generation {
            return Err(PmUserStreamRuntimeError::ConcurrentActivity);
        }
        {
            let state = self
                .shared
                .state
                .lock()
                .map_err(|_| PmUserStreamRuntimeError::StatePoisoned)?;
            state.validate_final_core(&ticket.core, after)?;
        }
        if self.shared.activity.generation() != ticket.core.activity_generation {
            return Err(PmUserStreamRuntimeError::ConcurrentActivity);
        }

        let FinalCutTicket {
            core,
            marker,
            rest_cut_identity,
        } = ticket;
        Ok(FinalCutJoinFields {
            scope: core.scope,
            proxy_maker: core.proxy_maker,
            stream_revision: core.state_generation,
            connection_epoch: core.connection_epoch,
            connection_open_generation: core.connection_open_generation,
            subscription_generation: core.subscription_generation,
            ping_generation: core.ping_generation,
            correlated_pong_generation: core.correlated_pong_generation,
            admitted_activity_generation: core.activity_generation,
            business_basis: core.business_basis,
            business_events: core.business_events,
            _same_authority: marker,
            rest_cut_identity,
        })
    }
}

impl fmt::Debug for PmUserStreamHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmUserStreamHandle(<move-only; ticket issuer>)")
    }
}

/// Final ticket minted only after complete same-authority REST collection.
/// It deliberately implements neither `Clone` nor serialization.
pub(super) struct FinalCutTicket {
    core: FinalCutCore,
    marker: PmSameCredentialAuthorityMarker,
    rest_cut_identity: Arc<PmRestCutIdentity>,
}

impl fmt::Debug for FinalCutTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FinalCutTicket")
            .field("epoch", &self.core.connection_epoch)
            .field("stream_revision", &self.core.state_generation)
            .field("activity_generation", &self.core.activity_generation)
            .field("rest", &"<same-authority complete cut>")
            .finish_non_exhaustive()
    }
}

/// Owner-free projection of one admitted user-stream order event.
///
/// Credential-owner fields and the raw frame are deliberately absent. Text
/// and association fields were already bounded by the wire parser, and the
/// enclosing runtime log has its own task-lifetime item cap.
#[derive(PartialEq, Eq)]
pub(super) struct PmUserStreamOrderProjection {
    connection_epoch: ConnectionEpoch,
    activity_generation: u64,
    id: PmVenueOrderId,
    condition: PmConditionId,
    token: PmTokenId,
    side: PmOrderSide,
    original_size: PmQuantity,
    size_matched: PmBookQuantity,
    price: PmPrice,
    event_kind: Box<str>,
    maker: EvmAddress,
    expiration: Option<u64>,
    order_type: Option<Box<str>>,
    outcome: Option<Box<str>>,
    status: Option<Box<str>>,
    created_at: Option<u64>,
    associate_trades: Option<Box<[PmFillId]>>,
    timestamp: Option<u64>,
}

impl PmUserStreamOrderProjection {
    #[must_use]
    pub(super) const fn connection_epoch(&self) -> ConnectionEpoch {
        self.connection_epoch
    }

    #[must_use]
    pub(super) const fn activity_generation(&self) -> u64 {
        self.activity_generation
    }

    #[must_use]
    pub(super) const fn id(&self) -> PmVenueOrderId {
        self.id
    }

    #[must_use]
    pub(super) const fn condition(&self) -> PmConditionId {
        self.condition
    }

    #[must_use]
    pub(super) const fn token(&self) -> PmTokenId {
        self.token
    }

    #[must_use]
    pub(super) const fn side(&self) -> PmOrderSide {
        self.side
    }

    #[must_use]
    pub(super) const fn original_size(&self) -> PmQuantity {
        self.original_size
    }

    #[must_use]
    pub(super) const fn size_matched(&self) -> PmBookQuantity {
        self.size_matched
    }

    #[must_use]
    pub(super) const fn price(&self) -> PmPrice {
        self.price
    }

    #[must_use]
    pub(super) fn event_kind(&self) -> &str {
        &self.event_kind
    }

    #[must_use]
    pub(super) const fn maker(&self) -> EvmAddress {
        self.maker
    }

    #[must_use]
    pub(super) const fn expiration(&self) -> Option<u64> {
        self.expiration
    }

    #[must_use]
    pub(super) fn order_type(&self) -> Option<&str> {
        self.order_type.as_deref()
    }

    #[must_use]
    pub(super) fn outcome(&self) -> Option<&str> {
        self.outcome.as_deref()
    }

    #[must_use]
    pub(super) fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    #[must_use]
    pub(super) const fn created_at(&self) -> Option<u64> {
        self.created_at
    }

    #[must_use]
    pub(super) fn associate_trades(&self) -> Option<&[PmFillId]> {
        self.associate_trades.as_deref()
    }

    #[must_use]
    pub(super) const fn timestamp(&self) -> Option<u64> {
        self.timestamp
    }

    fn duplicate_for_ticket(&self) -> Self {
        Self {
            connection_epoch: self.connection_epoch,
            activity_generation: self.activity_generation,
            id: self.id,
            condition: self.condition,
            token: self.token,
            side: self.side,
            original_size: self.original_size,
            size_matched: self.size_matched,
            price: self.price,
            event_kind: self.event_kind.clone(),
            maker: self.maker,
            expiration: self.expiration,
            order_type: self.order_type.clone(),
            outcome: self.outcome.clone(),
            status: self.status.clone(),
            created_at: self.created_at,
            associate_trades: self.associate_trades.clone(),
            timestamp: self.timestamp,
        }
    }
}

impl fmt::Debug for PmUserStreamOrderProjection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmUserStreamOrderProjection([REDACTED])")
    }
}

/// Owner-free projection of one maker leg inside an admitted trade event.
#[derive(PartialEq, Eq)]
pub(super) struct PmUserStreamMakerLegProjection {
    order_id: PmVenueOrderId,
    token: PmTokenId,
    side: PmOrderSide,
    price: PmPrice,
    matched_amount: PmQuantity,
    fee_rate_bps: Option<U256>,
    maker: EvmAddress,
}

impl PmUserStreamMakerLegProjection {
    #[must_use]
    pub(super) const fn order_id(&self) -> PmVenueOrderId {
        self.order_id
    }

    #[must_use]
    pub(super) const fn token(&self) -> PmTokenId {
        self.token
    }

    #[must_use]
    pub(super) const fn side(&self) -> PmOrderSide {
        self.side
    }

    #[must_use]
    pub(super) const fn price(&self) -> PmPrice {
        self.price
    }

    #[must_use]
    pub(super) const fn matched_amount(&self) -> PmQuantity {
        self.matched_amount
    }

    #[must_use]
    pub(super) const fn fee_rate_bps(&self) -> Option<U256> {
        self.fee_rate_bps
    }

    #[must_use]
    pub(super) const fn maker(&self) -> EvmAddress {
        self.maker
    }

    const fn duplicate_for_ticket(&self) -> Self {
        Self {
            order_id: self.order_id,
            token: self.token,
            side: self.side,
            price: self.price,
            matched_amount: self.matched_amount,
            fee_rate_bps: self.fee_rate_bps,
            maker: self.maker,
        }
    }
}

impl fmt::Debug for PmUserStreamMakerLegProjection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmUserStreamMakerLegProjection([REDACTED])")
    }
}

/// Owner-free projection of one admitted user-stream trade event.
#[derive(PartialEq, Eq)]
pub(super) struct PmUserStreamTradeProjection {
    connection_epoch: ConnectionEpoch,
    activity_generation: u64,
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
    maker_orders: Box<[PmUserStreamMakerLegProjection]>,
    maker: Option<EvmAddress>,
    timestamp: Option<u64>,
    match_time: Option<u64>,
    last_update: Option<u64>,
}

impl PmUserStreamTradeProjection {
    #[must_use]
    pub(super) const fn connection_epoch(&self) -> ConnectionEpoch {
        self.connection_epoch
    }

    #[must_use]
    pub(super) const fn activity_generation(&self) -> u64 {
        self.activity_generation
    }

    #[must_use]
    pub(super) const fn id(&self) -> PmFillId {
        self.id
    }

    #[must_use]
    pub(super) const fn condition(&self) -> PmConditionId {
        self.condition
    }

    #[must_use]
    pub(super) const fn token(&self) -> PmTokenId {
        self.token
    }

    #[must_use]
    pub(super) const fn side(&self) -> PmOrderSide {
        self.side
    }

    #[must_use]
    pub(super) const fn size(&self) -> PmQuantity {
        self.size
    }

    #[must_use]
    pub(super) const fn price(&self) -> PmPrice {
        self.price
    }

    #[must_use]
    pub(super) fn status(&self) -> &str {
        &self.status
    }

    #[must_use]
    pub(super) const fn order_id(&self) -> Option<PmVenueOrderId> {
        self.order_id
    }

    #[must_use]
    pub(super) const fn taker_order_id(&self) -> Option<PmVenueOrderId> {
        self.taker_order_id
    }

    #[must_use]
    pub(super) fn trader_side(&self) -> Option<&str> {
        self.trader_side.as_deref()
    }

    #[must_use]
    pub(super) fn transaction_hash(&self) -> Option<&str> {
        self.transaction_hash.as_deref()
    }

    #[must_use]
    pub(super) const fn fee_rate_bps(&self) -> Option<U256> {
        self.fee_rate_bps
    }

    #[must_use]
    pub(super) fn maker_orders(&self) -> &[PmUserStreamMakerLegProjection] {
        &self.maker_orders
    }

    #[must_use]
    pub(super) const fn maker(&self) -> Option<EvmAddress> {
        self.maker
    }

    #[must_use]
    pub(super) const fn timestamp(&self) -> Option<u64> {
        self.timestamp
    }

    #[must_use]
    pub(super) const fn match_time(&self) -> Option<u64> {
        self.match_time
    }

    #[must_use]
    pub(super) const fn last_update(&self) -> Option<u64> {
        self.last_update
    }

    fn duplicate_for_ticket(&self) -> Self {
        Self {
            connection_epoch: self.connection_epoch,
            activity_generation: self.activity_generation,
            id: self.id,
            condition: self.condition,
            token: self.token,
            side: self.side,
            size: self.size,
            price: self.price,
            status: self.status.clone(),
            order_id: self.order_id,
            taker_order_id: self.taker_order_id,
            trader_side: self.trader_side.clone(),
            transaction_hash: self.transaction_hash.clone(),
            fee_rate_bps: self.fee_rate_bps,
            maker_orders: self
                .maker_orders
                .iter()
                .map(PmUserStreamMakerLegProjection::duplicate_for_ticket)
                .collect(),
            maker: self.maker,
            timestamp: self.timestamp,
            match_time: self.match_time,
            last_update: self.last_update,
        }
    }
}

impl fmt::Debug for PmUserStreamTradeProjection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PmUserStreamTradeProjection([REDACTED])")
    }
}

/// Complete owner-free business fact retained from one user event.
#[derive(PartialEq, Eq)]
pub(super) enum PmUserStreamBusinessEventProjection {
    Order(PmUserStreamOrderProjection),
    Trade(PmUserStreamTradeProjection),
}

impl PmUserStreamBusinessEventProjection {
    #[must_use]
    pub(super) const fn connection_epoch(&self) -> ConnectionEpoch {
        match self {
            Self::Order(order) => order.connection_epoch(),
            Self::Trade(trade) => trade.connection_epoch(),
        }
    }

    #[must_use]
    pub(super) const fn activity_generation(&self) -> u64 {
        match self {
            Self::Order(order) => order.activity_generation(),
            Self::Trade(trade) => trade.activity_generation(),
        }
    }

    fn duplicate_for_ticket(&self) -> Self {
        match self {
            Self::Order(order) => Self::Order(order.duplicate_for_ticket()),
            Self::Trade(trade) => Self::Trade(trade.duplicate_for_ticket()),
        }
    }
}

impl fmt::Debug for PmUserStreamBusinessEventProjection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Order(_) => "PmUserStreamBusinessEventProjection::Order([REDACTED])",
            Self::Trade(_) => "PmUserStreamBusinessEventProjection::Trade([REDACTED])",
        })
    }
}

/// Typed final basis: complete REST collection is structurally present in
/// both variants. This is same-authority co-observation evidence, not an
/// exact-ID or lifecycle reconciliation grant; a later typed reconciler must
/// join the retained REST rows before any permit can consume it. Zero stream
/// events never become standalone evidence.
#[derive(PartialEq, Eq)]
pub(super) enum FinalCutBusinessBasis {
    StreamEventsAndRestJoined {
        current_epoch_event_start: usize,
        current_epoch_event_count: usize,
        last_stream_activity_generation: u64,
        open_order_rows: usize,
        trade_rows: usize,
    },
    RestBackedNoStreamEvents {
        through_activity_generation: u64,
        current_epoch_event_start: usize,
        open_order_rows: usize,
        trade_rows: usize,
    },
}

impl fmt::Debug for FinalCutBusinessBasis {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StreamEventsAndRestJoined {
                current_epoch_event_start,
                current_epoch_event_count,
                open_order_rows,
                trade_rows,
                ..
            } => formatter
                .debug_struct("FinalCutBusinessBasis::StreamEventsAndRestJoined")
                .field("current_epoch_event_start", current_epoch_event_start)
                .field("current_epoch_event_count", current_epoch_event_count)
                .field("open_order_rows", open_order_rows)
                .field("trade_rows", trade_rows)
                .finish(),
            Self::RestBackedNoStreamEvents {
                current_epoch_event_start,
                open_order_rows,
                trade_rows,
                ..
            } => formatter
                .debug_struct("FinalCutBusinessBasis::RestBackedNoStreamEvents")
                .field("current_epoch_event_start", current_epoch_event_start)
                .field("open_order_rows", open_order_rows)
                .field("trade_rows", trade_rows)
                .finish(),
        }
    }
}

/// Move-only fields released only by consuming and rechecking a final ticket.
pub(super) struct FinalCutJoinFields {
    scope: PmWireScope,
    proxy_maker: EvmAddress,
    stream_revision: u64,
    connection_epoch: ConnectionEpoch,
    connection_open_generation: u64,
    subscription_generation: u64,
    ping_generation: u64,
    correlated_pong_generation: u64,
    admitted_activity_generation: u64,
    business_basis: FinalCutBusinessBasis,
    business_events: Box<[PmUserStreamBusinessEventProjection]>,
    _same_authority: PmSameCredentialAuthorityMarker,
    rest_cut_identity: Arc<PmRestCutIdentity>,
}

impl FinalCutJoinFields {
    #[must_use]
    pub(super) const fn scope(&self) -> PmWireScope {
        self.scope
    }

    #[must_use]
    pub(super) const fn proxy_maker(&self) -> EvmAddress {
        self.proxy_maker
    }

    #[must_use]
    pub(super) const fn stream_revision(&self) -> u64 {
        self.stream_revision
    }

    #[must_use]
    pub(super) const fn connection_epoch(&self) -> ConnectionEpoch {
        self.connection_epoch
    }

    #[must_use]
    pub(super) const fn connection_open_generation(&self) -> u64 {
        self.connection_open_generation
    }

    #[must_use]
    pub(super) const fn subscription_generation(&self) -> u64 {
        self.subscription_generation
    }

    #[must_use]
    pub(super) const fn ping_generation(&self) -> u64 {
        self.ping_generation
    }

    #[must_use]
    pub(super) const fn correlated_pong_generation(&self) -> u64 {
        self.correlated_pong_generation
    }

    #[must_use]
    pub(super) const fn admitted_activity_generation(&self) -> u64 {
        self.admitted_activity_generation
    }

    #[must_use]
    pub(super) const fn business_basis(&self) -> &FinalCutBusinessBasis {
        &self.business_basis
    }

    /// Complete bounded task-lifetime business log. Every row is owner-free
    /// and stamped with its connection epoch and frame activity generation.
    #[must_use]
    pub(super) fn business_events(&self) -> &[PmUserStreamBusinessEventProjection] {
        &self.business_events
    }

    /// Exact subrange admitted in the currently heartbeat-ready epoch. Events
    /// before a reconnect remain available in `business_events` but cannot
    /// satisfy the current-epoch joined basis.
    #[must_use]
    pub(super) fn current_epoch_business_events(&self) -> &[PmUserStreamBusinessEventProjection] {
        let (start, count) = match &self.business_basis {
            FinalCutBusinessBasis::StreamEventsAndRestJoined {
                current_epoch_event_start,
                current_epoch_event_count,
                ..
            } => (*current_epoch_event_start, *current_epoch_event_count),
            FinalCutBusinessBasis::RestBackedNoStreamEvents {
                current_epoch_event_start,
                ..
            } => (*current_epoch_event_start, 0),
        };
        &self.business_events[start..start + count]
    }

    #[must_use]
    pub(super) const fn rest_cut_identity(&self) -> &Arc<PmRestCutIdentity> {
        &self.rest_cut_identity
    }
}

impl fmt::Debug for FinalCutJoinFields {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FinalCutJoinFields")
            .field("scope", &"<fixed; redacted>")
            .field("proxy_maker", &"<redacted>")
            .field("stream_revision", &self.stream_revision)
            .field("epoch", &self.connection_epoch)
            .field("subscription_generation", &self.subscription_generation)
            .field("ping_generation", &self.ping_generation)
            .field(
                "correlated_pong_generation",
                &self.correlated_pong_generation,
            )
            .field(
                "admitted_activity_generation",
                &self.admitted_activity_generation,
            )
            .field("business_basis", &self.business_basis)
            .field("business_event_count", &self.business_events.len())
            .field("rest_cut", &"<opaque allocation>")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Error)]
pub(super) enum PmUserStreamWsTaskError {
    #[error("authenticated user WebSocket task failed: {0}")]
    Run(PmUserWsRunError<PmUserStreamRuntimeError>),
    #[error("user-stream task-exit invalidation failed: {0}")]
    ExitInvalidation(PmUserStreamRuntimeError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(super) enum PmUserStreamRuntimeError {
    #[error("user-stream runtime scope, role, or proxy maker disagreed")]
    ConfigurationMismatch,
    #[error("user-stream activity began before runtime ownership was installed")]
    ActivityAlreadyStarted,
    #[error("user-stream runtime state lock was poisoned")]
    StatePoisoned,
    #[error("user-stream task already exited")]
    TaskExited,
    #[error("user-stream source activity generation overflowed or was zero")]
    ActivityGenerationOverflow,
    #[error("user-stream source activity was duplicated, reordered, or skipped")]
    ActivityDiscontinuity,
    #[error("user-stream source high-water preceded admitted evidence")]
    InvalidSourceHighWater,
    #[error("user-stream event carried a foreign condition or connection epoch")]
    ConnectionMismatch,
    #[error("user-stream lifecycle event was out of order")]
    LifecycleMismatch,
    #[error("user-stream bound frame was empty")]
    EmptyBoundFrame,
    #[error("user-stream business event was outside the exact condition/token/proxy scope")]
    ForeignBusinessScope,
    #[error("user-stream bounded business evidence capacity was exceeded")]
    BusinessEvidenceCapacityExceeded,
    #[error("user-stream business or state generation counter overflowed")]
    StateGenerationOverflow,
    #[error("user-stream is not current-epoch subscription/PING/correlated-PONG ready")]
    HeartbeatNotReady,
    #[error("user-stream activity is queued or changed during the checked command")]
    ConcurrentActivity,
    #[error("user-stream REST start, ticket, or current state no longer match")]
    TicketInvalidated,
    #[error("authenticated REST and user-stream evidence did not share one authority")]
    SameAuthorityMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UserStreamPhase {
    AwaitingOpen {
        replacement_epoch: Option<ConnectionEpoch>,
    },
    Opened,
    Subscribed,
    AwaitingPong,
    Ready,
    Retired,
    Terminal,
    Faulted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RetiredConnection {
    epoch: ConnectionEpoch,
    activity_generation: u64,
    reason: PmUserWsDisconnectReason,
}

struct PmUserStreamState {
    scope: PmWireScope,
    proxy_maker: EvmAddress,
    phase: UserStreamPhase,
    task_active: bool,
    state_generation: u64,
    admitted_activity_generation: u64,
    active_epoch: Option<ConnectionEpoch>,
    last_connection_epoch: Option<ConnectionEpoch>,
    connection_open_generation: Option<u64>,
    subscription_generation: Option<u64>,
    pending_ping_generation: Option<u64>,
    last_ping_generation: Option<u64>,
    correlated_pong_generation: Option<u64>,
    business_events: Vec<PmUserStreamBusinessEventProjection>,
    current_epoch_event_start: Option<usize>,
    last_current_epoch_business_activity_generation: Option<u64>,
    retired: Option<RetiredConnection>,
}

impl PmUserStreamState {
    const fn new(scope: PmWireScope, proxy_maker: EvmAddress) -> Self {
        Self {
            scope,
            proxy_maker,
            phase: UserStreamPhase::AwaitingOpen {
                replacement_epoch: None,
            },
            task_active: true,
            state_generation: 1,
            admitted_activity_generation: 0,
            active_epoch: None,
            last_connection_epoch: None,
            connection_open_generation: None,
            subscription_generation: None,
            pending_ping_generation: None,
            last_ping_generation: None,
            correlated_pong_generation: None,
            business_events: Vec::new(),
            current_epoch_event_start: None,
            last_current_epoch_business_activity_generation: None,
            retired: None,
        }
    }

    fn admit_ws_event(
        &mut self,
        event: PmUserWsEvent,
        source_high_water: u64,
    ) -> Result<(), PmUserStreamRuntimeError> {
        let generation = event.activity_generation();
        let result = match event {
            PmUserWsEvent::ConnectionOpened(observation) => self.admit_edge(
                observation.connection(),
                generation,
                source_high_water,
                UserStreamAdmission::ConnectionOpened,
            ),
            PmUserWsEvent::SubscriptionSent(observation) => self.admit_edge(
                observation.connection(),
                generation,
                source_high_water,
                UserStreamAdmission::SubscriptionSent,
            ),
            PmUserWsEvent::PingSent(observation) => self.admit_edge(
                observation.connection(),
                generation,
                source_high_water,
                UserStreamAdmission::PingSent,
            ),
            PmUserWsEvent::Pong(observation) => self.admit_edge(
                observation.connection(),
                generation,
                source_high_water,
                UserStreamAdmission::Pong,
            ),
            PmUserWsEvent::BoundFrame(frame) => {
                let observation = frame.observation();
                self.admit_edge(
                    observation.connection(),
                    generation,
                    source_high_water,
                    UserStreamAdmission::Business(frame.events()),
                )
            }
            PmUserWsEvent::ConnectionRetired(retired) => self.admit_edge(
                retired.observation().connection(),
                generation,
                source_high_water,
                UserStreamAdmission::ConnectionRetired(retired.reason()),
            ),
            PmUserWsEvent::ReconnectScheduled(reconnect) => {
                let retired = reconnect.retired();
                self.admit_edge(
                    retired.observation().connection(),
                    generation,
                    source_high_water,
                    UserStreamAdmission::ReconnectScheduled {
                        replacement_epoch: reconnect.replacement_epoch(),
                        retired_generation: retired.observation().activity_generation(),
                        reason: retired.reason(),
                    },
                )
            }
            PmUserWsEvent::RetryExhausted(retired) => self.admit_edge(
                retired.observation().connection(),
                generation,
                source_high_water,
                UserStreamAdmission::RetryExhausted(retired.reason()),
            ),
            PmUserWsEvent::Shutdown(observation) => self.admit_edge(
                observation.connection(),
                generation,
                source_high_water,
                UserStreamAdmission::Shutdown,
            ),
        };
        if result.is_err() {
            self.fail_closed(generation);
        }
        result
    }

    fn admit_edge(
        &mut self,
        connection: PmUserWsConnection,
        generation: u64,
        source_high_water: u64,
        admission: UserStreamAdmission<'_>,
    ) -> Result<(), PmUserStreamRuntimeError> {
        self.validate_next_activity(generation, source_high_water)?;
        if !self.task_active {
            return Err(PmUserStreamRuntimeError::TaskExited);
        }
        if connection.condition() != self.scope.condition() {
            return Err(PmUserStreamRuntimeError::ConnectionMismatch);
        }

        match admission {
            UserStreamAdmission::ConnectionOpened => self.open(connection, generation)?,
            UserStreamAdmission::SubscriptionSent => {
                self.require_active_connection(connection)?;
                if self.phase != UserStreamPhase::Opened {
                    return Err(PmUserStreamRuntimeError::LifecycleMismatch);
                }
                self.subscription_generation = Some(generation);
                self.phase = UserStreamPhase::Subscribed;
            }
            UserStreamAdmission::PingSent => {
                self.require_active_connection(connection)?;
                if !matches!(
                    self.phase,
                    UserStreamPhase::Subscribed | UserStreamPhase::Ready
                ) {
                    return Err(PmUserStreamRuntimeError::LifecycleMismatch);
                }
                self.pending_ping_generation = Some(generation);
                self.last_ping_generation = Some(generation);
                self.correlated_pong_generation = None;
                self.phase = UserStreamPhase::AwaitingPong;
            }
            UserStreamAdmission::Pong => {
                self.require_active_connection(connection)?;
                let ping = self
                    .pending_ping_generation
                    .take()
                    .ok_or(PmUserStreamRuntimeError::LifecycleMismatch)?;
                if self.phase != UserStreamPhase::AwaitingPong || generation <= ping {
                    return Err(PmUserStreamRuntimeError::LifecycleMismatch);
                }
                self.correlated_pong_generation = Some(generation);
                self.phase = UserStreamPhase::Ready;
            }
            UserStreamAdmission::Business(events) => {
                self.require_active_connection(connection)?;
                if !matches!(
                    self.phase,
                    UserStreamPhase::Subscribed
                        | UserStreamPhase::AwaitingPong
                        | UserStreamPhase::Ready
                ) {
                    return Err(PmUserStreamRuntimeError::LifecycleMismatch);
                }
                self.admit_business_events(events, connection.connection_epoch(), generation)?;
            }
            UserStreamAdmission::ConnectionRetired(reason) => {
                let epoch = connection.connection_epoch();
                match self.phase {
                    UserStreamPhase::AwaitingOpen { replacement_epoch } => {
                        if epoch.value() == 0
                            || replacement_epoch.is_some_and(|expected| expected != epoch)
                            || replacement_epoch.is_none() && self.last_connection_epoch.is_some()
                        {
                            return Err(PmUserStreamRuntimeError::ConnectionMismatch);
                        }
                        self.last_connection_epoch = Some(epoch);
                    }
                    UserStreamPhase::Opened
                    | UserStreamPhase::Subscribed
                    | UserStreamPhase::AwaitingPong
                    | UserStreamPhase::Ready => self.require_active_connection(connection)?,
                    _ => return Err(PmUserStreamRuntimeError::LifecycleMismatch),
                }
                self.retired = Some(RetiredConnection {
                    epoch,
                    activity_generation: generation,
                    reason,
                });
                self.active_epoch = None;
                self.clear_readiness();
                self.phase = UserStreamPhase::Retired;
            }
            UserStreamAdmission::ReconnectScheduled {
                replacement_epoch,
                retired_generation,
                reason,
            } => {
                let retired = self
                    .retired
                    .ok_or(PmUserStreamRuntimeError::LifecycleMismatch)?;
                let expected_replacement = retired
                    .epoch
                    .value()
                    .checked_add(1)
                    .ok_or(PmUserStreamRuntimeError::ActivityGenerationOverflow)?;
                if self.phase != UserStreamPhase::Retired
                    || connection.connection_epoch() != retired.epoch
                    || retired_generation != retired.activity_generation
                    || reason != retired.reason
                    || replacement_epoch.value() != expected_replacement
                {
                    return Err(PmUserStreamRuntimeError::LifecycleMismatch);
                }
                self.phase = UserStreamPhase::AwaitingOpen {
                    replacement_epoch: Some(replacement_epoch),
                };
            }
            UserStreamAdmission::RetryExhausted(reason) => {
                let retired = self
                    .retired
                    .ok_or(PmUserStreamRuntimeError::LifecycleMismatch)?;
                if self.phase != UserStreamPhase::Retired
                    || connection.connection_epoch() != retired.epoch
                    || reason != retired.reason
                {
                    return Err(PmUserStreamRuntimeError::LifecycleMismatch);
                }
                self.active_epoch = None;
                self.clear_readiness();
                self.phase = UserStreamPhase::Terminal;
            }
            UserStreamAdmission::Shutdown => {
                let epoch = connection.connection_epoch();
                let initial_attempt = matches!(
                    self.phase,
                    UserStreamPhase::AwaitingOpen {
                        replacement_epoch: None
                    }
                ) && self.last_connection_epoch.is_none()
                    && epoch.value() != 0;
                if self.active_epoch != Some(epoch)
                    && self.last_connection_epoch != Some(epoch)
                    && !initial_attempt
                {
                    return Err(PmUserStreamRuntimeError::ConnectionMismatch);
                }
                self.last_connection_epoch = Some(epoch);
                self.active_epoch = None;
                self.clear_readiness();
                self.task_active = false;
                self.phase = UserStreamPhase::Terminal;
            }
        }

        self.finish_activity(generation)
    }

    fn open(
        &mut self,
        connection: PmUserWsConnection,
        generation: u64,
    ) -> Result<(), PmUserStreamRuntimeError> {
        let replacement_epoch = match self.phase {
            UserStreamPhase::AwaitingOpen { replacement_epoch } => replacement_epoch,
            _ => return Err(PmUserStreamRuntimeError::LifecycleMismatch),
        };
        let epoch = connection.connection_epoch();
        if epoch.value() == 0
            || replacement_epoch.is_some_and(|expected| expected != epoch)
            || replacement_epoch.is_none() && self.last_connection_epoch.is_some()
        {
            return Err(PmUserStreamRuntimeError::ConnectionMismatch);
        }
        self.active_epoch = Some(epoch);
        self.last_connection_epoch = Some(epoch);
        self.connection_open_generation = Some(generation);
        self.subscription_generation = None;
        self.pending_ping_generation = None;
        self.last_ping_generation = None;
        self.correlated_pong_generation = None;
        self.current_epoch_event_start = Some(self.business_events.len());
        self.last_current_epoch_business_activity_generation = None;
        self.retired = None;
        self.phase = UserStreamPhase::Opened;
        Ok(())
    }

    fn validate_next_activity(
        &self,
        generation: u64,
        source_high_water: u64,
    ) -> Result<(), PmUserStreamRuntimeError> {
        let expected = self
            .admitted_activity_generation
            .checked_add(1)
            .ok_or(PmUserStreamRuntimeError::ActivityGenerationOverflow)?;
        if generation != expected {
            return Err(PmUserStreamRuntimeError::ActivityDiscontinuity);
        }
        if source_high_water < generation {
            return Err(PmUserStreamRuntimeError::InvalidSourceHighWater);
        }
        Ok(())
    }

    fn require_active_connection(
        &self,
        connection: PmUserWsConnection,
    ) -> Result<(), PmUserStreamRuntimeError> {
        if self.active_epoch != Some(connection.connection_epoch()) {
            Err(PmUserStreamRuntimeError::ConnectionMismatch)
        } else {
            Ok(())
        }
    }

    fn business_event_is_scoped(&self, event: &PmLiveUserEvent) -> bool {
        match event {
            PmLiveUserEvent::Order(order) => {
                order.condition() == self.scope.condition()
                    && order.token() == self.scope.token()
                    && order.maker() == Some(self.proxy_maker)
            }
            PmLiveUserEvent::Trade(trade) => {
                let top_level_bound =
                    trade.token() == self.scope.token() && trade.maker() == Some(self.proxy_maker);
                let configured_maker_leg_bound = trade.maker_orders().iter().any(|order| {
                    order.token() == self.scope.token() && order.maker() == self.proxy_maker
                });
                trade.condition() == self.scope.condition()
                    && (top_level_bound || configured_maker_leg_bound)
            }
        }
    }

    fn admit_business_events(
        &mut self,
        events: &[PmLiveUserEvent],
        connection_epoch: ConnectionEpoch,
        activity_generation: u64,
    ) -> Result<(), PmUserStreamRuntimeError> {
        if events.is_empty() {
            return Err(PmUserStreamRuntimeError::EmptyBoundFrame);
        }
        if events
            .iter()
            .any(|event| !self.business_event_is_scoped(event))
        {
            return Err(PmUserStreamRuntimeError::ForeignBusinessScope);
        }
        let retained_len = self
            .business_events
            .len()
            .checked_add(events.len())
            .ok_or(PmUserStreamRuntimeError::BusinessEvidenceCapacityExceeded)?;
        if retained_len > MAX_RETAINED_USER_BUSINESS_EVENTS {
            return Err(PmUserStreamRuntimeError::BusinessEvidenceCapacityExceeded);
        }
        let projected = events
            .iter()
            .map(|event| Self::project_business_event(event, connection_epoch, activity_generation))
            .collect::<Result<Vec<_>, _>>()?;
        self.business_events.extend(projected);
        self.last_current_epoch_business_activity_generation = Some(activity_generation);
        Ok(())
    }

    fn project_business_event(
        event: &PmLiveUserEvent,
        connection_epoch: ConnectionEpoch,
        activity_generation: u64,
    ) -> Result<PmUserStreamBusinessEventProjection, PmUserStreamRuntimeError> {
        match event {
            PmLiveUserEvent::Order(order) => Ok(PmUserStreamBusinessEventProjection::Order(
                PmUserStreamOrderProjection {
                    connection_epoch,
                    activity_generation,
                    id: order.id(),
                    condition: order.condition(),
                    token: order.token(),
                    side: order.side(),
                    original_size: order.original_size(),
                    size_matched: order.size_matched(),
                    price: order.price(),
                    event_kind: Box::from(order.event_kind()),
                    maker: order
                        .maker()
                        .ok_or(PmUserStreamRuntimeError::ForeignBusinessScope)?,
                    expiration: order.expiration(),
                    order_type: order.order_type().map(Box::from),
                    outcome: order.outcome().map(Box::from),
                    status: order.status().map(Box::from),
                    created_at: order.created_at(),
                    associate_trades: order.associate_trades().map(Box::from),
                    timestamp: order.timestamp(),
                },
            )),
            PmLiveUserEvent::Trade(trade) => Ok(PmUserStreamBusinessEventProjection::Trade(
                PmUserStreamTradeProjection {
                    connection_epoch,
                    activity_generation,
                    id: trade.id(),
                    condition: trade.condition(),
                    token: trade.token(),
                    side: trade.side(),
                    size: trade.size(),
                    price: trade.price(),
                    status: Box::from(trade.status()),
                    order_id: trade.order_id(),
                    taker_order_id: trade.taker_order_id(),
                    trader_side: trade.trader_side().map(Box::from),
                    transaction_hash: trade.transaction_hash().map(Box::from),
                    fee_rate_bps: trade.fee_rate_bps(),
                    maker_orders: trade
                        .maker_orders()
                        .iter()
                        .map(|maker_order| PmUserStreamMakerLegProjection {
                            order_id: maker_order.order_id(),
                            token: maker_order.token(),
                            side: maker_order.side(),
                            price: maker_order.price(),
                            matched_amount: maker_order.matched_amount(),
                            fee_rate_bps: maker_order.fee_rate_bps(),
                            maker: maker_order.maker(),
                        })
                        .collect(),
                    maker: trade.maker(),
                    timestamp: trade.timestamp(),
                    match_time: trade.match_time(),
                    last_update: trade.last_update(),
                },
            )),
        }
    }

    fn finish_activity(&mut self, generation: u64) -> Result<(), PmUserStreamRuntimeError> {
        self.admitted_activity_generation = generation;
        self.bump_state_generation()
    }

    fn bump_state_generation(&mut self) -> Result<(), PmUserStreamRuntimeError> {
        self.state_generation = self
            .state_generation
            .checked_add(1)
            .ok_or(PmUserStreamRuntimeError::StateGenerationOverflow)?;
        Ok(())
    }

    fn fail_closed(&mut self, observed_generation: u64) {
        if observed_generation > self.admitted_activity_generation {
            self.admitted_activity_generation = observed_generation;
        }
        self.task_active = false;
        self.state_generation = self.state_generation.saturating_add(1);
        self.active_epoch = None;
        self.clear_readiness();
        self.phase = UserStreamPhase::Faulted;
    }

    fn invalidate_task_exit(&mut self) {
        if !self.task_active {
            return;
        }
        self.task_active = false;
        self.state_generation = self.state_generation.saturating_add(1);
        self.active_epoch = None;
        self.clear_readiness();
        self.phase = UserStreamPhase::Terminal;
    }

    fn clear_readiness(&mut self) {
        self.connection_open_generation = None;
        self.subscription_generation = None;
        self.pending_ping_generation = None;
        self.last_ping_generation = None;
        self.correlated_pong_generation = None;
    }

    fn collection_boundary(
        &self,
        source_high_water: u64,
    ) -> Result<CollectionBoundary, PmUserStreamRuntimeError> {
        let evidence = self.ready_evidence()?;
        if source_high_water != self.admitted_activity_generation {
            return Err(PmUserStreamRuntimeError::ConcurrentActivity);
        }
        Ok(CollectionBoundary {
            state_generation: self.state_generation,
            connection_epoch: evidence.connection_epoch,
            activity_generation: self.admitted_activity_generation,
        })
    }

    fn validate_boundary(
        &self,
        boundary: CollectionBoundary,
        source_high_water: u64,
    ) -> Result<(), PmUserStreamRuntimeError> {
        let evidence = self.ready_evidence()?;
        if boundary.state_generation != self.state_generation
            || boundary.connection_epoch != evidence.connection_epoch
            || boundary.activity_generation != self.admitted_activity_generation
            || source_high_water != boundary.activity_generation
        {
            return Err(PmUserStreamRuntimeError::TicketInvalidated);
        }
        Ok(())
    }

    fn finish_rest_collection(
        &self,
        boundary: CollectionBoundary,
        source_high_water: u64,
        open_order_rows: usize,
        trade_rows: usize,
    ) -> Result<FinalCutCore, PmUserStreamRuntimeError> {
        self.validate_boundary(boundary, source_high_water)?;
        let evidence = self.ready_evidence()?;
        let current_epoch_event_start = self
            .current_epoch_event_start
            .ok_or(PmUserStreamRuntimeError::TicketInvalidated)?;
        let current_epoch_events = self
            .business_events
            .get(current_epoch_event_start..)
            .ok_or(PmUserStreamRuntimeError::TicketInvalidated)?;
        if current_epoch_events.iter().any(|event| {
            event.connection_epoch() != evidence.connection_epoch
                || event.activity_generation() <= evidence.subscription_generation
                || event.activity_generation() > self.admitted_activity_generation
        }) || current_epoch_events
            .last()
            .map(PmUserStreamBusinessEventProjection::activity_generation)
            != self.last_current_epoch_business_activity_generation
        {
            return Err(PmUserStreamRuntimeError::TicketInvalidated);
        }
        let business_basis = match (
            current_epoch_events.is_empty(),
            self.last_current_epoch_business_activity_generation,
        ) {
            (false, Some(last_stream_activity_generation)) => {
                FinalCutBusinessBasis::StreamEventsAndRestJoined {
                    current_epoch_event_start,
                    current_epoch_event_count: current_epoch_events.len(),
                    last_stream_activity_generation,
                    open_order_rows,
                    trade_rows,
                }
            }
            (true, None) => FinalCutBusinessBasis::RestBackedNoStreamEvents {
                through_activity_generation: self.admitted_activity_generation,
                current_epoch_event_start,
                open_order_rows,
                trade_rows,
            },
            _ => return Err(PmUserStreamRuntimeError::TicketInvalidated),
        };
        Ok(FinalCutCore {
            scope: self.scope,
            proxy_maker: self.proxy_maker,
            state_generation: self.state_generation,
            connection_epoch: evidence.connection_epoch,
            connection_open_generation: evidence.connection_open_generation,
            subscription_generation: evidence.subscription_generation,
            ping_generation: evidence.ping_generation,
            correlated_pong_generation: evidence.correlated_pong_generation,
            activity_generation: self.admitted_activity_generation,
            business_basis,
            business_events: self
                .business_events
                .iter()
                .map(PmUserStreamBusinessEventProjection::duplicate_for_ticket)
                .collect(),
        })
    }

    fn validate_final_core(
        &self,
        core: &FinalCutCore,
        source_high_water: u64,
    ) -> Result<(), PmUserStreamRuntimeError> {
        let evidence = self.ready_evidence()?;
        if core.scope != self.scope
            || core.proxy_maker != self.proxy_maker
            || core.state_generation != self.state_generation
            || core.connection_epoch != evidence.connection_epoch
            || core.connection_open_generation != evidence.connection_open_generation
            || core.subscription_generation != evidence.subscription_generation
            || core.ping_generation != evidence.ping_generation
            || core.correlated_pong_generation != evidence.correlated_pong_generation
            || core.activity_generation != self.admitted_activity_generation
            || source_high_water != core.activity_generation
            || core.business_events.as_ref() != self.business_events.as_slice()
            || !core.business_basis.matches_state(self)
        {
            return Err(PmUserStreamRuntimeError::TicketInvalidated);
        }
        Ok(())
    }

    fn ready_evidence(&self) -> Result<ReadyEvidence, PmUserStreamRuntimeError> {
        if !self.task_active || self.phase != UserStreamPhase::Ready {
            return Err(PmUserStreamRuntimeError::HeartbeatNotReady);
        }
        let evidence = ReadyEvidence {
            connection_epoch: self
                .active_epoch
                .ok_or(PmUserStreamRuntimeError::HeartbeatNotReady)?,
            connection_open_generation: self
                .connection_open_generation
                .ok_or(PmUserStreamRuntimeError::HeartbeatNotReady)?,
            subscription_generation: self
                .subscription_generation
                .ok_or(PmUserStreamRuntimeError::HeartbeatNotReady)?,
            ping_generation: self
                .last_ping_generation
                .ok_or(PmUserStreamRuntimeError::HeartbeatNotReady)?,
            correlated_pong_generation: self
                .correlated_pong_generation
                .ok_or(PmUserStreamRuntimeError::HeartbeatNotReady)?,
        };
        if !(evidence.connection_open_generation < evidence.subscription_generation
            && evidence.subscription_generation < evidence.ping_generation
            && evidence.ping_generation < evidence.correlated_pong_generation
            && evidence.correlated_pong_generation <= self.admitted_activity_generation)
        {
            return Err(PmUserStreamRuntimeError::HeartbeatNotReady);
        }
        Ok(evidence)
    }
}

impl FinalCutBusinessBasis {
    fn matches_state(&self, state: &PmUserStreamState) -> bool {
        match self {
            Self::StreamEventsAndRestJoined {
                current_epoch_event_start,
                current_epoch_event_count,
                last_stream_activity_generation,
                ..
            } => {
                state.current_epoch_event_start == Some(*current_epoch_event_start)
                    && state
                        .business_events
                        .len()
                        .checked_sub(*current_epoch_event_start)
                        == Some(*current_epoch_event_count)
                    && Some(*last_stream_activity_generation)
                        == state.last_current_epoch_business_activity_generation
            }
            Self::RestBackedNoStreamEvents {
                through_activity_generation,
                current_epoch_event_start,
                ..
            } => {
                state.current_epoch_event_start == Some(*current_epoch_event_start)
                    && state.business_events.len() == *current_epoch_event_start
                    && state
                        .last_current_epoch_business_activity_generation
                        .is_none()
                    && *through_activity_generation == state.admitted_activity_generation
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CollectionBoundary {
    state_generation: u64,
    connection_epoch: ConnectionEpoch,
    activity_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReadyEvidence {
    connection_epoch: ConnectionEpoch,
    connection_open_generation: u64,
    subscription_generation: u64,
    ping_generation: u64,
    correlated_pong_generation: u64,
}

struct FinalCutCore {
    scope: PmWireScope,
    proxy_maker: EvmAddress,
    state_generation: u64,
    connection_epoch: ConnectionEpoch,
    connection_open_generation: u64,
    subscription_generation: u64,
    ping_generation: u64,
    correlated_pong_generation: u64,
    activity_generation: u64,
    business_basis: FinalCutBusinessBasis,
    business_events: Box<[PmUserStreamBusinessEventProjection]>,
}

enum UserStreamAdmission<'a> {
    ConnectionOpened,
    SubscriptionSent,
    PingSent,
    Pong,
    Business(&'a [PmLiveUserEvent]),
    ConnectionRetired(PmUserWsDisconnectReason),
    ReconnectScheduled {
        replacement_epoch: ConnectionEpoch,
        retired_generation: u64,
        reason: PmUserWsDisconnectReason,
    },
    RetryExhausted(PmUserWsDisconnectReason),
    Shutdown,
}

#[cfg(test)]
mod tests {
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        sync::atomic::{AtomicU64, Ordering},
    };

    use reap_pm_core::{PmConditionId, PmMarketId, PmTokenId, U256};
    use reap_polymarket_wire::{PmLiveUserFrame, parse_live_user_frame};

    use super::*;

    const CONDITION: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
    const FOREIGN_CONDITION: &str =
        "0x2222222222222222222222222222222222222222222222222222222222222222";
    const MARKET: &str = "0x3333333333333333333333333333333333333333333333333333333333333333";
    const PROXY: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    const FOREIGN_PROXY: &str = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC";
    const OWNER: &str = "00000000-0000-4000-8000-000000000001";
    const ORDER: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const MAKER_ORDER: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const FILL: &str = "fill-redaction-canary";
    const TRANSACTION: &str = "0xfeed-redaction-canary";

    fn scope() -> PmWireScope {
        PmWireScope::new(
            PmConditionId::parse(CONDITION).unwrap(),
            PmMarketId::parse(MARKET).unwrap(),
            PmTokenId::new(U256::from_u64(123)).unwrap(),
        )
    }

    fn proxy() -> EvmAddress {
        EvmAddress::parse(PROXY).unwrap()
    }

    struct Harness {
        state: PmUserStreamState,
        high_water: u64,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                state: PmUserStreamState::new(scope(), proxy()),
                high_water: 0,
            }
        }

        fn admit(
            &mut self,
            epoch: u64,
            admission: TestAdmission<'_>,
        ) -> Result<(), PmUserStreamRuntimeError> {
            self.high_water = self.high_water.checked_add(1).unwrap();
            self.state.admit_test_edge(
                scope().condition(),
                ConnectionEpoch::new(epoch),
                self.high_water,
                self.high_water,
                admission,
            )
        }

        fn ready(&mut self, epoch: u64) {
            self.admit(epoch, TestAdmission::Open).unwrap();
            self.admit(epoch, TestAdmission::Subscription).unwrap();
            self.admit(epoch, TestAdmission::Ping).unwrap();
            self.admit(epoch, TestAdmission::Pong).unwrap();
        }

        fn boundary(&self) -> Result<CollectionBoundary, PmUserStreamRuntimeError> {
            self.state.collection_boundary(self.high_water)
        }
    }

    #[derive(Clone, Copy)]
    enum Lifecycle {
        Open,
        Subscription,
        Ping,
        Pong,
    }

    enum TestAdmission<'a> {
        Open,
        Subscription,
        Ping,
        Pong,
        Business(&'a [PmLiveUserEvent]),
        Retired,
        Reconnect {
            retired_generation: u64,
            replacement_epoch: u64,
        },
    }

    impl PmUserStreamState {
        fn admit_test_edge(
            &mut self,
            condition: PmConditionId,
            epoch: ConnectionEpoch,
            generation: u64,
            source_high_water: u64,
            admission: TestAdmission<'_>,
        ) -> Result<(), PmUserStreamRuntimeError> {
            let result = self.admit_test_edge_inner(
                condition,
                epoch,
                generation,
                source_high_water,
                admission,
            );
            if result.is_err() {
                self.fail_closed(generation);
            }
            result
        }

        fn admit_test_edge_inner(
            &mut self,
            condition: PmConditionId,
            epoch: ConnectionEpoch,
            generation: u64,
            source_high_water: u64,
            admission: TestAdmission<'_>,
        ) -> Result<(), PmUserStreamRuntimeError> {
            self.validate_next_activity(generation, source_high_water)?;
            if condition != self.scope.condition() {
                return Err(PmUserStreamRuntimeError::ConnectionMismatch);
            }
            match admission {
                TestAdmission::Open => {
                    let replacement = match self.phase {
                        UserStreamPhase::AwaitingOpen { replacement_epoch } => replacement_epoch,
                        _ => return Err(PmUserStreamRuntimeError::LifecycleMismatch),
                    };
                    if replacement.is_some_and(|expected| expected != epoch)
                        || replacement.is_none() && self.last_connection_epoch.is_some()
                    {
                        return Err(PmUserStreamRuntimeError::ConnectionMismatch);
                    }
                    self.active_epoch = Some(epoch);
                    self.last_connection_epoch = Some(epoch);
                    self.connection_open_generation = Some(generation);
                    self.subscription_generation = None;
                    self.pending_ping_generation = None;
                    self.last_ping_generation = None;
                    self.correlated_pong_generation = None;
                    self.current_epoch_event_start = Some(self.business_events.len());
                    self.last_current_epoch_business_activity_generation = None;
                    self.retired = None;
                    self.phase = UserStreamPhase::Opened;
                }
                TestAdmission::Subscription => {
                    self.require_test_epoch(epoch)?;
                    if self.phase != UserStreamPhase::Opened {
                        return Err(PmUserStreamRuntimeError::LifecycleMismatch);
                    }
                    self.subscription_generation = Some(generation);
                    self.phase = UserStreamPhase::Subscribed;
                }
                TestAdmission::Ping => {
                    self.require_test_epoch(epoch)?;
                    if !matches!(
                        self.phase,
                        UserStreamPhase::Subscribed | UserStreamPhase::Ready
                    ) {
                        return Err(PmUserStreamRuntimeError::LifecycleMismatch);
                    }
                    self.pending_ping_generation = Some(generation);
                    self.last_ping_generation = Some(generation);
                    self.correlated_pong_generation = None;
                    self.phase = UserStreamPhase::AwaitingPong;
                }
                TestAdmission::Pong => {
                    self.require_test_epoch(epoch)?;
                    if self.phase != UserStreamPhase::AwaitingPong
                        || self.pending_ping_generation.take().is_none()
                    {
                        return Err(PmUserStreamRuntimeError::LifecycleMismatch);
                    }
                    self.correlated_pong_generation = Some(generation);
                    self.phase = UserStreamPhase::Ready;
                }
                TestAdmission::Business(events) => {
                    self.require_test_epoch(epoch)?;
                    if !matches!(
                        self.phase,
                        UserStreamPhase::Subscribed
                            | UserStreamPhase::AwaitingPong
                            | UserStreamPhase::Ready
                    ) {
                        return Err(PmUserStreamRuntimeError::LifecycleMismatch);
                    }
                    self.admit_business_events(events, epoch, generation)?;
                }
                TestAdmission::Retired => {
                    match self.phase {
                        UserStreamPhase::AwaitingOpen { replacement_epoch } => {
                            if replacement_epoch.is_some_and(|expected| expected != epoch)
                                || replacement_epoch.is_none()
                                    && self.last_connection_epoch.is_some()
                            {
                                return Err(PmUserStreamRuntimeError::ConnectionMismatch);
                            }
                            self.last_connection_epoch = Some(epoch);
                        }
                        UserStreamPhase::Opened
                        | UserStreamPhase::Subscribed
                        | UserStreamPhase::AwaitingPong
                        | UserStreamPhase::Ready => self.require_test_epoch(epoch)?,
                        _ => return Err(PmUserStreamRuntimeError::LifecycleMismatch),
                    }
                    self.retired = Some(RetiredConnection {
                        epoch,
                        activity_generation: generation,
                        reason: PmUserWsDisconnectReason::SocketClosed,
                    });
                    self.active_epoch = None;
                    self.clear_readiness();
                    self.phase = UserStreamPhase::Retired;
                }
                TestAdmission::Reconnect {
                    retired_generation,
                    replacement_epoch,
                } => {
                    let retired = self.retired.unwrap();
                    if self.phase != UserStreamPhase::Retired
                        || retired.epoch != epoch
                        || retired.activity_generation != retired_generation
                        || replacement_epoch != epoch.value() + 1
                    {
                        return Err(PmUserStreamRuntimeError::LifecycleMismatch);
                    }
                    self.phase = UserStreamPhase::AwaitingOpen {
                        replacement_epoch: Some(ConnectionEpoch::new(replacement_epoch)),
                    };
                }
            }
            self.finish_activity(generation)
        }

        fn require_test_epoch(
            &self,
            epoch: ConnectionEpoch,
        ) -> Result<(), PmUserStreamRuntimeError> {
            if self.active_epoch == Some(epoch) {
                Ok(())
            } else {
                Err(PmUserStreamRuntimeError::ConnectionMismatch)
            }
        }
    }

    fn permutations(values: &mut [Lifecycle], at: usize, output: &mut Vec<[Lifecycle; 4]>) {
        if at == values.len() {
            output.push([values[0], values[1], values[2], values[3]]);
            return;
        }
        for index in at..values.len() {
            values.swap(at, index);
            permutations(values, at + 1, output);
            values.swap(at, index);
        }
    }

    #[test]
    fn only_open_subscription_ping_correlated_pong_is_ready_across_all_permutations() {
        let mut all = Vec::new();
        permutations(
            &mut [
                Lifecycle::Open,
                Lifecycle::Subscription,
                Lifecycle::Ping,
                Lifecycle::Pong,
            ],
            0,
            &mut all,
        );
        assert_eq!(all.len(), 24);
        for order in all {
            let mut harness = Harness::new();
            let mut admitted = true;
            for edge in order {
                let admission = match edge {
                    Lifecycle::Open => TestAdmission::Open,
                    Lifecycle::Subscription => TestAdmission::Subscription,
                    Lifecycle::Ping => TestAdmission::Ping,
                    Lifecycle::Pong => TestAdmission::Pong,
                };
                if harness.admit(7, admission).is_err() {
                    admitted = false;
                    break;
                }
            }
            let exact = matches!(
                order,
                [
                    Lifecycle::Open,
                    Lifecycle::Subscription,
                    Lifecycle::Ping,
                    Lifecycle::Pong
                ]
            );
            assert_eq!(admitted && harness.boundary().is_ok(), exact);
        }
    }

    #[test]
    fn unsolicited_duplicate_and_post_retirement_pongs_fail_closed() {
        let mut unsolicited = Harness::new();
        unsolicited.admit(1, TestAdmission::Open).unwrap();
        unsolicited.admit(1, TestAdmission::Subscription).unwrap();
        assert_eq!(
            unsolicited.admit(1, TestAdmission::Pong),
            Err(PmUserStreamRuntimeError::LifecycleMismatch)
        );

        let mut duplicate = Harness::new();
        duplicate.ready(1);
        assert_eq!(
            duplicate.admit(1, TestAdmission::Pong),
            Err(PmUserStreamRuntimeError::LifecycleMismatch)
        );

        let mut retired = Harness::new();
        retired.ready(1);
        retired.admit(1, TestAdmission::Retired).unwrap();
        assert_eq!(
            retired.admit(1, TestAdmission::Pong),
            Err(PmUserStreamRuntimeError::ConnectionMismatch)
        );
    }

    fn order_frame(condition: &str, token: u64, maker: &str) -> PmLiveUserFrame {
        parse_live_user_frame(
            format!(
                r#"{{"event_type":"order","id":"{ORDER}","owner":"{OWNER}","market":"{condition}","asset_id":"{token}","side":"BUY","original_size":"10","size_matched":"2.5","price":"0.4","type":"PLACEMENT","maker_address":"{maker}","timestamp":"1782753357257","associate_trades":["{FILL}"],"outcome":"YES","created_at":"1782753357000","expiration":"1782753399999","order_type":"GTC","status":"LIVE"}}"#
            )
            .as_bytes(),
        )
        .unwrap()
    }

    fn trade_frame(condition: &str, token: u64, maker: &str) -> PmLiveUserFrame {
        parse_live_user_frame(
            format!(
                r#"{{"event_type":"trade","id":"{FILL}","owner":"{OWNER}","market":"{condition}","asset_id":"{token}","side":"SELL","size":"2.5","price":"0.4","status":"MATCHED","order_id":"{ORDER}","taker_order_id":"{ORDER}","trader_side":"TAKER","transaction_hash":"{TRANSACTION}","fee_rate_bps":"30","maker_orders":[],"maker_address":"{maker}","timestamp":"1782753357257","match_time":"1782753357258","last_update":"1782753357259"}}"#
            )
            .as_bytes(),
        )
        .unwrap()
    }

    fn trade_with_maker_leg_frame(
        condition: &str,
        top_token: u64,
        top_maker: &str,
        leg_token: u64,
        leg_maker: &str,
    ) -> PmLiveUserFrame {
        parse_live_user_frame(
            format!(
                r#"{{"event_type":"trade","id":"{FILL}","owner":"{OWNER}","market":"{condition}","asset_id":"{top_token}","side":"SELL","size":"2.5","price":"0.4","status":"MATCHED","order_id":"{ORDER}","transaction_hash":"{TRANSACTION}","fee_rate_bps":"30","maker_orders":[{{"order_id":"{MAKER_ORDER}","asset_id":"{leg_token}","side":"BUY","price":"0.4","matched_amount":"2.5","fee_rate_bps":"20","owner":"{OWNER}","maker_address":"{leg_maker}"}}],"maker_address":"{top_maker}","timestamp":"1782753357257","match_time":"1782753357258","last_update":"1782753357259"}}"#
            )
            .as_bytes(),
        )
        .unwrap()
    }

    #[test]
    fn every_bound_order_and_trade_requires_an_exact_scope_binding() {
        for frame in [
            order_frame(CONDITION, 123, PROXY),
            trade_frame(CONDITION, 123, PROXY),
            // Polymarket may put the sibling outcome at trade top-level. The
            // configured token and proxy maker leg is the exact binding.
            trade_with_maker_leg_frame(CONDITION, 456, FOREIGN_PROXY, 123, PROXY),
        ] {
            let mut harness = Harness::new();
            harness.ready(1);
            assert!(
                harness
                    .admit(1, TestAdmission::Business(frame.events()))
                    .is_ok()
            );
        }

        for frame in [
            order_frame(FOREIGN_CONDITION, 123, PROXY),
            order_frame(CONDITION, 456, PROXY),
            order_frame(CONDITION, 123, FOREIGN_PROXY),
            trade_frame(FOREIGN_CONDITION, 123, PROXY),
            trade_frame(CONDITION, 456, PROXY),
            trade_frame(CONDITION, 123, FOREIGN_PROXY),
            trade_with_maker_leg_frame(FOREIGN_CONDITION, 456, FOREIGN_PROXY, 123, PROXY),
            trade_with_maker_leg_frame(CONDITION, 456, PROXY, 456, PROXY),
            trade_with_maker_leg_frame(CONDITION, 456, PROXY, 123, FOREIGN_PROXY),
        ] {
            let mut harness = Harness::new();
            harness.ready(1);
            assert_eq!(
                harness.admit(1, TestAdmission::Business(frame.events())),
                Err(PmUserStreamRuntimeError::ForeignBusinessScope)
            );
            assert!(harness.state.business_events.is_empty());
            assert_eq!(
                harness.boundary(),
                Err(PmUserStreamRuntimeError::HeartbeatNotReady)
            );
        }
    }

    #[test]
    fn a_foreign_event_rejects_the_whole_bound_frame_without_partial_retention() {
        let frame = parse_live_user_frame(
            format!(
                r#"[{{"event_type":"order","id":"{ORDER}","owner":"{OWNER}","market":"{CONDITION}","asset_id":"123","side":"BUY","original_size":"10","size_matched":"0","price":"0.4","type":"PLACEMENT","maker_address":"{PROXY}","timestamp":"1782753357257"}},{{"event_type":"order","id":"{MAKER_ORDER}","owner":"{OWNER}","market":"{CONDITION}","asset_id":"456","side":"BUY","original_size":"10","size_matched":"0","price":"0.4","type":"PLACEMENT","maker_address":"{PROXY}","timestamp":"1782753357258"}}]"#
            )
            .as_bytes(),
        )
        .unwrap();
        let mut harness = Harness::new();
        harness.ready(1);
        assert_eq!(
            harness.admit(1, TestAdmission::Business(frame.events())),
            Err(PmUserStreamRuntimeError::ForeignBusinessScope)
        );
        assert!(harness.state.business_events.is_empty());
        assert_eq!(harness.state.phase, UserStreamPhase::Faulted);
    }

    #[test]
    fn owner_free_projections_retain_exact_phase_b_order_and_fill_facts() {
        let order_frame = order_frame(CONDITION, 123, PROXY);
        let trade_frame = trade_with_maker_leg_frame(CONDITION, 456, FOREIGN_PROXY, 123, PROXY);
        let wire_order = match &order_frame.events()[0] {
            PmLiveUserEvent::Order(order) => order,
            PmLiveUserEvent::Trade(_) => unreachable!(),
        };
        let wire_trade = match &trade_frame.events()[0] {
            PmLiveUserEvent::Trade(trade) => trade,
            PmLiveUserEvent::Order(_) => unreachable!(),
        };

        let mut harness = Harness::new();
        harness.ready(9);
        harness
            .admit(9, TestAdmission::Business(order_frame.events()))
            .unwrap();
        harness
            .admit(9, TestAdmission::Business(trade_frame.events()))
            .unwrap();
        assert_eq!(harness.state.business_events.len(), 2);

        let projected_order = match &harness.state.business_events[0] {
            PmUserStreamBusinessEventProjection::Order(order) => order,
            PmUserStreamBusinessEventProjection::Trade(_) => unreachable!(),
        };
        assert_eq!(projected_order.connection_epoch(), ConnectionEpoch::new(9));
        assert_eq!(projected_order.activity_generation(), 5);
        assert_eq!(projected_order.id(), wire_order.id());
        assert_eq!(projected_order.condition(), wire_order.condition());
        assert_eq!(projected_order.token(), wire_order.token());
        assert_eq!(projected_order.side(), wire_order.side());
        assert_eq!(projected_order.original_size(), wire_order.original_size());
        assert_eq!(projected_order.size_matched(), wire_order.size_matched());
        assert_eq!(projected_order.price(), wire_order.price());
        assert_eq!(projected_order.event_kind(), wire_order.event_kind());
        assert_eq!(projected_order.maker(), wire_order.maker().unwrap());
        assert_eq!(projected_order.expiration(), wire_order.expiration());
        assert_eq!(projected_order.order_type(), wire_order.order_type());
        assert_eq!(projected_order.outcome(), wire_order.outcome());
        assert_eq!(projected_order.status(), wire_order.status());
        assert_eq!(projected_order.created_at(), wire_order.created_at());
        assert_eq!(
            projected_order.associate_trades(),
            wire_order.associate_trades()
        );
        assert_eq!(projected_order.timestamp(), wire_order.timestamp());

        let projected_trade = match &harness.state.business_events[1] {
            PmUserStreamBusinessEventProjection::Trade(trade) => trade,
            PmUserStreamBusinessEventProjection::Order(_) => unreachable!(),
        };
        assert_eq!(projected_trade.connection_epoch(), ConnectionEpoch::new(9));
        assert_eq!(projected_trade.activity_generation(), 6);
        assert_eq!(projected_trade.id(), wire_trade.id());
        assert_eq!(projected_trade.condition(), wire_trade.condition());
        assert_eq!(projected_trade.token(), wire_trade.token());
        assert_eq!(projected_trade.side(), wire_trade.side());
        assert_eq!(projected_trade.size(), wire_trade.size());
        assert_eq!(projected_trade.price(), wire_trade.price());
        assert_eq!(projected_trade.status(), wire_trade.status());
        assert_eq!(projected_trade.order_id(), wire_trade.order_id());
        assert_eq!(
            projected_trade.taker_order_id(),
            wire_trade.taker_order_id()
        );
        assert_eq!(projected_trade.trader_side(), wire_trade.trader_side());
        assert_eq!(
            projected_trade.transaction_hash(),
            wire_trade.transaction_hash()
        );
        assert_eq!(projected_trade.fee_rate_bps(), wire_trade.fee_rate_bps());
        assert_eq!(projected_trade.maker(), wire_trade.maker());
        assert_eq!(projected_trade.timestamp(), wire_trade.timestamp());
        assert_eq!(projected_trade.match_time(), wire_trade.match_time());
        assert_eq!(projected_trade.last_update(), wire_trade.last_update());
        assert_eq!(projected_trade.maker_orders().len(), 1);
        let projected_leg = &projected_trade.maker_orders()[0];
        let wire_leg = &wire_trade.maker_orders()[0];
        assert_eq!(projected_leg.order_id(), wire_leg.order_id());
        assert_eq!(projected_leg.token(), wire_leg.token());
        assert_eq!(projected_leg.side(), wire_leg.side());
        assert_eq!(projected_leg.price(), wire_leg.price());
        assert_eq!(projected_leg.matched_amount(), wire_leg.matched_amount());
        assert_eq!(projected_leg.fee_rate_bps(), wire_leg.fee_rate_bps());
        assert_eq!(projected_leg.maker(), wire_leg.maker());

        let redacted = format!(
            "{projected_order:?} {projected_trade:?} {projected_leg:?} {:?}",
            harness.state.business_events
        );
        for forbidden in [
            OWNER,
            ORDER,
            MAKER_ORDER,
            FILL,
            TRANSACTION,
            "2.5",
            "0.4",
            "MATCHED",
            "PLACEMENT",
        ] {
            assert!(!redacted.contains(forbidden), "debug leaked {forbidden}");
        }
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn cap_overflow_faults_without_truncating_or_partially_appending() {
        let frame = order_frame(CONDITION, 123, PROXY);
        let mut harness = Harness::new();
        harness.ready(1);
        for _ in 0..MAX_RETAINED_USER_BUSINESS_EVENTS {
            harness
                .admit(1, TestAdmission::Business(frame.events()))
                .unwrap();
        }
        assert_eq!(
            harness.admit(1, TestAdmission::Business(frame.events())),
            Err(PmUserStreamRuntimeError::BusinessEvidenceCapacityExceeded)
        );
        assert_eq!(
            harness.state.business_events.len(),
            MAX_RETAINED_USER_BUSINESS_EVENTS
        );
        assert_eq!(harness.state.phase, UserStreamPhase::Faulted);
    }

    #[test]
    fn queued_activity_invalidates_start_rest_completion_and_final_ticket_core() {
        let mut before_start = Harness::new();
        before_start.ready(1);
        assert_eq!(
            before_start
                .state
                .collection_boundary(before_start.high_water + 1),
            Err(PmUserStreamRuntimeError::ConcurrentActivity)
        );

        let mut during_rest = Harness::new();
        during_rest.ready(1);
        let boundary = during_rest.boundary().unwrap();
        assert!(matches!(
            during_rest
                .state
                .finish_rest_collection(boundary, during_rest.high_water + 1, 0, 0),
            Err(PmUserStreamRuntimeError::TicketInvalidated)
        ));

        let mut before_dispatch = Harness::new();
        before_dispatch.ready(1);
        let boundary = before_dispatch.boundary().unwrap();
        let ticket = before_dispatch
            .state
            .finish_rest_collection(boundary, before_dispatch.high_water, 0, 0)
            .unwrap();
        assert_eq!(
            before_dispatch
                .state
                .validate_final_core(&ticket, before_dispatch.high_water + 1),
            Err(PmUserStreamRuntimeError::TicketInvalidated)
        );
    }

    #[test]
    fn retirement_and_reconnect_invalidate_old_cut_and_require_the_replacement_sequence() {
        let mut harness = Harness::new();
        harness.ready(4);
        let old_boundary = harness.boundary().unwrap();
        harness.admit(4, TestAdmission::Retired).unwrap();
        assert_eq!(
            harness
                .state
                .validate_boundary(old_boundary, harness.high_water),
            Err(PmUserStreamRuntimeError::HeartbeatNotReady)
        );
        let retired_generation = harness.high_water;
        harness
            .admit(
                4,
                TestAdmission::Reconnect {
                    retired_generation,
                    replacement_epoch: 5,
                },
            )
            .unwrap();
        harness.admit(5, TestAdmission::Open).unwrap();
        harness.admit(5, TestAdmission::Subscription).unwrap();
        harness.admit(5, TestAdmission::Ping).unwrap();
        harness.admit(5, TestAdmission::Pong).unwrap();
        assert_eq!(harness.boundary().unwrap().connection_epoch.value(), 5);
    }

    #[test]
    fn reconnect_preserves_the_full_log_but_only_current_epoch_events_select_the_basis() {
        let order = order_frame(CONDITION, 123, PROXY);
        let trade = trade_frame(CONDITION, 123, PROXY);
        let mut harness = Harness::new();
        harness.ready(4);
        harness
            .admit(4, TestAdmission::Business(order.events()))
            .unwrap();
        harness.admit(4, TestAdmission::Retired).unwrap();
        let retired_generation = harness.high_water;
        harness
            .admit(
                4,
                TestAdmission::Reconnect {
                    retired_generation,
                    replacement_epoch: 5,
                },
            )
            .unwrap();
        harness.ready(5);

        let boundary = harness.boundary().unwrap();
        let quiet_current_epoch = harness
            .state
            .finish_rest_collection(boundary, harness.high_water, 0, 0)
            .unwrap();
        assert_eq!(quiet_current_epoch.business_events.len(), 1);
        assert_eq!(
            quiet_current_epoch.business_events[0].connection_epoch(),
            ConnectionEpoch::new(4)
        );
        assert!(matches!(
            quiet_current_epoch.business_basis,
            FinalCutBusinessBasis::RestBackedNoStreamEvents {
                current_epoch_event_start: 1,
                ..
            }
        ));

        harness
            .admit(5, TestAdmission::Business(trade.events()))
            .unwrap();
        let boundary = harness.boundary().unwrap();
        let joined = harness
            .state
            .finish_rest_collection(boundary, harness.high_water, 0, 1)
            .unwrap();
        assert_eq!(joined.business_events.len(), 2);
        assert_eq!(
            joined.business_events[0].connection_epoch(),
            ConnectionEpoch::new(4)
        );
        assert_eq!(
            joined.business_events[1].connection_epoch(),
            ConnectionEpoch::new(5)
        );
        assert!(matches!(
            joined.business_basis,
            FinalCutBusinessBasis::StreamEventsAndRestJoined {
                current_epoch_event_start: 1,
                current_epoch_event_count: 1,
                ..
            }
        ));
    }

    #[test]
    fn pre_open_failure_can_only_reconnect_into_the_exact_next_epoch() {
        let mut harness = Harness::new();
        harness.admit(11, TestAdmission::Retired).unwrap();
        let retired_generation = harness.high_water;
        harness
            .admit(
                11,
                TestAdmission::Reconnect {
                    retired_generation,
                    replacement_epoch: 12,
                },
            )
            .unwrap();
        assert_eq!(
            harness.admit(13, TestAdmission::Open),
            Err(PmUserStreamRuntimeError::ConnectionMismatch)
        );

        let mut exact = Harness::new();
        exact.admit(11, TestAdmission::Retired).unwrap();
        let retired_generation = exact.high_water;
        exact
            .admit(
                11,
                TestAdmission::Reconnect {
                    retired_generation,
                    replacement_epoch: 12,
                },
            )
            .unwrap();
        exact.admit(12, TestAdmission::Open).unwrap();
        exact.admit(12, TestAdmission::Subscription).unwrap();
        exact.admit(12, TestAdmission::Ping).unwrap();
        exact.admit(12, TestAdmission::Pong).unwrap();
        assert_eq!(exact.boundary().unwrap().connection_epoch.value(), 12);
    }

    #[test]
    fn an_admitted_business_frame_invalidates_an_older_final_ticket_core() {
        let mut harness = Harness::new();
        harness.ready(1);
        let boundary = harness.boundary().unwrap();
        let ticket = harness
            .state
            .finish_rest_collection(boundary, harness.high_water, 0, 0)
            .unwrap();
        let frame = order_frame(CONDITION, 123, PROXY);
        harness
            .admit(1, TestAdmission::Business(frame.events()))
            .unwrap();
        assert_eq!(
            harness
                .state
                .validate_final_core(&ticket, harness.high_water),
            Err(PmUserStreamRuntimeError::TicketInvalidated)
        );
    }

    #[test]
    fn source_generation_gap_duplicate_and_regression_are_terminal() {
        for observed in [0, 2, 7] {
            let mut state = PmUserStreamState::new(scope(), proxy());
            assert_eq!(
                state.admit_test_edge(
                    scope().condition(),
                    ConnectionEpoch::new(1),
                    observed,
                    observed,
                    TestAdmission::Open,
                ),
                Err(if observed == 0 || observed == 2 || observed == 7 {
                    PmUserStreamRuntimeError::ActivityDiscontinuity
                } else {
                    unreachable!()
                })
            );
            assert_eq!(state.phase, UserStreamPhase::Faulted);
        }

        let mut duplicate = Harness::new();
        duplicate.admit(1, TestAdmission::Open).unwrap();
        assert_eq!(
            duplicate.state.admit_test_edge(
                scope().condition(),
                ConnectionEpoch::new(1),
                1,
                1,
                TestAdmission::Subscription,
            ),
            Err(PmUserStreamRuntimeError::ActivityDiscontinuity)
        );
    }

    #[test]
    fn zero_and_nonzero_stream_business_have_distinct_rest_joined_bases() {
        let mut empty = Harness::new();
        empty.ready(1);
        let boundary = empty.boundary().unwrap();
        let empty_ticket = empty
            .state
            .finish_rest_collection(boundary, empty.high_water, 2, 3)
            .unwrap();
        assert!(matches!(
            empty_ticket.business_basis,
            FinalCutBusinessBasis::RestBackedNoStreamEvents {
                open_order_rows: 2,
                trade_rows: 3,
                ..
            }
        ));
        assert!(empty_ticket.business_events.is_empty());

        let frame = order_frame(CONDITION, 123, PROXY);
        let mut observed = Harness::new();
        observed.ready(1);
        observed
            .admit(1, TestAdmission::Business(frame.events()))
            .unwrap();
        let boundary = observed.boundary().unwrap();
        let observed_ticket = observed
            .state
            .finish_rest_collection(boundary, observed.high_water, 4, 5)
            .unwrap();
        assert!(matches!(
            observed_ticket.business_basis,
            FinalCutBusinessBasis::StreamEventsAndRestJoined {
                current_epoch_event_count: 1,
                open_order_rows: 4,
                trade_rows: 5,
                ..
            }
        ));
        assert_eq!(observed_ticket.business_events.len(), 1);
    }

    #[test]
    fn explicit_and_drop_task_exit_prevent_mint_even_after_clean_ready_flow() {
        let activity = Arc::new(AtomicU64::new(4));
        let shared = Arc::new(PmUserStreamShared {
            state: Mutex::new({
                let mut harness = Harness::new();
                harness.ready(1);
                harness.state
            }),
            activity: RuntimeUserActivityView::Test(Arc::clone(&activity)),
        });
        let mut explicit = PmUserStreamSink {
            shared: Arc::clone(&shared),
            task_exit_recorded: false,
        };
        explicit.invalidate_task_exit().unwrap();
        assert_eq!(
            shared.state.lock().unwrap().collection_boundary(4),
            Err(PmUserStreamRuntimeError::HeartbeatNotReady)
        );
        drop(explicit);

        let second_shared = Arc::new(PmUserStreamShared {
            state: Mutex::new({
                let mut harness = Harness::new();
                harness.ready(1);
                harness.state
            }),
            activity: RuntimeUserActivityView::Test(activity),
        });
        drop(PmUserStreamSink {
            shared: Arc::clone(&second_shared),
            task_exit_recorded: false,
        });
        assert_eq!(
            second_shared.state.lock().unwrap().collection_boundary(4),
            Err(PmUserStreamRuntimeError::HeartbeatNotReady)
        );
    }

    #[tokio::test]
    async fn cancelling_a_task_owned_sink_invalidates_rest_collection_start() {
        let activity = Arc::new(AtomicU64::new(4));
        let shared = Arc::new(PmUserStreamShared {
            state: Mutex::new({
                let mut harness = Harness::new();
                harness.ready(1);
                harness.state
            }),
            activity: RuntimeUserActivityView::Test(activity),
        });
        let sink = PmUserStreamSink {
            shared: Arc::clone(&shared),
            task_exit_recorded: false,
        };
        let task = tokio::spawn(async move {
            // Keep the sink alive across the suspension point exactly as
            // `PmUserStreamWsTask::run_to_exit` does while the role runs.
            std::future::pending::<()>().await;
            drop(sink);
        });
        tokio::task::yield_now().await;
        task.abort();
        assert!(task.await.is_err());
        assert_eq!(
            shared.state.lock().unwrap().collection_boundary(4),
            Err(PmUserStreamRuntimeError::HeartbeatNotReady)
        );
    }

    #[test]
    fn poisoned_state_is_terminal_and_never_recovers_inner() {
        let shared = Arc::new(PmUserStreamShared {
            state: Mutex::new(PmUserStreamState::new(scope(), proxy())),
            activity: RuntimeUserActivityView::Test(Arc::new(AtomicU64::new(0))),
        });
        let poison = Arc::clone(&shared);
        let _ = catch_unwind(AssertUnwindSafe(move || {
            let _guard = poison.state.lock().unwrap();
            panic!("synthetic state poison");
        }));
        let mut sink = PmUserStreamSink {
            shared,
            task_exit_recorded: false,
        };
        assert_eq!(
            sink.invalidate_task_exit(),
            Err(PmUserStreamRuntimeError::StatePoisoned)
        );
        drop(sink);
    }

    #[test]
    fn high_water_test_view_is_source_read_only_to_runtime_commands() {
        let activity = Arc::new(AtomicU64::new(4));
        let view = RuntimeUserActivityView::Test(Arc::clone(&activity));
        assert_eq!(view.generation(), 4);
        activity.store(5, Ordering::Release);
        assert_eq!(view.generation(), 5);
    }
}
