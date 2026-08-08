//! Fixed-capacity socket-to-actor request/ack channels.

use async_trait::async_trait;
use reap_polymarket_live_adapter::{
    PmPublicWsEvent, PmPublicWsEventSink, PmPublicWsReconnectDirective, PmPublicWsRetirement,
    PmPublicWsRunError, PmRestBookPurpose, PmRestBookSnapshotSink, PmRestResponseClock,
    PmUserWsEvent, PmUserWsEventSink, PmUserWsRunError,
};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

pub(super) type PublicTaskOutput = Result<(), PmPublicWsRunError<PmReadIngressSinkError>>;
pub(super) type UserTaskOutput = Result<(), PmUserWsRunError<PmReadIngressSinkError>>;

pub(super) enum PmPublicActorRequest {
    Event {
        event: PmPublicWsEvent,
        acknowledgement: oneshot::Sender<Result<(), ()>>,
    },
    Reconnect {
        retired: PmPublicWsRetirement,
        acknowledgement: oneshot::Sender<Result<PmPublicWsReconnectDirective, ()>>,
    },
    RestBook {
        purpose: PmRestBookPurpose,
        received: PmRestResponseClock,
        raw: Box<[u8]>,
        acknowledgement: oneshot::Sender<Result<(), ()>>,
    },
}

pub(super) struct PmRestBookIngressSink {
    actor: mpsc::Sender<PmPublicActorRequest>,
}

impl PmRestBookIngressSink {
    pub(super) const fn new(actor: mpsc::Sender<PmPublicActorRequest>) -> Self {
        Self { actor }
    }
}

#[async_trait]
impl PmRestBookSnapshotSink for PmRestBookIngressSink {
    type Output = ();
    type Error = PmReadIngressSinkError;

    async fn deliver_native_rest_book(
        &mut self,
        purpose: PmRestBookPurpose,
        received: PmRestResponseClock,
        raw: &[u8],
    ) -> Result<(), Self::Error> {
        let (acknowledgement, response) = oneshot::channel();
        send_bounded(
            &self.actor,
            PmPublicActorRequest::RestBook {
                purpose,
                received,
                raw: raw.into(),
                acknowledgement,
            },
            response,
        )
        .await
    }
}

pub(super) enum PmUserActorRequest {
    Event {
        event: PmUserWsEvent,
        acknowledgement: oneshot::Sender<Result<(), ()>>,
    },
}

pub(super) struct PmPublicIngressSink {
    actor: mpsc::Sender<PmPublicActorRequest>,
}

impl PmPublicIngressSink {
    pub(super) const fn new(actor: mpsc::Sender<PmPublicActorRequest>) -> Self {
        Self { actor }
    }
}

#[async_trait]
impl PmPublicWsEventSink for PmPublicIngressSink {
    type Error = PmReadIngressSinkError;

    async fn deliver_public_ws_event(&mut self, event: PmPublicWsEvent) -> Result<(), Self::Error> {
        let (acknowledgement, response) = oneshot::channel();
        send_bounded(
            &self.actor,
            PmPublicActorRequest::Event {
                event,
                acknowledgement,
            },
            response,
        )
        .await
    }

    async fn authorize_public_ws_reconnect(
        &mut self,
        retired: PmPublicWsRetirement,
    ) -> Result<PmPublicWsReconnectDirective, Self::Error> {
        let (acknowledgement, response) = oneshot::channel();
        send_bounded(
            &self.actor,
            PmPublicActorRequest::Reconnect {
                retired,
                acknowledgement,
            },
            response,
        )
        .await
    }
}

pub(super) struct PmUserIngressSink {
    actor: mpsc::Sender<PmUserActorRequest>,
}

impl PmUserIngressSink {
    pub(super) const fn new(actor: mpsc::Sender<PmUserActorRequest>) -> Self {
        Self { actor }
    }
}

#[async_trait]
impl PmUserWsEventSink for PmUserIngressSink {
    type Error = PmReadIngressSinkError;

    async fn deliver_user_ws_event(&mut self, event: PmUserWsEvent) -> Result<(), Self::Error> {
        let (acknowledgement, response) = oneshot::channel();
        send_bounded(
            &self.actor,
            PmUserActorRequest::Event {
                event,
                acknowledgement,
            },
            response,
        )
        .await
    }
}

async fn send_bounded<T, R>(
    actor: &mpsc::Sender<T>,
    request: T,
    response: oneshot::Receiver<Result<R, ()>>,
) -> Result<R, PmReadIngressSinkError> {
    actor
        .send(request)
        .await
        .map_err(|_| PmReadIngressSinkError::ActorClosed)?;
    response
        .await
        .map_err(|_| PmReadIngressSinkError::ActorClosed)?
        .map_err(|()| PmReadIngressSinkError::ActorRejected)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(in crate::composition::product::authenticated_loopback) enum PmReadIngressSinkError {
    #[error("authenticated read-ingress actor channel closed")]
    ActorClosed,
    #[error("authenticated read-ingress actor rejected typed evidence")]
    ActorRejected,
}

#[cfg(test)]
pub(super) async fn bounded_round_trip_for_test(
    sender: &mpsc::Sender<u8>,
    value: u8,
    response: oneshot::Receiver<Result<u8, ()>>,
) -> Result<u8, PmReadIngressSinkError> {
    send_bounded(sender, value, response).await
}
