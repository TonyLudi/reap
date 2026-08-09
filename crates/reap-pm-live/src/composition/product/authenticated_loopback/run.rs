//! Owned authenticated-loopback actor and task choreography.
//!
//! Public/user sockets, private reads, REST book, and independent mutation
//! time/transport tasks are statically owned here. Endpoint I/O stays in
//! supervised children; the sole actor performs only bounded admission and
//! incremental durability progress.

#[cfg(test)]
mod loopback_fixture_tests;
#[cfg(test)]
mod vertical_tests;

use std::sync::Arc;

use reap_pm_strategy::PmQuoteModel;
use reap_polymarket_live_adapter::{PmLiveAdapterError, PmProductClockError};
use thiserror::Error;

use super::mutation_time::{
    PmMutationTimeError, PmMutationTimePoll, PmMutationTimeShutdownError, PmMutationTimeSupervisor,
};
use super::read_ingress::{
    PmAuthenticatedReadIngress, PmHttpDispatchOutcome, PmReadIngressActor, PmReadIngressActorError,
    PmReadIngressServiceError, PmReadIngressServiceOutcome, PmReadIngressShutdownError,
};
use super::startup::{
    PmAuthenticatedLoopbackReady, PmAuthenticatedLoopbackShutdownError,
    PmAuthenticatedLoopbackStopped,
};
use super::supervision::PmMutationTask;
use crate::coordinator::{
    PmAuthenticatedBridgeFailure, PmAuthenticatedCancelTaskFinish,
    PmAuthenticatedCancelTaskOutcome, PmAuthenticatedExecutionError,
    PmAuthenticatedPlaceTaskFinish, PmAuthenticatedPlaceTaskOutcome,
    PmAuthenticatedTaskPreparationError, PmCoordinatorError, PmGoalFWriterFailure,
    PmLiveCancelCompletion, PmLivePlaceCompletion, PmPreparedMutationKind, PmProductEffect,
};
use crate::private_monitor::PmLiveOccurrenceError;

const MAX_AUTHENTICATED_BRIDGE_SERVICE_TURNS: usize = 65_536;

/// Static authenticated-loopback run with independent place/cancel tasks.
///
/// The task fields are independent: a place transport await does not own the
/// cancel worker or prevent a cancel task from being launched. Completed
/// values remain in this owner until a borrowed coordinator admission exists;
/// after admission, the corresponding worker remains unavailable until the
/// exact Goal-F applied proof is consumed.
pub(super) struct PmAuthenticatedLoopbackRun<M: PmQuoteModel> {
    ready: PmAuthenticatedLoopbackReady<M>,
    read_ingress: Option<PmAuthenticatedReadIngress>,
    read_terminal_failure: Option<Arc<PmReadIngressServiceError>>,
    mutation_time: Option<PmMutationTimeSupervisor>,
    place_task: Option<PmMutationTask<PmAuthenticatedPlaceTaskOutcome>>,
    cancel_task: Option<PmMutationTask<PmAuthenticatedCancelTaskOutcome>>,
    pending_place_completion: Option<PmLivePlaceCompletion>,
    pending_cancel_completion: Option<PmLiveCancelCompletion>,
    retained_place_outcome: Option<PmAuthenticatedPlaceTaskOutcome>,
    retained_cancel_outcome: Option<PmAuthenticatedCancelTaskOutcome>,
    place_bridge_pending: bool,
    cancel_bridge_pending: bool,
    place_bridge_deadline: Option<tokio::time::Instant>,
    cancel_bridge_deadline: Option<tokio::time::Instant>,
    place_bridge_turns: usize,
    cancel_bridge_turns: usize,
    place_completion_confirmed: bool,
    cancel_completion_confirmed: bool,
    place_task_terminal_failure: Option<PmMutationTaskTerminalFailure>,
    cancel_task_terminal_failure: Option<PmMutationTaskTerminalFailure>,
    place_execution_failure: Option<Arc<PmAuthenticatedExecutionError>>,
    cancel_execution_failure: Option<Arc<PmAuthenticatedExecutionError>>,
    place_bridge_failure: Option<PmAuthenticatedBridgeFailure>,
    cancel_bridge_failure: Option<PmAuthenticatedBridgeFailure>,
    goal_f_writer_failure: Option<PmGoalFWriterFailure>,
    terminal_secondary: Option<Box<PmAuthenticatedLoopbackRunError>>,
    shutdown_effect_counts: [u64; 5],
    shutdown_control_issued: bool,
    shutdown_unsent_quote_retained: bool,
}

impl<M: PmQuoteModel> PmAuthenticatedLoopbackReady<M> {
    pub(super) fn into_run(mut self) -> PmAuthenticatedLoopbackRun<M> {
        let place_time = self
            .place_server_time_http
            .take()
            .expect("authenticated ready owner retains place-time role once");
        let cancel_time = self
            .cancel_server_time_http
            .take()
            .expect("authenticated ready owner retains cancel-time role once");
        PmAuthenticatedLoopbackRun {
            ready: self,
            read_ingress: None,
            read_terminal_failure: None,
            mutation_time: Some(PmMutationTimeSupervisor::start(place_time, cancel_time)),
            place_task: None,
            cancel_task: None,
            pending_place_completion: None,
            pending_cancel_completion: None,
            retained_place_outcome: None,
            retained_cancel_outcome: None,
            place_bridge_pending: false,
            cancel_bridge_pending: false,
            place_bridge_deadline: None,
            cancel_bridge_deadline: None,
            place_bridge_turns: 0,
            cancel_bridge_turns: 0,
            place_completion_confirmed: false,
            cancel_completion_confirmed: false,
            place_task_terminal_failure: None,
            cancel_task_terminal_failure: None,
            place_execution_failure: None,
            cancel_execution_failure: None,
            place_bridge_failure: None,
            cancel_bridge_failure: None,
            goal_f_writer_failure: None,
            terminal_secondary: None,
            shutdown_effect_counts: [0; 5],
            shutdown_control_issued: false,
            shutdown_unsent_quote_retained: false,
        }
    }
}

