use super::*;

impl PmPublicCaptureRun {
    /// Reduces the exact delta/top delivery before it can enter the public
    /// lane. A lane rejection retains the already-reduced capability for the
    /// canonical Full-fault enactment path.
    pub fn reduce_then_enqueue_pm_book(
        &mut self,
        delivery: PmPublicBookDelivery,
    ) -> Result<PmBookTransition, PmPublicBookPipelineError> {
        let (transition, delivery) = self
            .reduce_pm_book_update_owned(delivery)
            .map_err(PmPublicBookPipelineError::Reduce)?;
        self.enqueue_reduced_pm_book(delivery)
            .map_err(PmPublicBookPipelineError::Lane)?;
        Ok(transition)
    }

    fn reduce_pm_book_update_owned(
        &mut self,
        delivery: PmPublicBookDelivery,
    ) -> Result<(PmBookTransition, PmPublicBookDelivery), PmPublicCaptureRunError> {
        if self.artifact_terminal() {
            return Err(PmPublicCaptureRunError::PmBookReduceRunTerminal { delivery });
        }
        if self.has_pending_pm_lane_fault() {
            return Err(PmPublicCaptureRunError::PmBookReducePendingLaneFault { delivery });
        }
        if !self.pm_lifecycle.accepts_live_input() {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            return Err(PmPublicCaptureRunError::PmBookReduceInvalidPhase { delivery });
        }
        let expected_kind = match delivery.envelope().payload().update() {
            reap_pm_core::PmBookUpdate::DeltaBatch(_) => PendingPmBookKind::DeltaBatch,
            reap_pm_core::PmBookUpdate::TopCheck(_) => PendingPmBookKind::TopCheck,
            reap_pm_core::PmBookUpdate::Snapshot(_)
            | reap_pm_core::PmBookUpdate::TickSizeChanged { .. } => PendingPmBookKind::DeltaBatch,
        };
        if !self.pending_pm_book_matches(&delivery, expected_kind) {
            return Err(PmPublicCaptureRunError::PmBookReductionOrderMismatch { delivery });
        }
        let clock = delivery.envelope().received_clock();
        match self
            .roles
            .reduce_pm_book_update(delivery, &mut self.pm_reducer)
        {
            Ok((transition, delivery)) => {
                self.consume_pending_pm_book();
                Ok((transition, delivery))
            }
            Err(PmCaptureBookReduceFailure::Reduce {
                source,
                unavailable,
            }) => {
                self.clear_pending_pm_book_reductions();
                let (terminal_pm, terminal_okx) = self.terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::SnapshotReducer,
                    reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                    reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                    clock.local_wall_receive_ns(),
                    clock.monotonic_receive_ns(),
                );
                self.terminal_pm_unavailable = terminal_pm;
                self.terminal_okx_unavailable = terminal_okx;
                Err(PmPublicCaptureRunError::PmBookReduce {
                    source,
                    unavailable,
                })
            }
            Err(PmCaptureBookReduceFailure::Route(source)) => {
                self.clear_pending_pm_book_reductions();
                let (terminal_pm, terminal_okx) = self.terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::Route,
                    reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                    reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                    clock.local_wall_receive_ns(),
                    clock.monotonic_receive_ns(),
                );
                self.terminal_pm_unavailable = terminal_pm;
                self.terminal_okx_unavailable = terminal_okx;
                Err(source.into())
            }
        }
    }

    pub(super) fn enqueue_reduced_pm_book(
        &mut self,
        delivery: PmPublicBookDelivery,
    ) -> Result<(), PmPublicLaneAdmissionError<PmPublicBookDelivery>> {
        if self.artifact_terminal() {
            return Err(PmPublicLaneAdmissionError::RunTerminal { delivery });
        }
        let envelope = delivery.envelope();
        let authority_id = delivery.authority_id();
        let source = envelope.source();
        let connection = envelope.connection_id();
        let ordering = envelope.ordering();
        if authority_id != self.roles.authority_id() {
            return Err(PmPublicLaneAdmissionError::RouteAuthorityMismatch { delivery });
        }
        if !self.pm_lifecycle.accepts_live_input()
            || !self
                .roles
                .pm_delivery_is_current(authority_id, source, connection, ordering)
        {
            return Err(PmPublicLaneAdmissionError::RouteScopeMismatch { delivery });
        }
        let pending = PendingPmBookReduction::from_delivery(&delivery);
        match self.public_lane.enqueue_pm_book(delivery) {
            Ok(()) => Ok(()),
            Err(failure) => {
                if failure.is_full() {
                    let epoch = failure.delivery().envelope().ordering().connection_epoch();
                    match self
                        .pm_reducer
                        .begin_pending_external_fault(epoch, PmExternalBookFault::Overflow)
                    {
                        Ok(reducer_fault_authority) => {
                            let pending =
                                PendingPmBookLaneFault::new(pending, reducer_fault_authority);
                            if let Err(pending) = self.register_pending_pm_book_lane_fault(pending)
                            {
                                let _ = self.pm_reducer.finalize_pending_external_fault(
                                    pending.reducer_fault_authority(),
                                    PmExternalBookFault::InvalidTransition,
                                );
                                self.terminalize_plain(
                                    PmPublicCaptureTerminalCause::InternalInvariant,
                                );
                            }
                        }
                        Err(_) => {
                            self.terminalize_plain(PmPublicCaptureTerminalCause::InternalInvariant);
                        }
                    }
                } else {
                    let clock = failure.delivery().envelope().received_clock();
                    let _ = self.roles.apply_pm_reducer_external_fault(
                        &mut self.pm_reducer,
                        PmExternalBookFault::InvalidTransition,
                        PmPublicReadinessReason::InvalidTransition,
                    );
                    let (pm_unavailable, okx_unavailable) = self.terminalize_with_receive_evidence(
                        PmPublicCaptureTerminalCause::Lane,
                        reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                        reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                        clock.local_wall_receive_ns(),
                        clock.monotonic_receive_ns(),
                    );
                    self.terminal_pm_unavailable = pm_unavailable;
                    self.terminal_okx_unavailable = okx_unavailable;
                }
                Err(PmPublicLaneAdmissionError::Lane(failure))
            }
        }
    }

    /// Records an explicit disconnect, synchronously applies the reducer
    /// fault, and admits the must-deliver unavailable occurrence.
    pub async fn record_pm_disconnected(
        &mut self,
        local_wall_receive_ns: u64,
        monotonic_ns: u64,
    ) -> Result<reap_polymarket_adapter::PmPublicSessionFault, PmPublicCaptureRunError> {
        let delivery = self
            .record_pm_disconnected_routed(local_wall_receive_ns, monotonic_ns)
            .await?;
        self.admit_pm_unavailable_or_terminal(delivery)
    }

    async fn record_pm_disconnected_routed(
        &mut self,
        local_wall_receive_ns: u64,
        monotonic_ns: u64,
    ) -> Result<PmPublicUnavailableDelivery, PmPublicCaptureRunError> {
        self.ensure_active()?;
        self.ensure_no_pending_pm_book_lane_fault()?;
        if !self.pm_lifecycle.accepts_live_input() {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            self.fail_reducer_after_terminal_write_error();
            return Err(PmPublicCaptureRunError::InvalidLifecyclePhase);
        }
        if let Err(source) = self
            .roles
            .preflight_pm_reducer_external_fault(&self.pm_reducer)
        {
            return Err(self.terminalize_reducer_sync_failure(
                source,
                local_wall_receive_ns,
                monotonic_ns,
                None,
            ));
        }
        let unavailable = match self
            .record_pm_disconnected_session(local_wall_receive_ns, monotonic_ns)
            .await
        {
            Ok(unavailable) => unavailable,
            Err(source) => {
                if self.artifact_terminal() {
                    self.fail_reducer_after_terminal_write_error();
                }
                return Err(source);
            }
        };
        match self.roles.apply_pm_reducer_external_fault(
            &mut self.pm_reducer,
            PmExternalBookFault::Disconnect,
            PmPublicReadinessReason::Disconnected,
        ) {
            Ok(_) => {
                self.clear_pending_pm_book_reductions();
                Ok(unavailable)
            }
            Err(source) => Err(self.terminalize_reducer_sync_failure(
                source,
                local_wall_receive_ns,
                monotonic_ns,
                Some(unavailable),
            )),
        }
    }

    /// Previews heartbeat state without mutation, durably records the exact
    /// lifecycle result, and only then enacts and compares the session
    /// transition. A proven timeout also invalidates the reducer atomically.
    pub async fn record_pm_heartbeat_ping_sent(
        &mut self,
        local_wall_now_ns: u64,
        monotonic_ns: u64,
    ) -> Result<(), PmPublicCaptureRunError> {
        self.ensure_active()?;
        self.ensure_no_pending_pm_book_lane_fault()?;
        if !self.pm_lifecycle.accepts_live_input() {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            return Err(PmPublicCaptureRunError::InvalidLifecyclePhase);
        }
        let preview = self.roles.preview_pm_heartbeat(monotonic_ns);
        match preview {
            Ok(PmPublicHeartbeatAction::Idle) => Err(PmPublicCaptureRunError::HeartbeatPingNotDue),
            Ok(PmPublicHeartbeatAction::SendPing) => {
                self.ensure_no_pending_pm_book_reductions()?;
                self.record_and_enact_pm_heartbeat_ping(local_wall_now_ns, monotonic_ns)
                    .await
            }
            Err(source @ PmPublicSessionError::HeartbeatTimeout { .. }) => {
                self.record_and_enact_pm_heartbeat_timeout(local_wall_now_ns, monotonic_ns, source)
                    .await
            }
            Err(source) => {
                if let Err(sync) = self
                    .roles
                    .preflight_pm_reducer_external_fault(&self.pm_reducer)
                {
                    return Err(self.terminalize_reducer_sync_failure(
                        sync,
                        local_wall_now_ns,
                        monotonic_ns,
                        None,
                    ));
                }
                self.fail_reducer_after_terminal_write_error();
                let (pm_unavailable, okx_unavailable) = self.terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::IngressSessionClassification,
                    reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                    reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                    local_wall_now_ns,
                    monotonic_ns,
                );
                self.terminal_okx_unavailable = okx_unavailable;
                Err(PmPublicCaptureRunError::PmHeartbeat {
                    source,
                    unavailable: pm_unavailable,
                })
            }
        }
    }

    async fn record_and_enact_pm_heartbeat_ping(
        &mut self,
        local_wall_now_ns: u64,
        monotonic_ns: u64,
    ) -> Result<(), PmPublicCaptureRunError> {
        if let Err(source) = self
            .roles
            .preflight_pm_reducer_external_fault(&self.pm_reducer)
        {
            return Err(self.terminalize_reducer_sync_failure(
                source,
                local_wall_now_ns,
                monotonic_ns,
                None,
            ));
        }
        let epoch = reap_pm_core::ConnectionEpoch::new(self.roles.pm_epoch());
        if let Err(source) = self.writer.preflight_pm_lifecycle(
            epoch,
            monotonic_ns,
            PmCaptureLifecycle::HeartbeatPingSent,
        ) {
            self.fail_reducer_after_terminal_write_error();
            self.terminalize_plain(PmPublicCaptureTerminalCause::CaptureWriter);
            return Err(source.into());
        }
        if let Err(source) = self
            .writer
            .record_lifecycle(epoch, monotonic_ns, PmCaptureLifecycle::HeartbeatPingSent)
            .await
        {
            self.fail_reducer_after_terminal_write_error();
            self.terminalize_plain(PmPublicCaptureTerminalCause::CaptureWriter);
            return Err(source.into());
        }
        let enacted = self
            .roles
            .poll_pm_heartbeat(local_wall_now_ns, monotonic_ns);
        if enacted != Ok(PmPublicHeartbeatAction::SendPing) {
            self.fail_reducer_after_terminal_write_error();
            let (pm_unavailable, okx_unavailable) = self.terminalize_with_receive_evidence(
                PmPublicCaptureTerminalCause::InternalInvariant,
                reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                local_wall_now_ns,
                monotonic_ns,
            );
            self.terminal_pm_unavailable = pm_unavailable;
            self.terminal_okx_unavailable = okx_unavailable;
            return Err(PmPublicCaptureRunError::HeartbeatTransitionMismatch);
        }
        Ok(())
    }

    async fn record_and_enact_pm_heartbeat_timeout(
        &mut self,
        local_wall_now_ns: u64,
        monotonic_ns: u64,
        preview_error: PmPublicSessionError,
    ) -> Result<(), PmPublicCaptureRunError> {
        if let Err(source) = self
            .roles
            .preflight_pm_reducer_external_fault(&self.pm_reducer)
        {
            return Err(self.terminalize_reducer_sync_failure(
                source,
                local_wall_now_ns,
                monotonic_ns,
                None,
            ));
        }
        let epoch = self.roles.pm_epoch();
        let event = PmCaptureLifecycle::Disconnected {
            local_wall_receive_ns: local_wall_now_ns,
            reason: PmCaptureDisconnectReason::HeartbeatTimeout,
        };
        if let Err(source) = self.writer.preflight_pm_lifecycle(
            reap_pm_core::ConnectionEpoch::new(epoch),
            monotonic_ns,
            event,
        ) {
            self.fail_reducer_after_terminal_write_error();
            self.terminalize_plain(PmPublicCaptureTerminalCause::CaptureWriter);
            return Err(source.into());
        }
        if let Err(source) = self
            .writer
            .record_lifecycle(
                reap_pm_core::ConnectionEpoch::new(epoch),
                monotonic_ns,
                event,
            )
            .await
        {
            self.fail_reducer_after_terminal_write_error();
            self.terminalize_plain(PmPublicCaptureTerminalCause::CaptureWriter);
            return Err(source.into());
        }
        let enacted = self
            .roles
            .poll_pm_heartbeat(local_wall_now_ns, monotonic_ns);
        if enacted != Err(preview_error) {
            self.fail_reducer_after_terminal_write_error();
            let (pm_unavailable, okx_unavailable) = self.terminalize_with_receive_evidence(
                PmPublicCaptureTerminalCause::InternalInvariant,
                reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                local_wall_now_ns,
                monotonic_ns,
            );
            self.terminal_pm_unavailable = pm_unavailable;
            self.terminal_okx_unavailable = okx_unavailable;
            return Err(PmPublicCaptureRunError::HeartbeatTransitionMismatch);
        }
        let unavailable = match self.roles.take_pm_unavailable() {
            Ok(Some(unavailable)) => unavailable,
            Ok(None) => {
                self.terminalize_plain(PmPublicCaptureTerminalCause::InternalInvariant);
                return Err(PmPublicCaptureRunError::MissingUnavailableOccurrence);
            }
            Err(source) => {
                self.terminalize_plain(PmPublicCaptureTerminalCause::Route);
                return Err(source.into());
            }
        };
        if let Err(source) = self.roles.apply_pm_reducer_external_fault(
            &mut self.pm_reducer,
            PmExternalBookFault::HeartbeatTimeout,
            PmPublicReadinessReason::HeartbeatTimeout,
        ) {
            return Err(self.terminalize_reducer_sync_failure(
                source,
                local_wall_now_ns,
                monotonic_ns,
                Some(unavailable),
            ));
        }
        self.clear_pending_pm_book_reductions();
        self.pm_disconnected_epoch = Some(epoch);
        self.pm_lifecycle = PublicLifecyclePhase::Disconnected;
        let _ = self.public_lane.purge_public_route(
            self.roles.authority_id(),
            self.pm_route.source(),
            self.pm_route.connection(),
            reap_pm_core::ConnectionEpoch::new(epoch),
        );
        self.admit_pm_unavailable_or_terminal(unavailable)?;
        Err(PmPublicCaptureRunError::PmHeartbeat {
            source: preview_error,
            unavailable: None,
        })
    }

    /// Advances the owned session and reducer to the same next PM epoch after
    /// a reducer-coupled disconnect or heartbeat timeout.
    pub async fn record_pm_reconnect_scheduled(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<Duration, PmPublicCaptureRunError> {
        self.ensure_active()?;
        self.ensure_no_pending_pm_book_reductions()?;
        if !self.pm_lifecycle.accepts_reconnect()
            || self.pm_disconnected_epoch != Some(self.roles.pm_epoch())
        {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            self.fail_reducer_after_terminal_write_error();
            return Err(PmPublicCaptureRunError::InvalidLifecyclePhase);
        }
        if let Err(source) = self.roles.preflight_pm_reducer_reconnect(&self.pm_reducer) {
            return Err(self.terminalize_reducer_sync_failure(source, 0, monotonic_ns, None));
        }
        let prior_epoch = self.roles.pm_epoch();
        let delay = match self
            .record_pm_reconnect_scheduled_session(monotonic_ns)
            .await
        {
            Ok(delay) => delay,
            Err(source) => {
                if self.artifact_terminal() {
                    self.fail_reducer_after_terminal_write_error();
                }
                return Err(source);
            }
        };
        let next_epoch = self.roles.pm_epoch();
        if let Err(source) =
            self.roles
                .synchronize_pm_reducer_epoch(prior_epoch, next_epoch, &mut self.pm_reducer)
        {
            return Err(self.terminalize_reducer_sync_failure(source, 0, monotonic_ns, None));
        }
        Ok(delay)
    }

    pub(super) fn terminalize_reducer_sync_failure(
        &mut self,
        source: PmPublicReducerSyncError,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
        unavailable: Option<PmPublicUnavailableDelivery>,
    ) -> PmPublicCaptureRunError {
        let (terminal_pm, terminal_okx) = if local_wall_receive_ns == 0 {
            self.terminalize_plain(PmPublicCaptureTerminalCause::SnapshotReducer);
            (None, None)
        } else {
            self.terminalize_with_receive_evidence(
                PmPublicCaptureTerminalCause::SnapshotReducer,
                reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                local_wall_receive_ns,
                monotonic_receive_ns,
            )
        };
        self.terminal_pm_unavailable = terminal_pm;
        self.terminal_okx_unavailable = terminal_okx;
        PmPublicCaptureRunError::PmReducerSync {
            source,
            unavailable,
        }
    }

    pub(super) fn fail_reducer_after_terminal_write_error(&mut self) {
        let _ = self.roles.apply_pm_reducer_external_fault(
            &mut self.pm_reducer,
            PmExternalBookFault::InvalidTransition,
            PmPublicReadinessReason::InvalidTransition,
        );
    }
}
