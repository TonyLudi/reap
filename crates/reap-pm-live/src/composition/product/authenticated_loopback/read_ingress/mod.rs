//! Owned authenticated-loopback read ingress.
//!
//! Socket and HTTP tasks own only transport roles and bounded typed channel ends. The
//! product actor remains the sole owner of the occurrence issuer, canonical
//! coordinator, public capture, and shared control clock. Endpoint I/O never
//! runs on that actor; authenticated response edges are stamped in the owned
//! HTTP task before any inter-task queue delay.

mod book;
mod channel;
mod http;
mod public;
mod shutdown;
mod user;

#[cfg(test)]
mod tests;

use std::{fmt, time::Duration};

use reap_pm_core::{PmFillQueryCursor, PmVenueOrderKey, ReceivedEventClock};
use reap_pm_strategy::PmQuoteModel;
use reap_polymarket_live_adapter::{
    PmActorProductClock, PmAuthenticatedHttpOwner, PmAuthenticatedUserWsRole,
    PmPrivateReadProductClock, PmPublicHttpRole, PmPublicMarketWsRole, PmReadServerTimeHttpRole,
    PmRestBookPurpose,
};
use thiserror::Error;
use tokio::sync::mpsc;

use self::channel::{
    PmPublicActorRequest, PmPublicIngressSink, PmUserActorRequest, PmUserIngressSink,
    PublicTaskOutput, UserTaskOutput,
};
pub(super) use self::shutdown::PmReadIngressShutdownError;
use self::shutdown::{book_shutdown_ready, http_shutdown_ready};
use super::supervision::PmAbortableTask;
use crate::{
    PmCompleteServiceCounts,
    composition::{
        PmProductPublicIngressError, PmPublicBookPipelineError, PmPublicCaptureRunError,
        PmPublicTerminalTickApplyError,
    },
    coordinator::{PmCoordinator, PmCoordinatorError, PmProductEffect, PmRefreshEffectKind},
    private_monitor::{
        PmLiveAccountQueryTicket, PmLiveHttpQueryFailure, PmLiveOccurrenceError,
        PmLiveOccurrenceIssuer, PmLiveOpenOrdersQueryTicket, PmLiveOrderDetailQueryTicket,
        PmLiveReconciliationQueryTicket,
    },
    public_routes::PmPublicBookDelivery,
};

const READ_ACTOR_CHANNEL_CAPACITY: usize = 32;
const READ_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(65);
const READ_SHUTDOWN_POLL_BACKOFF: Duration = Duration::from_millis(1);

/// Actor-side access to the sole canonical owners needed by read ingress.
///
/// This is a short-lived borrow, never a shared owner. It cannot escape the
/// product run or be moved into either socket task.
pub(super) struct PmReadIngressActor<'a, M: PmQuoteModel> {
    pub(super) coordinator: &'a mut PmCoordinator<M>,
    pub(super) issuer: &'a mut PmLiveOccurrenceIssuer,
    pub(super) actor_clock: &'a mut PmActorProductClock,
}

impl<'a, M: PmQuoteModel> PmReadIngressActor<'a, M> {
    pub(super) const fn new(
        coordinator: &'a mut PmCoordinator<M>,
        issuer: &'a mut PmLiveOccurrenceIssuer,
        actor_clock: &'a mut PmActorProductClock,
    ) -> Self {
        Self {
            coordinator,
            issuer,
            actor_clock,
        }
    }

    pub(super) fn service_after_ingress(
        &mut self,
    ) -> Result<PmCompleteServiceCounts, PmReadIngressActorError> {
        let service_clock = self.actor_clock.observe_control_edge()?.received_clock();
        self.coordinator
            .service_turn(service_clock.monotonic_receive_ns())
            .map_err(Into::into)
    }

    fn drain_shutdown_copied_effects(&mut self, counts: &mut [u64; 5]) {
        while let Some(effect) = self.coordinator.pop_shutdown_copied_effect() {
            let index = match effect {
                PmProductEffect::PlaceGtcPostOnly(_) => 0,
                PmProductEffect::CancelOwned(_) => 1,
                PmProductEffect::DurableRecord(_) => 2,
                PmProductEffect::HealthMetricAudit(_) => 3,
                PmProductEffect::FailClosedHaltOrCancel(_) => 4,
                PmProductEffect::ReconciliationRefresh(_) => {
                    unreachable!("shutdown copied-effect drain cannot skip a refresh")
                }
            };
            counts[index] = counts[index].saturating_add(1);
        }
    }
}

/// Owns both read-only sockets, the authenticated HTTP capability, and all
/// bounded actor handoff endpoints.
pub(super) struct PmAuthenticatedReadIngress {
    http_dispatch: http::PmHttpReadSender,
    public_requests: mpsc::Receiver<PmPublicActorRequest>,
    user_requests: mpsc::Receiver<PmUserActorRequest>,
    public_shutdown: reap_polymarket_live_adapter::PmPublicWsShutdownHandle,
    user_shutdown: reap_polymarket_live_adapter::PmUserWsShutdownHandle,
    public_task: Option<PmAbortableTask<PublicTaskOutput>>,
    user_task: Option<PmAbortableTask<UserTaskOutput>>,
    http_task: Option<PmAbortableTask<http::PmHttpTaskOutput>>,
    book_dispatch: book::PmBookRequestSender,
    book_task: Option<PmAbortableTask<book::PmBookTaskOutput>>,
    pending_book: Option<(
        PmRestBookPurpose,
        tokio::sync::oneshot::Receiver<book::PmBookRequestResult>,
    )>,
    pending_book_demand: Option<PmRestBookPurpose>,
    next_source: u8,
    public_state: public::PmPublicIngressState,
    user_state: user::PmUserIngressState,
    pending_http: Option<PmPendingHttpQuery>,
}

