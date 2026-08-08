//! Dedicated authenticated-HTTP read worker.
//!
//! The product actor can only submit one of the four fixed read purposes and
//! poll its purpose-typed, one-shot result. The worker owns every capability
//! that can await network I/O, finishes pagination internally, and never
//! exposes a raw response or an incomplete assembly.

use reap_pm_core::{PmFillQueryCursor, PmVenueOrderKey};
use reap_polymarket_auth::FixedOrderId;
use reap_polymarket_live_adapter::{
    PmAccountBalanceAllowance, PmAuthenticatedHttpOwner, PmCompleteOpenOrdersCut,
    PmCompleteTradesCut, PmExactOrderObservation, PmLiveAdapterError, PmOpenOrdersCutProgress,
    PmPrivateReadEdgeClock, PmPrivateReadProductClock, PmProductClockError,
    PmReadServerTimeHttpRole, PmTradesCutProgress,
};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

use crate::private_monitor::PmLiveHttpQueryFailure;

/// At most four fixed-purpose requests can wait behind the request currently
/// owned by the HTTP worker.
const AUTHENTICATED_HTTP_COMMAND_CAPACITY: usize = 4;

pub(super) type PmAuthenticatedHttpResponse<T> =
    oneshot::Receiver<PmAuthenticatedHttpReadResult<T>>;
pub(super) type PmAuthenticatedHttpShutdownReceipt = oneshot::Receiver<()>;
pub(super) type PmHttpTaskOutput = Result<(), PmAuthenticatedHttpTaskError>;

/// Complete account evidence. Neither half can cross the worker boundary by
/// itself.
#[derive(Debug)]
pub(super) struct PmCompleteAccountRead {
    collateral: PmAccountBalanceAllowance,
    conditional: PmAccountBalanceAllowance,
}

impl PmCompleteAccountRead {
    pub(super) fn into_parts(self) -> (PmAccountBalanceAllowance, PmAccountBalanceAllowance) {
        (self.collateral, self.conditional)
    }
}

/// Complete exact-order evidence retains the coordinator-selected identity.
#[derive(Debug)]
pub(super) struct PmCompleteOrderDetailRead {
    requested_order: PmVenueOrderKey,
    observation: PmExactOrderObservation,
}

impl PmCompleteOrderDetailRead {
    pub(super) const fn requested_order(&self) -> PmVenueOrderKey {
        self.requested_order
    }

    pub(super) fn into_parts(self) -> (PmVenueOrderKey, PmExactOrderObservation) {
        (self.requested_order, self.observation)
    }
}

/// Complete account-plus-fill evidence retains the canonical watermark used
/// to request this exact cut.
#[derive(Debug)]
pub(super) struct PmCompleteReconciliationRead {
    collateral: PmAccountBalanceAllowance,
    conditional: PmAccountBalanceAllowance,
    requested_after: Option<PmFillQueryCursor>,
    trades: PmCompleteTradesCut,
}

impl PmCompleteReconciliationRead {
    #[allow(
        clippy::type_complexity,
        reason = "the atomic reconciliation cut deliberately exposes all four typed parts together"
    )]
    pub(super) fn into_parts(
        self,
    ) -> (
        PmAccountBalanceAllowance,
        PmAccountBalanceAllowance,
        Option<PmFillQueryCursor>,
        PmCompleteTradesCut,
    ) {
        (
            self.collateral,
            self.conditional,
            self.requested_after,
            self.trades,
        )
    }
}

/// The only successful result form. `completed_at` is sampled by the worker
/// immediately after its final authenticated response is parsed.
#[derive(Debug)]
pub(super) struct PmAuthenticatedHttpCompletion<T> {
    completed_at: PmPrivateReadEdgeClock,
    value: T,
}

impl<T> PmAuthenticatedHttpCompletion<T> {
    pub(super) fn into_parts(self) -> (PmPrivateReadEdgeClock, T) {
        (self.completed_at, self.value)
    }
}

/// A bounded result: complete typed evidence, a recoverable dependency
/// invalidation, or a terminal supervisor contradiction. No adapter error,
/// URL, response body, partial page, or credential material crosses here.
#[derive(Debug)]
pub(super) enum PmAuthenticatedHttpReadResult<T> {
    Complete(PmAuthenticatedHttpCompletion<T>),
    Recoverable {
        completed_at: PmPrivateReadEdgeClock,
        failure: PmLiveHttpQueryFailure,
    },
    Terminal {
        completed_at: PmPrivateReadEdgeClock,
        failure: PmAuthenticatedHttpTerminalFailure,
    },
    CompletionClockFailed(PmProductClockError),
}

