//! Production-selected authenticated user-stream integration.
//!
//! This path owns no credentials and constructs no order-entry authority. It
//! consumes only an already selected read role, feeds its credential-bound
//! frames through the sole product coordinator, and exposes immutable state
//! changes plus the coordinator's pending effects to the caller's handler.

use std::fmt;

use async_trait::async_trait;
use reap_pm_core::ConnectionEpoch;
use reap_pm_strategy::PmQuoteModel;
use reap_polymarket_live_adapter::{
    PmProductionSelectedUserWsRole, PmUserWsDisconnectReason, PmUserWsEvent, PmUserWsEventSink,
    PmUserWsRunError, PmUserWsShutdownSignal,
};

use crate::composition::{PmProductRun, PmProductRunError};
use crate::coordinator::PmProductEffect;
use crate::private_monitor::{PmLiveOccurrenceIssuer, PmReadOnlyPrivateProjection};
use crate::private_user_ws::{PmUserIngressAction, PmUserIngressError, PmUserIngressState};

/// One state transition accepted from the production-selected authenticated
/// user stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmProductionPrivateStateChange {
    ConnectionAvailable {
        connection_epoch: ConnectionEpoch,
    },
    PrivateFrameApplied {
        connection_epoch: ConnectionEpoch,
    },
    ConnectionUnavailable {
        connection_epoch: ConnectionEpoch,
        reason: PmUserWsDisconnectReason,
    },
    Shutdown {
        connection_epoch: ConnectionEpoch,
    },
}

/// Synchronous product-actor callback for immutable private state and the
/// coordinator effects pending after it.
///
/// Implementations should copy or enqueue the narrow values they need. The
/// projection borrow cannot be retained and provides no reducer, credential,
/// transport, or mutation authority. Effects are delivered by value so the
/// bounded coordinator output queue cannot silently saturate while the user
/// stream is being driven.
pub trait PmProductionPrivateUserWsHandler: Send {
    fn on_private_state_change(
        &mut self,
        change: PmProductionPrivateStateChange,
        state: PmReadOnlyPrivateProjection<'_>,
    );

    fn on_product_effect(&mut self, effect: PmProductEffect);
}

#[derive(Debug)]
pub enum PmProductionPrivateUserWsError {
    Occurrence,
    Protocol(&'static str),
    Product(Box<PmProductRunError>),
}

impl fmt::Display for PmProductionPrivateUserWsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Occurrence => {
                formatter.write_str("production private user occurrence was rejected")
            }
            Self::Protocol(reason) => write!(
                formatter,
                "production private user lifecycle violated its contract: {reason}"
            ),
            Self::Product(error) => write!(formatter, "PM product rejected private input: {error}"),
        }
    }
}

impl std::error::Error for PmProductionPrivateUserWsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Product(error) => Some(error.as_ref()),
            Self::Occurrence | Self::Protocol(_) => None,
        }
    }
}

impl From<PmProductRunError> for PmProductionPrivateUserWsError {
    fn from(error: PmProductRunError) -> Self {
        Self::Product(Box::new(error))
    }
}

impl<M> PmProductRun<M>
where
    M: PmQuoteModel + Send,
{
    /// Runs one already-selected production authenticated user stream through
    /// this product's canonical order/fill state.
    ///
    /// A unique fill immediately updates `provisional_deltas()` and creates
    /// the existing fill-refresh obligation. A duplicate fill cannot move the
    /// provisional position twice. An authoritative account/fill cut remains
    /// responsible for publishing the converged venue position.
    ///
    /// This method admits reads only. It does not construct a signer, HMAC
    /// role, place/cancel request, or production mutation transport.
    pub async fn run_production_selected_private_user_ws<H>(
        &mut self,
        role: PmProductionSelectedUserWsRole,
        shutdown: PmUserWsShutdownSignal,
        handler: &mut H,
    ) -> Result<(), PmUserWsRunError<PmProductionPrivateUserWsError>>
    where
        H: PmProductionPrivateUserWsHandler,
    {
        debug_assert!(!role.production_order_entry_authorized());
        let condition = role.condition();
        let mut sink = PmProductionPrivateUserWsSink {
            run: self,
            issuer: PmLiveOccurrenceIssuer::new(condition),
            lifecycle: PmUserIngressState::new(condition),
            handler,
        };
        role.run(shutdown, &mut sink).await
    }
}

struct PmProductionPrivateUserWsSink<'a, M, H>
where
    M: PmQuoteModel,
    H: PmProductionPrivateUserWsHandler,
{
    run: &'a mut PmProductRun<M>,
    issuer: PmLiveOccurrenceIssuer,
    lifecycle: PmUserIngressState,
    handler: &'a mut H,
}