impl PmAuthenticatedReadIngress {
    pub(super) fn start(
        public_ws: PmPublicMarketWsRole,
        user_ws: PmAuthenticatedUserWsRole,
        http: PmAuthenticatedHttpOwner,
        read_server_time: PmReadServerTimeHttpRole,
        private_read_clock: PmPrivateReadProductClock,
        book_http: PmPublicHttpRole,
    ) -> Self {
        let public_scope = public_ws.scope();
        let initial_public_epoch = public_ws.transport_policy().initial_connection_epoch();
        let user_condition = user_ws.condition();
        let (public_sender, public_requests) = mpsc::channel(READ_ACTOR_CHANNEL_CAPACITY);
        let (user_sender, user_requests) = mpsc::channel(READ_ACTOR_CHANNEL_CAPACITY);
        let (public_shutdown, public_signal) =
            reap_polymarket_live_adapter::pm_public_ws_shutdown_channel();
        let (user_shutdown, user_signal) =
            reap_polymarket_live_adapter::pm_user_ws_shutdown_channel();

        let book_actor = public_sender.clone();
        let public_task = PmAbortableTask::new(tokio::spawn(async move {
            let mut sink = PmPublicIngressSink::new(public_sender);
            public_ws.run(public_signal, &mut sink).await
        }));
        let user_task = PmAbortableTask::new(tokio::spawn(async move {
            let mut sink = PmUserIngressSink::new(user_sender);
            user_ws.run(user_signal, &mut sink).await
        }));
        let (http_dispatch, http_receiver) = http::pm_http_read_channel();
        let http_task = PmAbortableTask::new(tokio::spawn(http::run_http_worker(
            http,
            read_server_time,
            private_read_clock,
            http_receiver,
        )));
        let (book_dispatch, book_receiver) = book::pm_book_request_channel();
        let book_task = PmAbortableTask::new(tokio::spawn(book::run_book_worker(
            book_http,
            channel::PmRestBookIngressSink::new(book_actor),
            book_receiver,
        )));

        Self {
            http_dispatch,
            public_requests,
            user_requests,
            public_shutdown,
            user_shutdown,
            public_task: Some(public_task),
            user_task: Some(user_task),
            http_task: Some(http_task),
            book_dispatch,
            book_task: Some(book_task),
            pending_book: None,
            pending_book_demand: None,
            next_source: 0,
            public_state: public::PmPublicIngressState::new(
                public_scope.condition(),
                public_scope.market(),
                public_scope.token(),
                initial_public_epoch,
            ),
            user_state: user::PmUserIngressState::new(user_condition),
            pending_http: None,
        }
    }

    pub(super) fn request_rest_book(
        &mut self,
        purpose: PmRestBookPurpose,
    ) -> Result<PmBookRequestOutcome, PmReadIngressActorError> {
        if self.pending_book.is_some() {
            self.retain_book_demand(purpose)?;
            return Ok(PmBookRequestOutcome::Occupied);
        }
        match self.book_dispatch.try_request(purpose) {
            Ok(response) => {
                self.pending_book = Some((purpose, response));
                Ok(PmBookRequestOutcome::Requested(purpose))
            }
            Err(book::PmBookDispatchError::Full) => {
                self.retain_book_demand(purpose)?;
                Ok(PmBookRequestOutcome::Backpressured)
            }
            Err(book::PmBookDispatchError::Closed) => {
                Err(PmReadIngressActorError::RestBookWorkerClosed)
            }
        }
    }

    pub(super) fn mark_controlled_shutdown_issued(&mut self) {
        self.user_state.mark_controlled_shutdown_issued();
    }

    fn retain_book_demand(
        &mut self,
        purpose: PmRestBookPurpose,
    ) -> Result<(), PmReadIngressActorError> {
        self.pending_book_demand = merge_book_demand(self.pending_book_demand, purpose)
            .map_err(PmReadIngressActorError::PublicProtocol)?;
        Ok(())
    }

