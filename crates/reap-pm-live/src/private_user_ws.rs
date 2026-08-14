//! Shared authenticated user-stream lifecycle admission.
//!
//! Both loopback evidence and production-selected reads pass through this
//! state machine before they can reach the sole PM product coordinator.

use reap_pm_core::{ConnectionEpoch, PmConditionId, ReceivedEventClock};
use reap_polymarket_live_adapter::{PmUserWsEvent, PmUserWsObservation, PmUserWsRetirement};

use crate::private_monitor::{
    PmLiveConnectionInput, PmLiveInternalControlOccurrence, PmLiveOccurrenceError,
    PmLiveOccurrenceIssuer, PmLivePrivateInput, PmLiveRetirementInput, PmLiveRetirementOutcome,
};

#[derive(Debug)]
pub(crate) enum PmUserIngressAction {
    None,
    ConnectionAvailable(PmLiveConnectionInput),
    PrivateFrame(PmLivePrivateInput),
    ConnectionUnavailable(PmLiveRetirementInput),
    Shutdown(PmLiveInternalControlOccurrence),
}

#[derive(Debug)]
#[cfg_attr(
    not(any(test, feature = "loopback-evidence")),
    allow(
        dead_code,
        reason = "production maps occurrence details to a closed public error"
    )
)]
pub(crate) enum PmUserIngressError {
    Occurrence(PmLiveOccurrenceError),
    Protocol(&'static str),
}

impl From<PmLiveOccurrenceError> for PmUserIngressError {
    fn from(error: PmLiveOccurrenceError) -> Self {
        Self::Occurrence(error)
    }
}

/// Actor-local proof of the exact open -> subscribed -> active -> retired
/// user-stream lifecycle.
pub(crate) struct PmUserIngressState {
    condition: PmConditionId,
    pre_open: Option<PmUserWsObservation>,
    active_epoch: Option<ConnectionEpoch>,
    retired_epoch: Option<ConnectionEpoch>,
    controlled_shutdown_issued: bool,
}

impl PmUserIngressState {
    pub(crate) const fn new(condition: PmConditionId) -> Self {
        Self {
            condition,
            pre_open: None,
            active_epoch: None,
            retired_epoch: None,
            controlled_shutdown_issued: false,
        }
    }

    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn mark_controlled_shutdown_issued(&mut self) {
        self.controlled_shutdown_issued = true;
    }

