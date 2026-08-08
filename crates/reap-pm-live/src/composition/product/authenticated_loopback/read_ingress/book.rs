//! Off-actor native REST `/book` seed and resynchronization worker.

use reap_polymarket_live_adapter::{PmPublicHttpRole, PmRestBookDeliveryError, PmRestBookPurpose};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

use super::channel::{PmReadIngressSinkError, PmRestBookIngressSink};

const PUBLIC_BOOK_COMMAND_CAPACITY: usize = 1;

pub(super) type PmBookTaskOutput = Result<(), PmBookTaskError>;
pub(super) type PmBookRequestResult = Result<(), PmRestBookDeliveryError<PmReadIngressSinkError>>;

pub(super) struct PmBookRequestSender {
    sender: mpsc::Sender<PmBookCommand>,
}

impl PmBookRequestSender {
    pub(super) fn try_request(
        &self,
        purpose: PmRestBookPurpose,
    ) -> Result<oneshot::Receiver<PmBookRequestResult>, PmBookDispatchError> {
        let (completion, response) = oneshot::channel();
        self.sender
            .try_send(PmBookCommand::Fetch {
                purpose,
                completion,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => PmBookDispatchError::Full,
                mpsc::error::TrySendError::Closed(_) => PmBookDispatchError::Closed,
            })?;
        Ok(response)
    }

    pub(super) fn try_shutdown(&self) -> Result<oneshot::Receiver<()>, PmBookDispatchError> {
        let (acknowledgement, receipt) = oneshot::channel();
        self.sender
            .try_send(PmBookCommand::Shutdown { acknowledgement })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => PmBookDispatchError::Full,
                mpsc::error::TrySendError::Closed(_) => PmBookDispatchError::Closed,
            })?;
        Ok(receipt)
    }
}

pub(super) struct PmBookRequestReceiver {
    receiver: mpsc::Receiver<PmBookCommand>,
}

pub(super) fn pm_book_request_channel() -> (PmBookRequestSender, PmBookRequestReceiver) {
    let (sender, receiver) = mpsc::channel(PUBLIC_BOOK_COMMAND_CAPACITY);
    (
        PmBookRequestSender { sender },
        PmBookRequestReceiver { receiver },
    )
}

pub(super) async fn run_book_worker(
    role: PmPublicHttpRole,
    mut sink: PmRestBookIngressSink,
    mut commands: PmBookRequestReceiver,
) -> PmBookTaskOutput {
    while let Some(command) = commands.receiver.recv().await {
        match command {
            PmBookCommand::Fetch {
                purpose,
                completion,
            } => {
                let result = match purpose {
                    PmRestBookPurpose::Seed => role.seed_book(&mut sink).await,
                    PmRestBookPurpose::Resync => role.resync_book(&mut sink).await,
                };
                let _ = completion.send(result);
            }
            PmBookCommand::Shutdown { acknowledgement } => {
                let _ = acknowledgement.send(());
                return Ok(());
            }
        }
    }
    Err(PmBookTaskError::CommandChannelClosed)
}

enum PmBookCommand {
    Fetch {
        purpose: PmRestBookPurpose,
        completion: oneshot::Sender<PmBookRequestResult>,
    },
    Shutdown {
        acknowledgement: oneshot::Sender<()>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(super) enum PmBookDispatchError {
    #[error("public REST-book command channel is at fixed capacity")]
    Full,
    #[error("public REST-book command channel is closed")]
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(in crate::composition::product::authenticated_loopback) enum PmBookTaskError {
    #[error("public REST-book command authority dropped without controlled shutdown")]
    CommandChannelClosed,
}
