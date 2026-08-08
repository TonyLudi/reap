//! Owned authenticated task-capacity handshakes.
//!
//! Capacity is removed from the purpose supervisor before a Goal-F dispatch
//! is dequeued. The returned task owns the complete worker and can therefore
//! be moved into a supervised `'static` task without borrowing the product
//! actor. Once the dispatch moves, `run` directly awaits authentication,
//! durability, and transport; there is deliberately no cancellation/select
//! branch. The worker returns to the supervisor still unavailable until the
//! exact Goal-F bridge-applied proof is consumed.

use reap_pm_strategy::PmQuoteModel;
use reap_polymarket_live_adapter::PmAuthorizedMutationServerTime;
use thiserror::Error;

use super::worker::{PmAuthenticatedCancelWorker, PmAuthenticatedPlaceWorker};
use super::{PmAuthenticatedExecutionError, PmLoopbackAuthenticatedMutationWorkers};
use crate::coordinator::live_completion::{PmLiveCancelCompletion, PmLivePlaceCompletion};
use crate::coordinator::{PmCoordinator, PmCoordinatorError};
use crate::coordinator::{PmPreparedCancelDispatch, PmPreparedPlaceDispatch};
use reap_pm_state::PmOwnedCancelIntent;

impl PmLoopbackAuthenticatedMutationWorkers {
    /// Takes owned place capacity before touching the coordinator dispatch
    /// queue. If unavailable, the head quote remains owned by the coordinator
    /// and is moved to authenticated backpressure so a following cancel can
    /// reach its independent worker.
    pub(crate) fn prepare_place_task<M: PmQuoteModel>(
        &mut self,
        coordinator: &mut PmCoordinator<M>,
        timestamp: PmAuthorizedMutationServerTime,
        monotonic_effect_ns: u64,
    ) -> Result<PmAuthenticatedPlaceTask, PmAuthenticatedTaskPreparationError> {
        let Some(worker) = self.place.take() else {
            return Err(place_unavailable(coordinator));
        };
        if !worker.is_available() {
            self.place = Some(worker);
            return Err(place_unavailable(coordinator));
        }
        match coordinator.take_prepared_place_for_live(monotonic_effect_ns) {
            Ok(dispatch) => Ok(PmAuthenticatedPlaceTask {
                worker,
                dispatch,
                timestamp,
            }),
            Err(error) => {
                self.place = Some(worker);
                Err(PmAuthenticatedTaskPreparationError::Coordinator(Box::new(
                    error,
                )))
            }
        }
    }

    /// Takes exact cancel capacity before moving the durable dispatch and its
    /// owned-cancel proof from the canonical coordinator.
    pub(crate) fn prepare_cancel_task<M: PmQuoteModel>(
        &mut self,
        coordinator: &mut PmCoordinator<M>,
        timestamp: PmAuthorizedMutationServerTime,
        monotonic_effect_ns: u64,
    ) -> Result<PmAuthenticatedCancelTask, PmAuthenticatedTaskPreparationError> {
        let Some(worker) = self.cancel.take() else {
            return Err(PmAuthenticatedTaskPreparationError::WorkerUnavailable);
        };
        if !worker.is_available() {
            self.cancel = Some(worker);
            return Err(PmAuthenticatedTaskPreparationError::WorkerUnavailable);
        }
        match coordinator.take_prepared_cancel_for_live(monotonic_effect_ns) {
            Ok((dispatch, intent)) => Ok(PmAuthenticatedCancelTask {
                worker,
                dispatch,
                intent,
                timestamp,
            }),
            Err(error) => {
                self.cancel = Some(worker);
                Err(PmAuthenticatedTaskPreparationError::Coordinator(Box::new(
                    error,
                )))
            }
        }
    }

    /// Returns a finished worker to its sole slot before exposing the typed
    /// completion. A successful worker remains unavailable internally until
    /// `confirm_goal_f_bridge` consumes the exact applied proof.
    pub(crate) fn finish_place_task(
        &mut self,
        outcome: PmAuthenticatedPlaceTaskOutcome,
    ) -> PmAuthenticatedPlaceTaskFinish {
        if self.place.is_some() {
            return PmAuthenticatedPlaceTaskFinish::SlotOccupied(outcome);
        }
        let PmAuthenticatedPlaceTaskOutcome { worker, result } = outcome;
        self.place = Some(worker);
        match result {
            Ok(completion) => PmAuthenticatedPlaceTaskFinish::Completion(completion),
            Err(error) => PmAuthenticatedPlaceTaskFinish::Failed(error),
        }
    }

    pub(crate) fn finish_cancel_task(
        &mut self,
        outcome: PmAuthenticatedCancelTaskOutcome,
    ) -> PmAuthenticatedCancelTaskFinish {
        if self.cancel.is_some() {
            return PmAuthenticatedCancelTaskFinish::SlotOccupied(outcome);
        }
        let PmAuthenticatedCancelTaskOutcome { worker, result } = outcome;
        self.cancel = Some(worker);
        match result {
            Ok(completion) => PmAuthenticatedCancelTaskFinish::Completion(completion),
            Err(error) => PmAuthenticatedCancelTaskFinish::Failed(error),
        }
    }
}