/// Failures which invalidate the owned HTTP supervisor rather than merely
/// invalidating one private dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::composition::product::authenticated_loopback) enum PmAuthenticatedHttpTerminalFailure
{
    CredentialAuthorityClosed,
    CredentialAuthorityTaskFailed,
    ScopeMismatch,
    ContractViolation,
}

/// Move-only actor-side dispatch authority. It deliberately has no async send
/// method, so the product actor cannot wait behind endpoint I/O or backpressure.
pub(super) struct PmHttpReadSender {
    commands: mpsc::Sender<PmAuthenticatedHttpCommand>,
}

impl PmHttpReadSender {
    /// Reserves bounded worker capacity before the actor pops a coordinator
    /// effect or mints its matching occurrence ticket. Sending through the
    /// returned move-only permit is then infallible.
    pub(super) fn try_reserve(
        &self,
    ) -> Result<PmHttpReadPermit<'_>, PmAuthenticatedHttpDispatchError> {
        self.commands
            .try_reserve()
            .map(|permit| PmHttpReadPermit { permit })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(()) => PmAuthenticatedHttpDispatchError::Full,
                mpsc::error::TrySendError::Closed(()) => PmAuthenticatedHttpDispatchError::Closed,
            })
    }

    pub(super) fn try_request_shutdown(
        &self,
    ) -> Result<PmAuthenticatedHttpShutdownReceipt, PmAuthenticatedHttpDispatchError> {
        let (acknowledgement, receipt) = oneshot::channel();
        self.try_dispatch(PmAuthenticatedHttpCommand::Shutdown { acknowledgement })?;
        Ok(receipt)
    }

    fn try_dispatch(
        &self,
        command: PmAuthenticatedHttpCommand,
    ) -> Result<(), PmAuthenticatedHttpDispatchError> {
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => PmAuthenticatedHttpDispatchError::Full,
                mpsc::error::TrySendError::Closed(_) => PmAuthenticatedHttpDispatchError::Closed,
            })
    }
}

/// One already-reserved worker queue slot. Each purpose-specific send consumes
/// the permit and returns the correspondingly typed response receiver.
pub(super) struct PmHttpReadPermit<'a> {
    permit: mpsc::Permit<'a, PmAuthenticatedHttpCommand>,
}