    fn service_book_once(&mut self) -> Result<PmBookServiceOutcome, PmReadIngressActorError> {
        if self.pending_book.is_none()
            && let Some(purpose) = self.pending_book_demand.take()
        {
            match self.book_dispatch.try_request(purpose) {
                Ok(response) => self.pending_book = Some((purpose, response)),
                Err(book::PmBookDispatchError::Full) => {
                    self.pending_book_demand = Some(purpose);
                    return Ok(PmBookServiceOutcome::Pending);
                }
                Err(book::PmBookDispatchError::Closed) => {
                    return Err(PmReadIngressActorError::RestBookWorkerClosed);
                }
            }
        }
        let Some((purpose, mut response)) = self.pending_book.take() else {
            return Ok(PmBookServiceOutcome::Idle);
        };
        match response.try_recv() {
            Ok(Ok(())) => Ok(PmBookServiceOutcome::Complete(purpose)),
            Ok(Err(error)) => Err(PmReadIngressActorError::RestBook(Box::new(error))),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                self.pending_book = Some((purpose, response));
                Ok(PmBookServiceOutcome::Pending)
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                Err(PmReadIngressActorError::RestBookResponseClosed)
            }
        }
    }

    /// Reserves the HTTP worker slot before taking the head refresh effect or
    /// minting its purpose ticket. Caller-selected cursors and order IDs never
    /// cross this boundary.
    pub(super) fn dispatch_next_refresh<M: PmQuoteModel>(
        &mut self,
        actor: &mut PmReadIngressActor<'_, M>,
    ) -> Result<PmHttpDispatchOutcome, PmReadIngressActorError> {
        if self.pending_http.is_some() {
            return Ok(PmHttpDispatchOutcome::Occupied);
        }
        let permit = match self.http_dispatch.try_reserve() {
            Ok(permit) => permit,
            Err(http::PmAuthenticatedHttpDispatchError::Full) => {
                return Ok(PmHttpDispatchOutcome::Backpressured);
            }
            Err(http::PmAuthenticatedHttpDispatchError::Closed) => {
                return Err(PmReadIngressActorError::HttpWorkerClosed);
            }
        };
        let Some(refresh) = actor.coordinator.peek_reconciliation_refresh_effect() else {
            return Ok(PmHttpDispatchOutcome::NoRefreshEffect);
        };
        if !actor.coordinator.refresh_effect_matches_scope(refresh) {
            return Err(PmReadIngressActorError::HttpProtocol(
                "read refresh effect did not match the configured product scope",
            ));
        }
        let request_ns = actor
            .actor_clock
            .observe_control_edge()?
            .received_clock()
            .monotonic_receive_ns();
        let reserved = match refresh.kind() {
            PmRefreshEffectKind::OpenOrders => {
                let ticket = actor.issuer.begin_open_orders_query(request_ns)?;
                PmReservedHttpQuery::OpenOrders(ticket)
            }
            PmRefreshEffectKind::OrderDetail(requested_order) => {
                let ticket = actor.issuer.begin_order_detail_query(request_ns)?;
                PmReservedHttpQuery::OrderDetail {
                    ticket,
                    requested_order,
                }
            }
            PmRefreshEffectKind::Account => {
                let ticket = actor.issuer.begin_account_query(request_ns)?;
                PmReservedHttpQuery::Account(ticket)
            }
            PmRefreshEffectKind::CompleteReconciliation => {
                let requested_after = actor.coordinator.current_fill_query_cursor();
                let ticket = actor.issuer.begin_reconciliation_query(request_ns)?;
                PmReservedHttpQuery::Reconciliation {
                    ticket,
                    requested_after,
                }
            }
        };
        let taken = actor
            .coordinator
            .pop_reconciliation_refresh_effect()
            .expect("peeked refresh remains at the FIFO head until dispatch");
        debug_assert_eq!(taken, refresh);
        let pending = match reserved {
            PmReservedHttpQuery::OpenOrders(ticket) => PmPendingHttpQuery::OpenOrders {
                ticket,
                response: permit.send_open_orders(),
            },
            PmReservedHttpQuery::OrderDetail {
                ticket,
                requested_order,
            } => PmPendingHttpQuery::OrderDetail {
                ticket,
                requested_order,
                response: permit.send_order_detail(requested_order),
            },
            PmReservedHttpQuery::Account(ticket) => PmPendingHttpQuery::Account {
                ticket,
                response: permit.send_account(),
            },
            PmReservedHttpQuery::Reconciliation {
                ticket,
                requested_after,
            } => PmPendingHttpQuery::Reconciliation {
                ticket,
                requested_after,
                response: permit.send_reconciliation(requested_after),
            },
        };
        self.pending_http = Some(pending);
        Ok(PmHttpDispatchOutcome::Dispatched(refresh.kind()))
    }

    /// Polls exactly one already-dispatched complete HTTP cut. This never
    /// waits for endpoint I/O; an unfinished one-shot is returned as Pending.
    pub(super) fn service_http_once<M: PmQuoteModel>(
        &mut self,
        actor: &mut PmReadIngressActor<'_, M>,
    ) -> Result<PmAuthenticatedReadOutcome, PmReadIngressActorError> {
        let Some(pending) = self.pending_http.take() else {
            return Ok(PmAuthenticatedReadOutcome::Idle);
        };
        let ready = match pending.poll() {
            Ok(PmPendingHttpPoll::Ready(ready)) => ready,
            Ok(PmPendingHttpPoll::Pending(still_pending)) => {
                self.pending_http = Some(still_pending);
                return Ok(PmAuthenticatedReadOutcome::Pending);
            }
            Err(()) => return Err(PmReadIngressActorError::HttpResponseClosed),
        };
        service_http_result(ready, actor)
    }

    /// Drains at most one transport result into the sole product actor.
    ///
    /// The fixed four-source rotation prevents sustained socket traffic from
    /// starving an already-complete authenticated HTTP or REST-book result.
    /// Pending worker responses do not consume a turn.
    pub(super) async fn service_once<M: PmQuoteModel>(
        &mut self,
        actor: &mut PmReadIngressActor<'_, M>,
    ) -> Result<PmReadIngressServiceOutcome, PmReadIngressServiceError> {
        self.service_once_with_mode(actor, false).await
    }

    async fn service_once_with_mode<M: PmQuoteModel>(
        &mut self,
        actor: &mut PmReadIngressActor<'_, M>,
        shutting_down: bool,
    ) -> Result<PmReadIngressServiceOutcome, PmReadIngressServiceError> {
        if !shutting_down {
            self.reject_unexpected_task_exit().await?;
        }
        for offset in 0..4_u8 {
            let source = (self.next_source + offset) % 4;
            let serviced = match source {
                0 => match self.user_requests.try_recv() {
                    Ok(request) => {
                        user::service_user_request(request, &mut self.user_state, actor)
                            .await
                            .map_err(|error| PmReadIngressServiceError::Actor(Box::new(error)))?;
                        Some(PmReadIngressServiceOutcome::User)
                    }
                    Err(mpsc::error::TryRecvError::Disconnected) if !shutting_down => {
                        return Err(self.take_user_task_exit().await);
                    }
                    Err(
                        mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected,
                    ) => None,
                },
                1 => match self.public_requests.try_recv() {
                    Ok(request) => {
                        let outcome =
                            public::service_public_request(request, &mut self.public_state, actor)
                                .await
                                .map_err(|error| {
                                    PmReadIngressServiceError::Actor(Box::new(error))
                                })?;
                        match outcome {
                            public::PmPublicServiceOutcome::Admitted => {}
                            public::PmPublicServiceOutcome::SubscriptionReady => {
                                match self.request_rest_book(PmRestBookPurpose::Seed) {
                                    Ok(_) => {}
                                    Err(PmReadIngressActorError::RestBookWorkerClosed) => {
                                        let failure =
                                            join_book_task(self.book_task.take(), false).await;
                                        return Err(PmReadIngressServiceError::BookTaskExited(
                                            failure,
                                        ));
                                    }
                                    Err(error) => {
                                        return Err(PmReadIngressServiceError::Actor(Box::new(
                                            error,
                                        )));
                                    }
                                }
                            }
                            public::PmPublicServiceOutcome::ResyncRequired => {
                                match self.request_rest_book(PmRestBookPurpose::Resync) {
                                    Ok(_) => {}
                                    Err(PmReadIngressActorError::RestBookWorkerClosed) => {
                                        let failure =
                                            join_book_task(self.book_task.take(), false).await;
                                        return Err(PmReadIngressServiceError::BookTaskExited(
                                            failure,
                                        ));
                                    }
                                    Err(error) => {
                                        return Err(PmReadIngressServiceError::Actor(Box::new(
                                            error,
                                        )));
                                    }
                                }
                            }
                        }
                        Some(PmReadIngressServiceOutcome::Public)
                    }
                    Err(mpsc::error::TryRecvError::Disconnected) if !shutting_down => {
                        return Err(self.take_public_task_exit().await);
                    }
                    Err(
                        mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected,
                    ) => None,
                },
                2 => {
                    let outcome = match self.service_http_once(actor) {
                        Ok(outcome) => outcome,
                        Err(PmReadIngressActorError::HttpResponseClosed) => {
                            let failure = join_http_task(self.http_task.take(), false).await;
                            return Err(PmReadIngressServiceError::HttpTaskExited(failure));
                        }
                        Err(error) => {
                            return Err(PmReadIngressServiceError::Actor(Box::new(error)));
                        }
                    };
                    match outcome {
                        PmAuthenticatedReadOutcome::Idle | PmAuthenticatedReadOutcome::Pending => {
                            None
                        }
                        ready => Some(PmReadIngressServiceOutcome::Http(ready)),
                    }
                }
                3 => {
                    let outcome = match self.service_book_once() {
                        Ok(outcome) => outcome,
                        Err(PmReadIngressActorError::RestBookResponseClosed) => {
                            let failure = join_book_task(self.book_task.take(), false).await;
                            return Err(PmReadIngressServiceError::BookTaskExited(failure));
                        }
                        Err(error) => {
                            return Err(PmReadIngressServiceError::Actor(Box::new(error)));
                        }
                    };
                    match outcome {
                        PmBookServiceOutcome::Idle | PmBookServiceOutcome::Pending => None,
                        ready => Some(PmReadIngressServiceOutcome::Book(ready)),
                    }
                }
                _ => unreachable!("four-source rotation is closed over 0..4"),
            };
            if let Some(serviced) = serviced {
                self.next_source = (source + 1) % 4;
                return Ok(serviced);
            }
        }
        Ok(PmReadIngressServiceOutcome::Idle)
    }

    async fn reject_unexpected_task_exit(&mut self) -> Result<(), PmReadIngressServiceError> {
        if self
            .public_task
            .as_ref()
            .is_some_and(PmAbortableTask::is_finished)
        {
            return Err(self.take_public_task_exit().await);
        }
        if self
            .user_task
            .as_ref()
            .is_some_and(PmAbortableTask::is_finished)
        {
            return Err(self.take_user_task_exit().await);
        }
        if self
            .http_task
            .as_ref()
            .is_some_and(PmAbortableTask::is_finished)
        {
            let failure = join_http_task(self.http_task.take(), false).await;
            return Err(PmReadIngressServiceError::HttpTaskExited(failure));
        }
        if self
            .book_task
            .as_ref()
            .is_some_and(PmAbortableTask::is_finished)
        {
            let failure = join_book_task(self.book_task.take(), false).await;
            return Err(PmReadIngressServiceError::BookTaskExited(failure));
        }
        Ok(())
    }

    async fn take_public_task_exit(&mut self) -> PmReadIngressServiceError {
        let failure = join_public_task(self.public_task.take(), false).await;
        PmReadIngressServiceError::PublicTaskExited(failure)
    }

    async fn take_user_task_exit(&mut self) -> PmReadIngressServiceError {
        let failure = join_user_task(self.user_task.take(), false).await;
        PmReadIngressServiceError::UserTaskExited(failure)
    }

    pub(super) async fn take_http_task_exit(&mut self) -> PmReadIngressServiceError {
        let failure = join_http_task(self.http_task.take(), false).await;
        PmReadIngressServiceError::HttpTaskExited(failure)
    }

    /// Stops socket production, drains every already-admitted actor barrier,
    /// then joins both role tasks. A bounded timeout aborts only read/socket
    /// tasks and remains part of returned shutdown evidence.
    pub(super) async fn shutdown<M: PmQuoteModel>(
        mut self,
        actor: &mut PmReadIngressActor<'_, M>,
        shutdown_effect_counts: &mut [u64; 5],
    ) -> Result<(), PmReadIngressShutdownError> {
        self.public_shutdown.request_shutdown();
        self.user_shutdown.request_shutdown();
        let deadline = tokio::time::Instant::now() + READ_SHUTDOWN_TIMEOUT;
        let mut timed_out = false;
        let mut actor_failure = None;
        let mut http_receipt = None;
        let mut book_receipt = None;
        let mut http_stopped = false;
        let mut book_stopped = false;
        let mut public_failure = None;
        let mut user_failure = None;
        let mut http_failure = None;
        let mut book_failure = None;
        let mut refresh_unresolved = false;
        let mut book_obligation_unresolved = false;
        loop {
            match self.service_once_with_mode(actor, true).await {
                Ok(_) => {}
                Err(PmReadIngressServiceError::Actor(error)) => {
                    if actor_failure.is_none() {
                        actor_failure = Some(error);
                    }
                }
                Err(PmReadIngressServiceError::PublicTaskExited(failure)) => {
                    if public_failure.is_none() {
                        public_failure = failure;
                    }
                }
                Err(PmReadIngressServiceError::UserTaskExited(failure)) => {
                    if user_failure.is_none() {
                        user_failure = failure;
                    }
                }
                Err(PmReadIngressServiceError::HttpTaskExited(failure)) => {
                    if http_failure.is_none() {
                        http_failure = failure;
                    }
                    http_stopped = true;
                }
                Err(PmReadIngressServiceError::BookTaskExited(failure)) => {
                    if book_failure.is_none() {
                        book_failure = failure;
                    }
                    book_stopped = true;
                }
            }
            actor.drain_shutdown_copied_effects(shutdown_effect_counts);

            let public_socket_stopped = self
                .public_task
                .as_ref()
                .is_none_or(PmAbortableTask::is_finished);
            let user_socket_stopped = self
                .user_task
                .as_ref()
                .is_none_or(PmAbortableTask::is_finished);
            let sockets_drained = public_socket_stopped
                && user_socket_stopped
                && self.public_requests.is_empty()
                && self.user_requests.is_empty();

            // The REST-book worker is the final producer of public actor
            // requests. It must finish every retained Seed/Resync demand and
            // the actor must acknowledge the resulting book before the
            // private HTTP worker may be stopped.
            if book_shutdown_ready(
                sockets_drained,
                self.pending_book.is_some(),
                self.pending_book_demand.is_some(),
            ) && book_receipt.is_none()
                && !book_stopped
            {
                match self.book_dispatch.try_shutdown() {
                    Ok(receipt) => book_receipt = Some(receipt),
                    Err(book::PmBookDispatchError::Full) => {}
                    Err(book::PmBookDispatchError::Closed) => {
                        if book_failure.is_none() {
                            book_failure = join_book_task(self.book_task.take(), false).await;
                        }
                        book_stopped = true;
                    }
                }
            }
            if let Some(receipt) = book_receipt.as_mut() {
                match receipt.try_recv() {
                    Ok(()) => book_stopped = true,
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => book_stopped = true,
                }
            }

            // Once transport producers are quiet, keep advancing the sole
            // actor even when no channel item is ready. A complete HTTP cut
            // may already be queued below a persistence bridge, and retained
            // private/reconciliation admission needs a later turn to retry.
            if book_stopped && sockets_drained && self.pending_http.is_none() {
                if let Err(error) = actor.service_after_ingress()
                    && actor_failure.is_none()
                {
                    actor_failure = Some(Box::new(error));
                }
                actor.drain_shutdown_copied_effects(shutdown_effect_counts);
            }
            let read_unresolved_counts = actor.coordinator.read_shutdown_unresolved_counts();
            let read_lanes_quiescent = read_unresolved_counts.into_iter().all(|count| count == 0);

            let refresh_quiescent = if book_stopped
                && sockets_drained
                && self.pending_http.is_none()
                && !http_stopped
            {
                match self.dispatch_next_refresh(actor) {
                    Ok(PmHttpDispatchOutcome::NoRefreshEffect) => true,
                    Ok(
                        PmHttpDispatchOutcome::Occupied
                        | PmHttpDispatchOutcome::Backpressured
                        | PmHttpDispatchOutcome::Dispatched(_),
                    ) => false,
                    Err(PmReadIngressActorError::HttpWorkerClosed) => {
                        if http_failure.is_none() {
                            http_failure = join_http_task(self.http_task.take(), false).await;
                        }
                        http_stopped = true;
                        false
                    }
                    Err(error) => {
                        if actor_failure.is_none() {
                            actor_failure = Some(Box::new(error));
                        }
                        false
                    }
                }
            } else {
                false
            };

            if http_shutdown_ready(
                book_stopped,
                sockets_drained,
                self.pending_http.is_some(),
                refresh_quiescent,
                read_lanes_quiescent,
            ) && http_receipt.is_none()
                && !http_stopped
            {
                match self.http_dispatch.try_request_shutdown() {
                    Ok(receipt) => http_receipt = Some(receipt),
                    Err(http::PmAuthenticatedHttpDispatchError::Full) => {}
                    Err(http::PmAuthenticatedHttpDispatchError::Closed) => {
                        if http_failure.is_none() {
                            http_failure = join_http_task(self.http_task.take(), false).await;
                        }
                        http_stopped = true;
                    }
                }
            }
            if let Some(receipt) = http_receipt.as_mut() {
                match receipt.try_recv() {
                    Ok(()) => http_stopped = true,
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => http_stopped = true,
                }
            }
            if http_stopped && book_stopped && sockets_drained {
                refresh_unresolved = actor
                    .coordinator
                    .peek_reconciliation_refresh_effect()
                    .is_some()
                    || self.pending_http.is_some();
                book_obligation_unresolved = self.pending_book.is_some()
                    || self.pending_book_demand.is_some()
                    || !self.public_requests.is_empty();
                if (!refresh_unresolved && !book_obligation_unresolved && read_lanes_quiescent)
                    || http_failure.is_some()
                    || book_failure.is_some()
                {
                    break;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                timed_out = true;
                break;
            }
            tokio::time::sleep(READ_SHUTDOWN_POLL_BACKOFF).await;
        }

        // Preserve obligations even when the overall bound, rather than a
        // clean worker receipt, ended the drain.
        refresh_unresolved |= actor
            .coordinator
            .peek_reconciliation_refresh_effect()
            .is_some()
            || self.pending_http.is_some();
        book_obligation_unresolved |= self.pending_book.is_some()
            || self.pending_book_demand.is_some()
            || !self.public_requests.is_empty();
        let read_unresolved_counts = actor.coordinator.read_shutdown_unresolved_counts();

        if public_failure.is_none() && self.public_task.is_some() {
            public_failure = join_public_task(self.public_task.take(), timed_out).await;
        }
        if user_failure.is_none() && self.user_task.is_some() {
            user_failure = join_user_task(self.user_task.take(), timed_out).await;
        }
        if http_failure.is_none() && self.http_task.is_some() {
            http_failure = join_http_task(self.http_task.take(), timed_out).await;
        }
        if book_failure.is_none() && self.book_task.is_some() {
            book_failure = join_book_task(self.book_task.take(), timed_out).await;
        }
        if actor_failure.is_none()
            && public_failure.is_none()
            && user_failure.is_none()
            && http_failure.is_none()
            && book_failure.is_none()
            && read_unresolved_counts.into_iter().all(|count| count == 0)
            && !refresh_unresolved
            && !book_obligation_unresolved
            && !timed_out
        {
            Ok(())
        } else {
            Err(PmReadIngressShutdownError {
                actor: actor_failure,
                public: public_failure,
                user: user_failure,
                http: http_failure,
                book: book_failure,
                read_unresolved_counts,
                refresh_unresolved,
                book_obligation_unresolved,
                timed_out,
            })
        }
    }
}

