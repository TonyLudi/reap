//! Authenticated user-stream event admission on the product actor.

use reap_pm_core::{ConnectionEpoch, PmConditionId, ReceivedEventClock};
use reap_pm_strategy::PmQuoteModel;
use reap_polymarket_live_adapter::{PmUserWsEvent, PmUserWsObservation};

use super::{PmReadIngressActor, PmReadIngressActorError};
use crate::private_monitor::PmLiveRetirementOutcome;

/// Actor-local lifecycle proof. A transport open is deliberately only
/// pre-open: canonical private availability starts after the authenticated
/// subscription write is observed.
pub(super) struct PmUserIngressState {
    condition: PmConditionId,
    pre_open: Option<PmUserWsObservation>,
    active_epoch: Option<ConnectionEpoch>,
    retired_epoch: Option<ConnectionEpoch>,
    controlled_shutdown_issued: bool,
}

impl PmUserIngressState {
    pub(super) const fn new(condition: PmConditionId) -> Self {
        Self {
            condition,
            pre_open: None,
            active_epoch: None,
            retired_epoch: None,
            controlled_shutdown_issued: false,
        }
    }

    pub(super) fn mark_controlled_shutdown_issued(&mut self) {
        self.controlled_shutdown_issued = true;
    }
}

pub(super) async fn service_user_request<M: PmQuoteModel>(
    request: super::channel::PmUserActorRequest,
    state: &mut PmUserIngressState,
    actor: &mut PmReadIngressActor<'_, M>,
) -> Result<(), PmReadIngressActorError> {
    let super::channel::PmUserActorRequest::Event {
        event,
        acknowledgement,
    } = request;
    let result = service_user_event(event, state, actor);
    let acknowledgement_result = result.as_ref().map_err(|_| ()).copied();
    let _ = acknowledgement.send(acknowledgement_result);
    result
}

fn service_user_event<M: PmQuoteModel>(
    event: PmUserWsEvent,
    state: &mut PmUserIngressState,
    actor: &mut PmReadIngressActor<'_, M>,
) -> Result<(), PmReadIngressActorError> {
    let admitted = match event {
        PmUserWsEvent::ConnectionOpened(observation) => {
            validate_condition(observation, state)?;
            if state.pre_open.is_some() || state.active_epoch.is_some() {
                return Err(PmReadIngressActorError::UserProtocol(
                    "user connection opened before the prior epoch retired",
                ));
            }
            state.pre_open = Some(observation);
            false
        }
        PmUserWsEvent::SubscriptionSent(observation) => {
            validate_condition(observation, state)?;
            let Some(opened) = state.pre_open.take() else {
                return Err(PmReadIngressActorError::UserProtocol(
                    "user subscription had no retained transport open",
                ));
            };
            if opened.connection().connection_epoch() != observation.connection().connection_epoch()
            {
                state.pre_open = Some(opened);
                return Err(PmReadIngressActorError::UserProtocol(
                    "user subscription did not match the pre-open epoch",
                ));
            }
            let input = actor.issuer.start_user_connection(observation)?;
            actor.coordinator.connect_private_live(input)?;
            state.active_epoch = Some(observation.connection().connection_epoch());
            true
        }
        PmUserWsEvent::BoundFrame(frame) => {
            let epoch = frame.observation().connection().connection_epoch();
            if state.active_epoch != Some(epoch) {
                return Err(PmReadIngressActorError::UserProtocol(
                    "user frame arrived outside the subscribed active epoch",
                ));
            }
            let input = actor.issuer.issue_user_frame(frame)?;
            let _report = actor.coordinator.ingest_private_live(input)?;
            true
        }
        PmUserWsEvent::ConnectionRetired(retirement) => {
            let observation = retirement.observation();
            validate_condition(observation, state)?;
            let epoch = observation.connection().connection_epoch();
            if state.active_epoch != Some(epoch)
                && state
                    .pre_open
                    .is_none_or(|opened| opened.connection().connection_epoch() != epoch)
            {
                return Err(PmReadIngressActorError::UserProtocol(
                    "user retirement did not match active or pre-open epoch",
                ));
            }
            match actor.issuer.retire_user_connection(retirement)? {
                PmLiveRetirementOutcome::Active(input) => {
                    actor.coordinator.mark_private_live_unavailable(input)?;
                    state.active_epoch = None;
                    state.retired_epoch = Some(epoch);
                    true
                }
                PmLiveRetirementOutcome::PreOpen(_) => {
                    state.pre_open = None;
                    state.retired_epoch = Some(epoch);
                    false
                }
            }
        }
        PmUserWsEvent::Shutdown(observation) => {
            validate_condition(observation, state)?;
            if state.controlled_shutdown_issued {
                return Ok(());
            }
            let clock = observation.clock();
            let received = ReceivedEventClock::new(
                None,
                clock.local_wall_receive_ns(),
                clock.monotonic_receive_ns(),
            )
            .map_err(|_| {
                PmReadIngressActorError::UserProtocol("invalid user shutdown edge clock")
            })?;
            let occurrence = actor.issuer.issue_user_shutdown_control(received)?;
            actor.coordinator.request_live_shutdown(occurrence)?;
            true
        }
        PmUserWsEvent::PingSent(observation) | PmUserWsEvent::Pong(observation) => {
            validate_active(observation, state)?;
            false
        }
        PmUserWsEvent::ReconnectScheduled(reconnect) => {
            validate_condition(reconnect.retired().observation(), state)?;
            if state.retired_epoch
                != Some(
                    reconnect
                        .retired()
                        .observation()
                        .connection()
                        .connection_epoch(),
                )
            {
                return Err(PmReadIngressActorError::UserProtocol(
                    "user reconnect did not refer to the retired epoch",
                ));
            }
            false
        }
        PmUserWsEvent::RetryExhausted(retirement) => {
            validate_condition(retirement.observation(), state)?;
            if state.retired_epoch != Some(retirement.observation().connection().connection_epoch())
            {
                return Err(PmReadIngressActorError::UserProtocol(
                    "user retry exhaustion did not refer to the retired epoch",
                ));
            }
            false
        }
    };
    if admitted {
        let _ = actor.service_after_ingress()?;
    }
    Ok(())
}

fn validate_condition(
    observation: PmUserWsObservation,
    state: &PmUserIngressState,
) -> Result<(), PmReadIngressActorError> {
    if observation.connection().condition() == state.condition {
        Ok(())
    } else {
        Err(PmReadIngressActorError::UserProtocol(
            "user transport condition did not match configured condition",
        ))
    }
}

fn validate_active(
    observation: PmUserWsObservation,
    state: &PmUserIngressState,
) -> Result<(), PmReadIngressActorError> {
    validate_condition(observation, state)?;
    if state.active_epoch == Some(observation.connection().connection_epoch()) {
        Ok(())
    } else {
        Err(PmReadIngressActorError::UserProtocol(
            "user observation did not match the subscribed active epoch",
        ))
    }
}
