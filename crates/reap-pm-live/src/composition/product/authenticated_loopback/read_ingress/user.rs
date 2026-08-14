//! Authenticated user-stream event admission on the product actor.

use reap_pm_strategy::PmQuoteModel;
use reap_polymarket_live_adapter::PmUserWsEvent;

use super::{PmReadIngressActor, PmReadIngressActorError};
pub(super) use crate::private_user_ws::PmUserIngressState;
use crate::private_user_ws::{PmUserIngressAction, PmUserIngressError};

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
    let action = state
        .admit(event, actor.issuer)
        .map_err(map_ingress_error)?;
    let admitted = match action {
        PmUserIngressAction::None => false,
        PmUserIngressAction::ConnectionAvailable(input) => {
            actor.coordinator.connect_private_live(input)?;
            true
        }
        PmUserIngressAction::PrivateFrame(input) => {
            let _report = actor.coordinator.ingest_private_live(input)?;
            true
        }
        PmUserIngressAction::ConnectionUnavailable(input) => {
            actor.coordinator.mark_private_live_unavailable(input)?;
            true
        }
        PmUserIngressAction::Shutdown(occurrence) => {
            actor.coordinator.request_live_shutdown(occurrence)?;
            true
        }
    };
    if admitted {
        let _ = actor.service_after_ingress()?;
    }
    Ok(())
}

fn map_ingress_error(error: PmUserIngressError) -> PmReadIngressActorError {
    match error {
        PmUserIngressError::Occurrence(error) => PmReadIngressActorError::Occurrence(error),
        PmUserIngressError::Protocol(reason) => PmReadIngressActorError::UserProtocol(reason),
    }
}