fn merge_book_demand(
    retained: Option<PmRestBookPurpose>,
    requested: PmRestBookPurpose,
) -> Result<Option<PmRestBookPurpose>, &'static str> {
    match (retained, requested) {
        (None, requested) => Ok(Some(requested)),
        (Some(PmRestBookPurpose::Seed), PmRestBookPurpose::Seed)
        | (Some(PmRestBookPurpose::Resync), PmRestBookPurpose::Resync) => Ok(retained),
        (Some(PmRestBookPurpose::Seed), PmRestBookPurpose::Resync) => {
            Ok(Some(PmRestBookPurpose::Resync))
        }
        (Some(PmRestBookPurpose::Resync), PmRestBookPurpose::Seed) => {
            Err("initial REST-book seed was requested after canonical resynchronization")
        }
    }
}

enum PmReservedHttpQuery {
    OpenOrders(PmLiveOpenOrdersQueryTicket),
    OrderDetail {
        ticket: PmLiveOrderDetailQueryTicket,
        requested_order: PmVenueOrderKey,
    },
    Account(PmLiveAccountQueryTicket),
    Reconciliation {
        ticket: PmLiveReconciliationQueryTicket,
        requested_after: Option<PmFillQueryCursor>,
    },
}

enum PmPendingHttpQuery {
    OpenOrders {
        ticket: PmLiveOpenOrdersQueryTicket,
        response: http::PmAuthenticatedHttpResponse<
            reap_polymarket_live_adapter::PmCompleteOpenOrdersCut,
        >,
    },
    OrderDetail {
        ticket: PmLiveOrderDetailQueryTicket,
        requested_order: PmVenueOrderKey,
        response: http::PmAuthenticatedHttpResponse<http::PmCompleteOrderDetailRead>,
    },
    Account {
        ticket: PmLiveAccountQueryTicket,
        response: http::PmAuthenticatedHttpResponse<http::PmCompleteAccountRead>,
    },
    Reconciliation {
        ticket: PmLiveReconciliationQueryTicket,
        requested_after: Option<PmFillQueryCursor>,
        response: http::PmAuthenticatedHttpResponse<http::PmCompleteReconciliationRead>,
    },
}

