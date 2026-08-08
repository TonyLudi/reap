//! Public market-WebSocket durability/session admission on the product actor.

use reap_pm_core::{PmBookUpdate, PmConditionId, PmMarketId, PmTokenId};
use reap_pm_strategy::PmQuoteModel;
use reap_polymarket_live_adapter::{
    PmPublicWsEvent, PmPublicWsReconnectDirective, PmPublicWsRetirement, PmRestBookSnapshotSink,
};

use super::{PmReadIngressActor, PmReadIngressActorError};
use crate::composition::{PmProductPublicIngress, PmProductPublicIngressOutcome};

pub(super) struct PmPublicIngressState {
    condition: PmConditionId,
    market: PmMarketId,
    token: PmTokenId,
    initial_authorization: Option<reap_pm_core::ConnectionEpoch>,
    active_epoch: Option<reap_pm_core::ConnectionEpoch>,
    retired_epoch: Option<reap_pm_core::ConnectionEpoch>,
    authorized_replacement: Option<reap_pm_core::ConnectionEpoch>,
}

impl PmPublicIngressState {
    pub(super) const fn new(
        condition: PmConditionId,
        market: PmMarketId,
        token: PmTokenId,
        initial_epoch: reap_pm_core::ConnectionEpoch,
    ) -> Self {
        Self {
            condition,
            market,
            token,
            initial_authorization: Some(initial_epoch),
            active_epoch: None,
            retired_epoch: None,
            authorized_replacement: None,
        }
    }
}

