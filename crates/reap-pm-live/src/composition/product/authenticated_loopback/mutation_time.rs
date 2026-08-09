//! Off-actor `/time` workers for independent place and exact-cancel grants.

use reap_polymarket_live_adapter::{
    PmCancelMutationTimeProof, PmCancelServerTimeHttpRole, PmLiveAdapterError,
    PmPlaceMutationTimeProof, PmPlaceServerTimeHttpRole,
};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

use super::supervision::PmAbortableTask;

const MUTATION_TIME_COMMAND_CAPACITY: usize = 1;

enum PmMutationTimeCommand<P> {
    Fetch {
        completion: oneshot::Sender<Result<P, PmLiveAdapterError>>,
    },
    Shutdown {
        acknowledgement: oneshot::Sender<()>,
    },
}

#[async_trait::async_trait]
trait PmMutationTimeSource: Send + 'static {
    type Proof: Send + 'static;

    async fn fresh_time(&self) -> Result<Self::Proof, PmLiveAdapterError>;
}

#[async_trait::async_trait]
impl PmMutationTimeSource for PmPlaceServerTimeHttpRole {
    type Proof = PmPlaceMutationTimeProof;

    async fn fresh_time(&self) -> Result<Self::Proof, PmLiveAdapterError> {
        self.fresh_place_time().await
    }
}

#[async_trait::async_trait]
impl PmMutationTimeSource for PmCancelServerTimeHttpRole {
    type Proof = PmCancelMutationTimeProof;

    async fn fresh_time(&self) -> Result<Self::Proof, PmLiveAdapterError> {
        self.fresh_cancel_time().await
    }
}

async fn run_time_worker<R>(
    role: R,
    mut commands: mpsc::Receiver<PmMutationTimeCommand<R::Proof>>,
) -> Result<(), PmMutationTimeTaskError>
where
    R: PmMutationTimeSource,
{
    while let Some(command) = commands.recv().await {
        match command {
            PmMutationTimeCommand::Fetch { completion } => {
                let _ = completion.send(role.fresh_time().await);
            }
            PmMutationTimeCommand::Shutdown { acknowledgement } => {
                let _ = acknowledgement.send(());
                return Ok(());
            }
        }
    }
    Err(PmMutationTimeTaskError::CommandChannelClosed)
}

struct PmMutationTimeEndpoint<P> {
    sender: mpsc::Sender<PmMutationTimeCommand<P>>,
    pending: Option<oneshot::Receiver<Result<P, PmLiveAdapterError>>>,
    task: Option<PmAbortableTask<Result<(), PmMutationTimeTaskError>>>,
}

impl<P: Send + 'static> PmMutationTimeEndpoint<P> {
    fn start<R>(role: R) -> Self
    where
        R: PmMutationTimeSource<Proof = P>,
    {
        let (sender, receiver) = mpsc::channel(MUTATION_TIME_COMMAND_CAPACITY);
        Self {
            sender,
            pending: None,
            task: Some(PmAbortableTask::new(tokio::spawn(run_time_worker(
                role, receiver,
            )))),
        }
    }

    fn request(&mut self) -> Result<(), PmMutationTimeError> {
        if self.pending.is_some() {
            return Err(PmMutationTimeError::RequestOccupied);
        }
        let (completion, response) = oneshot::channel();
        self.sender
            .try_send(PmMutationTimeCommand::Fetch { completion })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => PmMutationTimeError::ChannelFull,
                mpsc::error::TrySendError::Closed(_) => PmMutationTimeError::ChannelClosed,
            })?;
        self.pending = Some(response);
        Ok(())
    }

    fn poll(&mut self) -> Result<PmMutationTimePoll<P>, PmMutationTimeError> {
        let Some(response) = self.pending.as_mut() else {
            return Ok(PmMutationTimePoll::NotRequested);
        };
        match response.try_recv() {
            Ok(result) => {
                self.pending = None;
                result
                    .map(PmMutationTimePoll::Ready)
                    .map_err(PmMutationTimeError::Http)
            }
            Err(oneshot::error::TryRecvError::Empty) => Ok(PmMutationTimePoll::Pending),
            Err(oneshot::error::TryRecvError::Closed) => {
                self.pending = None;
                Err(PmMutationTimeError::ResponseClosed)
            }
        }
    }

    const fn request_pending(&self) -> bool {
        self.pending.is_some()
    }

    async fn shutdown(mut self) -> Result<(), PmMutationTimeEndpointShutdownError> {
        let (acknowledgement, receipt) = oneshot::channel();
        let signal = self
            .sender
            .send(PmMutationTimeCommand::Shutdown { acknowledgement })
            .await
            .map_err(|_| PmMutationTimeSignalFailure::ChannelClosed)
            .err();
        let receipt = if signal.is_none() {
            receipt
                .await
                .map_err(|_| PmMutationTimeSignalFailure::ReceiptClosed)
                .err()
        } else {
            None
        };
        let task = match self.task.take() {
            Some(task) => match task.join().await {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(PmMutationTimeTaskExitFailure::Task(error)),
                Err(error) if error.is_cancelled() => {
                    Some(PmMutationTimeTaskExitFailure::Cancelled)
                }
                Err(error) if error.is_panic() => Some(PmMutationTimeTaskExitFailure::Panicked),
                Err(_) => Some(PmMutationTimeTaskExitFailure::JoinFailed),
            },
            None => Some(PmMutationTimeTaskExitFailure::MissingTask),
        };
        let pending = match self.pending.take() {
            Some(mut response) => match response.try_recv() {
                Ok(Ok(_proof)) => Some(PmPendingMutationTimeShutdownEvidence::ReadyUnconsumed),
                Ok(Err(error)) => {
                    Some(PmPendingMutationTimeShutdownEvidence::Http(Box::new(error)))
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    Some(PmPendingMutationTimeShutdownEvidence::NotCompleted)
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    Some(PmPendingMutationTimeShutdownEvidence::ResponseClosed)
                }
            },
            None => None,
        };
        if signal.is_none() && receipt.is_none() && task.is_none() && pending.is_none() {
            Ok(())
        } else {
            Err(PmMutationTimeEndpointShutdownError {
                pending,
                signal,
                receipt,
                task,
            })
        }
    }
}