impl PmPendingHttpQuery {
    fn poll(self) -> Result<PmPendingHttpPoll, ()> {
        match self {
            Self::OpenOrders {
                ticket,
                mut response,
            } => match response.try_recv() {
                Ok(result) => Ok(PmPendingHttpPoll::Ready(PmReadyHttpQuery::OpenOrders {
                    ticket,
                    result,
                })),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    Ok(PmPendingHttpPoll::Pending(Self::OpenOrders {
                        ticket,
                        response,
                    }))
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => Err(()),
            },
            Self::OrderDetail {
                ticket,
                requested_order,
                mut response,
            } => match response.try_recv() {
                Ok(result) => Ok(PmPendingHttpPoll::Ready(PmReadyHttpQuery::OrderDetail {
                    ticket,
                    requested_order,
                    result,
                })),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    Ok(PmPendingHttpPoll::Pending(Self::OrderDetail {
                        ticket,
                        requested_order,
                        response,
                    }))
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => Err(()),
            },
            Self::Account {
                ticket,
                mut response,
            } => match response.try_recv() {
                Ok(result) => Ok(PmPendingHttpPoll::Ready(PmReadyHttpQuery::Account {
                    ticket,
                    result,
                })),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    Ok(PmPendingHttpPoll::Pending(Self::Account {
                        ticket,
                        response,
                    }))
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => Err(()),
            },
            Self::Reconciliation {
                ticket,
                requested_after,
                mut response,
            } => match response.try_recv() {
                Ok(result) => Ok(PmPendingHttpPoll::Ready(PmReadyHttpQuery::Reconciliation {
                    ticket,
                    requested_after,
                    result,
                })),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    Ok(PmPendingHttpPoll::Pending(Self::Reconciliation {
                        ticket,
                        requested_after,
                        response,
                    }))
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => Err(()),
            },
        }
    }
}