pub(super) async fn service_public_request<M: PmQuoteModel>(
    request: super::channel::PmPublicActorRequest,
    state: &mut PmPublicIngressState,
    actor: &mut PmReadIngressActor<'_, M>,
) -> Result<PmPublicServiceOutcome, PmReadIngressActorError> {
    match request {
        super::channel::PmPublicActorRequest::Event {
            event,
            acknowledgement,
        } => {
            let result = service_public_event(event, state, actor).await;
            let _ = acknowledgement.send(result.as_ref().map(|_| ()).map_err(|_| ()));
            result
        }
        super::channel::PmPublicActorRequest::Reconnect {
            retired,
            acknowledgement,
        } => {
            let result = authorize_reconnect(retired, state, actor).await;
            let _ = acknowledgement.send(result.as_ref().copied().map_err(|_| ()));
            result.map(|_| PmPublicServiceOutcome::Admitted)
        }
        super::channel::PmPublicActorRequest::RestBook {
            purpose,
            received,
            raw,
            acknowledgement,
        } => {
            let result = service_rest_book(purpose, received, &raw, actor).await;
            let _ = acknowledgement.send(result.as_ref().map(|_| ()).map_err(|_| ()));
            result
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PmPublicServiceOutcome {
    Admitted,
    SubscriptionReady,
    ResyncRequired,
}

async fn service_public_event<M: PmQuoteModel>(
    event: PmPublicWsEvent,
    state: &mut PmPublicIngressState,
    actor: &mut PmReadIngressActor<'_, M>,
) -> Result<PmPublicServiceOutcome, PmReadIngressActorError> {
    let mut outcome = PmPublicServiceOutcome::Admitted;
    match event {
        PmPublicWsEvent::ConnectionOpened(observation) => {
            validate_connection(observation.connection(), state)?;
            let epoch = observation.connection().connection_epoch();
            state.validate_authorized_open(epoch)?;
            let mut ingress = PmProductPublicIngress::new(actor.coordinator.public_capture_mut());
            ingress
                .record_pm_connection_started(observation.clock().monotonic_receive_ns())
                .await?;
            state.active_epoch = Some(epoch);
            state.initial_authorization = None;
            state.authorized_replacement = None;
        }
        PmPublicWsEvent::SubscriptionSent(observation) => {
            validate_observation(observation, state)?;
            let mut ingress = PmProductPublicIngress::new(actor.coordinator.public_capture_mut());
            ingress
                .record_pm_subscription_sent(observation.clock().monotonic_receive_ns())
                .await?;
            match ingress
                .issue_and_enqueue_pm_metadata(observation.clock().local_wall_receive_ns())
                .await?
            {
                PmProductPublicIngressOutcome::Enqueued(()) => {}
                PmProductPublicIngressOutcome::ResyncRequired(_) => {
                    return Err(PmReadIngressActorError::PublicProtocol(
                        "public metadata admission required a cold restart",
                    ));
                }
            }
            let _ = actor.service_after_ingress()?;
            outcome = PmPublicServiceOutcome::SubscriptionReady;
        }
        PmPublicWsEvent::PingSent(observation) => {
            validate_observation(observation, state)?;
            let clock = observation.clock();
            let mut ingress = PmProductPublicIngress::new(actor.coordinator.public_capture_mut());
            ingress
                .record_pm_heartbeat_ping_sent(
                    clock.local_wall_receive_ns(),
                    clock.monotonic_receive_ns(),
                )
                .await?;
        }
        PmPublicWsEvent::Pong(observation) => {
            validate_observation(observation, state)?;
            outcome = capture_and_reduce(observation.clock(), b"PONG", actor).await?;
        }
        PmPublicWsEvent::RawData(raw) => {
            validate_connection(raw.connection(), state)?;
            if state.active_epoch != Some(raw.connection().connection_epoch()) {
                return Err(PmReadIngressActorError::PublicProtocol(
                    "public raw frame did not match the active epoch",
                ));
            }
            outcome = capture_and_reduce(raw.clock(), raw.bytes(), actor).await?;
        }
        PmPublicWsEvent::ConnectionRetired(retired) => {
            retire_public(retired, state, actor).await?;
        }
        PmPublicWsEvent::ReconnectScheduled(reconnect) => {
            if state.retired_epoch != Some(reconnect.retired().connection().connection_epoch())
                || state.authorized_replacement != Some(reconnect.replacement_epoch())
            {
                return Err(PmReadIngressActorError::PublicProtocol(
                    "public reconnect evidence did not match actor authorization",
                ));
            }
        }
        PmPublicWsEvent::ReconnectStopped(retired) => {
            if state.retired_epoch != Some(retired.connection().connection_epoch()) {
                return Err(PmReadIngressActorError::PublicProtocol(
                    "public reconnect stop did not match retired epoch",
                ));
            }
        }
        PmPublicWsEvent::Shutdown(observation) => {
            validate_observation(observation, state)?;
            let retirement_clock = observation.clock();
            let mut ingress = PmProductPublicIngress::new(actor.coordinator.public_capture_mut());
            ingress
                .record_pm_disconnected(
                    retirement_clock.local_wall_receive_ns(),
                    retirement_clock.monotonic_receive_ns(),
                )
                .await?;
            state.retired_epoch = state.active_epoch.take();
            let _ = actor.service_after_ingress()?;
        }
    }
    Ok(outcome)
}

impl PmPublicIngressState {
    pub(super) fn validate_authorized_open(
        &self,
        epoch: reap_pm_core::ConnectionEpoch,
    ) -> Result<(), PmReadIngressActorError> {
        let authorized = self.active_epoch.is_none()
            && (self.initial_authorization == Some(epoch)
                || self.authorized_replacement == Some(epoch));
        if authorized {
            Ok(())
        } else {
            Err(PmReadIngressActorError::PublicProtocol(
                "public connection epoch was not actor-authorized",
            ))
        }
    }
}

async fn retire_public<M: PmQuoteModel>(
    retired: PmPublicWsRetirement,
    state: &mut PmPublicIngressState,
    actor: &mut PmReadIngressActor<'_, M>,
) -> Result<(), PmReadIngressActorError> {
    validate_connection(retired.connection(), state)?;
    let epoch = retired.connection().connection_epoch();
    if state.active_epoch != Some(epoch) {
        return Err(PmReadIngressActorError::PublicProtocol(
            "public retirement did not match active epoch",
        ));
    }
    let clock = retired.clock();
    let mut ingress = PmProductPublicIngress::new(actor.coordinator.public_capture_mut());
    ingress
        .record_pm_disconnected(clock.local_wall_receive_ns(), clock.monotonic_receive_ns())
        .await?;
    state.active_epoch = None;
    state.retired_epoch = Some(epoch);
    let _ = actor.service_after_ingress()?;
    Ok(())
}

async fn authorize_reconnect<M: PmQuoteModel>(
    retired: PmPublicWsRetirement,
    state: &mut PmPublicIngressState,
    actor: &mut PmReadIngressActor<'_, M>,
) -> Result<PmPublicWsReconnectDirective, PmReadIngressActorError> {
    let epoch = retired.connection().connection_epoch();
    if state.active_epoch.is_some() || state.retired_epoch != Some(epoch) {
        return Err(PmReadIngressActorError::PublicProtocol(
            "public reconnect request did not match retired epoch",
        ));
    }
    let scheduled = actor.actor_clock.observe_control_edge()?.received_clock();
    let mut ingress = PmProductPublicIngress::new(actor.coordinator.public_capture_mut());
    let authorization = ingress
        .record_pm_reconnect_scheduled(scheduled.monotonic_receive_ns())
        .await?;
    if authorization.retired_epoch() != epoch {
        return Err(PmReadIngressActorError::PublicProtocol(
            "canonical reconnect authorization retired another epoch",
        ));
    }
    state.authorized_replacement = Some(authorization.replacement_epoch());
    Ok(authorization.into_transport_directive())
}

async fn capture_and_reduce<M: PmQuoteModel>(
    clock: reap_polymarket_live_adapter::PmPublicWsEdgeClock,
    raw: &[u8],
    actor: &mut PmReadIngressActor<'_, M>,
) -> Result<PmPublicServiceOutcome, PmReadIngressActorError> {
    let batch = {
        let mut ingress = PmProductPublicIngress::new(actor.coordinator.public_capture_mut());
        ingress
            .capture_pm_public(
                clock.local_wall_receive_ns(),
                clock.monotonic_receive_ns(),
                raw,
            )
            .await?
    };
    reduce_batch(batch, actor).await
}

async fn service_rest_book<M: PmQuoteModel>(
    purpose: reap_polymarket_live_adapter::PmRestBookPurpose,
    received: reap_polymarket_live_adapter::PmRestResponseClock,
    raw: &[u8],
    actor: &mut PmReadIngressActor<'_, M>,
) -> Result<PmPublicServiceOutcome, PmReadIngressActorError> {
    let captured = {
        let mut ingress = PmProductPublicIngress::new(actor.coordinator.public_capture_mut());
        ingress
            .rest_book_capture_sink()
            .deliver_native_rest_book(purpose, received, raw)
            .await?
    };
    if captured.purpose() != purpose {
        return Err(PmReadIngressActorError::PublicProtocol(
            "REST-book capture changed the requested purpose",
        ));
    }
    reduce_batch(captured.into_batch(), actor).await
}

async fn reduce_batch<M: PmQuoteModel>(
    mut batch: crate::capture_roles::PmPublicCaptureBatch,
    actor: &mut PmReadIngressActor<'_, M>,
) -> Result<PmPublicServiceOutcome, PmReadIngressActorError> {
    let mut ingress = PmProductPublicIngress::new(actor.coordinator.public_capture_mut());
    let flow = batch.take_snapshot_flow();
    let unavailable = batch.take_unavailable();
    let books = batch.into_books();

    if unavailable.is_some() {
        let mut books = books.into_iter();
        let delivery = books.next().ok_or(PmReadIngressActorError::PublicProtocol(
            "public unavailable batch omitted its terminal delivery",
        ))?;
        if books.next().is_some()
            || !matches!(
                delivery.envelope().payload().update(),
                PmBookUpdate::TickSizeChanged { .. }
            )
        {
            return Err(PmReadIngressActorError::PublicProtocol(
                "public unavailable batch had an unsupported shape",
            ));
        }
        let _ = ingress.apply_terminal_tick_invalidation(delivery)?;
        return Err(PmReadIngressActorError::PublicProtocol(
            "public metadata drift requires a cold authenticated restart",
        ));
    }

    if let Some(flow) = flow {
        let mut books = books.into_iter();
        let delivery = books.next().ok_or(PmReadIngressActorError::PublicProtocol(
            "public snapshot flow omitted its delivery",
        ))?;
        if books.next().is_some() {
            return Err(PmReadIngressActorError::PublicProtocol(
                "public snapshot flow carried multiple deliveries",
            ));
        }
        match ingress
            .commit_then_enqueue_pm_snapshot(delivery, flow)
            .await?
        {
            PmProductPublicIngressOutcome::Enqueued(()) => {}
            PmProductPublicIngressOutcome::ResyncRequired(_) => {
                return Ok(PmPublicServiceOutcome::ResyncRequired);
            }
        }
    } else {
        for delivery in books {
            match ingress.reduce_then_enqueue_pm_book(delivery).await? {
                PmProductPublicIngressOutcome::Enqueued(_) => {}
                PmProductPublicIngressOutcome::ResyncRequired(_) => {
                    return Ok(PmPublicServiceOutcome::ResyncRequired);
                }
            }
        }
    }
    let _ = actor.service_after_ingress()?;
    Ok(PmPublicServiceOutcome::Admitted)
}

fn validate_observation(
    observation: reap_polymarket_live_adapter::PmPublicWsObservation,
    state: &PmPublicIngressState,
) -> Result<(), PmReadIngressActorError> {
    validate_connection(observation.connection(), state)?;
    if state.active_epoch != Some(observation.connection().connection_epoch()) {
        return Err(PmReadIngressActorError::PublicProtocol(
            "public observation did not match active epoch",
        ));
    }
    Ok(())
}

fn validate_connection(
    connection: reap_polymarket_live_adapter::PmPublicWsConnection,
    state: &PmPublicIngressState,
) -> Result<(), PmReadIngressActorError> {
    let actual = connection.scope();
    if actual.condition() == state.condition
        && actual.market() == state.market
        && actual.token() == state.token
    {
        Ok(())
    } else {
        Err(PmReadIngressActorError::PublicProtocol(
            "public transport scope did not match configured scope",
        ))
    }
}