fn place_unavailable<M: PmQuoteModel>(
    coordinator: &mut PmCoordinator<M>,
) -> PmAuthenticatedTaskPreparationError {
    match coordinator.quarantine_place_for_authenticated_backpressure() {
        Ok(()) => PmAuthenticatedTaskPreparationError::WorkerUnavailable,
        Err(coordinator) => PmAuthenticatedTaskPreparationError::Backpressure {
            primary: Box::new(PmAuthenticatedExecutionError::WorkerUnavailable),
            coordinator: Box::new(coordinator),
        },
    }
}

/// Place task owns the sole worker capacity plus one exact durable dispatch.
/// It is `Send + 'static`; the product supervisor must join it, never abort it.
pub(crate) struct PmAuthenticatedPlaceTask {
    worker: PmAuthenticatedPlaceWorker,
    dispatch: PmPreparedPlaceDispatch,
    timestamp: PmAuthorizedMutationServerTime,
}

impl PmAuthenticatedPlaceTask {
    pub(crate) async fn run(mut self) -> PmAuthenticatedPlaceTaskOutcome {
        let result = self.worker.run_task(self.dispatch, self.timestamp).await;
        PmAuthenticatedPlaceTaskOutcome {
            worker: self.worker,
            result,
        }
    }
}

pub(crate) struct PmAuthenticatedCancelTask {
    worker: PmAuthenticatedCancelWorker,
    dispatch: PmPreparedCancelDispatch,
    intent: PmOwnedCancelIntent,
    timestamp: PmAuthorizedMutationServerTime,
}

impl PmAuthenticatedCancelTask {
    pub(crate) async fn run(mut self) -> PmAuthenticatedCancelTaskOutcome {
        let result = self
            .worker
            .run_task(self.dispatch, self.intent, self.timestamp)
            .await;
        PmAuthenticatedCancelTaskOutcome {
            worker: self.worker,
            result,
        }
    }
}

#[must_use = "a finished place worker and its result must return to the sole supervisor"]
pub(crate) struct PmAuthenticatedPlaceTaskOutcome {
    worker: PmAuthenticatedPlaceWorker,
    result: Result<PmLivePlaceCompletion, PmAuthenticatedExecutionError>,
}

#[must_use = "a finished cancel worker and its result must return to the sole supervisor"]
pub(crate) struct PmAuthenticatedCancelTaskOutcome {
    worker: PmAuthenticatedCancelWorker,
    result: Result<PmLiveCancelCompletion, PmAuthenticatedExecutionError>,
}

#[must_use = "completion or retained task authority must be handled"]
#[allow(
    clippy::large_enum_variant,
    reason = "the finish handoff retains the exact move-only worker outcome inline until the sole supervisor consumes it"
)]
pub(crate) enum PmAuthenticatedPlaceTaskFinish {
    Completion(PmLivePlaceCompletion),
    Failed(PmAuthenticatedExecutionError),
    SlotOccupied(PmAuthenticatedPlaceTaskOutcome),
}

#[must_use = "completion or retained task authority must be handled"]
#[allow(
    clippy::large_enum_variant,
    reason = "the finish handoff retains the exact move-only worker outcome inline until the sole supervisor consumes it"
)]
pub(crate) enum PmAuthenticatedCancelTaskFinish {
    Completion(PmLiveCancelCompletion),
    Failed(PmAuthenticatedExecutionError),
    SlotOccupied(PmAuthenticatedCancelTaskOutcome),
}

#[derive(Debug, Error)]
pub(crate) enum PmAuthenticatedTaskPreparationError {
    #[error("authenticated purpose worker is unavailable or already in flight")]
    WorkerUnavailable,
    #[error("coordinator could not yield the exact prepared dispatch: {0}")]
    Coordinator(Box<PmCoordinatorError>),
    #[error("{primary}; retaining the head quote under backpressure also failed: {coordinator}")]
    Backpressure {
        primary: Box<PmAuthenticatedExecutionError>,
        coordinator: Box<PmCoordinatorError>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_static<T: Send + 'static>() {}

    #[test]
    fn owned_tasks_and_outcomes_are_send_static_without_borrowed_permits() {
        assert_send_static::<PmAuthenticatedPlaceTask>();
        assert_send_static::<PmAuthenticatedCancelTask>();
        assert_send_static::<PmAuthenticatedPlaceTaskOutcome>();
        assert_send_static::<PmAuthenticatedCancelTaskOutcome>();

        let source = include_str!("task.rs")
            .split_once("\n#[cfg(test)]\nmod tests")
            .expect("task source retains one unit-test boundary")
            .0;
        assert!(!source.contains("Permit<'"));
        assert!(!source.contains("tokio::select!"));
        assert!(source.contains("let Some(worker) = self.place.take()"));
        assert!(source.contains("coordinator.take_prepared_place_for_live"));
        assert!(source.contains("let Some(worker) = self.cancel.take()"));
        assert!(source.contains("coordinator.take_prepared_cancel_for_live"));
    }
}