#[allow(
    clippy::large_enum_variant,
    reason = "the single actor-owned pending HTTP slot retains its typed completion inline until canonical admission"
)]
enum PmPendingHttpPoll {
    Pending(PmPendingHttpQuery),
    Ready(PmReadyHttpQuery),
}

#[allow(
    clippy::large_enum_variant,
    reason = "the bounded actor handoff preserves each purpose-typed complete cut inline without detached or shared authority"
)]
enum PmReadyHttpQuery {
    OpenOrders {
        ticket: PmLiveOpenOrdersQueryTicket,
        result: http::PmAuthenticatedHttpReadResult<
            reap_polymarket_live_adapter::PmCompleteOpenOrdersCut,
        >,
    },
    OrderDetail {
        ticket: PmLiveOrderDetailQueryTicket,
        requested_order: PmVenueOrderKey,
        result: http::PmAuthenticatedHttpReadResult<http::PmCompleteOrderDetailRead>,
    },
    Account {
        ticket: PmLiveAccountQueryTicket,
        result: http::PmAuthenticatedHttpReadResult<http::PmCompleteAccountRead>,
    },
    Reconciliation {
        ticket: PmLiveReconciliationQueryTicket,
        requested_after: Option<PmFillQueryCursor>,
        result: http::PmAuthenticatedHttpReadResult<http::PmCompleteReconciliationRead>,
    },
}

fn service_http_result<M: PmQuoteModel>(
    ready: PmReadyHttpQuery,
    actor: &mut PmReadIngressActor<'_, M>,
) -> Result<PmAuthenticatedReadOutcome, PmReadIngressActorError> {
    match ready {
        PmReadyHttpQuery::OpenOrders { ticket, result } => match result {
            http::PmAuthenticatedHttpReadResult::Complete(completion) => {
                let (edge, cut) = completion.into_parts();
                let received = private_read_received_clock(edge)?;
                let service_ns = actor
                    .actor_clock
                    .observe_control_edge()?
                    .received_clock()
                    .monotonic_receive_ns();
                let input = actor
                    .issuer
                    .complete_open_orders_query(ticket, received, service_ns, cut)?;
                let _ = actor.coordinator.ingest_open_orders_live(input)?;
                let _ = actor.service_after_ingress()?;
                Ok(PmAuthenticatedReadOutcome::Complete)
            }
            failure => service_open_orders_failure(ticket, failure, actor),
        },
        PmReadyHttpQuery::OrderDetail {
            ticket,
            requested_order,
            result,
        } => match result {
            http::PmAuthenticatedHttpReadResult::Complete(completion) => {
                let (edge, detail) = completion.into_parts();
                if detail.requested_order() != requested_order {
                    return Err(PmReadIngressActorError::HttpProtocol(
                        "exact-order worker response changed the actor-owned identity",
                    ));
                }
                let (returned_order, observation) = detail.into_parts();
                let received = private_read_received_clock(edge)?;
                let service_ns = actor
                    .actor_clock
                    .observe_control_edge()?
                    .received_clock()
                    .monotonic_receive_ns();
                let input = actor.issuer.complete_order_detail_query(
                    ticket,
                    received,
                    service_ns,
                    returned_order,
                    observation,
                )?;
                let _ = actor.coordinator.ingest_order_detail_live(input)?;
                let _ = actor.service_after_ingress()?;
                Ok(PmAuthenticatedReadOutcome::Complete)
            }
            failure => service_order_detail_failure(ticket, failure, actor),
        },
        PmReadyHttpQuery::Account { ticket, result } => match result {
            http::PmAuthenticatedHttpReadResult::Complete(completion) => {
                let (edge, account) = completion.into_parts();
                let (collateral, conditional) = account.into_parts();
                let received = private_read_received_clock(edge)?;
                let service_ns = actor
                    .actor_clock
                    .observe_control_edge()?
                    .received_clock()
                    .monotonic_receive_ns();
                let input = actor.issuer.complete_account_query(
                    ticket,
                    received,
                    service_ns,
                    collateral,
                    conditional,
                )?;
                let _ = actor.coordinator.ingest_account_live(input)?;
                let _ = actor.service_after_ingress()?;
                Ok(PmAuthenticatedReadOutcome::Complete)
            }
            failure => service_account_failure(ticket, failure, actor),
        },
        PmReadyHttpQuery::Reconciliation {
            ticket,
            requested_after,
            result,
        } => match result {
            http::PmAuthenticatedHttpReadResult::Complete(completion) => {
                let (edge, reconciliation) = completion.into_parts();
                let (collateral, conditional, returned_after, trades) = reconciliation.into_parts();
                if returned_after != requested_after {
                    return Err(PmReadIngressActorError::HttpProtocol(
                        "reconciliation worker response changed the canonical fill cursor",
                    ));
                }
                let received = private_read_received_clock(edge)?;
                let service_ns = actor
                    .actor_clock
                    .observe_control_edge()?
                    .received_clock()
                    .monotonic_receive_ns();
                let input = actor.issuer.complete_reconciliation_query(
                    ticket,
                    received,
                    service_ns,
                    collateral,
                    conditional,
                    returned_after,
                    trades,
                )?;
                let _ = actor.coordinator.ingest_reconciliation_live(input)?;
                let _ = actor.service_after_ingress()?;
                Ok(PmAuthenticatedReadOutcome::Complete)
            }
            failure => service_reconciliation_failure(ticket, failure, actor),
        },
    }
}

fn private_read_received_clock(
    edge: reap_polymarket_live_adapter::PmPrivateReadEdgeClock,
) -> Result<ReceivedEventClock, PmReadIngressActorError> {
    ReceivedEventClock::new(
        None,
        edge.local_wall_receive_ns(),
        edge.monotonic_receive_ns(),
    )
    .map_err(|_| PmReadIngressActorError::HttpProtocol("invalid private HTTP receive-edge clock"))
}