impl<M: PmQuoteModel + Send + 'static> PmAuthenticatedLoopbackRun<M> {
    pub(super) fn start_read_ingress(&mut self) -> Result<(), PmAuthenticatedLoopbackRunError> {
        if self.read_ingress.is_some() {
            return Err(PmAuthenticatedLoopbackRunError::ReadIngressAlreadyStarted);
        }
        if self.ready.public_ws.is_none()
            || self.ready.authenticated_user_ws.is_none()
            || self.ready.authenticated_http.is_none()
            || self.ready.read_server_time_http.is_none()
            || self.ready.private_read_clock.is_none()
            || self.ready.book_http.is_none()
        {
            return Err(PmAuthenticatedLoopbackRunError::ReadIngressRoleMissing);
        }
        let public_ws = self
            .ready
            .public_ws
            .take()
            .ok_or(PmAuthenticatedLoopbackRunError::ReadIngressRoleMissing)?;
        let user_ws = self
            .ready
            .authenticated_user_ws
            .take()
            .ok_or(PmAuthenticatedLoopbackRunError::ReadIngressRoleMissing)?;
        let http = self
            .ready
            .authenticated_http
            .take()
            .ok_or(PmAuthenticatedLoopbackRunError::ReadIngressRoleMissing)?;
        let read_time = self
            .ready
            .read_server_time_http
            .take()
            .ok_or(PmAuthenticatedLoopbackRunError::ReadIngressRoleMissing)?;
        let private_clock = self
            .ready
            .private_read_clock
            .take()
            .ok_or(PmAuthenticatedLoopbackRunError::ReadIngressRoleMissing)?;
        let book = self
            .ready
            .book_http
            .take()
            .ok_or(PmAuthenticatedLoopbackRunError::ReadIngressRoleMissing)?;
        self.read_ingress = Some(PmAuthenticatedReadIngress::start(
            public_ws,
            user_ws,
            http,
            read_time,
            private_clock,
            book,
        ));
        Ok(())
    }

    pub(super) fn dispatch_next_read_refresh(
        &mut self,
    ) -> Result<PmHttpDispatchOutcome, PmAuthenticatedLoopbackRunError> {
        let read = self
            .read_ingress
            .as_mut()
            .ok_or(PmAuthenticatedLoopbackRunError::ReadIngressNotStarted)?;
        let ready = &mut self.ready;
        let mut actor = PmReadIngressActor::new(
            ready.coordinator.as_mut(),
            &mut ready.occurrence_issuer,
            &mut ready.actor_clock,
        );
        read.dispatch_next_refresh(&mut actor).map_err(Into::into)
    }

    pub(super) async fn service_read_ingress_once(
        &mut self,
    ) -> Result<PmReadIngressServiceOutcome, PmAuthenticatedLoopbackRunError> {
        if let Some(error) = self.read_terminal_failure.as_ref() {
            return Err(PmAuthenticatedLoopbackRunError::ReadIngressTerminal(
                Arc::clone(error),
            ));
        }
        let service = {
            let read = self
                .read_ingress
                .as_mut()
                .ok_or(PmAuthenticatedLoopbackRunError::ReadIngressNotStarted)?;
            let ready = &mut self.ready;
            let mut actor = PmReadIngressActor::new(
                ready.coordinator.as_mut(),
                &mut ready.occurrence_issuer,
                &mut ready.actor_clock,
            );
            read.service_once(&mut actor).await
        };
        match service {
            Ok(outcome) => {
                self.reject_any_goal_f_failure()?;
                Ok(outcome)
            }
            Err(error) => {
                let error = Arc::new(error);
                self.read_terminal_failure = Some(Arc::clone(&error));
                let read_error = PmAuthenticatedLoopbackRunError::ReadIngressTerminal(error);
                if let Err(primary) = self.reject_any_goal_f_failure() {
                    self.retain_terminal_secondary(read_error);
                    Err(primary)
                } else {
                    Err(read_error)
                }
            }
        }
    }

    /// Requests or polls a place-purpose `/time` task. Only a same-domain
    /// authorized proof can remove worker capacity or dequeue Goal-F.
    pub(super) fn start_place(
        &mut self,
    ) -> Result<PmMutationStartOutcome, PmAuthenticatedLoopbackRunError> {
        if self.place_task.is_some()
            || self.pending_place_completion.is_some()
            || self.retained_place_outcome.is_some()
            || self.place_bridge_pending
            || self.place_task_terminal_failure.is_some()
            || self.place_execution_failure.is_some()
        {
            return Err(PmAuthenticatedLoopbackRunError::PlaceOccupied);
        }
        self.ready.occurrence_issuer.require_known_epoch()?;
        let time = self
            .mutation_time
            .as_mut()
            .ok_or(PmAuthenticatedLoopbackRunError::MutationTimeUnavailable)?;
        let pending = match time.poll_place()? {
            PmMutationTimePoll::NotRequested => {
                time.request_place()?;
                return Ok(PmMutationStartOutcome::TimeRequested);
            }
            PmMutationTimePoll::Pending => return Ok(PmMutationStartOutcome::PendingTime),
            PmMutationTimePoll::Ready(pending) => pending,
        };
        let timestamp = self
            .ready
            .place_time_finalizer
            .authorize_loopback_place(pending)?;
        let effect_clock = self.ready.actor_clock.observe_control_edge()?;
        let task = self.ready.mutation_workers.prepare_place_task(
            &mut self.ready.coordinator,
            timestamp,
            effect_clock.received_clock().monotonic_receive_ns(),
        )?;
        self.place_task = Some(PmMutationTask::new(tokio::spawn(task.run())));
        self.place_completion_confirmed = false;
        Ok(PmMutationStartOutcome::Started)
    }

    /// Exact-cancel has its own time proof, worker slot, and task. An in-flight
    /// placement therefore cannot consume this admission authority.
    pub(super) fn start_cancel(
        &mut self,
    ) -> Result<PmMutationStartOutcome, PmAuthenticatedLoopbackRunError> {
        if self.cancel_task.is_some()
            || self.pending_cancel_completion.is_some()
            || self.retained_cancel_outcome.is_some()
            || self.cancel_bridge_pending
            || self.cancel_task_terminal_failure.is_some()
            || self.cancel_execution_failure.is_some()
        {
            return Err(PmAuthenticatedLoopbackRunError::CancelOccupied);
        }
        self.ready.occurrence_issuer.require_known_epoch()?;
        let time = self
            .mutation_time
            .as_mut()
            .ok_or(PmAuthenticatedLoopbackRunError::MutationTimeUnavailable)?;
        let pending = match time.poll_cancel()? {
            PmMutationTimePoll::NotRequested => {
                time.request_cancel()?;
                return Ok(PmMutationStartOutcome::TimeRequested);
            }
            PmMutationTimePoll::Pending => return Ok(PmMutationStartOutcome::PendingTime),
            PmMutationTimePoll::Ready(pending) => pending,
        };
        let timestamp = self
            .ready
            .cancel_time_finalizer
            .authorize_loopback_cancel(pending)?;
        let effect_clock = self.ready.actor_clock.observe_control_edge()?;
        let task = self.ready.mutation_workers.prepare_cancel_task(
            &mut self.ready.coordinator,
            timestamp,
            effect_clock.received_clock().monotonic_receive_ns(),
        )?;
        self.cancel_task = Some(PmMutationTask::new(tokio::spawn(task.run())));
        self.cancel_completion_confirmed = false;
        Ok(PmMutationStartOutcome::Started)
    }

    pub(super) fn place_task_finished(&self) -> bool {
        self.place_task
            .as_ref()
            .is_some_and(PmMutationTask::is_finished)
    }

    pub(super) fn cancel_task_finished(&self) -> bool {
        self.cancel_task
            .as_ref()
            .is_some_and(PmMutationTask::is_finished)
    }

    /// Gracefully joins the place task, restores its sole worker, admits the
    /// completion only after obtaining a borrowed coordinator permit, and
    /// drives the exact second durability barrier to confirmation.
    pub(super) async fn finish_place(
        &mut self,
    ) -> Result<PmMutationFinishOutcome, PmAuthenticatedLoopbackRunError> {
        if self.place_completion_confirmed
            && self.place_task.is_none()
            && self.pending_place_completion.is_none()
            && !self.place_bridge_pending
        {
            return Ok(PmMutationFinishOutcome::Applied);
        }
        if self.pending_place_completion.is_none() && !self.place_bridge_pending {
            let task = self
                .place_task
                .as_mut()
                .ok_or(PmAuthenticatedLoopbackRunError::NoPlaceTask)?;
            if !task.is_finished() {
                return Ok(PmMutationFinishOutcome::PendingTask);
            }
            let outcome = match task.join().await {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.place_task.take();
                    let failure = PmMutationTaskTerminalFailure::from_join_error(&error);
                    self.place_task_terminal_failure = Some(failure);
                    return Err(PmAuthenticatedLoopbackRunError::TaskJoin(failure));
                }
            };
            self.place_task.take();
            match self.ready.mutation_workers.finish_place_task(outcome) {
                PmAuthenticatedPlaceTaskFinish::Completion(completion) => {
                    self.pending_place_completion = Some(completion);
                }
                PmAuthenticatedPlaceTaskFinish::Failed(error) => {
                    let error = Arc::new(error);
                    self.place_execution_failure = Some(Arc::clone(&error));
                    return Err(PmAuthenticatedLoopbackRunError::Execution(error));
                }
                PmAuthenticatedPlaceTaskFinish::SlotOccupied(outcome) => {
                    self.retained_place_outcome = Some(outcome);
                    return Err(PmAuthenticatedLoopbackRunError::PlaceWorkerSlotOccupied);
                }
            }
        }
        self.admit_pending_place()?;
        self.advance_goal_f_bridge(true)
    }

    pub(super) async fn finish_cancel(
        &mut self,
    ) -> Result<PmMutationFinishOutcome, PmAuthenticatedLoopbackRunError> {
        if self.cancel_completion_confirmed
            && self.cancel_task.is_none()
            && self.pending_cancel_completion.is_none()
            && !self.cancel_bridge_pending
        {
            return Ok(PmMutationFinishOutcome::Applied);
        }
        if self.pending_cancel_completion.is_none() && !self.cancel_bridge_pending {
            let task = self
                .cancel_task
                .as_mut()
                .ok_or(PmAuthenticatedLoopbackRunError::NoCancelTask)?;
            if !task.is_finished() {
                return Ok(PmMutationFinishOutcome::PendingTask);
            }
            let outcome = match task.join().await {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.cancel_task.take();
                    let failure = PmMutationTaskTerminalFailure::from_join_error(&error);
                    self.cancel_task_terminal_failure = Some(failure);
                    return Err(PmAuthenticatedLoopbackRunError::TaskJoin(failure));
                }
            };
            self.cancel_task.take();
            match self.ready.mutation_workers.finish_cancel_task(outcome) {
                PmAuthenticatedCancelTaskFinish::Completion(completion) => {
                    self.pending_cancel_completion = Some(completion);
                }
                PmAuthenticatedCancelTaskFinish::Failed(error) => {
                    let error = Arc::new(error);
                    self.cancel_execution_failure = Some(Arc::clone(&error));
                    return Err(PmAuthenticatedLoopbackRunError::Execution(error));
                }
                PmAuthenticatedCancelTaskFinish::SlotOccupied(outcome) => {
                    self.retained_cancel_outcome = Some(outcome);
                    return Err(PmAuthenticatedLoopbackRunError::CancelWorkerSlotOccupied);
                }
            }
        }
        self.admit_pending_cancel()?;
        self.advance_goal_f_bridge(false)
    }

    fn admit_pending_place(&mut self) -> Result<(), PmAuthenticatedLoopbackRunError> {
        if self.place_bridge_pending {
            return Ok(());
        }
        let ready = &mut self.ready;
        let admission = ready.coordinator.reserve_authenticated_completion()?;
        let clock = ready.actor_clock.observe_control_edge()?.received_clock();
        let occurrence = ready.occurrence_issuer.issue_mutation_completion(clock)?;
        let completion = self
            .pending_place_completion
            .take()
            .expect("place admission follows a retained joined completion");
        self.place_bridge_pending = true;
        self.place_bridge_deadline =
            Some(tokio::time::Instant::now() + ready.goal_f_bridge_timeout);
        self.place_bridge_turns = 0;
        admission.admit_place(occurrence, completion)?;
        Ok(())
    }

    fn admit_pending_cancel(&mut self) -> Result<(), PmAuthenticatedLoopbackRunError> {
        if self.cancel_bridge_pending {
            return Ok(());
        }
        let ready = &mut self.ready;
        let admission = ready.coordinator.reserve_authenticated_completion()?;
        let clock = ready.actor_clock.observe_control_edge()?.received_clock();
        let occurrence = ready.occurrence_issuer.issue_mutation_completion(clock)?;
        let completion = self
            .pending_cancel_completion
            .take()
            .expect("cancel admission follows a retained joined completion");
        self.cancel_bridge_pending = true;
        self.cancel_bridge_deadline =
            Some(tokio::time::Instant::now() + ready.goal_f_bridge_timeout);
        self.cancel_bridge_turns = 0;
        admission.admit_cancel(occurrence, completion)?;
        Ok(())
    }

    fn advance_goal_f_bridge(
        &mut self,
        expected_place: bool,
    ) -> Result<PmMutationFinishOutcome, PmAuthenticatedLoopbackRunError> {
        // A writer acknowledgement may already be queued when a previous
        // service turn reached its wall-clock bound. Consume that exact proof
        // before consulting either deadline or turn count.
        self.reject_any_goal_f_failure()?;
        if self.confirm_available_bridge(expected_place)? {
            return Ok(PmMutationFinishOutcome::Applied);
        }
        let (deadline, turns) = if expected_place {
            (
                &mut self.place_bridge_deadline,
                &mut self.place_bridge_turns,
            )
        } else {
            (
                &mut self.cancel_bridge_deadline,
                &mut self.cancel_bridge_turns,
            )
        };
        let exact_deadline =
            (*deadline).ok_or(PmAuthenticatedLoopbackRunError::BridgeStateMissing)?;
        if tokio::time::Instant::now() >= exact_deadline {
            return Err(PmAuthenticatedLoopbackRunError::BridgeDurabilityTimeout);
        }
        if *turns >= MAX_AUTHENTICATED_BRIDGE_SERVICE_TURNS {
            return Err(PmAuthenticatedLoopbackRunError::BridgeServiceBoundExceeded);
        }
        *turns += 1;
        let service_clock = self
            .ready
            .actor_clock
            .observe_control_edge()?
            .received_clock();
        let service = self
            .ready
            .coordinator
            .service_turn(service_clock.monotonic_receive_ns());
        self.finish_goal_f_boundary(service)?;
        if self.confirm_available_bridge(expected_place)? {
            return Ok(PmMutationFinishOutcome::Applied);
        }
        let poll_clock = self
            .ready
            .actor_clock
            .observe_control_edge()?
            .received_clock();
        let occurrence = self
            .ready
            .occurrence_issuer
            .issue_persistence_poll(poll_clock)?;
        let poll = self
            .ready
            .coordinator
            .poll_persistence_live(occurrence, poll_clock.monotonic_receive_ns());
        let _ = self.finish_goal_f_boundary(poll)?;
        if self.confirm_available_bridge(expected_place)? {
            Ok(PmMutationFinishOutcome::Applied)
        } else {
            Ok(PmMutationFinishOutcome::PendingBridge)
        }
    }

    fn reject_any_bridge_failure(&mut self) -> Result<(), PmAuthenticatedLoopbackRunError> {
        if let Some(failure) = self.ready.coordinator.take_authenticated_bridge_failure() {
            if failure.is_place() {
                self.place_bridge_failure = Some(failure);
            } else {
                self.cancel_bridge_failure = Some(failure);
            }
            return Err(PmAuthenticatedLoopbackRunError::BridgePersistence(failure));
        }
        self.place_bridge_failure
            .or(self.cancel_bridge_failure)
            .map_or(Ok(()), |failure| {
                Err(PmAuthenticatedLoopbackRunError::BridgePersistence(failure))
            })
    }

    fn reject_any_goal_f_failure(&mut self) -> Result<(), PmAuthenticatedLoopbackRunError> {
        if let Some(failure) = self.goal_f_writer_failure {
            return Err(PmAuthenticatedLoopbackRunError::GoalFWriter(failure));
        }
        if let Some(failure) = self.ready.coordinator.take_goal_f_writer_failure() {
            self.goal_f_writer_failure = Some(failure);
            return Err(PmAuthenticatedLoopbackRunError::GoalFWriter(failure));
        }
        self.reject_any_bridge_failure()
    }

    fn finish_goal_f_boundary<T>(
        &mut self,
        result: Result<T, PmCoordinatorError>,
    ) -> Result<T, PmAuthenticatedLoopbackRunError> {
        if let Err(primary) = self.reject_any_goal_f_failure() {
            if let Err(secondary) = result {
                self.retain_terminal_secondary(secondary.into());
            }
            return Err(primary);
        }
        result.map_err(Into::into)
    }

    fn retain_terminal_secondary(&mut self, error: PmAuthenticatedLoopbackRunError) {
        if self.terminal_secondary.is_none() {
            self.terminal_secondary = Some(Box::new(error));
        }
    }

    fn confirm_available_bridge(
        &mut self,
        expected_place: bool,
    ) -> Result<bool, PmAuthenticatedLoopbackRunError> {
        let Some(applied) = self.ready.coordinator.take_authenticated_bridge_applied() else {
            return Ok(false);
        };
        let actual_place = applied.is_place();
        let completes_expected = actual_place == expected_place;
        if let Err(error) = self.ready.mutation_workers.confirm_goal_f_bridge(applied) {
            let error = Arc::new(error);
            if actual_place {
                self.place_execution_failure = Some(Arc::clone(&error));
            } else {
                self.cancel_execution_failure = Some(Arc::clone(&error));
            }
            return Err(PmAuthenticatedLoopbackRunError::Execution(error));
        }
        if actual_place {
            self.place_bridge_pending = false;
            self.place_bridge_deadline = None;
            self.place_completion_confirmed = true;
        } else {
            self.cancel_bridge_pending = false;
            self.cancel_bridge_deadline = None;
            self.cancel_completion_confirmed = true;
        }
        Ok(completes_expected)
    }

    fn place_work_outstanding(&self) -> bool {
        self.place_task.is_some()
            || self.pending_place_completion.is_some()
            || self.place_bridge_pending
    }

    fn cancel_work_outstanding(&self) -> bool {
        self.cancel_task.is_some()
            || self.pending_cancel_completion.is_some()
            || self.cancel_bridge_pending
    }

    fn cancel_time_pending(&self) -> bool {
        self.mutation_time
            .as_ref()
            .is_some_and(PmMutationTimeSupervisor::cancel_request_pending)
    }

    fn place_time_pending(&self) -> bool {
        self.mutation_time
            .as_ref()
            .is_some_and(PmMutationTimeSupervisor::place_request_pending)
    }

    fn prepared_cancel_pending(&self) -> bool {
        self.ready.coordinator.next_prepared_mutation_kind() == Some(PmPreparedMutationKind::Cancel)
    }

    fn pump_goal_f_once(&mut self) -> Result<(), PmAuthenticatedLoopbackRunError> {
        let service_clock = self
            .ready
            .actor_clock
            .observe_control_edge()?
            .received_clock();
        let service = self
            .ready
            .coordinator
            .service_turn(service_clock.monotonic_receive_ns());
        self.finish_goal_f_boundary(service)?;
        let poll_clock = self
            .ready
            .actor_clock
            .observe_control_edge()?
            .received_clock();
        let occurrence = self
            .ready
            .occurrence_issuer
            .issue_persistence_poll(poll_clock)?;
        let poll = self
            .ready
            .coordinator
            .poll_persistence_live(occurrence, poll_clock.monotonic_receive_ns());
        let _ = self.finish_goal_f_boundary(poll)?;
        self.drain_shutdown_copied_effects();
        Ok(())
    }

    /// Drains the complete bounded Goal-F path after read producers have
    /// stopped but before either durability owner is closed.
    ///
    /// A poll can move a failed write out of the mutation queue and into the
    /// persistence rank. The typed failure projection does not exist until a
    /// following service turn reduces that rank, so an empty mutation queue
    /// alone is never sufficient shutdown evidence.
    async fn drive_final_goal_f_quiescence(
        &mut self,
    ) -> Result<(), PmAuthenticatedLoopbackRunError> {
        let deadline = tokio::time::Instant::now() + self.ready.controlled_shutdown_timeout;
        let secondary_preceded_drain = self.terminal_secondary.is_some();
        let mut first_boundary_error = None;
        loop {
            if let Err(primary) = self.reject_any_goal_f_failure() {
                if let Some(secondary) = first_boundary_error.take()
                    && !secondary_preceded_drain
                {
                    // Prefer the chronologically earlier generic boundary to
                    // any generic displaced by the same typed projection.
                    self.terminal_secondary = Some(secondary);
                }
                return Err(primary);
            }
            self.drain_shutdown_copied_effects();
            if self.ready.coordinator.goal_f_shutdown_quiescent() {
                return first_boundary_error.map_or(Ok(()), |error| Err(*error));
            }

            if let Err(error) = self.pump_goal_f_once() {
                if matches!(
                    &error,
                    PmAuthenticatedLoopbackRunError::GoalFWriter(_)
                        | PmAuthenticatedLoopbackRunError::BridgePersistence(_)
                ) {
                    if let Some(secondary) = first_boundary_error.take()
                        && !secondary_preceded_drain
                    {
                        self.terminal_secondary = Some(secondary);
                    }
                    return Err(error);
                }
                if first_boundary_error.is_none() {
                    first_boundary_error = Some(Box::new(error));
                }
            }
            // `pump_goal_f_once` checks projections after both its service and
            // poll boundaries. Consult once more before allowing the wall
            // clock bound to replace an already-materialized writer failure.
            if let Err(primary) = self.reject_any_goal_f_failure() {
                if let Some(secondary) = first_boundary_error.take()
                    && !secondary_preceded_drain
                {
                    self.terminal_secondary = Some(secondary);
                }
                return Err(primary);
            }
            if tokio::time::Instant::now() >= deadline {
                let timeout = PmAuthenticatedLoopbackRunError::GoalFShutdownQuiescenceTimeout;
                if let Some(primary) = first_boundary_error {
                    self.retain_terminal_secondary(timeout);
                    return Err(*primary);
                }
                return Err(timeout);
            }
            tokio::task::yield_now().await;
        }
    }

    async fn pump_shutdown_read_once(&mut self) -> Result<(), PmAuthenticatedLoopbackRunError> {
        if self.read_terminal_failure.is_some() {
            return Ok(());
        }
        if self.read_ingress.is_some() {
            // Classify a child exit before attempting another dispatch so a
            // closed sender cannot mask the exact typed task primary.
            let _ = self.service_read_ingress_once().await?;
            match self.dispatch_next_read_refresh() {
                Ok(_) => {}
                Err(PmAuthenticatedLoopbackRunError::ReadIngressActor(
                    PmReadIngressActorError::HttpWorkerClosed,
                )) => {
                    let failure = self
                        .read_ingress
                        .as_mut()
                        .expect("read ingress remains owned during shutdown")
                        .take_http_task_exit()
                        .await;
                    let failure = Arc::new(failure);
                    self.read_terminal_failure = Some(Arc::clone(&failure));
                    return Err(PmAuthenticatedLoopbackRunError::ReadIngressTerminal(
                        failure,
                    ));
                }
                Err(error) => return Err(error),
            }
            self.drain_shutdown_copied_effects();
        }
        Ok(())
    }

    fn drain_shutdown_copied_effects(&mut self) {
        while let Some(effect) = self.ready.coordinator.pop_shutdown_copied_effect() {
            let index = match effect {
                PmProductEffect::PlaceGtcPostOnly(_) => 0,
                PmProductEffect::CancelOwned(_) => 1,
                PmProductEffect::DurableRecord(_) => 2,
                PmProductEffect::HealthMetricAudit(_) => 3,
                PmProductEffect::FailClosedHaltOrCancel(_) => 4,
                PmProductEffect::ReconciliationRefresh(_) => {
                    unreachable!("shutdown diagnostic drain cannot skip a refresh")
                }
            };
            self.shutdown_effect_counts[index] =
                self.shutdown_effect_counts[index].saturating_add(1);
        }
    }

    fn suppress_shutdown_quote(&mut self) -> Result<bool, PmAuthenticatedLoopbackRunError> {
        if self.ready.coordinator.next_prepared_mutation_kind()
            != Some(PmPreparedMutationKind::Quote)
        {
            return Ok(false);
        }
        self.ready
            .coordinator
            .suppress_place_for_controlled_shutdown()?;
        Ok(true)
    }

    /// Establishes canonical owned-order safety before any read, credential,
    /// or journal owner is stopped.
    ///
    /// The run is consumed by shutdown, so placement is disabled at this
    /// boundary. An already-started place is first joined and bridged. Only
    /// then does the sole occurrence issuer mint one shutdown control; this
    /// ensures an accepted in-flight place is visible when the canonical
    /// reducer prepares its exact-owned cancel. Durable but unsent quotes are
    /// retained without transport so they cannot block that cancel.
    async fn drive_controlled_shutdown_safety_until(
        &mut self,
        deadline: tokio::time::Instant,
    ) -> Result<(), PmAuthenticatedLoopbackRunError> {
        while self.place_work_outstanding() && self.place_bridge_failure.is_none() {
            match self.finish_place().await? {
                PmMutationFinishOutcome::PendingTask
                | PmMutationFinishOutcome::PendingBridge
                | PmMutationFinishOutcome::Applied => {}
            }
            self.pump_shutdown_read_once().await?;
            if self.place_work_outstanding() {
                self.pump_goal_f_once()?;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(self.controlled_shutdown_bound_error());
            }
            tokio::task::yield_now().await;
        }

        while self.suppress_shutdown_quote()? {
            self.shutdown_unsent_quote_retained = true;
        }

        if !self.shutdown_control_issued {
            let control_clock = self
                .ready
                .actor_clock
                .observe_control_edge()?
                .received_clock();
            let stop = self
                .ready
                .occurrence_issuer
                .issue_internal_control(control_clock)?;
            self.ready.coordinator.request_live_shutdown(stop)?;
            self.shutdown_control_issued = true;
            if let Some(read) = self.read_ingress.as_mut() {
                read.mark_controlled_shutdown_issued();
            }
        }

        loop {
            self.pump_goal_f_once()?;
            self.pump_shutdown_read_once().await?;
            while self.suppress_shutdown_quote()? {
                self.shutdown_unsent_quote_retained = true;
            }

            if self.cancel_work_outstanding() && self.cancel_bridge_failure.is_none() {
                match self.finish_cancel().await? {
                    PmMutationFinishOutcome::PendingTask
                    | PmMutationFinishOutcome::PendingBridge
                    | PmMutationFinishOutcome::Applied => {}
                }
            } else if self.prepared_cancel_pending() {
                let _ = self.start_cancel()?;
            }

            let no_dispatch_work = !self.place_work_outstanding()
                && !self.cancel_work_outstanding()
                && !self.prepared_cancel_pending();
            let fully_reconciled = self.ready.coordinator.owned_shutdown_safety_settled();
            let explicitly_unsent = self.shutdown_unsent_quote_retained
                && self.ready.coordinator.owned_shutdown_venue_safety_settled()
                && !self
                    .ready
                    .coordinator
                    .owned_shutdown_has_may_have_sent_unbound();
            if no_dispatch_work && (fully_reconciled || explicitly_unsent) {
                return if let Some(error) = self.retained_terminal_failure() {
                    Err(error)
                } else if self.shutdown_unsent_quote_retained
                    || self
                        .ready
                        .coordinator
                        .owned_shutdown_has_unbound_nonterminal()
                {
                    Err(PmAuthenticatedLoopbackRunError::UnsentPreparedQuoteRetained)
                } else {
                    Ok(())
                };
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(self.controlled_shutdown_bound_error());
            }
            tokio::task::yield_now().await;
        }
    }

    async fn drive_controlled_shutdown_safety(
        &mut self,
    ) -> Result<(), PmAuthenticatedLoopbackRunError> {
        let deadline = tokio::time::Instant::now() + self.ready.controlled_shutdown_timeout;
        self.drive_controlled_shutdown_safety_until(deadline).await
    }

    fn retained_terminal_failure(&self) -> Option<PmAuthenticatedLoopbackRunError> {
        if let Some(failure) = self.goal_f_writer_failure {
            return Some(PmAuthenticatedLoopbackRunError::GoalFWriter(failure));
        }
        if let Some(failure) = self.place_bridge_failure.or(self.cancel_bridge_failure) {
            return Some(PmAuthenticatedLoopbackRunError::BridgePersistence(failure));
        }
        if let Some(error) = self.read_terminal_failure.as_ref() {
            return Some(PmAuthenticatedLoopbackRunError::ReadIngressTerminal(
                Arc::clone(error),
            ));
        }
        if let Some(error) = self.place_execution_failure.as_ref() {
            return Some(PmAuthenticatedLoopbackRunError::Execution(Arc::clone(
                error,
            )));
        }
        if let Some(error) = self.cancel_execution_failure.as_ref() {
            return Some(PmAuthenticatedLoopbackRunError::Execution(Arc::clone(
                error,
            )));
        }
        if let Some(failure) = self
            .place_task_terminal_failure
            .or(self.cancel_task_terminal_failure)
        {
            return Some(PmAuthenticatedLoopbackRunError::TaskJoin(failure));
        }
        if self.retained_place_outcome.is_some() {
            return Some(PmAuthenticatedLoopbackRunError::PlaceWorkerSlotOccupied);
        }
        self.retained_cancel_outcome
            .as_ref()
            .map(|_| PmAuthenticatedLoopbackRunError::CancelWorkerSlotOccupied)
    }

    fn controlled_shutdown_bound_error(&self) -> PmAuthenticatedLoopbackRunError {
        self.retained_terminal_failure().unwrap_or_else(|| {
            if self.place_bridge_failure.is_some()
                || self
                    .ready
                    .coordinator
                    .owned_shutdown_has_may_have_sent_unbound()
            {
                PmAuthenticatedLoopbackRunError::MayHaveSentPlaceUnresolved
            } else {
                PmAuthenticatedLoopbackRunError::ControlledShutdownSafetyTimeout
            }
        })
    }

    async fn drain_mutation_work_after_failure(&mut self) -> PmMutationCleanupEvidence {
        let mut evidence = PmMutationCleanupEvidence::new();

        // At most a late place and the one shutdown-owned cancel can cross
        // this owner. Each stage receives a fresh composition-level bound;
        // endpoint tasks themselves retain their stricter request bounds and
        // are always joined rather than aborted.
        for stage in 0..4 {
            let deadline = tokio::time::Instant::now() + self.ready.controlled_shutdown_timeout;
            if let Err(error) = self.drive_controlled_shutdown_safety_until(deadline).await {
                evidence.resume[stage] = Some(Box::new(error));
            }

            while self.place_task.is_some() || self.cancel_task.is_some() {
                if self.place_task_finished()
                    && let Err(error) = self.finish_place().await
                {
                    evidence.place.get_or_insert(Box::new(error));
                }
                if self.cancel_task_finished()
                    && let Err(error) = self.finish_cancel().await
                {
                    evidence.cancel.get_or_insert(Box::new(error));
                }
                if self.place_task.is_some() || self.cancel_task.is_some() {
                    if let Err(error) = self.pump_shutdown_read_once().await {
                        evidence.read.get_or_insert(Box::new(error));
                    }
                    if let Err(error) = self.pump_goal_f_once() {
                        evidence.goal_f.get_or_insert(Box::new(error));
                    }
                    tokio::task::yield_now().await;
                }
            }

            if self.shutdown_safety_reached_terminal_projection() {
                break;
            }
        }

        evidence.unresolved_place_work = self.place_work_outstanding();
        evidence.unresolved_cancel_work = self.cancel_work_outstanding();
        evidence.pending_place_time = self.place_time_pending();
        evidence.pending_cancel_time = self.cancel_time_pending();
        evidence.prepared_cancel = self.prepared_cancel_pending();
        evidence.may_have_sent_unbound = self.place_bridge_failure.is_some()
            || self
                .ready
                .coordinator
                .owned_shutdown_has_may_have_sent_unbound();
        evidence.unsent_quote_retained = self.shutdown_unsent_quote_retained;
        evidence.unbound_nonterminal = self
            .ready
            .coordinator
            .owned_shutdown_has_unbound_nonterminal();
        evidence.venue_safety_unsettled =
            !self.ready.coordinator.owned_shutdown_venue_safety_settled();
        evidence
    }

    fn shutdown_safety_reached_terminal_projection(&self) -> bool {
        self.shutdown_control_issued
            && !self.place_work_outstanding()
            && !self.cancel_work_outstanding()
            && !self.prepared_cancel_pending()
            && !self.cancel_time_pending()
            && (self.ready.coordinator.owned_shutdown_safety_settled()
                || (self.shutdown_unsent_quote_retained
                    && self.ready.coordinator.owned_shutdown_venue_safety_settled()
                    && !self
                        .ready
                        .coordinator
                        .owned_shutdown_has_may_have_sent_unbound()))
    }

    /// Controlled shutdown never aborts mutation tasks. Both purpose tasks are
    /// joined and bridged before durability owners are closed, even if the
    /// first purpose reports an error.
    pub(super) async fn shutdown(
        mut self,
    ) -> Result<PmAuthenticatedLoopbackStopped, PmAuthenticatedLoopbackRunShutdownError> {
        // A failure observed before shutdown remains the chronological
        // primary even if best-effort Stop/cancel/reconciliation encounters
        // a later error. Both are retained in the aggregate.
        let retained_safety_at_entry = self.retained_terminal_failure().map(Box::new);
        let safety_drive = self
            .drive_controlled_shutdown_safety()
            .await
            .err()
            .map(Box::new);
        // A typed child/task failure can also latch during the safety drive
        // before a later actor error escapes through `?`. Consult retained
        // state again so that newly observed typed evidence remains primary.
        let retained_safety_primary =
            retained_safety_at_entry.or_else(|| self.retained_terminal_failure().map(Box::new));
        let (mut safety, safety_secondary) = match (retained_safety_primary, safety_drive) {
            (Some(primary), secondary) => (Some(primary), secondary),
            (None, primary) => (primary, None),
        };
        let mut cleanup = if safety.is_some() {
            // Endpoint transports are bounded. Continue servicing sockets
            // until every already-dispatched mutation task returns rather
            // than letting Drop abort it while reporting the safety primary.
            self.drain_mutation_work_after_failure().await
        } else {
            PmMutationCleanupEvidence::new()
        };
        let read = if let Some(read) = self.read_ingress.take() {
            let ready = &mut self.ready;
            let mut actor = PmReadIngressActor::new(
                ready.coordinator.as_mut(),
                &mut ready.occurrence_issuer,
                &mut ready.actor_clock,
            );
            read.shutdown(&mut actor, &mut self.shutdown_effect_counts)
                .await
                .err()
                .map(Box::new)
        } else {
            None
        };
        // Read shutdown performs its own final bounded actor turns. One of
        // those turns can reduce a writer failure that did not exist when the
        // safety loop last checked, so inspect the sealed projection before
        // doing anything else. Then drive the persistence queue, retained
        // admission, scheduler ranks, and copied consequences to quiescence.
        let final_goal_f = match self.reject_any_goal_f_failure() {
            Err(error) => Some(Box::new(error)),
            Ok(()) => self
                .drive_final_goal_f_quiescence()
                .await
                .err()
                .map(Box::new),
        };
        let goal_f_quiescence = if safety.is_none() {
            safety = final_goal_f;
            None
        } else {
            final_goal_f
        };
        // `finish_goal_f_boundary` can displace a generic coordinator error
        // while selecting an exact writer/bridge projection during this final
        // post-read drain. Consume the secondary only now, after the last
        // Goal-F boundary, so it cannot disappear with the Run owner.
        cleanup.boundary_secondary = self.terminal_secondary.take();
        cleanup.goal_f_unresolved_counts =
            self.ready.coordinator.goal_f_shutdown_unresolved_counts();
        let mutation_time = if let Some(time) = self.mutation_time.take() {
            time.shutdown().await.err().map(Box::new)
        } else {
            None
        };
        let owner = match self.ready.shutdown().await {
            Ok(mut stopped) if safety.is_none() && read.is_none() && mutation_time.is_none() => {
                stopped.set_shutdown_effect_counts(self.shutdown_effect_counts);
                return Ok(stopped);
            }
            owner => owner,
        };
        Err(PmAuthenticatedLoopbackRunShutdownError {
            safety,
            safety_secondary,
            goal_f_quiescence,
            read,
            mutation_time,
            owner: owner.err().map(Box::new),
            shutdown_effect_counts: self.shutdown_effect_counts,
            cleanup,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PmMutationStartOutcome {
    TimeRequested,
    PendingTime,
    Started,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PmMutationFinishOutcome {
    PendingTask,
    PendingBridge,
    Applied,
}

#[derive(Debug, Error)]
pub(super) enum PmAuthenticatedLoopbackRunError {
    #[error("authenticated read ingress is already started")]
    ReadIngressAlreadyStarted,
    #[error("authenticated read ingress has not been started")]
    ReadIngressNotStarted,
    #[error("one or more move-only read-ingress roles were already consumed")]
    ReadIngressRoleMissing,
    #[error("independent mutation-time supervisor is unavailable")]
    MutationTimeUnavailable,
    #[error("authenticated place task or bridge is already occupied")]
    PlaceOccupied,
    #[error("authenticated cancel task or bridge is already occupied")]
    CancelOccupied,
    #[error("no authenticated place task or retained completion exists")]
    NoPlaceTask,
    #[error("no authenticated cancel task or retained completion exists")]
    NoCancelTask,
    #[error("finished place worker could not return to its sole slot")]
    PlaceWorkerSlotOccupied,
    #[error("finished cancel worker could not return to its sole slot")]
    CancelWorkerSlotOccupied,
    #[error("bounded Goal-F bridge service did not reach the exact applied proof")]
    BridgeServiceBoundExceeded,
    #[error("Goal-F bridge pending state omitted its deadline")]
    BridgeStateMissing,
    #[error("Goal-F bridge durability did not complete before its configured deadline")]
    BridgeDurabilityTimeout,
    #[error(transparent)]
    BridgePersistence(PmAuthenticatedBridgeFailure),
    #[error(transparent)]
    GoalFWriter(PmGoalFWriterFailure),
    #[error(
        "Goal-F shutdown could not drain every persistence rank before its configured deadline"
    )]
    GoalFShutdownQuiescenceTimeout,
    #[error(
        "controlled shutdown could not prove exact owned-order safety before its durability deadline"
    )]
    ControlledShutdownSafetyTimeout,
    #[error(
        "controlled shutdown retained a durable prepared quote without sending or fabricating a backend result"
    )]
    UnsentPreparedQuoteRetained,
    #[error(
        "controlled shutdown retained a may-have-sent place without a reconciled venue identity"
    )]
    MayHaveSentPlaceUnresolved,
    #[error(transparent)]
    LiveAdapter(#[from] PmLiveAdapterError),
    #[error(transparent)]
    MutationTime(#[from] PmMutationTimeError),
    #[error(transparent)]
    ReadIngressActor(#[from] PmReadIngressActorError),
    #[error("authenticated read-ingress child failed terminally: {0}")]
    ReadIngressTerminal(Arc<PmReadIngressServiceError>),
    #[error(transparent)]
    Clock(#[from] PmProductClockError),
    #[error(transparent)]
    Occurrence(#[from] PmLiveOccurrenceError),
    #[error(transparent)]
    Preparation(#[from] PmAuthenticatedTaskPreparationError),
    #[error(transparent)]
    Coordinator(Box<PmCoordinatorError>),
    #[error(transparent)]
    Execution(Arc<PmAuthenticatedExecutionError>),
    #[error("authenticated mutation task failed to join: {0}")]
    TaskJoin(PmMutationTaskTerminalFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(super) enum PmMutationTaskTerminalFailure {
    #[error("task was cancelled or aborted")]
    Cancelled,
    #[error("task panicked")]
    Panicked,
    #[error("task ended without a classified join outcome")]
    Unclassified,
}

impl PmMutationTaskTerminalFailure {
    fn from_join_error(error: &tokio::task::JoinError) -> Self {
        if error.is_cancelled() {
            Self::Cancelled
        } else if error.is_panic() {
            Self::Panicked
        } else {
            Self::Unclassified
        }
    }
}

impl From<PmCoordinatorError> for PmAuthenticatedLoopbackRunError {
    fn from(error: PmCoordinatorError) -> Self {
        Self::Coordinator(Box::new(error))
    }
}

#[derive(Debug)]
struct PmMutationCleanupEvidence {
    resume: [Option<Box<PmAuthenticatedLoopbackRunError>>; 4],
    place: Option<Box<PmAuthenticatedLoopbackRunError>>,
    cancel: Option<Box<PmAuthenticatedLoopbackRunError>>,
    read: Option<Box<PmAuthenticatedLoopbackRunError>>,
    goal_f: Option<Box<PmAuthenticatedLoopbackRunError>>,
    unresolved_place_work: bool,
    unresolved_cancel_work: bool,
    pending_place_time: bool,
    pending_cancel_time: bool,
    prepared_cancel: bool,
    may_have_sent_unbound: bool,
    unsent_quote_retained: bool,
    unbound_nonterminal: bool,
    venue_safety_unsettled: bool,
    boundary_secondary: Option<Box<PmAuthenticatedLoopbackRunError>>,
    goal_f_unresolved_counts: [usize; 7],
}

impl PmMutationCleanupEvidence {
    fn new() -> Self {
        Self {
            resume: std::array::from_fn(|_| None),
            place: None,
            cancel: None,
            read: None,
            goal_f: None,
            unresolved_place_work: false,
            unresolved_cancel_work: false,
            pending_place_time: false,
            pending_cancel_time: false,
            prepared_cancel: false,
            may_have_sent_unbound: false,
            unsent_quote_retained: false,
            unbound_nonterminal: false,
            venue_safety_unsettled: false,
            boundary_secondary: None,
            goal_f_unresolved_counts: [0; 7],
        }
    }
}

#[derive(Debug, Error)]
#[error(
    "authenticated run drain failed: safety={safety:?}, safety_secondary={safety_secondary:?}, goal_f_quiescence={goal_f_quiescence:?}, cleanup={cleanup:?}, read={read:?}, mutation_time={mutation_time:?}, owner={owner:?}, shutdown_effect_counts={shutdown_effect_counts:?}"
)]
pub(super) struct PmAuthenticatedLoopbackRunShutdownError {
    safety: Option<Box<PmAuthenticatedLoopbackRunError>>,
    safety_secondary: Option<Box<PmAuthenticatedLoopbackRunError>>,
    goal_f_quiescence: Option<Box<PmAuthenticatedLoopbackRunError>>,
    cleanup: PmMutationCleanupEvidence,
    read: Option<Box<PmReadIngressShutdownError>>,
    mutation_time: Option<Box<PmMutationTimeShutdownError>>,
    owner: Option<Box<PmAuthenticatedLoopbackShutdownError>>,
    shutdown_effect_counts: [u64; 5],
}