    pub(crate) fn admit(
        &mut self,
        event: PmUserWsEvent,
        issuer: &mut PmLiveOccurrenceIssuer,
    ) -> Result<PmUserIngressAction, PmUserIngressError> {
        match event {
            PmUserWsEvent::ConnectionOpened(observation) => {
                self.validate_condition(observation)?;
                if self.pre_open.is_some() || self.active_epoch.is_some() {
                    return Err(PmUserIngressError::Protocol(
                        "user connection opened before the prior epoch retired",
                    ));
                }
                self.pre_open = Some(observation);
                Ok(PmUserIngressAction::None)
            }
            PmUserWsEvent::SubscriptionSent(observation) => {
                self.validate_condition(observation)?;
                let Some(opened) = self.pre_open.take() else {
                    return Err(PmUserIngressError::Protocol(
                        "user subscription had no retained transport open",
                    ));
                };
                if opened.connection().connection_epoch()
                    != observation.connection().connection_epoch()
                {
                    self.pre_open = Some(opened);
                    return Err(PmUserIngressError::Protocol(
                        "user subscription did not match the pre-open epoch",
                    ));
                }
                let input = issuer.start_user_connection(observation)?;
                self.active_epoch = Some(observation.connection().connection_epoch());
                Ok(PmUserIngressAction::ConnectionAvailable(input))
            }
            PmUserWsEvent::BoundFrame(frame) => {
                let epoch = frame.observation().connection().connection_epoch();
                if self.active_epoch != Some(epoch) {
                    return Err(PmUserIngressError::Protocol(
                        "user frame arrived outside the subscribed active epoch",
                    ));
                }
                Ok(PmUserIngressAction::PrivateFrame(
                    issuer.issue_user_frame(frame)?,
                ))
            }
            PmUserWsEvent::ConnectionRetired(retirement) => self.retire(retirement, issuer),
            PmUserWsEvent::Shutdown(observation) => {
                self.validate_condition(observation)?;
                if self.controlled_shutdown_issued {
                    return Ok(PmUserIngressAction::None);
                }
                let clock = observation.clock();
                let received = ReceivedEventClock::new(
                    None,
                    clock.local_wall_receive_ns(),
                    clock.monotonic_receive_ns(),
                )
                .map_err(|_| PmUserIngressError::Protocol("invalid user shutdown edge clock"))?;
                let occurrence = issuer.issue_user_shutdown_control(received)?;
                self.controlled_shutdown_issued = true;
                Ok(PmUserIngressAction::Shutdown(occurrence))
            }
            PmUserWsEvent::PingSent(observation) | PmUserWsEvent::Pong(observation) => {
                self.validate_active(observation)?;
                Ok(PmUserIngressAction::None)
            }
            PmUserWsEvent::ReconnectScheduled(reconnect) => {
                self.validate_condition(reconnect.retired().observation())?;
                if self.retired_epoch
                    != Some(
                        reconnect
                            .retired()
                            .observation()
                            .connection()
                            .connection_epoch(),
                    )
                {
                    return Err(PmUserIngressError::Protocol(
                        "user reconnect did not refer to the retired epoch",
                    ));
                }
                Ok(PmUserIngressAction::None)
            }
            PmUserWsEvent::RetryExhausted(retirement) => {
                self.validate_condition(retirement.observation())?;
                if self.retired_epoch
                    != Some(retirement.observation().connection().connection_epoch())
                {
                    return Err(PmUserIngressError::Protocol(
                        "user retry exhaustion did not refer to the retired epoch",
                    ));
                }
                Ok(PmUserIngressAction::None)
            }
        }
    }

    fn retire(
        &mut self,
        retirement: PmUserWsRetirement,
        issuer: &mut PmLiveOccurrenceIssuer,
    ) -> Result<PmUserIngressAction, PmUserIngressError> {
        let observation = retirement.observation();
        self.validate_condition(observation)?;
        let epoch = observation.connection().connection_epoch();
        if self.active_epoch != Some(epoch)
            && self
                .pre_open
                .is_none_or(|opened| opened.connection().connection_epoch() != epoch)
        {
            return Err(PmUserIngressError::Protocol(
                "user retirement did not match active or pre-open epoch",
            ));
        }
        match issuer.retire_user_connection(retirement)? {
            PmLiveRetirementOutcome::Active(input) => {
                self.active_epoch = None;
                self.retired_epoch = Some(epoch);
                Ok(PmUserIngressAction::ConnectionUnavailable(input))
            }
            PmLiveRetirementOutcome::PreOpen(_) => {
                self.pre_open = None;
                self.retired_epoch = Some(epoch);
                Ok(PmUserIngressAction::None)
            }
        }
    }

    fn validate_condition(
        &self,
        observation: PmUserWsObservation,
    ) -> Result<(), PmUserIngressError> {
        if observation.connection().condition() == self.condition {
            Ok(())
        } else {
            Err(PmUserIngressError::Protocol(
                "user transport condition did not match configured condition",
            ))
        }
    }

    fn validate_active(&self, observation: PmUserWsObservation) -> Result<(), PmUserIngressError> {
        self.validate_condition(observation)?;
        if self.active_epoch == Some(observation.connection().connection_epoch()) {
            Ok(())
        } else {
            Err(PmUserIngressError::Protocol(
                "user observation did not match the subscribed active epoch",
            ))
        }
    }
}