fn classify_http_failure<T>(
    result: http::PmAuthenticatedHttpReadResult<T>,
) -> Result<
    (
        ReceivedEventClock,
        PmLiveHttpQueryFailure,
        Option<http::PmAuthenticatedHttpTerminalFailure>,
    ),
    PmReadIngressActorError,
> {
    match result {
        http::PmAuthenticatedHttpReadResult::Recoverable {
            completed_at,
            failure,
        } => Ok((private_read_received_clock(completed_at)?, failure, None)),
        http::PmAuthenticatedHttpReadResult::Terminal {
            completed_at,
            failure,
        } => {
            let query_failure = match failure {
                http::PmAuthenticatedHttpTerminalFailure::ScopeMismatch => {
                    PmLiveHttpQueryFailure::ScopeMismatch
                }
                http::PmAuthenticatedHttpTerminalFailure::ContractViolation => {
                    PmLiveHttpQueryFailure::ContractViolation
                }
                http::PmAuthenticatedHttpTerminalFailure::CredentialAuthorityClosed
                | http::PmAuthenticatedHttpTerminalFailure::CredentialAuthorityTaskFailed => {
                    PmLiveHttpQueryFailure::Authentication
                }
            };
            Ok((
                private_read_received_clock(completed_at)?,
                query_failure,
                Some(failure),
            ))
        }
        http::PmAuthenticatedHttpReadResult::CompletionClockFailed(error) => {
            Err(PmReadIngressActorError::Clock(error))
        }
        http::PmAuthenticatedHttpReadResult::Complete(_) => Err(
            PmReadIngressActorError::HttpProtocol("complete HTTP result entered failure path"),
        ),
    }
}

macro_rules! service_http_failure {
    ($name:ident, $ticket:ty, $issuer:ident, $coordinator:ident) => {
        fn $name<M: PmQuoteModel, T>(
            ticket: $ticket,
            result: http::PmAuthenticatedHttpReadResult<T>,
            actor: &mut PmReadIngressActor<'_, M>,
        ) -> Result<PmAuthenticatedReadOutcome, PmReadIngressActorError> {
            let (received, failure, terminal) = classify_http_failure(result)?;
            let input = actor.issuer.$issuer(ticket, received, failure)?;
            actor.coordinator.$coordinator(input)?;
            let _ = actor.service_after_ingress()?;
            if let Some(terminal) = terminal {
                Err(PmReadIngressActorError::HttpTerminal(terminal))
            } else {
                Ok(PmAuthenticatedReadOutcome::DependencyInvalidated(failure))
            }
        }
    };
}

service_http_failure!(
    service_open_orders_failure,
    PmLiveOpenOrdersQueryTicket,
    fail_open_orders_query,
    ingest_open_orders_live_failure
);
service_http_failure!(
    service_order_detail_failure,
    PmLiveOrderDetailQueryTicket,
    fail_order_detail_query,
    ingest_order_detail_live_failure
);
service_http_failure!(
    service_account_failure,
    PmLiveAccountQueryTicket,
    fail_account_query,
    ingest_account_live_failure
);
service_http_failure!(
    service_reconciliation_failure,
    PmLiveReconciliationQueryTicket,
    fail_reconciliation_query,
    ingest_reconciliation_live_failure
);

async fn join_public_task(
    task: Option<PmAbortableTask<PublicTaskOutput>>,
    abort: bool,
) -> Option<PmPublicTaskFailure> {
    let Some(mut task) = task else {
        return Some(PmPublicTaskFailure::Join(PmReadJoinFailure::MissingTask));
    };
    let result = if abort {
        task.abort_and_join().await
    } else {
        task.join().await
    };
    match result {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(PmPublicTaskFailure::Role(Box::new(error))),
        Err(error) => Some(PmPublicTaskFailure::Join(classify_join(error, abort))),
    }
}

async fn join_user_task(
    task: Option<PmAbortableTask<UserTaskOutput>>,
    abort: bool,
) -> Option<PmUserTaskFailure> {
    let Some(mut task) = task else {
        return Some(PmUserTaskFailure::Join(PmReadJoinFailure::MissingTask));
    };
    let result = if abort {
        task.abort_and_join().await
    } else {
        task.join().await
    };
    match result {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(PmUserTaskFailure::Role(Box::new(error))),
        Err(error) => Some(PmUserTaskFailure::Join(classify_join(error, abort))),
    }
}

async fn join_http_task(
    task: Option<PmAbortableTask<http::PmHttpTaskOutput>>,
    abort: bool,
) -> Option<PmHttpTaskFailure> {
    let Some(mut task) = task else {
        return Some(PmHttpTaskFailure::Join(PmReadJoinFailure::MissingTask));
    };
    let result = if abort {
        task.abort_and_join().await
    } else {
        task.join().await
    };
    match result {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(PmHttpTaskFailure::Task(error)),
        Err(error) => Some(PmHttpTaskFailure::Join(classify_join(error, abort))),
    }
}

async fn join_book_task(
    task: Option<PmAbortableTask<book::PmBookTaskOutput>>,
    abort: bool,
) -> Option<PmBookTaskFailure> {
    let Some(mut task) = task else {
        return Some(PmBookTaskFailure::Join(PmReadJoinFailure::MissingTask));
    };
    let result = if abort {
        task.abort_and_join().await
    } else {
        task.join().await
    };
    match result {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(PmBookTaskFailure::Task(error)),
        Err(error) => Some(PmBookTaskFailure::Join(classify_join(error, abort))),
    }
}

