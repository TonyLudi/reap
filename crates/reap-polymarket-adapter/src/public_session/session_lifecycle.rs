//! Heartbeat, invalidation, and reconnect transitions for the PM public
//! session.
//!
//! Keeping transport lifecycle authority together makes it clear that
//! reconnect epochs, attempts, and backoff remain owned by the canonical
//! session rather than by a socket worker.

use super::*;

impl PmPublicSession {
    pub fn poll_heartbeat(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<PmPublicHeartbeatAction, PmPublicSessionError> {
        let result = self.poll_heartbeat_inner(monotonic_now_ns);
        if let Err(error) = result {
            self.invalidate_for_error(error);
        }
        result
    }

    /// Polls heartbeat state with a local wall clock so timeout invalidation
    /// can emit one evidence-bearing unavailable occurrence.
    pub fn poll_heartbeat_with_receive_evidence(
        &mut self,
        local_wall_now_ns: u64,
        monotonic_now_ns: u64,
    ) -> Result<PmPublicHeartbeatAction, PmPublicSessionError> {
        let result = self.poll_heartbeat_inner(monotonic_now_ns);
        if let Err(error) = result {
            self.invalidate_for_error_with_receive_evidence(
                error,
                local_wall_now_ns,
                monotonic_now_ns,
            );
        }
        result
    }

    /// Previews the exact heartbeat transition without mutating session state.
    ///
    /// Active composition owners use this to preflight and durably record the
    /// corresponding lifecycle event before enacting the session transition.
    /// In particular, an `Idle` preview does not advance the monotonic clock,
    /// and a timeout preview does not invalidate the session or consume its
    /// one unavailable occurrence.
    pub fn preview_heartbeat(
        &self,
        monotonic_now_ns: u64,
    ) -> Result<PmPublicHeartbeatAction, PmPublicSessionError> {
        self.ensure_subscribed()?;
        self.validate_monotonic(monotonic_now_ns)?;
        if let Some(deadline_ns) = self.attempt.heartbeat.pong_deadline_ns {
            return if monotonic_now_ns >= deadline_ns {
                Err(PmPublicSessionError::HeartbeatTimeout { deadline_ns })
            } else {
                Ok(PmPublicHeartbeatAction::Idle)
            };
        }
        let next_ping_ns = self
            .attempt
            .heartbeat
            .next_ping_ns
            .ok_or(PmPublicSessionError::InvalidHeartbeatState)?;
        if monotonic_now_ns < next_ping_ns {
            return Ok(PmPublicHeartbeatAction::Idle);
        }
        monotonic_now_ns
            .checked_add(self.heartbeat_config.pong_timeout_ns())
            .ok_or(PmPublicSessionError::HeartbeatDeadlineOverflow)?;
        Ok(PmPublicHeartbeatAction::SendPing)
    }

    fn poll_heartbeat_inner(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<PmPublicHeartbeatAction, PmPublicSessionError> {
        let action = self.preview_heartbeat(monotonic_now_ns)?;
        self.attempt.last_monotonic_ns = Some(monotonic_now_ns);
        if action == PmPublicHeartbeatAction::Idle {
            return Ok(action);
        }
        let deadline_ns = monotonic_now_ns
            .checked_add(self.heartbeat_config.pong_timeout_ns())
            .ok_or(PmPublicSessionError::HeartbeatDeadlineOverflow)?;
        self.attempt.heartbeat.next_ping_ns = None;
        self.attempt.heartbeat.pong_deadline_ns = Some(deadline_ns);
        Ok(action)
    }

    pub(super) fn receive_pong(
        &mut self,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<PmPublicSessionBatch, PmPublicSessionError> {
        let deadline_ns = self
            .attempt
            .heartbeat
            .pong_deadline_ns
            .ok_or(PmPublicSessionError::UnexpectedPong)?;
        if monotonic_receive_ns >= deadline_ns {
            return Err(PmPublicSessionError::HeartbeatTimeout { deadline_ns });
        }
        let next_ping_ns = monotonic_receive_ns
            .checked_add(self.heartbeat_config.ping_interval_ns())
            .ok_or(PmPublicSessionError::HeartbeatDeadlineOverflow)?;
        self.attempt.heartbeat.pong_deadline_ns = None;
        self.attempt.heartbeat.next_ping_ns = Some(next_ping_ns);
        self.attempt.last_monotonic_ns = Some(monotonic_receive_ns);
        Ok(PmPublicSessionBatch::from_heartbeat(
            PmPublicHeartbeatEvidence {
                connection_epoch: self.connection_epoch,
                local_wall_receive_ns,
                monotonic_receive_ns,
            },
        ))
    }

    /// Commits the exact reconnect transition previously available through
    /// [`Self::preview_after_failure_transition`].
    pub fn after_failure_transition(
        &mut self,
    ) -> Result<PmPublicReconnectTransition, PmPublicSessionError> {
        let transition = match self.preview_after_failure_transition() {
            Ok(transition) => transition,
            Err(
                error @ (PmPublicSessionError::ConnectionEpochOverflow
                | PmPublicSessionError::ReconnectAttemptOverflow),
            ) => {
                self.attempt.subscription_sent = false;
                self.attempt.requires_reconnect = true;
                self.attempt.flow_open = false;
                self.attempt.current_snapshot_revision = None;
                self.attempt.pending_snapshot_flow = None;
                self.attempt.heartbeat = HeartbeatState::disconnected();
                self.last_fault = Some(PmPublicSessionFault::Overflow);
                self.supervisor.mark_fatal();
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        let reached_flow_open = self.attempt.reached_flow_open;
        self.connection_epoch = transition.replacement_epoch;
        self.attempt = AttemptState::new();
        self.last_fault = None;
        self.pending_unavailable = None;
        self.unavailable_required = false;
        self.reconnect_attempt = transition.reconnect_attempt;
        let delay = self.supervisor.after_failure(reached_flow_open);
        debug_assert_eq!(delay, transition.delay);
        Ok(transition)
    }

    /// Purely previews the exact epoch/attempt/delay transition without
    /// mutating session or backoff state.
    pub fn preview_after_failure_transition(
        &self,
    ) -> Result<PmPublicReconnectTransition, PmPublicSessionError> {
        if self.health() == ConnectionStatusKind::Fatal {
            return Err(PmPublicSessionError::SessionFatal);
        }
        if self.attempt.requires_reconnect && self.unavailable_required {
            return if self.pending_unavailable.is_some() {
                Err(PmPublicSessionError::UnavailableOccurrencePending)
            } else {
                Err(PmPublicSessionError::UnavailableOccurrenceMissing)
            };
        }
        let next_epoch = self
            .connection_epoch
            .value()
            .checked_add(1)
            .ok_or(PmPublicSessionError::ConnectionEpochOverflow)?;
        let reconnect_attempt = if self.attempt.reached_flow_open {
            1
        } else {
            self.reconnect_attempt
                .checked_add(1)
                .ok_or(PmPublicSessionError::ReconnectAttemptOverflow)?
        };
        Ok(PmPublicReconnectTransition {
            retired_epoch: self.connection_epoch,
            replacement_epoch: ConnectionEpoch::new(next_epoch),
            reconnect_attempt,
            delay: self
                .supervisor
                .preview_after_failure(self.attempt.reached_flow_open),
            reset_after_flow_open: self.attempt.reached_flow_open,
        })
    }

    /// Fixture/replay compatibility wrapper. Live transport composition must
    /// use [`Self::after_failure_transition`] so attempt and epoch authority
    /// cannot be reconstructed from a bare duration.
    pub fn after_failure(&mut self) -> Result<Duration, PmPublicSessionError> {
        self.after_failure_transition()
            .map(PmPublicReconnectTransition::delay)
    }

    /// Fixture/replay compatibility wrapper for legacy duration consumers.
    pub fn preview_after_failure(
        &self,
    ) -> Result<(ConnectionEpoch, Duration), PmPublicSessionError> {
        self.preview_after_failure_transition()
            .map(|transition| (transition.replacement_epoch(), transition.delay()))
    }

    pub fn preflight_invalidate_with_receive_evidence(
        &self,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<(), PmPublicSessionError> {
        if self.pending_unavailable.is_some() {
            return Ok(());
        }
        ReceivedEventClock::new(None, local_wall_receive_ns, monotonic_receive_ns)
            .map_err(PmPublicSessionError::Envelope)?;
        self.validate_monotonic(monotonic_receive_ns)?;
        self.attempt
            .local_ingress_sequence
            .checked_add(1)
            .ok_or(PmPublicSessionError::IngressSequenceOverflow)?;
        Ok(())
    }

    /// Invalidates the attempt and records one session-sequenced unavailable
    /// occurrence. Connection faults never claim a venue timestamp.
    pub fn invalidate_with_receive_evidence(
        &mut self,
        fault: PmPublicSessionFault,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> Result<(), PmPublicSessionError> {
        if self.pending_unavailable.is_none() {
            let clock = ReceivedEventClock::new(None, local_wall_receive_ns, monotonic_receive_ns)
                .map_err(PmPublicSessionError::Envelope)?;
            self.validate_monotonic(monotonic_receive_ns)?;
            let next_ingress = self
                .attempt
                .local_ingress_sequence
                .checked_add(1)
                .ok_or(PmPublicSessionError::IngressSequenceOverflow)?;
            let ordering = EventOrdering::new(
                self.connection_epoch,
                None,
                None,
                None,
                IngressSequence::new(next_ingress),
            )
            .map_err(PmPublicSessionError::Envelope)?;
            self.attempt.local_ingress_sequence = next_ingress;
            self.attempt.last_monotonic_ns = Some(monotonic_receive_ns);
            self.pending_unavailable = Some(PmPublicUnavailableOccurrence {
                source: self.role.source(),
                connection_id: self.role.connection(),
                received_clock: clock,
                ordering,
                fault,
            });
        }
        self.invalidate(fault);
        Ok(())
    }

    pub fn invalidate(&mut self, fault: PmPublicSessionFault) {
        if !self.attempt.requires_reconnect {
            self.unavailable_required = true;
        }
        self.last_fault = Some(fault);
        self.attempt.subscription_sent = false;
        self.attempt.requires_reconnect = true;
        self.attempt.flow_open = false;
        self.attempt.current_snapshot_revision = None;
        self.attempt.pending_snapshot_flow = None;
        self.attempt.heartbeat = HeartbeatState::disconnected();
        if self.health() != ConnectionStatusKind::Fatal {
            self.supervisor.mark_disconnected();
        }
    }
}