pub(super) struct PmMutationTimeSupervisor {
    place: PmMutationTimeEndpoint<PmPlaceMutationTimeProof>,
    cancel: PmMutationTimeEndpoint<PmCancelMutationTimeProof>,
}

impl PmMutationTimeSupervisor {
    pub(super) fn start(
        place: PmPlaceServerTimeHttpRole,
        cancel: PmCancelServerTimeHttpRole,
    ) -> Self {
        Self {
            place: PmMutationTimeEndpoint::start(place),
            cancel: PmMutationTimeEndpoint::start(cancel),
        }
    }

    pub(super) fn request_place(&mut self) -> Result<(), PmMutationTimeError> {
        self.place.request()
    }

    pub(super) fn request_cancel(&mut self) -> Result<(), PmMutationTimeError> {
        self.cancel.request()
    }

    pub(super) fn poll_place(
        &mut self,
    ) -> Result<PmMutationTimePoll<PmPlaceMutationTimeProof>, PmMutationTimeError> {
        self.place.poll()
    }

    pub(super) fn poll_cancel(
        &mut self,
    ) -> Result<PmMutationTimePoll<PmCancelMutationTimeProof>, PmMutationTimeError> {
        self.cancel.poll()
    }

    pub(super) const fn place_request_pending(&self) -> bool {
        self.place.request_pending()
    }

    pub(super) const fn cancel_request_pending(&self) -> bool {
        self.cancel.request_pending()
    }

    pub(super) async fn shutdown(self) -> Result<(), PmMutationTimeShutdownError> {
        let (place, cancel) = tokio::join!(self.place.shutdown(), self.cancel.shutdown());
        match (place, cancel) {
            (Ok(()), Ok(())) => Ok(()),
            (place, cancel) => Err(PmMutationTimeShutdownError {
                place: place.err(),
                cancel: cancel.err(),
            }),
        }
    }
}

pub(super) enum PmMutationTimePoll<P> {
    NotRequested,
    Pending,
    Ready(P),
}

#[derive(Debug, Error)]
pub(super) enum PmMutationTimeError {
    #[error("mutation-time request is already outstanding")]
    RequestOccupied,
    #[error("mutation-time request channel is at its fixed capacity")]
    ChannelFull,
    #[error("mutation-time request channel is closed")]
    ChannelClosed,
    #[error("mutation-time response channel closed without a result")]
    ResponseClosed,
    #[error(transparent)]
    Http(#[from] PmLiveAdapterError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(super) enum PmMutationTimeTaskError {
    #[error("mutation-time command authority dropped without controlled shutdown")]
    CommandChannelClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(super) enum PmMutationTimeSignalFailure {
    #[error("mutation-time command channel closed before shutdown")]
    ChannelClosed,
    #[error("mutation-time shutdown acknowledgement channel closed")]
    ReceiptClosed,
}

#[derive(Debug, Error)]
pub(super) enum PmPendingMutationTimeShutdownEvidence {
    #[error("a fresh mutation-time proof completed but was not consumed")]
    ReadyUnconsumed,
    #[error("the pending mutation-time request failed: {0}")]
    Http(Box<PmLiveAdapterError>),
    #[error("the pending mutation-time request did not complete before its worker exited")]
    NotCompleted,
    #[error("the pending mutation-time response channel closed without a result")]
    ResponseClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(super) enum PmMutationTimeTaskExitFailure {
    #[error("mutation-time worker task owner was missing")]
    MissingTask,
    #[error("mutation-time worker task was cancelled")]
    Cancelled,
    #[error("mutation-time worker task panicked")]
    Panicked,
    #[error("mutation-time worker task join failed")]
    JoinFailed,
    #[error(transparent)]
    Task(#[from] PmMutationTimeTaskError),
}

#[derive(Debug, Error)]
#[error(
    "mutation-time endpoint shutdown failed: pending={pending:?}, signal={signal:?}, receipt={receipt:?}, task={task:?}"
)]
pub(super) struct PmMutationTimeEndpointShutdownError {
    pending: Option<PmPendingMutationTimeShutdownEvidence>,
    signal: Option<PmMutationTimeSignalFailure>,
    receipt: Option<PmMutationTimeSignalFailure>,
    task: Option<PmMutationTimeTaskExitFailure>,
}

#[derive(Debug, Error)]
#[error("mutation-time shutdown failed: place={place:?}, cancel={cancel:?}")]
pub(super) struct PmMutationTimeShutdownError {
    place: Option<PmMutationTimeEndpointShutdownError>,
    cancel: Option<PmMutationTimeEndpointShutdownError>,
}