fn classify_join(error: tokio::task::JoinError, abort: bool) -> PmReadJoinFailure {
    if error.is_cancelled() && abort {
        PmReadJoinFailure::TimedOutAndAborted
    } else if error.is_cancelled() {
        PmReadJoinFailure::Cancelled
    } else if error.is_panic() {
        PmReadJoinFailure::Panicked
    } else {
        PmReadJoinFailure::Unclassified
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PmReadIngressServiceOutcome {
    Idle,
    User,
    Public,
    Http(PmAuthenticatedReadOutcome),
    Book(PmBookServiceOutcome),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PmBookRequestOutcome {
    Occupied,
    Backpressured,
    Requested(PmRestBookPurpose),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PmBookServiceOutcome {
    Idle,
    Pending,
    Complete(PmRestBookPurpose),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PmAuthenticatedReadOutcome {
    Idle,
    Pending,
    Complete,
    DependencyInvalidated(PmLiveHttpQueryFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PmHttpDispatchOutcome {
    NoRefreshEffect,
    Occupied,
    Backpressured,
    Dispatched(PmRefreshEffectKind),
}

#[derive(Debug, Error)]
pub(super) enum PmReadIngressServiceError {
    #[error("authenticated read-ingress actor failed: {0}")]
    Actor(Box<PmReadIngressActorError>),
    #[error("public WebSocket role task exited outside controlled shutdown: {0:?}")]
    PublicTaskExited(Option<PmPublicTaskFailure>),
    #[error("user WebSocket role task exited outside controlled shutdown: {0:?}")]
    UserTaskExited(Option<PmUserTaskFailure>),
    #[error("authenticated HTTP role task exited outside controlled shutdown: {0:?}")]
    HttpTaskExited(Option<PmHttpTaskFailure>),
    #[error("public REST-book role task exited outside controlled shutdown: {0:?}")]
    BookTaskExited(Option<PmBookTaskFailure>),
}

#[derive(Debug)]
pub(super) enum PmReadIngressActorError {
    Occurrence(PmLiveOccurrenceError),
    Coordinator(Box<PmCoordinatorError>),
    Clock(reap_polymarket_live_adapter::PmProductClockError),
    PublicCapture(Box<PmPublicCaptureRunError>),
    PublicBook(Box<PmProductPublicIngressError<PmPublicBookDelivery>>),
    PublicMetadata(
        Box<PmProductPublicIngressError<crate::public_routes::PmPublicMetadataDelivery>>,
    ),
    PublicTick(Box<PmPublicTerminalTickApplyError>),
    PublicProtocol(&'static str),
    UserProtocol(&'static str),
    HttpProtocol(&'static str),
    HttpWorkerClosed,
    HttpResponseClosed,
    HttpTerminal(http::PmAuthenticatedHttpTerminalFailure),
    RestBookWorkerClosed,
    RestBookResponseClosed,
    RestBook(
        Box<reap_polymarket_live_adapter::PmRestBookDeliveryError<channel::PmReadIngressSinkError>>,
    ),
}

impl fmt::Display for PmReadIngressActorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Occurrence(error) => write!(formatter, "live occurrence failed: {error}"),
            Self::Coordinator(error) => write!(formatter, "coordinator ingress failed: {error}"),
            Self::Clock(error) => write!(formatter, "shared product clock failed: {error}"),
            Self::PublicCapture(error) => write!(formatter, "public capture failed: {error}"),
            Self::PublicBook(error) => write!(formatter, "public book ingress failed: {error:?}"),
            Self::PublicMetadata(error) => {
                write!(formatter, "public metadata ingress failed: {error:?}")
            }
            Self::PublicTick(error) => write!(formatter, "terminal tick cleanup failed: {error:?}"),
            Self::PublicProtocol(reason) => write!(
                formatter,
                "public transport evidence violated its contract: {reason}"
            ),
            Self::UserProtocol(reason) => write!(
                formatter,
                "user transport evidence violated its contract: {reason}"
            ),
            Self::HttpProtocol(reason) => write!(
                formatter,
                "authenticated HTTP evidence violated its contract: {reason}"
            ),
            Self::HttpWorkerClosed => formatter.write_str("authenticated HTTP worker is closed"),
            Self::HttpResponseClosed => formatter
                .write_str("authenticated HTTP response channel closed without a typed result"),
            Self::HttpTerminal(failure) => write!(
                formatter,
                "authenticated HTTP supervisor failed terminally: {failure:?}"
            ),
            Self::RestBookWorkerClosed => formatter.write_str("public REST-book worker is closed"),
            Self::RestBookResponseClosed => {
                formatter.write_str("public REST-book worker closed its response without a result")
            }
            Self::RestBook(error) => write!(formatter, "public REST-book request failed: {error}"),
        }
    }
}

impl std::error::Error for PmReadIngressActorError {}

impl From<PmLiveOccurrenceError> for PmReadIngressActorError {
    fn from(error: PmLiveOccurrenceError) -> Self {
        Self::Occurrence(error)
    }
}

impl From<PmCoordinatorError> for PmReadIngressActorError {
    fn from(error: PmCoordinatorError) -> Self {
        Self::Coordinator(Box::new(error))
    }
}

impl From<reap_polymarket_live_adapter::PmProductClockError> for PmReadIngressActorError {
    fn from(error: reap_polymarket_live_adapter::PmProductClockError) -> Self {
        Self::Clock(error)
    }
}

impl From<PmPublicCaptureRunError> for PmReadIngressActorError {
    fn from(error: PmPublicCaptureRunError) -> Self {
        Self::PublicCapture(Box::new(error))
    }
}

impl From<PmProductPublicIngressError<PmPublicBookDelivery>> for PmReadIngressActorError {
    fn from(error: PmProductPublicIngressError<PmPublicBookDelivery>) -> Self {
        Self::PublicBook(Box::new(error))
    }
}

impl From<PmProductPublicIngressError<crate::public_routes::PmPublicMetadataDelivery>>
    for PmReadIngressActorError
{
    fn from(
        error: PmProductPublicIngressError<crate::public_routes::PmPublicMetadataDelivery>,
    ) -> Self {
        Self::PublicMetadata(Box::new(error))
    }
}

impl From<PmPublicTerminalTickApplyError> for PmReadIngressActorError {
    fn from(error: PmPublicTerminalTickApplyError) -> Self {
        Self::PublicTick(Box::new(error))
    }
}

impl From<PmPublicBookPipelineError> for PmReadIngressActorError {
    fn from(error: PmPublicBookPipelineError) -> Self {
        Self::PublicProtocol(match error {
            PmPublicBookPipelineError::Reduce(_) => "public book reducer rejected delivery",
            PmPublicBookPipelineError::Lane(_) => "public book lane rejected delivery",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(super) enum PmReadJoinFailure {
    #[error("read-side role task was missing")]
    MissingTask,
    #[error("read-side role task was cancelled")]
    Cancelled,
    #[error("read-side role task panicked")]
    Panicked,
    #[error("read-side role task join failed")]
    Unclassified,
    #[error("read-side role exceeded shutdown bound and was aborted")]
    TimedOutAndAborted,
}

#[derive(Debug)]
pub(super) enum PmPublicTaskFailure {
    Role(Box<reap_polymarket_live_adapter::PmPublicWsRunError<channel::PmReadIngressSinkError>>),
    Join(PmReadJoinFailure),
}

#[derive(Debug)]
pub(super) enum PmUserTaskFailure {
    Role(Box<reap_polymarket_live_adapter::PmUserWsRunError<channel::PmReadIngressSinkError>>),
    Join(PmReadJoinFailure),
}

#[derive(Debug)]
pub(super) enum PmHttpTaskFailure {
    Task(http::PmAuthenticatedHttpTaskError),
    Join(PmReadJoinFailure),
}

#[derive(Debug)]
pub(super) enum PmBookTaskFailure {
    Task(book::PmBookTaskError),
    Join(PmReadJoinFailure),
}