#[async_trait]
impl<M, H> PmUserWsEventSink for PmProductionPrivateUserWsSink<'_, M, H>
where
    M: PmQuoteModel + Send,
    H: PmProductionPrivateUserWsHandler,
{
    type Error = PmProductionPrivateUserWsError;

    async fn deliver_user_ws_event(&mut self, event: PmUserWsEvent) -> Result<(), Self::Error> {
        let edge = PmUserEventEdge::from_event(&event);
        let action = self
            .lifecycle
            .admit(event, &mut self.issuer)
            .map_err(map_ingress_error)?;
        let Some(change) = self.apply(action, edge)? else {
            return Ok(());
        };

        self.run.service_turn(edge.monotonic_receive_ns)?;
        self.handler
            .on_private_state_change(change, self.run.private_projection());
        while let Some(effect) = self.run.pop_effect() {
            self.handler.on_product_effect(effect);
        }
        Ok(())
    }
}

impl<M, H> PmProductionPrivateUserWsSink<'_, M, H>
where
    M: PmQuoteModel,
    H: PmProductionPrivateUserWsHandler,
{
    fn apply(
        &mut self,
        action: PmUserIngressAction,
        edge: PmUserEventEdge,
    ) -> Result<Option<PmProductionPrivateStateChange>, PmProductionPrivateUserWsError> {
        match action {
            PmUserIngressAction::None => Ok(None),
            PmUserIngressAction::ConnectionAvailable(input) => {
                self.run.connect_private_live(input)?;
                Ok(Some(PmProductionPrivateStateChange::ConnectionAvailable {
                    connection_epoch: edge.connection_epoch,
                }))
            }
            PmUserIngressAction::PrivateFrame(input) => {
                let _report = self.run.ingest_private_live(input)?;
                Ok(Some(PmProductionPrivateStateChange::PrivateFrameApplied {
                    connection_epoch: edge.connection_epoch,
                }))
            }
            PmUserIngressAction::ConnectionUnavailable(input) => {
                self.run.mark_private_live_unavailable(input)?;
                Ok(Some(
                    PmProductionPrivateStateChange::ConnectionUnavailable {
                        connection_epoch: edge.connection_epoch,
                        reason: edge.disconnect_reason.expect("retirement carries a reason"),
                    },
                ))
            }
            PmUserIngressAction::Shutdown(input) => {
                self.run.request_live_shutdown(input)?;
                Ok(Some(PmProductionPrivateStateChange::Shutdown {
                    connection_epoch: edge.connection_epoch,
                }))
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PmUserEventEdge {
    connection_epoch: ConnectionEpoch,
    monotonic_receive_ns: u64,
    disconnect_reason: Option<PmUserWsDisconnectReason>,
}

impl PmUserEventEdge {
    fn from_event(event: &PmUserWsEvent) -> Self {
        match event {
            PmUserWsEvent::ConnectionOpened(observation)
            | PmUserWsEvent::SubscriptionSent(observation)
            | PmUserWsEvent::PingSent(observation)
            | PmUserWsEvent::Pong(observation)
            | PmUserWsEvent::Shutdown(observation) => Self::from_observation(*observation, None),
            PmUserWsEvent::BoundFrame(frame) => Self::from_observation(frame.observation(), None),
            PmUserWsEvent::ConnectionRetired(retirement)
            | PmUserWsEvent::RetryExhausted(retirement) => {
                Self::from_observation(retirement.observation(), Some(retirement.reason()))
            }
            PmUserWsEvent::ReconnectScheduled(reconnect) => Self::from_observation(
                reconnect.retired().observation(),
                Some(reconnect.retired().reason()),
            ),
        }
    }

    fn from_observation(
        observation: reap_polymarket_live_adapter::PmUserWsObservation,
        disconnect_reason: Option<PmUserWsDisconnectReason>,
    ) -> Self {
        Self {
            connection_epoch: observation.connection().connection_epoch(),
            monotonic_receive_ns: observation.clock().monotonic_receive_ns(),
            disconnect_reason,
        }
    }
}

fn map_ingress_error(error: PmUserIngressError) -> PmProductionPrivateUserWsError {
    match error {
        PmUserIngressError::Occurrence(_) => PmProductionPrivateUserWsError::Occurrence,
        PmUserIngressError::Protocol(reason) => PmProductionPrivateUserWsError::Protocol(reason),
    }
}
