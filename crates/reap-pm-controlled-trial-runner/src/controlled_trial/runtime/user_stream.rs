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
    time::Duration,
};

use async_trait::async_trait;
use reap_pm_core::{
    ConnectionEpoch, EvmAddress, PmBookQuantity, PmConditionId, PmFillId, PmOrderSide, PmPrice,
    PmQuantity, PmTokenId, PmVenueOrderId, U256,
};
use reap_polymarket_live_adapter::{
    PmAuthenticatedUserWsRole, PmUserWsActivityView, PmUserWsConnection, PmUserWsDisconnectReason,
    PmUserWsEdgeClock, PmUserWsEvent, PmUserWsEventSink, PmUserWsRunError, PmUserWsShutdownSignal,
};
use reap_polymarket_wire::{PmLiveUserEvent, PmWireScope};
use thiserror::Error;

use super::private_reads::{
    PmFreshAuthenticatedRestCut, PmRestCutIdentity, PmSameAuthorityRestJoin,
    PmSameCredentialAuthorityMarker, PmSameCredentialUserWsInput, PmUserRestCollectionStart,
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

impl PmUserStreamShared {
    /// Recheck one immutable online-preflight core against both the admitted
    /// state and the source-owned activity high-water. Sampling on both sides
    /// of each state validation rejects activity already queued at the socket
    /// edge as well as activity admitted while this command is in progress.
    fn recheck_online_preflight_core(
        &self,
        core: &FinalCutCore,
    ) -> Result<(), PmUserStreamRuntimeError> {
        let before = self.activity.generation();
        {
            let state = self
                .state
                .lock()
                .map_err(|_| PmUserStreamRuntimeError::StatePoisoned)?;
            state.validate_final_core(core, before)?;
        }
        let after = self.activity.generation();
        if before != core.activity_generation || after != core.activity_generation {
            return Err(PmUserStreamRuntimeError::ConcurrentActivity);
        }
        {
            let state = self
                .state
                .lock()
                .map_err(|_| PmUserStreamRuntimeError::StatePoisoned)?;
            state.validate_final_core(core, after)?;
        }
        if self.activity.generation() != core.activity_generation {
            return Err(PmUserStreamRuntimeError::ConcurrentActivity);
        }
        Ok(())
    }
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
        self.consume_online_preflight_lease(ticket.into_online_preflight_lease())
    }

    /// Consume and reissue the same move-only online-preflight lease only while
    /// its complete source core, authority marker, reconnect history, clocks
    /// and activity high-water still exactly match the live runtime.
    pub(super) fn recheck_online_preflight_lease(
        &mut self,
        lease: PmUserOnlinePreflightLease,
    ) -> Result<PmUserOnlinePreflightLease, PmUserStreamRuntimeError> {
        self.recheck_online_preflight_ticket(&lease.ticket)?;
        Ok(lease)
    }

    /// Final consuming lease recheck. No ticket or lease survives this call;
    /// on success it releases the already-existing typed join fields, while a
    /// failure drops the sole stale lease.
    pub(super) fn consume_online_preflight_lease(
        &mut self,
        lease: PmUserOnlinePreflightLease,
    ) -> Result<FinalCutJoinFields, PmUserStreamRuntimeError> {
        let lease = self.recheck_online_preflight_lease(lease)?;
        Ok(lease.ticket.into_join_fields())
    }

    fn recheck_online_preflight_ticket(
        &self,
        ticket: &FinalCutTicket,
    ) -> Result<(), PmUserStreamRuntimeError> {
        if !ticket.marker.same_instance(&self.marker) {
            return Err(PmUserStreamRuntimeError::SameAuthorityMismatch);
        }
        self.shared.recheck_online_preflight_core(&ticket.core)
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

impl FinalCutTicket {
    /// Convert the sole final ticket into the sole online-preflight lease. The
    /// conversion consumes every field, so a ticket and lease cannot coexist.
    pub(super) fn into_online_preflight_lease(self) -> PmUserOnlinePreflightLease {
        PmUserOnlinePreflightLease { ticket: self }
    }

    fn into_join_fields(self) -> FinalCutJoinFields {
        let FinalCutTicket {
            core,
            marker,
            rest_cut_identity,
        } = self;
        FinalCutJoinFields {
            scope: core.scope,
            proxy_maker: core.proxy_maker,
            stream_revision: core.state_generation,
            initial_connection_epoch: core.initial_connection_epoch,
            connection_epoch: core.connection_epoch,
            connection_open_generation: core.connection_open_generation,
            connection_open_clock: core.connection_open_clock,
            subscription_generation: core.subscription_generation,
            subscription_clock: core.subscription_clock,
            ping_generation: core.ping_generation,
            ping_clock: core.ping_clock,
            correlated_pong_generation: core.correlated_pong_generation,
            correlated_pong_clock: core.correlated_pong_clock,
            admitted_activity_generation: core.activity_generation,
            reconnect_count: core.reconnect_count,
            reconnect_history: core.reconnect_history,
            business_basis: core.business_basis,
            business_events: core.business_events,
            _same_authority: marker,
            rest_cut_identity,
        }
    }
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

/// Move-only online preflight lease retaining the complete same-authority
/// user/REST ticket. Inspection is borrowed only; no caller can supply a
/// clock, generation, hash, authority marker, event, or REST identity.
#[must_use = "the online preflight lease must be rechecked and consumed"]
pub(super) struct PmUserOnlinePreflightLease {
    ticket: FinalCutTicket,
}

impl PmUserOnlinePreflightLease {
    /// Exact L2 signer retained by the same-credential marker that owns both
    /// this user-stream ticket and its authenticated REST allocation. This is
    /// a borrowed value projection, never a caller-supplied account claim.
    #[must_use]
    pub(super) fn signer(&self) -> reap_polymarket_auth::EoaAddress {
        self.ticket.marker.signer()
    }

    #[must_use]
    pub(super) const fn scope(&self) -> PmWireScope {
        self.ticket.core.scope
    }

    #[must_use]
    pub(super) const fn proxy_maker(&self) -> EvmAddress {
        self.ticket.core.proxy_maker
    }

    #[must_use]
    pub(super) const fn stream_revision(&self) -> u64 {
        self.ticket.core.state_generation
    }

    #[must_use]
    pub(super) const fn initial_connection_epoch(&self) -> ConnectionEpoch {
        self.ticket.core.initial_connection_epoch
    }

    #[must_use]
    pub(super) const fn current_connection_epoch(&self) -> ConnectionEpoch {
        self.ticket.core.connection_epoch
    }

    #[must_use]
    pub(super) const fn connection_open_generation(&self) -> u64 {
        self.ticket.core.connection_open_generation
    }

    #[must_use]
    pub(super) const fn connection_open_clock(&self) -> PmUserWsEdgeClock {
        self.ticket.core.connection_open_clock
    }

    #[must_use]
    pub(super) const fn subscription_generation(&self) -> u64 {
        self.ticket.core.subscription_generation
    }

    #[must_use]
    pub(super) const fn subscription_clock(&self) -> PmUserWsEdgeClock {
        self.ticket.core.subscription_clock
    }

    #[must_use]
    pub(super) const fn ping_generation(&self) -> u64 {
        self.ticket.core.ping_generation
    }

    #[must_use]
    pub(super) const fn ping_clock(&self) -> PmUserWsEdgeClock {
        self.ticket.core.ping_clock
    }

    #[must_use]
    pub(super) const fn correlated_pong_generation(&self) -> u64 {
        self.ticket.core.correlated_pong_generation
    }

    #[must_use]
    pub(super) const fn correlated_pong_clock(&self) -> PmUserWsEdgeClock {
        self.ticket.core.correlated_pong_clock
    }

    #[must_use]
    pub(super) const fn admitted_activity_generation(&self) -> u64 {
        self.ticket.core.activity_generation
    }

    #[must_use]
    pub(super) const fn reconnect_count(&self) -> u8 {
        self.ticket.core.reconnect_count
    }

    #[must_use]
    pub(super) fn reconnect_history(&self) -> &[PmUserStreamReconnectEvidence] {
        &self.ticket.core.reconnect_history
    }

    #[must_use]
    pub(super) const fn business_basis(&self) -> &FinalCutBusinessBasis {
        &self.ticket.core.business_basis
    }

    #[must_use]
    pub(super) fn business_events(&self) -> &[PmUserStreamBusinessEventProjection] {
        &self.ticket.core.business_events
    }

    #[must_use]
    pub(super) fn current_epoch_business_events(&self) -> &[PmUserStreamBusinessEventProjection] {
        let (start, count) = self.ticket.core.business_basis.current_epoch_event_range();
        &self.ticket.core.business_events[start..start + count]
    }

    /// Join the retained allocation to the exact fresh authenticated REST cut
    /// without releasing either side's `Arc`.
    #[must_use]
    pub(super) fn matches_fresh_rest_cut(&self, cut: &PmFreshAuthenticatedRestCut) -> bool {
        cut.matches_rest_cut_identity(&self.ticket.rest_cut_identity)
    }

    #[must_use]
    pub(super) const fn open_order_rows(&self) -> usize {
        self.ticket.core.business_basis.rest_row_counts().0
    }

    #[must_use]
    pub(super) const fn trade_rows(&self) -> usize {
        self.ticket.core.business_basis.rest_row_counts().1
    }
}

impl fmt::Debug for PmUserOnlinePreflightLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmUserOnlinePreflightLease")
            .field("stream_revision", &self.stream_revision())
            .field("initial_epoch", &self.initial_connection_epoch())
            .field("current_epoch", &self.current_connection_epoch())
            .field(
                "admitted_activity_generation",
                &self.admitted_activity_generation(),
            )
            .field("reconnect_count", &self.reconnect_count())
            .field("business_basis", &self.business_basis())
            .field("rest", &"<same-authority opaque allocation>")
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
    initial_connection_epoch: ConnectionEpoch,
    connection_epoch: ConnectionEpoch,
    connection_open_generation: u64,
    connection_open_clock: PmUserWsEdgeClock,
    subscription_generation: u64,
    subscription_clock: PmUserWsEdgeClock,
    ping_generation: u64,
    ping_clock: PmUserWsEdgeClock,
    correlated_pong_generation: u64,
    correlated_pong_clock: PmUserWsEdgeClock,
    admitted_activity_generation: u64,
    reconnect_count: u8,
    reconnect_history: Box<[PmUserStreamReconnectEvidence]>,
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
    pub(super) const fn initial_connection_epoch(&self) -> ConnectionEpoch {
        self.initial_connection_epoch
    }

    #[must_use]
    pub(super) const fn connection_epoch(&self) -> ConnectionEpoch {
        self.connection_epoch
    }

    #[must_use]
    pub(super) const fn current_connection_epoch(&self) -> ConnectionEpoch {
        self.connection_epoch
    }

    #[must_use]
    pub(super) const fn connection_open_generation(&self) -> u64 {
        self.connection_open_generation
    }

    #[must_use]
    pub(super) const fn connection_open_clock(&self) -> PmUserWsEdgeClock {
        self.connection_open_clock
    }

    #[must_use]
    pub(super) const fn subscription_generation(&self) -> u64 {
        self.subscription_generation
    }

    #[must_use]
    pub(super) const fn subscription_clock(&self) -> PmUserWsEdgeClock {
        self.subscription_clock
    }

    #[must_use]
    pub(super) const fn ping_generation(&self) -> u64 {
        self.ping_generation
    }

    #[must_use]
    pub(super) const fn latest_ping_clock(&self) -> PmUserWsEdgeClock {
        self.ping_clock
    }

    #[must_use]
    pub(super) const fn correlated_pong_generation(&self) -> u64 {
        self.correlated_pong_generation
    }

    /// Source-issued monotonic PONG edge retained for the later same-domain
    /// public-book lease age comparison. This module does not claim freshness.
    #[must_use]
    pub(super) const fn correlated_pong_clock(&self) -> PmUserWsEdgeClock {
        self.correlated_pong_clock
    }

    #[must_use]
    pub(super) const fn admitted_activity_generation(&self) -> u64 {
        self.admitted_activity_generation
    }

    #[must_use]
    pub(super) const fn reconnect_count(&self) -> u8 {
        self.reconnect_count
    }

    #[must_use]
    pub(super) fn reconnect_history(&self) -> &[PmUserStreamReconnectEvidence] {
        &self.reconnect_history
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

    /// Join the retained allocation to the exact fresh authenticated REST cut
    /// without releasing either side's `Arc`.
    #[must_use]
    pub(super) fn matches_fresh_rest_cut(&self, cut: &PmFreshAuthenticatedRestCut) -> bool {
        cut.matches_rest_cut_identity(&self.rest_cut_identity)
    }
}

impl fmt::Debug for FinalCutJoinFields {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FinalCutJoinFields")
            .field("scope", &"<fixed; redacted>")
            .field("proxy_maker", &"<redacted>")
            .field("stream_revision", &self.stream_revision)
            .field("initial_epoch", &self.initial_connection_epoch)
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
            .field("reconnect_count", &self.reconnect_count)
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
    #[error("user-stream source monotonic clock regressed")]
    ClockRegression,
    #[error("user-stream lifecycle generation and source-clock evidence disagreed")]
    ClockEvidenceMismatch,
    #[error("user-stream reconnect attempt or retained history was inconsistent")]
    ReconnectHistoryMismatch,
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
    clock: PmUserWsEdgeClock,
    reason: PmUserWsDisconnectReason,
    connection_open: Option<UserStreamLifecycleEdge>,
    subscription: Option<UserStreamLifecycleEdge>,
    latest_ping: Option<UserStreamLifecycleEdge>,
    correlated_pong: Option<UserStreamLifecycleEdge>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UserStreamLifecycleEdge {
    activity_generation: u64,
    clock: PmUserWsEdgeClock,
}

/// Closed, source-issued history for one retired connection and its scheduled
/// replacement. No caller clock or inferred timestamp can enter this value.
#[derive(PartialEq, Eq)]
pub(super) struct PmUserStreamReconnectEvidence {
    retired_epoch: ConnectionEpoch,
    replacement_epoch: ConnectionEpoch,
    reconnect_attempt: u8,
    reason: PmUserWsDisconnectReason,
    backoff: Duration,
    connection_open: Option<UserStreamLifecycleEdge>,
    subscription: Option<UserStreamLifecycleEdge>,
    latest_ping: Option<UserStreamLifecycleEdge>,
    correlated_pong: Option<UserStreamLifecycleEdge>,
    retirement: UserStreamLifecycleEdge,
    reconnect_scheduled: UserStreamLifecycleEdge,
}

impl PmUserStreamReconnectEvidence {
    #[must_use]
    pub(super) const fn retired_epoch(&self) -> ConnectionEpoch {
        self.retired_epoch
    }

    #[must_use]
    pub(super) const fn replacement_epoch(&self) -> ConnectionEpoch {
        self.replacement_epoch
    }

    #[must_use]
    pub(super) const fn reconnect_attempt(&self) -> u8 {
        self.reconnect_attempt
    }

    #[must_use]
    pub(super) const fn reason(&self) -> PmUserWsDisconnectReason {
        self.reason
    }

    #[must_use]
    pub(super) const fn backoff(&self) -> Duration {
        self.backoff
    }

    #[must_use]
    pub(super) const fn connection_open_generation(&self) -> Option<u64> {
        match self.connection_open {
            Some(edge) => Some(edge.activity_generation),
            None => None,
        }
    }

    #[must_use]
    pub(super) const fn connection_open_clock(&self) -> Option<PmUserWsEdgeClock> {
        match self.connection_open {
            Some(edge) => Some(edge.clock),
            None => None,
        }
    }

    #[must_use]
    pub(super) const fn subscription_generation(&self) -> Option<u64> {
        match self.subscription {
            Some(edge) => Some(edge.activity_generation),
            None => None,
        }
    }

    #[must_use]
    pub(super) const fn subscription_clock(&self) -> Option<PmUserWsEdgeClock> {
        match self.subscription {
            Some(edge) => Some(edge.clock),
            None => None,
        }
    }

    #[must_use]
    pub(super) const fn latest_ping_generation(&self) -> Option<u64> {
        match self.latest_ping {
            Some(edge) => Some(edge.activity_generation),
            None => None,
        }
    }

    #[must_use]
    pub(super) const fn latest_ping_clock(&self) -> Option<PmUserWsEdgeClock> {
        match self.latest_ping {
            Some(edge) => Some(edge.clock),
            None => None,
        }
    }

    #[must_use]
    pub(super) const fn correlated_pong_generation(&self) -> Option<u64> {
        match self.correlated_pong {
            Some(edge) => Some(edge.activity_generation),
            None => None,
        }
    }

    #[must_use]
    pub(super) const fn correlated_pong_clock(&self) -> Option<PmUserWsEdgeClock> {
        match self.correlated_pong {
            Some(edge) => Some(edge.clock),
            None => None,
        }
    }

    #[must_use]
    pub(super) const fn retirement_activity_generation(&self) -> u64 {
        self.retirement.activity_generation
    }

    #[must_use]
    pub(super) const fn retirement_clock(&self) -> PmUserWsEdgeClock {
        self.retirement.clock
    }

    #[must_use]
    pub(super) const fn reconnect_activity_generation(&self) -> u64 {
        self.reconnect_scheduled.activity_generation
    }

    #[must_use]
    pub(super) const fn reconnect_clock(&self) -> PmUserWsEdgeClock {
        self.reconnect_scheduled.clock
    }

    fn duplicate_for_ticket(&self) -> Self {
        Self {
            retired_epoch: self.retired_epoch,
            replacement_epoch: self.replacement_epoch,
            reconnect_attempt: self.reconnect_attempt,
            reason: self.reason,
            backoff: self.backoff,
            connection_open: self.connection_open,
            subscription: self.subscription,
            latest_ping: self.latest_ping,
            correlated_pong: self.correlated_pong,
            retirement: self.retirement,
            reconnect_scheduled: self.reconnect_scheduled,
        }
    }
}

impl fmt::Debug for PmUserStreamReconnectEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PmUserStreamReconnectEvidence")
            .field("retired_epoch", &self.retired_epoch)
            .field("replacement_epoch", &self.replacement_epoch)
            .field("reconnect_attempt", &self.reconnect_attempt)
            .field("reason", &self.reason)
            .field("source_clocks_retained", &true)
            .finish_non_exhaustive()
    }
}

struct PmUserStreamState {
    scope: PmWireScope,
    proxy_maker: EvmAddress,
    phase: UserStreamPhase,
    task_active: bool,
    state_generation: u64,
    admitted_activity_generation: u64,
    last_admitted_clock: Option<PmUserWsEdgeClock>,
    initial_connection_epoch: Option<ConnectionEpoch>,
    active_epoch: Option<ConnectionEpoch>,
    last_connection_epoch: Option<ConnectionEpoch>,
    connection_open_generation: Option<u64>,
    connection_open_clock: Option<PmUserWsEdgeClock>,
    subscription_generation: Option<u64>,
    subscription_clock: Option<PmUserWsEdgeClock>,
    pending_ping_generation: Option<u64>,
    pending_ping_clock: Option<PmUserWsEdgeClock>,
    last_ping_generation: Option<u64>,
    last_ping_clock: Option<PmUserWsEdgeClock>,
    correlated_pong_generation: Option<u64>,
    correlated_pong_clock: Option<PmUserWsEdgeClock>,
    business_events: Vec<PmUserStreamBusinessEventProjection>,
    current_epoch_event_start: Option<usize>,
    last_current_epoch_business_activity_generation: Option<u64>,
    retired: Option<RetiredConnection>,
    reconnect_history: Vec<PmUserStreamReconnectEvidence>,
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
            last_admitted_clock: None,
            initial_connection_epoch: None,
            active_epoch: None,
            last_connection_epoch: None,
            connection_open_generation: None,
            connection_open_clock: None,
            subscription_generation: None,
            subscription_clock: None,
            pending_ping_generation: None,
            pending_ping_clock: None,
            last_ping_generation: None,
            last_ping_clock: None,
            correlated_pong_generation: None,
            correlated_pong_clock: None,
            business_events: Vec::new(),
            current_epoch_event_start: None,
            last_current_epoch_business_activity_generation: None,
            retired: None,
            reconnect_history: Vec::new(),
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
                observation.clock(),
                UserStreamAdmission::ConnectionOpened,
            ),
            PmUserWsEvent::SubscriptionSent(observation) => self.admit_edge(
                observation.connection(),
                generation,
                source_high_water,
                observation.clock(),
                UserStreamAdmission::SubscriptionSent,
            ),
            PmUserWsEvent::PingSent(observation) => self.admit_edge(
                observation.connection(),
                generation,
                source_high_water,
                observation.clock(),
                UserStreamAdmission::PingSent,
            ),
            PmUserWsEvent::Pong(observation) => self.admit_edge(
                observation.connection(),
                generation,
                source_high_water,
                observation.clock(),
                UserStreamAdmission::Pong,
            ),
            PmUserWsEvent::BoundFrame(frame) => {
                let observation = frame.observation();
                self.admit_edge(
                    observation.connection(),
                    generation,
                    source_high_water,
                    observation.clock(),
                    UserStreamAdmission::Business(frame.events()),
                )
            }
            PmUserWsEvent::ConnectionRetired(retired) => {
                let observation = retired.observation();
                self.admit_edge(
                    observation.connection(),
                    generation,
                    source_high_water,
                    observation.clock(),
                    UserStreamAdmission::ConnectionRetired(retired.reason()),
                )
            }
            PmUserWsEvent::ReconnectScheduled(reconnect) => {
                let retired = reconnect.retired();
                let retired_observation = retired.observation();
                self.admit_edge(
                    retired_observation.connection(),
                    generation,
                    source_high_water,
                    reconnect.scheduled_clock(),
                    UserStreamAdmission::ReconnectScheduled {
                        replacement_epoch: reconnect.replacement_epoch(),
                        retired_generation: retired_observation.activity_generation(),
                        retired_clock: retired_observation.clock(),
                        reason: retired.reason(),
                        reconnect_attempt: reconnect.reconnect_attempt(),
                        backoff: reconnect.backoff(),
                    },
                )
            }
            PmUserWsEvent::RetryExhausted(retired) => {
                let observation = retired.observation();
                self.admit_edge(
                    observation.connection(),
                    generation,
                    source_high_water,
                    observation.clock(),
                    UserStreamAdmission::RetryExhausted(retired.reason()),
                )
            }
            PmUserWsEvent::Shutdown(observation) => self.admit_edge(
                observation.connection(),
                generation,
                source_high_water,
                observation.clock(),
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
        clock: PmUserWsEdgeClock,
        admission: UserStreamAdmission<'_>,
    ) -> Result<(), PmUserStreamRuntimeError> {
        self.validate_next_activity(generation, source_high_water)?;
        self.validate_next_clock(clock)?;
        if !self.task_active {
            return Err(PmUserStreamRuntimeError::TaskExited);
        }
        if connection.condition() != self.scope.condition() {
            return Err(PmUserStreamRuntimeError::ConnectionMismatch);
        }

        match admission {
            UserStreamAdmission::ConnectionOpened => self.open(connection, generation, clock)?,
            UserStreamAdmission::SubscriptionSent => {
                self.require_active_connection(connection)?;
                if self.phase != UserStreamPhase::Opened {
                    return Err(PmUserStreamRuntimeError::LifecycleMismatch);
                }
                self.subscription_generation = Some(generation);
                self.subscription_clock = Some(clock);
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
                self.pending_ping_clock = Some(clock);
                self.last_ping_generation = Some(generation);
                self.last_ping_clock = Some(clock);
                self.correlated_pong_generation = None;
                self.correlated_pong_clock = None;
                self.phase = UserStreamPhase::AwaitingPong;
            }
            UserStreamAdmission::Pong => {
                self.require_active_connection(connection)?;
                let ping = self
                    .pending_ping_generation
                    .take()
                    .ok_or(PmUserStreamRuntimeError::LifecycleMismatch)?;
                let ping_clock = self
                    .pending_ping_clock
                    .take()
                    .ok_or(PmUserStreamRuntimeError::ClockEvidenceMismatch)?;
                if self.phase != UserStreamPhase::AwaitingPong
                    || generation <= ping
                    || clock.monotonic_receive_ns() < ping_clock.monotonic_receive_ns()
                {
                    return Err(PmUserStreamRuntimeError::LifecycleMismatch);
                }
                self.correlated_pong_generation = Some(generation);
                self.correlated_pong_clock = Some(clock);
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
                        self.initial_connection_epoch.get_or_insert(epoch);
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
                    clock,
                    reason,
                    connection_open: Self::lifecycle_edge(
                        self.connection_open_generation,
                        self.connection_open_clock,
                    )?,
                    subscription: Self::lifecycle_edge(
                        self.subscription_generation,
                        self.subscription_clock,
                    )?,
                    latest_ping: Self::lifecycle_edge(
                        self.last_ping_generation,
                        self.last_ping_clock,
                    )?,
                    correlated_pong: Self::lifecycle_edge(
                        self.correlated_pong_generation,
                        self.correlated_pong_clock,
                    )?,
                });
                self.active_epoch = None;
                self.clear_readiness();
                self.phase = UserStreamPhase::Retired;
            }
            UserStreamAdmission::ReconnectScheduled {
                replacement_epoch,
                retired_generation,
                retired_clock,
                reason,
                reconnect_attempt,
                backoff,
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
                if retired_clock != retired.clock {
                    return Err(PmUserStreamRuntimeError::ClockEvidenceMismatch);
                }
                let expected_attempt = self
                    .reconnect_history
                    .len()
                    .checked_add(1)
                    .and_then(|value| u8::try_from(value).ok())
                    .ok_or(PmUserStreamRuntimeError::ReconnectHistoryMismatch)?;
                if reconnect_attempt != expected_attempt {
                    return Err(PmUserStreamRuntimeError::ReconnectHistoryMismatch);
                }
                self.reconnect_history.push(PmUserStreamReconnectEvidence {
                    retired_epoch: retired.epoch,
                    replacement_epoch,
                    reconnect_attempt,
                    reason,
                    backoff,
                    connection_open: retired.connection_open,
                    subscription: retired.subscription,
                    latest_ping: retired.latest_ping,
                    correlated_pong: retired.correlated_pong,
                    retirement: UserStreamLifecycleEdge {
                        activity_generation: retired.activity_generation,
                        clock: retired.clock,
                    },
                    reconnect_scheduled: UserStreamLifecycleEdge {
                        activity_generation: generation,
                        clock,
                    },
                });
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

        self.finish_activity(generation, clock)
    }

    fn open(
        &mut self,
        connection: PmUserWsConnection,
        generation: u64,
        clock: PmUserWsEdgeClock,
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
        self.initial_connection_epoch.get_or_insert(epoch);
        self.active_epoch = Some(epoch);
        self.last_connection_epoch = Some(epoch);
        self.connection_open_generation = Some(generation);
        self.connection_open_clock = Some(clock);
        self.subscription_generation = None;
        self.subscription_clock = None;
        self.pending_ping_generation = None;
        self.pending_ping_clock = None;
        self.last_ping_generation = None;
        self.last_ping_clock = None;
        self.correlated_pong_generation = None;
        self.correlated_pong_clock = None;
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

    fn validate_next_clock(
        &self,
        clock: PmUserWsEdgeClock,
    ) -> Result<(), PmUserStreamRuntimeError> {
        if self
            .last_admitted_clock
            .is_some_and(|previous| clock.monotonic_receive_ns() < previous.monotonic_receive_ns())
        {
            return Err(PmUserStreamRuntimeError::ClockRegression);
        }
        Ok(())
    }

    fn lifecycle_edge(
        generation: Option<u64>,
        clock: Option<PmUserWsEdgeClock>,
    ) -> Result<Option<UserStreamLifecycleEdge>, PmUserStreamRuntimeError> {
        match (generation, clock) {
            (Some(activity_generation), Some(clock)) => Ok(Some(UserStreamLifecycleEdge {
                activity_generation,
                clock,
            })),
            (None, None) => Ok(None),
            _ => Err(PmUserStreamRuntimeError::ClockEvidenceMismatch),
        }
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

    fn finish_activity(
        &mut self,
        generation: u64,
        clock: PmUserWsEdgeClock,
    ) -> Result<(), PmUserStreamRuntimeError> {
        self.admitted_activity_generation = generation;
        self.last_admitted_clock = Some(clock);
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
        self.connection_open_clock = None;
        self.subscription_generation = None;
        self.subscription_clock = None;
        self.pending_ping_generation = None;
        self.pending_ping_clock = None;
        self.last_ping_generation = None;
        self.last_ping_clock = None;
        self.correlated_pong_generation = None;
        self.correlated_pong_clock = None;
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
            initial_connection_epoch: evidence.initial_connection_epoch,
            connection_epoch: evidence.connection_epoch,
            connection_open_generation: evidence.connection_open_generation,
            connection_open_clock: evidence.connection_open_clock,
            subscription_generation: evidence.subscription_generation,
            subscription_clock: evidence.subscription_clock,
            ping_generation: evidence.ping_generation,
            ping_clock: evidence.ping_clock,
            correlated_pong_generation: evidence.correlated_pong_generation,
            correlated_pong_clock: evidence.correlated_pong_clock,
            activity_generation: self.admitted_activity_generation,
            reconnect_count: u8::try_from(self.reconnect_history.len())
                .map_err(|_| PmUserStreamRuntimeError::ReconnectHistoryMismatch)?,
            reconnect_history: self
                .reconnect_history
                .iter()
                .map(PmUserStreamReconnectEvidence::duplicate_for_ticket)
                .collect(),
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
            || core.initial_connection_epoch != evidence.initial_connection_epoch
            || core.connection_epoch != evidence.connection_epoch
            || core.connection_open_generation != evidence.connection_open_generation
            || core.connection_open_clock != evidence.connection_open_clock
            || core.subscription_generation != evidence.subscription_generation
            || core.subscription_clock != evidence.subscription_clock
            || core.ping_generation != evidence.ping_generation
            || core.ping_clock != evidence.ping_clock
            || core.correlated_pong_generation != evidence.correlated_pong_generation
            || core.correlated_pong_clock != evidence.correlated_pong_clock
            || core.activity_generation != self.admitted_activity_generation
            || usize::from(core.reconnect_count) != self.reconnect_history.len()
            || core.reconnect_history.as_ref() != self.reconnect_history.as_slice()
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
            initial_connection_epoch: self
                .initial_connection_epoch
                .ok_or(PmUserStreamRuntimeError::HeartbeatNotReady)?,
            connection_epoch: self
                .active_epoch
                .ok_or(PmUserStreamRuntimeError::HeartbeatNotReady)?,
            connection_open_generation: self
                .connection_open_generation
                .ok_or(PmUserStreamRuntimeError::HeartbeatNotReady)?,
            connection_open_clock: self
                .connection_open_clock
                .ok_or(PmUserStreamRuntimeError::HeartbeatNotReady)?,
            subscription_generation: self
                .subscription_generation
                .ok_or(PmUserStreamRuntimeError::HeartbeatNotReady)?,
            subscription_clock: self
                .subscription_clock
                .ok_or(PmUserStreamRuntimeError::HeartbeatNotReady)?,
            ping_generation: self
                .last_ping_generation
                .ok_or(PmUserStreamRuntimeError::HeartbeatNotReady)?,
            ping_clock: self
                .last_ping_clock
                .ok_or(PmUserStreamRuntimeError::HeartbeatNotReady)?,
            correlated_pong_generation: self
                .correlated_pong_generation
                .ok_or(PmUserStreamRuntimeError::HeartbeatNotReady)?,
            correlated_pong_clock: self
                .correlated_pong_clock
                .ok_or(PmUserStreamRuntimeError::HeartbeatNotReady)?,
        };
        if !(evidence.connection_open_generation < evidence.subscription_generation
            && evidence.subscription_generation < evidence.ping_generation
            && evidence.ping_generation < evidence.correlated_pong_generation
            && evidence.correlated_pong_generation <= self.admitted_activity_generation)
        {
            return Err(PmUserStreamRuntimeError::HeartbeatNotReady);
        }
        let clocks = [
            evidence.connection_open_clock,
            evidence.subscription_clock,
            evidence.ping_clock,
            evidence.correlated_pong_clock,
        ];
        if clocks
            .windows(2)
            .any(|pair| pair[1].monotonic_receive_ns() < pair[0].monotonic_receive_ns())
        {
            return Err(PmUserStreamRuntimeError::ClockRegression);
        }
        self.validate_reconnect_history(
            evidence.initial_connection_epoch,
            evidence.connection_epoch,
        )?;
        Ok(evidence)
    }

    fn validate_reconnect_history(
        &self,
        initial_epoch: ConnectionEpoch,
        current_epoch: ConnectionEpoch,
    ) -> Result<(), PmUserStreamRuntimeError> {
        let mut expected_retired_epoch = initial_epoch;
        let mut prior_clock = None;
        for (index, entry) in self.reconnect_history.iter().enumerate() {
            let expected_attempt = u8::try_from(index + 1)
                .map_err(|_| PmUserStreamRuntimeError::ReconnectHistoryMismatch)?;
            let expected_replacement = entry
                .retired_epoch
                .value()
                .checked_add(1)
                .ok_or(PmUserStreamRuntimeError::ReconnectHistoryMismatch)?;
            if entry.reconnect_attempt != expected_attempt
                || entry.retired_epoch != expected_retired_epoch
                || entry.replacement_epoch.value() != expected_replacement
                || entry.retirement.activity_generation
                    >= entry.reconnect_scheduled.activity_generation
            {
                return Err(PmUserStreamRuntimeError::ReconnectHistoryMismatch);
            }
            let mut lifecycle = [
                entry.connection_open,
                entry.subscription,
                entry.latest_ping,
                entry.correlated_pong,
                Some(entry.retirement),
                Some(entry.reconnect_scheduled),
            ]
            .into_iter()
            .flatten();
            if let Some(first) = lifecycle.next() {
                if prior_clock.is_some_and(|clock: PmUserWsEdgeClock| {
                    first.clock.monotonic_receive_ns() < clock.monotonic_receive_ns()
                }) {
                    return Err(PmUserStreamRuntimeError::ClockRegression);
                }
                let mut previous = first;
                for edge in lifecycle {
                    if edge.activity_generation <= previous.activity_generation
                        || edge.clock.monotonic_receive_ns() < previous.clock.monotonic_receive_ns()
                    {
                        return Err(PmUserStreamRuntimeError::ReconnectHistoryMismatch);
                    }
                    previous = edge;
                }
                prior_clock = Some(previous.clock);
            }
            expected_retired_epoch = entry.replacement_epoch;
        }
        if expected_retired_epoch != current_epoch {
            return Err(PmUserStreamRuntimeError::ReconnectHistoryMismatch);
        }
        if prior_clock.is_some_and(|clock| {
            self.connection_open_clock
                .is_some_and(|opened| opened.monotonic_receive_ns() < clock.monotonic_receive_ns())
        }) {
            return Err(PmUserStreamRuntimeError::ClockRegression);
        }
        Ok(())
    }
}

impl FinalCutBusinessBasis {
    const fn rest_row_counts(&self) -> (usize, usize) {
        match self {
            Self::StreamEventsAndRestJoined {
                open_order_rows,
                trade_rows,
                ..
            }
            | Self::RestBackedNoStreamEvents {
                open_order_rows,
                trade_rows,
                ..
            } => (*open_order_rows, *trade_rows),
        }
    }

    const fn current_epoch_event_range(&self) -> (usize, usize) {
        match self {
            Self::StreamEventsAndRestJoined {
                current_epoch_event_start,
                current_epoch_event_count,
                ..
            } => (*current_epoch_event_start, *current_epoch_event_count),
            Self::RestBackedNoStreamEvents {
                current_epoch_event_start,
                ..
            } => (*current_epoch_event_start, 0),
        }
    }

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
    initial_connection_epoch: ConnectionEpoch,
    connection_epoch: ConnectionEpoch,
    connection_open_generation: u64,
    connection_open_clock: PmUserWsEdgeClock,
    subscription_generation: u64,
    subscription_clock: PmUserWsEdgeClock,
    ping_generation: u64,
    ping_clock: PmUserWsEdgeClock,
    correlated_pong_generation: u64,
    correlated_pong_clock: PmUserWsEdgeClock,
}

struct FinalCutCore {
    scope: PmWireScope,
    proxy_maker: EvmAddress,
    state_generation: u64,
    initial_connection_epoch: ConnectionEpoch,
    connection_epoch: ConnectionEpoch,
    connection_open_generation: u64,
    connection_open_clock: PmUserWsEdgeClock,
    subscription_generation: u64,
    subscription_clock: PmUserWsEdgeClock,
    ping_generation: u64,
    ping_clock: PmUserWsEdgeClock,
    correlated_pong_generation: u64,
    correlated_pong_clock: PmUserWsEdgeClock,
    activity_generation: u64,
    reconnect_count: u8,
    reconnect_history: Box<[PmUserStreamReconnectEvidence]>,
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
        retired_clock: PmUserWsEdgeClock,
        reason: PmUserWsDisconnectReason,
        reconnect_attempt: u8,
        backoff: Duration,
    },
    RetryExhausted(PmUserWsDisconnectReason),
    Shutdown,
}

#[cfg(test)]
mod tests {
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
    };

    use reap_pm_core::{PmConditionId, PmMarketId, PmTokenId, U256};
    use reap_polymarket_auth::EoaAddress;
    use reap_polymarket_wire::{PmLiveUserFrame, parse_live_user_frame};

    use super::*;

    const CONDITION: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
    const FOREIGN_CONDITION: &str =
        "0x2222222222222222222222222222222222222222222222222222222222222222";
    const MARKET: &str = "0x3333333333333333333333333333333333333333333333333333333333333333";
    const PROXY: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    const SIGNER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
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

    fn signer() -> EoaAddress {
        EoaAddress::parse(SIGNER).unwrap()
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
            let next_generation = self.high_water.checked_add(1).unwrap();
            self.admit_with_clock(epoch, admission, test_clock(next_generation))
        }

        fn admit_with_clock(
            &mut self,
            epoch: u64,
            admission: TestAdmission<'_>,
            clock: PmUserWsEdgeClock,
        ) -> Result<(), PmUserStreamRuntimeError> {
            self.high_water = self.high_water.checked_add(1).unwrap();
            self.state.admit_test_edge(
                scope().condition(),
                ConnectionEpoch::new(epoch),
                self.high_water,
                self.high_water,
                clock,
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
            reconnect_attempt: u8,
            retired_clock: Option<PmUserWsEdgeClock>,
        },
    }

    fn test_clock(value: u64) -> PmUserWsEdgeClock {
        PmUserWsEdgeClock::new(1_000_000 + value, value).unwrap()
    }

    fn online_preflight_fixture(
        harness: Harness,
        open_order_rows: usize,
        trade_rows: usize,
    ) -> (
        PmUserStreamHandle,
        PmUserOnlinePreflightLease,
        Arc<PmUserStreamShared>,
        Arc<AtomicU64>,
    ) {
        let boundary = harness.boundary().unwrap();
        let core = harness
            .state
            .finish_rest_collection(boundary, harness.high_water, open_order_rows, trade_rows)
            .unwrap();
        let activity = Arc::new(AtomicU64::new(harness.high_water));
        let shared = Arc::new(PmUserStreamShared {
            state: Mutex::new(harness.state),
            activity: RuntimeUserActivityView::Test(Arc::clone(&activity)),
        });
        let (marker, rest_cut_identity) =
            PmSameCredentialAuthorityMarker::test_support_online_preflight_seal(
                scope(),
                signer(),
                proxy(),
                Arc::clone(&activity),
            );
        let ticket_marker = marker.fork_for_same_authority();
        let handle = PmUserStreamHandle {
            shared: Arc::clone(&shared),
            marker,
        };
        let lease = FinalCutTicket {
            core,
            marker: ticket_marker,
            rest_cut_identity,
        }
        .into_online_preflight_lease();
        (handle, lease, shared, activity)
    }

    fn admit_shared(
        shared: &Arc<PmUserStreamShared>,
        activity: &Arc<AtomicU64>,
        epoch: u64,
        admission: TestAdmission<'_>,
    ) -> Result<(), PmUserStreamRuntimeError> {
        let generation = activity.fetch_add(1, Ordering::AcqRel) + 1;
        shared.state.lock().unwrap().admit_test_edge(
            scope().condition(),
            ConnectionEpoch::new(epoch),
            generation,
            generation,
            test_clock(generation),
            admission,
        )
    }

    impl PmUserStreamState {
        fn admit_test_edge(
            &mut self,
            condition: PmConditionId,
            epoch: ConnectionEpoch,
            generation: u64,
            source_high_water: u64,
            clock: PmUserWsEdgeClock,
            admission: TestAdmission<'_>,
        ) -> Result<(), PmUserStreamRuntimeError> {
            let result = self.admit_test_edge_inner(
                condition,
                epoch,
                generation,
                source_high_water,
                clock,
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
            clock: PmUserWsEdgeClock,
            admission: TestAdmission<'_>,
        ) -> Result<(), PmUserStreamRuntimeError> {
            self.validate_next_activity(generation, source_high_water)?;
            self.validate_next_clock(clock)?;
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
                    self.initial_connection_epoch.get_or_insert(epoch);
                    self.active_epoch = Some(epoch);
                    self.last_connection_epoch = Some(epoch);
                    self.connection_open_generation = Some(generation);
                    self.connection_open_clock = Some(clock);
                    self.subscription_generation = None;
                    self.subscription_clock = None;
                    self.pending_ping_generation = None;
                    self.pending_ping_clock = None;
                    self.last_ping_generation = None;
                    self.last_ping_clock = None;
                    self.correlated_pong_generation = None;
                    self.correlated_pong_clock = None;
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
                    self.subscription_clock = Some(clock);
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
                    self.pending_ping_clock = Some(clock);
                    self.last_ping_generation = Some(generation);
                    self.last_ping_clock = Some(clock);
                    self.correlated_pong_generation = None;
                    self.correlated_pong_clock = None;
                    self.phase = UserStreamPhase::AwaitingPong;
                }
                TestAdmission::Pong => {
                    self.require_test_epoch(epoch)?;
                    if self.phase != UserStreamPhase::AwaitingPong
                        || self.pending_ping_generation.take().is_none()
                        || self.pending_ping_clock.take().is_none()
                    {
                        return Err(PmUserStreamRuntimeError::LifecycleMismatch);
                    }
                    self.correlated_pong_generation = Some(generation);
                    self.correlated_pong_clock = Some(clock);
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
                            self.initial_connection_epoch.get_or_insert(epoch);
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
                        clock,
                        reason: PmUserWsDisconnectReason::SocketClosed,
                        connection_open: Self::lifecycle_edge(
                            self.connection_open_generation,
                            self.connection_open_clock,
                        )?,
                        subscription: Self::lifecycle_edge(
                            self.subscription_generation,
                            self.subscription_clock,
                        )?,
                        latest_ping: Self::lifecycle_edge(
                            self.last_ping_generation,
                            self.last_ping_clock,
                        )?,
                        correlated_pong: Self::lifecycle_edge(
                            self.correlated_pong_generation,
                            self.correlated_pong_clock,
                        )?,
                    });
                    self.active_epoch = None;
                    self.clear_readiness();
                    self.phase = UserStreamPhase::Retired;
                }
                TestAdmission::Reconnect {
                    retired_generation,
                    replacement_epoch,
                    reconnect_attempt,
                    retired_clock,
                } => {
                    let retired = self.retired.unwrap();
                    if self.phase != UserStreamPhase::Retired
                        || retired.epoch != epoch
                        || retired.activity_generation != retired_generation
                        || replacement_epoch != epoch.value() + 1
                    {
                        return Err(PmUserStreamRuntimeError::LifecycleMismatch);
                    }
                    if retired_clock.is_some_and(|value| value != retired.clock) {
                        return Err(PmUserStreamRuntimeError::ClockEvidenceMismatch);
                    }
                    let expected_attempt = self
                        .reconnect_history
                        .len()
                        .checked_add(1)
                        .and_then(|value| u8::try_from(value).ok())
                        .ok_or(PmUserStreamRuntimeError::ReconnectHistoryMismatch)?;
                    if reconnect_attempt != expected_attempt {
                        return Err(PmUserStreamRuntimeError::ReconnectHistoryMismatch);
                    }
                    let replacement_epoch = ConnectionEpoch::new(replacement_epoch);
                    self.reconnect_history.push(PmUserStreamReconnectEvidence {
                        retired_epoch: retired.epoch,
                        replacement_epoch,
                        reconnect_attempt,
                        reason: retired.reason,
                        backoff: Duration::from_millis(1),
                        connection_open: retired.connection_open,
                        subscription: retired.subscription,
                        latest_ping: retired.latest_ping,
                        correlated_pong: retired.correlated_pong,
                        retirement: UserStreamLifecycleEdge {
                            activity_generation: retired.activity_generation,
                            clock: retired.clock,
                        },
                        reconnect_scheduled: UserStreamLifecycleEdge {
                            activity_generation: generation,
                            clock,
                        },
                    });
                    self.phase = UserStreamPhase::AwaitingOpen {
                        replacement_epoch: Some(replacement_epoch),
                    };
                }
            }
            self.finish_activity(generation, clock)
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

    #[test]
    fn pong_with_an_older_source_monotonic_clock_fails_closed() {
        let mut harness = Harness::new();
        harness.admit(3, TestAdmission::Open).unwrap();
        harness.admit(3, TestAdmission::Subscription).unwrap();
        harness
            .admit_with_clock(
                3,
                TestAdmission::Ping,
                PmUserWsEdgeClock::new(2_000_000, 100).unwrap(),
            )
            .unwrap();
        assert_eq!(
            harness.admit_with_clock(
                3,
                TestAdmission::Pong,
                PmUserWsEdgeClock::new(2_000_001, 99).unwrap(),
            ),
            Err(PmUserStreamRuntimeError::ClockRegression)
        );
        assert_eq!(harness.state.phase, UserStreamPhase::Faulted);
        assert_eq!(
            harness.boundary(),
            Err(PmUserStreamRuntimeError::HeartbeatNotReady)
        );
    }

    #[test]
    fn reconnect_rejects_a_mismatched_retirement_clock() {
        let mut harness = Harness::new();
        harness.ready(4);
        harness.admit(4, TestAdmission::Retired).unwrap();
        let retired_generation = harness.high_water;
        let wrong_retired_clock = PmUserWsEdgeClock::new(9_000_000, 4).unwrap();
        assert_eq!(
            harness.admit(
                4,
                TestAdmission::Reconnect {
                    retired_generation,
                    replacement_epoch: 5,
                    reconnect_attempt: 1,
                    retired_clock: Some(wrong_retired_clock),
                },
            ),
            Err(PmUserStreamRuntimeError::ClockEvidenceMismatch)
        );
        assert_eq!(harness.state.phase, UserStreamPhase::Faulted);
        assert!(harness.state.reconnect_history.is_empty());
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
    fn online_preflight_source_basis_is_inspectable_recheckable_and_consumed_once() {
        let frame = order_frame(CONDITION, 123, PROXY);
        let mut harness = Harness::new();
        harness.ready(3);
        harness
            .admit(3, TestAdmission::Business(frame.events()))
            .unwrap();
        let (mut handle, lease, _shared, _activity) = online_preflight_fixture(harness, 2, 3);

        assert_eq!(lease.scope(), scope());
        assert_eq!(lease.proxy_maker(), proxy());
        assert_eq!(lease.stream_revision(), 6);
        assert_eq!(lease.initial_connection_epoch(), ConnectionEpoch::new(3));
        assert_eq!(lease.current_connection_epoch(), ConnectionEpoch::new(3));
        assert_eq!(lease.connection_open_generation(), 1);
        assert_eq!(lease.subscription_generation(), 2);
        assert_eq!(lease.ping_generation(), 3);
        assert_eq!(lease.correlated_pong_generation(), 4);
        assert_eq!(lease.admitted_activity_generation(), 5);
        assert_eq!(lease.connection_open_clock(), test_clock(1));
        assert_eq!(lease.subscription_clock(), test_clock(2));
        assert_eq!(lease.ping_clock(), test_clock(3));
        assert_eq!(lease.correlated_pong_clock(), test_clock(4));
        assert_eq!(lease.reconnect_count(), 0);
        assert!(lease.reconnect_history().is_empty());
        assert_eq!(lease.open_order_rows(), 2);
        assert_eq!(lease.trade_rows(), 3);
        assert_eq!(lease.business_basis().rest_row_counts(), (2, 3));
        assert_eq!(lease.business_events().len(), 1);
        assert_eq!(lease.current_epoch_business_events().len(), 1);

        let lease = handle.recheck_online_preflight_lease(lease).unwrap();
        assert_eq!(lease.business_events().len(), 1);
        let fields = handle.consume_online_preflight_lease(lease).unwrap();
        assert_eq!(fields.scope(), scope());
        assert_eq!(fields.proxy_maker(), proxy());
        assert_eq!(fields.stream_revision(), 6);
        assert_eq!(fields.initial_connection_epoch(), ConnectionEpoch::new(3));
        assert_eq!(fields.current_connection_epoch(), ConnectionEpoch::new(3));
        assert_eq!(fields.connection_open_generation(), 1);
        assert_eq!(fields.connection_open_clock(), test_clock(1));
        assert_eq!(fields.subscription_generation(), 2);
        assert_eq!(fields.subscription_clock(), test_clock(2));
        assert_eq!(fields.ping_generation(), 3);
        assert_eq!(fields.latest_ping_clock(), test_clock(3));
        assert_eq!(fields.correlated_pong_generation(), 4);
        assert_eq!(fields.correlated_pong_clock(), test_clock(4));
        assert_eq!(fields.admitted_activity_generation(), 5);
        assert_eq!(fields.reconnect_count(), 0);
        assert!(fields.reconnect_history().is_empty());
        assert_eq!(fields.business_basis().rest_row_counts(), (2, 3));
        assert_eq!(fields.business_events().len(), 1);
        assert_eq!(fields.current_epoch_business_events().len(), 1);
    }

    #[test]
    fn online_preflight_recheck_rejects_queued_reconnect_clock_state_and_exit_drift() {
        let mut queued = Harness::new();
        queued.ready(1);
        let (mut queued_handle, queued_lease, _queued_shared, queued_activity) =
            online_preflight_fixture(queued, 0, 0);
        queued_activity.fetch_add(1, Ordering::AcqRel);
        assert!(
            queued_handle
                .recheck_online_preflight_lease(queued_lease)
                .is_err()
        );

        let mut retired = Harness::new();
        retired.ready(4);
        let (mut retired_handle, retired_lease, retired_shared, retired_activity) =
            online_preflight_fixture(retired, 0, 0);
        admit_shared(
            &retired_shared,
            &retired_activity,
            4,
            TestAdmission::Retired,
        )
        .unwrap();
        assert!(matches!(
            retired_handle.recheck_online_preflight_lease(retired_lease),
            Err(PmUserStreamRuntimeError::HeartbeatNotReady)
        ));

        let mut reconnected = Harness::new();
        reconnected.ready(7);
        let (mut reconnected_handle, old_epoch_lease, reconnected_shared, reconnected_activity) =
            online_preflight_fixture(reconnected, 0, 0);
        admit_shared(
            &reconnected_shared,
            &reconnected_activity,
            7,
            TestAdmission::Retired,
        )
        .unwrap();
        let retired_generation = reconnected_activity.load(Ordering::Acquire);
        admit_shared(
            &reconnected_shared,
            &reconnected_activity,
            7,
            TestAdmission::Reconnect {
                retired_generation,
                replacement_epoch: 8,
                reconnect_attempt: 1,
                retired_clock: None,
            },
        )
        .unwrap();
        admit_shared(
            &reconnected_shared,
            &reconnected_activity,
            8,
            TestAdmission::Open,
        )
        .unwrap();
        admit_shared(
            &reconnected_shared,
            &reconnected_activity,
            8,
            TestAdmission::Subscription,
        )
        .unwrap();
        admit_shared(
            &reconnected_shared,
            &reconnected_activity,
            8,
            TestAdmission::Ping,
        )
        .unwrap();
        admit_shared(
            &reconnected_shared,
            &reconnected_activity,
            8,
            TestAdmission::Pong,
        )
        .unwrap();
        assert!(
            reconnected_handle
                .recheck_online_preflight_lease(old_epoch_lease)
                .is_err()
        );

        let mut clock_drift = Harness::new();
        clock_drift.ready(2);
        let (mut clock_handle, clock_lease, clock_shared, _clock_activity) =
            online_preflight_fixture(clock_drift, 0, 0);
        clock_shared.state.lock().unwrap().correlated_pong_clock =
            Some(PmUserWsEdgeClock::new(9_000_000, 9_000_000).unwrap());
        assert!(matches!(
            clock_handle.recheck_online_preflight_lease(clock_lease),
            Err(PmUserStreamRuntimeError::TicketInvalidated)
        ));

        let mut state_drift = Harness::new();
        state_drift.ready(5);
        let (mut state_handle, state_lease, state_shared, _state_activity) =
            online_preflight_fixture(state_drift, 0, 0);
        state_shared.state.lock().unwrap().state_generation += 1;
        assert!(matches!(
            state_handle.recheck_online_preflight_lease(state_lease),
            Err(PmUserStreamRuntimeError::TicketInvalidated)
        ));

        let mut exited = Harness::new();
        exited.ready(6);
        let (mut exited_handle, exited_lease, exited_shared, _exited_activity) =
            online_preflight_fixture(exited, 0, 0);
        exited_shared.state.lock().unwrap().invalidate_task_exit();
        assert!(matches!(
            exited_handle.recheck_online_preflight_lease(exited_lease),
            Err(PmUserStreamRuntimeError::HeartbeatNotReady)
        ));
    }

    #[test]
    fn final_core_rejects_a_clock_value_detached_from_its_source_generation() {
        let mut harness = Harness::new();
        harness.ready(2);
        let boundary = harness.boundary().unwrap();
        let mut core = harness
            .state
            .finish_rest_collection(boundary, harness.high_water, 0, 0)
            .unwrap();
        core.correlated_pong_clock = PmUserWsEdgeClock::new(
            core.correlated_pong_clock.local_wall_receive_ns() + 1,
            core.correlated_pong_clock.monotonic_receive_ns() + 1,
        )
        .unwrap();
        assert_eq!(
            harness.state.validate_final_core(&core, harness.high_water),
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
                    reconnect_attempt: 1,
                    retired_clock: None,
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
    fn final_cut_retains_initial_current_epochs_and_exact_reconnect_clock_history() {
        let mut harness = Harness::new();
        harness.ready(4);
        let first_ready = harness.state.ready_evidence().unwrap();
        harness.admit(4, TestAdmission::Retired).unwrap();
        let first_retired_generation = harness.high_water;
        let first_retired_clock = harness.state.retired.unwrap().clock;
        harness
            .admit(
                4,
                TestAdmission::Reconnect {
                    retired_generation: first_retired_generation,
                    replacement_epoch: 5,
                    reconnect_attempt: 1,
                    retired_clock: Some(first_retired_clock),
                },
            )
            .unwrap();
        harness.ready(5);
        harness.admit(5, TestAdmission::Retired).unwrap();
        let second_retired_generation = harness.high_water;
        let second_retired_clock = harness.state.retired.unwrap().clock;
        harness
            .admit(
                5,
                TestAdmission::Reconnect {
                    retired_generation: second_retired_generation,
                    replacement_epoch: 6,
                    reconnect_attempt: 2,
                    retired_clock: Some(second_retired_clock),
                },
            )
            .unwrap();
        harness.ready(6);

        let boundary = harness.boundary().unwrap();
        let core = harness
            .state
            .finish_rest_collection(boundary, harness.high_water, 0, 0)
            .unwrap();
        assert_eq!(core.initial_connection_epoch, ConnectionEpoch::new(4));
        assert_eq!(core.connection_epoch, ConnectionEpoch::new(6));
        assert_eq!(core.reconnect_count, 2);
        assert_eq!(core.reconnect_history.len(), 2);
        assert_eq!(core.reconnect_history[0].reconnect_attempt(), 1);
        assert_eq!(core.reconnect_history[0].retired_epoch().value(), 4);
        assert_eq!(core.reconnect_history[0].replacement_epoch().value(), 5);
        assert_eq!(
            core.reconnect_history[0].retirement_activity_generation(),
            first_retired_generation
        );
        assert_eq!(
            core.reconnect_history[0].retirement_clock(),
            first_retired_clock
        );
        assert_eq!(
            core.reconnect_history[0].connection_open_clock(),
            Some(first_ready.connection_open_clock)
        );
        assert_eq!(
            core.reconnect_history[0].correlated_pong_clock(),
            Some(first_ready.correlated_pong_clock)
        );
        assert_eq!(core.reconnect_history[1].reconnect_attempt(), 2);
        assert_eq!(core.reconnect_history[1].retired_epoch().value(), 5);
        assert_eq!(core.reconnect_history[1].replacement_epoch().value(), 6);
        assert_eq!(
            core.correlated_pong_clock.monotonic_receive_ns(),
            core.correlated_pong_generation
        );
        assert!(
            core.reconnect_history[1]
                .reconnect_clock()
                .monotonic_receive_ns()
                < core.connection_open_clock.monotonic_receive_ns()
        );
    }

    #[test]
    fn reconnect_attempt_count_and_current_epoch_must_match_history() {
        let mut wrong_attempt = Harness::new();
        wrong_attempt.ready(7);
        wrong_attempt.admit(7, TestAdmission::Retired).unwrap();
        let retired_generation = wrong_attempt.high_water;
        assert_eq!(
            wrong_attempt.admit(
                7,
                TestAdmission::Reconnect {
                    retired_generation,
                    replacement_epoch: 8,
                    reconnect_attempt: 2,
                    retired_clock: None,
                },
            ),
            Err(PmUserStreamRuntimeError::ReconnectHistoryMismatch)
        );

        let mut wrong_current = Harness::new();
        wrong_current.ready(9);
        wrong_current.state.initial_connection_epoch = Some(ConnectionEpoch::new(8));
        assert_eq!(
            wrong_current.boundary(),
            Err(PmUserStreamRuntimeError::ReconnectHistoryMismatch)
        );
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
                    reconnect_attempt: 1,
                    retired_clock: None,
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
                    reconnect_attempt: 1,
                    retired_clock: None,
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
                    reconnect_attempt: 1,
                    retired_clock: None,
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
                    test_clock(observed.saturating_add(1)),
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
                test_clock(2),
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