impl PmHttpReadPermit<'_> {
    pub(super) fn send_open_orders(self) -> PmAuthenticatedHttpResponse<PmCompleteOpenOrdersCut> {
        let (completion, response) = oneshot::channel();
        self.permit
            .send(PmAuthenticatedHttpCommand::OpenOrders { completion });
        response
    }

    pub(super) fn send_account(self) -> PmAuthenticatedHttpResponse<PmCompleteAccountRead> {
        let (completion, response) = oneshot::channel();
        self.permit
            .send(PmAuthenticatedHttpCommand::Account { completion });
        response
    }

    pub(super) fn send_order_detail(
        self,
        requested_order: PmVenueOrderKey,
    ) -> PmAuthenticatedHttpResponse<PmCompleteOrderDetailRead> {
        let (completion, response) = oneshot::channel();
        self.permit.send(PmAuthenticatedHttpCommand::OrderDetail {
            requested_order,
            completion,
        });
        response
    }

    pub(super) fn send_reconciliation(
        self,
        requested_after: Option<PmFillQueryCursor>,
    ) -> PmAuthenticatedHttpResponse<PmCompleteReconciliationRead> {
        let (completion, response) = oneshot::channel();
        self.permit
            .send(PmAuthenticatedHttpCommand::Reconciliation {
                requested_after,
                completion,
            });
        response
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(super) enum PmAuthenticatedHttpDispatchError {
    #[error("authenticated HTTP command channel is at its fixed capacity")]
    Full,
    #[error("authenticated HTTP worker command channel is closed")]
    Closed,
}

/// Move-only worker-side endpoint of the fixed-capacity request channel.
pub(super) struct PmHttpReadReceiver {
    commands: mpsc::Receiver<PmAuthenticatedHttpCommand>,
}

/// Constructs the sole actor dispatch authority and the move-only worker
/// receiver. The literal capacity is intentionally visible to static policy.
pub(super) fn pm_http_read_channel() -> (PmHttpReadSender, PmHttpReadReceiver) {
    let (commands, receiver) = mpsc::channel(AUTHENTICATED_HTTP_COMMAND_CAPACITY);
    (
        PmHttpReadSender { commands },
        PmHttpReadReceiver { commands: receiver },
    )
}

/// Runs exactly one sequential owner loop. Every endpoint await remains in
/// this task, never in the actor or a shared transport wrapper.
pub(super) async fn run_http_worker(
    mut http: PmAuthenticatedHttpOwner,
    server_time: PmReadServerTimeHttpRole,
    mut completion_clock: PmPrivateReadProductClock,
    mut receiver: PmHttpReadReceiver,
) -> PmHttpTaskOutput {
    while let Some(command) = receiver.commands.recv().await {
        match command {
            PmAuthenticatedHttpCommand::OpenOrders { completion } => {
                let result = read_open_orders(&mut http, &server_time).await;
                let _ = completion.send(stamp_result(&mut completion_clock, result));
            }
            PmAuthenticatedHttpCommand::Account { completion } => {
                let result = read_account(&mut http, &server_time).await;
                let _ = completion.send(stamp_result(&mut completion_clock, result));
            }
            PmAuthenticatedHttpCommand::OrderDetail {
                requested_order,
                completion,
            } => {
                let result = read_order_detail(&mut http, &server_time, requested_order).await;
                let _ = completion.send(stamp_result(&mut completion_clock, result));
            }
            PmAuthenticatedHttpCommand::Reconciliation {
                requested_after,
                completion,
            } => {
                let result = read_reconciliation(&mut http, &server_time, requested_after).await;
                let _ = completion.send(stamp_result(&mut completion_clock, result));
            }
            PmAuthenticatedHttpCommand::Shutdown { acknowledgement } => {
                let _ = acknowledgement.send(());
                return Ok(());
            }
        }
    }
    Err(PmAuthenticatedHttpTaskError::CommandChannelClosed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(in crate::composition::product::authenticated_loopback) enum PmAuthenticatedHttpTaskError {
    #[error("authenticated HTTP actor dispatch authority dropped without a shutdown command")]
    CommandChannelClosed,
}

enum PmAuthenticatedHttpCommand {
    OpenOrders {
        completion: oneshot::Sender<PmAuthenticatedHttpReadResult<PmCompleteOpenOrdersCut>>,
    },
    Account {
        completion: oneshot::Sender<PmAuthenticatedHttpReadResult<PmCompleteAccountRead>>,
    },
    OrderDetail {
        requested_order: PmVenueOrderKey,
        completion: oneshot::Sender<PmAuthenticatedHttpReadResult<PmCompleteOrderDetailRead>>,
    },
    Reconciliation {
        requested_after: Option<PmFillQueryCursor>,
        completion: oneshot::Sender<PmAuthenticatedHttpReadResult<PmCompleteReconciliationRead>>,
    },
    Shutdown {
        acknowledgement: oneshot::Sender<()>,
    },
}

async fn read_open_orders(
    http: &mut PmAuthenticatedHttpOwner,
    server_time: &PmReadServerTimeHttpRole,
) -> Result<PmCompleteOpenOrdersCut, PmReadOperationFailure> {
    let first_time = server_time.fresh_read_server_time().await?;
    let mut progress = http.reconciliation().begin_open_orders(first_time).await?;
    loop {
        match progress {
            PmOpenOrdersCutProgress::Complete(cut) => return Ok(cut),
            PmOpenOrdersCutProgress::Incomplete(assembly) => {
                let next_time = server_time.fresh_read_server_time().await?;
                progress = http
                    .reconciliation()
                    .continue_open_orders(next_time, assembly)
                    .await?;
            }
        }
    }
}

async fn read_account(
    http: &mut PmAuthenticatedHttpOwner,
    server_time: &PmReadServerTimeHttpRole,
) -> Result<PmCompleteAccountRead, PmReadOperationFailure> {
    let collateral_time = server_time.fresh_read_server_time().await?;
    let collateral = http
        .account()
        .collateral_balance_allowance(collateral_time)
        .await?;
    let conditional_time = server_time.fresh_read_server_time().await?;
    let conditional = http
        .account()
        .conditional_balance_allowance(conditional_time)
        .await?;
    Ok(PmCompleteAccountRead {
        collateral,
        conditional,
    })
}

async fn read_order_detail(
    http: &mut PmAuthenticatedHttpOwner,
    server_time: &PmReadServerTimeHttpRole,
    requested_order: PmVenueOrderKey,
) -> Result<PmCompleteOrderDetailRead, PmReadOperationFailure> {
    let order_id = FixedOrderId::parse(requested_order.id().as_str()).map_err(|_| {
        PmReadOperationFailure::Terminal(PmAuthenticatedHttpTerminalFailure::ContractViolation)
    })?;
    let read_time = server_time.fresh_read_server_time().await?;
    let observation = http
        .reconciliation()
        .exact_local_order_detail(read_time, order_id)
        .await?;
    Ok(PmCompleteOrderDetailRead {
        requested_order,
        observation,
    })
}

async fn read_reconciliation(
    http: &mut PmAuthenticatedHttpOwner,
    server_time: &PmReadServerTimeHttpRole,
    requested_after: Option<PmFillQueryCursor>,
) -> Result<PmCompleteReconciliationRead, PmReadOperationFailure> {
    let collateral_time = server_time.fresh_read_server_time().await?;
    let collateral = http
        .account()
        .collateral_balance_allowance(collateral_time)
        .await?;
    let conditional_time = server_time.fresh_read_server_time().await?;
    let conditional = http
        .account()
        .conditional_balance_allowance(conditional_time)
        .await?;

    let first_time = server_time.fresh_read_server_time().await?;
    let mut progress = http.reconciliation().begin_trades(first_time).await?;
    let trades = loop {
        match progress {
            PmTradesCutProgress::Complete(cut) => break cut,
            PmTradesCutProgress::Incomplete(assembly) => {
                let next_time = server_time.fresh_read_server_time().await?;
                progress = http
                    .reconciliation()
                    .continue_trades(next_time, assembly)
                    .await?;
            }
        }
    };

    Ok(PmCompleteReconciliationRead {
        collateral,
        conditional,
        requested_after,
        trades,
    })
}

fn stamp_result<T>(
    completion_clock: &mut PmPrivateReadProductClock,
    result: Result<T, PmReadOperationFailure>,
) -> PmAuthenticatedHttpReadResult<T> {
    let completed_at = match completion_clock.observe_authenticated_read_complete() {
        Ok(completed_at) => completed_at,
        Err(error) => return PmAuthenticatedHttpReadResult::CompletionClockFailed(error),
    };
    match result {
        Ok(value) => PmAuthenticatedHttpReadResult::Complete(PmAuthenticatedHttpCompletion {
            completed_at,
            value,
        }),
        Err(PmReadOperationFailure::Recoverable(failure)) => {
            PmAuthenticatedHttpReadResult::Recoverable {
                completed_at,
                failure,
            }
        }
        Err(PmReadOperationFailure::Terminal(failure)) => PmAuthenticatedHttpReadResult::Terminal {
            completed_at,
            failure,
        },
    }
}

enum PmReadOperationFailure {
    Recoverable(PmLiveHttpQueryFailure),
    Terminal(PmAuthenticatedHttpTerminalFailure),
}

impl From<PmLiveAdapterError> for PmReadOperationFailure {
    fn from(error: PmLiveAdapterError) -> Self {
        match error {
            PmLiveAdapterError::CredentialAuthorityClosed => {
                Self::Terminal(PmAuthenticatedHttpTerminalFailure::CredentialAuthorityClosed)
            }
            PmLiveAdapterError::CredentialAuthorityTaskFailed => {
                Self::Terminal(PmAuthenticatedHttpTerminalFailure::CredentialAuthorityTaskFailed)
            }
            PmLiveAdapterError::CredentialOwnerMismatch
            | PmLiveAdapterError::ExactOrderIdentityMismatch
            | PmLiveAdapterError::ExactOrderMakerMismatch
            | PmLiveAdapterError::ExactOrderScopeMismatch => {
                Self::Terminal(PmAuthenticatedHttpTerminalFailure::ScopeMismatch)
            }
            PmLiveAdapterError::InvalidConfiguration(_)
            | PmLiveAdapterError::InvalidUserSubscription => {
                Self::Terminal(PmAuthenticatedHttpTerminalFailure::ContractViolation)
            }
            PmLiveAdapterError::RequestTimeout => {
                Self::Recoverable(PmLiveHttpQueryFailure::Timeout)
            }
            PmLiveAdapterError::RequestFailed
            | PmLiveAdapterError::TransportBuild
            | PmLiveAdapterError::ResponseBodyRead
            | PmLiveAdapterError::ProductClock => {
                Self::Recoverable(PmLiveHttpQueryFailure::Transport)
            }
            PmLiveAdapterError::Redirect { .. } | PmLiveAdapterError::UnexpectedStatus { .. } => {
                Self::Recoverable(PmLiveHttpQueryFailure::HttpStatus)
            }
            PmLiveAdapterError::ResponseBodyTooLarge { .. }
            | PmLiveAdapterError::Wire(_)
            | PmLiveAdapterError::PrivateWire(_) => {
                Self::Recoverable(PmLiveHttpQueryFailure::MalformedResponse)
            }
            PmLiveAdapterError::PaginationCursorCycle => {
                Self::Recoverable(PmLiveHttpQueryFailure::PaginationCycle)
            }
            PmLiveAdapterError::PaginationPageLimit | PmLiveAdapterError::PaginationRowLimit => {
                Self::Recoverable(PmLiveHttpQueryFailure::IncompleteResponse)
            }
            PmLiveAdapterError::Auth(_) | PmLiveAdapterError::InvalidAuthenticatedHeaders => {
                Self::Recoverable(PmLiveHttpQueryFailure::Authentication)
            }
        }
    }
}
