use super::*;

#[derive(Clone, Copy)]
enum PendingPmOverflowAuthority {
    Book,
    Metadata,
}

struct EnactedFull<D, U> {
    rejected_delivery: D,
    rejected_ordering: reap_pm_core::EventOrdering,
    unavailable: U,
    reducer_reason: Option<PmPublicReadinessReason>,
    purged_queued_deliveries: usize,
}

impl PmPublicCaptureRun {
    #[allow(
        clippy::result_large_err,
        reason = "fail-closed admission must return the exact move-only routed delivery and unavailable evidence without a hidden allocation"
    )]
    pub async fn enact_pm_book_lane_failure(
        &mut self,
        failure: PmPublicLaneAdmissionError<PmPublicBookDelivery>,
        local_wall_now_ns: u64,
        monotonic_now_ns: u64,
    ) -> Result<
        PmPublicLaneFaultEnactment<reap_polymarket_adapter::PmPublicSessionFault>,
        PmPublicLaneEnactError<PmPublicBookDelivery>,
    > {
        let terminal_tick = matches!(
            &failure,
            PmPublicLaneAdmissionError::Lane(failure)
                if matches!(
                    failure.delivery().envelope().payload().update(),
                    reap_pm_core::PmBookUpdate::TickSizeChanged { .. }
                )
        );
        if terminal_tick {
            return self.enact_pm_terminal_tick_lane_failure(
                failure,
                local_wall_now_ns,
                monotonic_now_ns,
            );
        }
        if self.artifact_terminal() && self.pending_pm_book_lane_fault.is_none() {
            return Err(PmPublicLaneEnactError::RunTerminal { failure });
        }
        if !matches!(failure, PmPublicLaneAdmissionError::Lane(_)) {
            return Err(PmPublicLaneEnactError::PendingBookFaultMismatch { failure });
        }
        if !self.pending_pm_book_lane_fault_matches(failure.delivery()) {
            return Err(PmPublicLaneEnactError::PendingBookFaultMismatch { failure });
        }
        let pending_route = {
            let delivery = failure.delivery();
            let envelope = delivery.envelope();
            (
                delivery.authority_id(),
                envelope.source(),
                envelope.connection_id(),
                envelope.ordering().connection_epoch(),
            )
        };
        let result = self
            .enact_pm_lane_failure(
                failure,
                local_wall_now_ns,
                monotonic_now_ns,
                PendingPmOverflowAuthority::Book,
                |delivery| {
                    let envelope = delivery.envelope();
                    (
                        delivery.authority_id(),
                        envelope.source(),
                        envelope.connection_id(),
                        envelope.ordering(),
                        envelope.received_clock(),
                    )
                },
            )
            .await;
        if result.is_err() {
            if self.pm_reducer.pending_external_fault() == Some(PmExternalBookFault::Overflow) {
                if let Some(pending) = self.pending_pm_book_lane_fault.as_ref() {
                    let _ = self.pm_reducer.finalize_pending_external_fault(
                        pending.reducer_fault_authority(),
                        PmExternalBookFault::InvalidTransition,
                    );
                }
            } else if self.pm_reducer.readiness().is_ready() {
                let _ = self.roles.apply_pm_reducer_external_fault(
                    &mut self.pm_reducer,
                    PmExternalBookFault::InvalidTransition,
                    PmPublicReadinessReason::InvalidTransition,
                );
            }
            self.clear_pending_pm_book_reductions();
            if !self.artifact_terminal() {
                let _ = self.public_lane.purge_public_route(
                    pending_route.0,
                    pending_route.1,
                    pending_route.2,
                    pending_route.3,
                );
                let (pm_unavailable, okx_unavailable) = self.terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::Lane,
                    reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                    reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                    local_wall_now_ns,
                    monotonic_now_ns,
                );
                self.terminal_pm_unavailable = pm_unavailable;
                self.terminal_okx_unavailable = okx_unavailable;
            }
        }
        self.clear_pending_pm_book_lane_fault();
        match result {
            Ok(enacted) => {
                let fault = match self.admit_pm_unavailable_or_terminal(enacted.unavailable) {
                    Ok(fault) => fault,
                    Err(PmPublicCaptureRunError::NotificationAdmission(failure)) => {
                        return Err(PmPublicLaneEnactError::NotificationAdmission {
                            delivery: enacted.rejected_delivery,
                            failure,
                        });
                    }
                    Err(_) => unreachable!("notification admission has one typed failure"),
                };
                Ok(PmPublicLaneFaultEnactment {
                    rejected_ordering: enacted.rejected_ordering,
                    unavailable_fault: fault,
                    reducer_reason: enacted.reducer_reason,
                    purged_queued_deliveries: enacted.purged_queued_deliveries,
                })
            }
            Err(source) => Err(source),
        }
    }

    fn enact_pm_terminal_tick_lane_failure(
        &mut self,
        failure: PmPublicLaneAdmissionError<PmPublicBookDelivery>,
        local_wall_now_ns: u64,
        monotonic_now_ns: u64,
    ) -> Result<
        PmPublicLaneFaultEnactment<reap_polymarket_adapter::PmPublicSessionFault>,
        PmPublicLaneEnactError<PmPublicBookDelivery>,
    > {
        if self.artifact_terminal() {
            return Err(PmPublicLaneEnactError::RunTerminal { failure });
        }
        let PmPublicLaneAdmissionError::Lane(failure) = failure else {
            unreachable!("terminal tick detection requires a lane admission failure");
        };
        let authenticated = self
            .public_lane
            .authenticate_lane_failure(failure)
            .map_err(|failure| PmPublicLaneEnactError::LaneStateMismatch {
                failure: PmPublicLaneAdmissionError::Lane(failure),
            })?;
        let (delivery, action) = match authenticated {
            PmAuthenticatedPublicLaneFailure::Full { delivery, action } => (delivery, action),
            PmAuthenticatedPublicLaneFailure::DuplicateKey { delivery } => {
                return Err(PmPublicLaneEnactError::DuplicateKey { delivery });
            }
        };
        if action != SaturationAction::InvalidateStreamAndResync {
            return Err(PmPublicLaneEnactError::UnexpectedAction { delivery, action });
        }
        let envelope = delivery.envelope();
        let authority_id = delivery.authority_id();
        let source = envelope.source();
        let connection = envelope.connection_id();
        let ordering = envelope.ordering();
        let received_clock = envelope.received_clock();
        let reap_pm_core::PmBookUpdate::TickSizeChanged { old, new } = envelope.payload().update()
        else {
            unreachable!("guarded terminal tick delivery");
        };
        let (old, new) = (*old, *new);
        if !self.pm_lifecycle.accepts_live_input()
            || !self.roles.pm_terminal_tick_delivery_is_current(
                authority_id,
                source,
                connection,
                ordering,
            )
        {
            return Err(PmPublicLaneEnactError::RouteScopeMismatch { delivery });
        }
        if let Err(fault_error) = self.roles.preflight_terminal_tick_size_aged(
            source,
            authority_id,
            connection,
            ordering,
            received_clock,
            monotonic_now_ns,
            &self.pm_reducer,
        ) {
            let purged_queued_deliveries = self.public_lane.purge_public_route(
                authority_id,
                source,
                connection,
                ordering.connection_epoch(),
            );
            let (terminal_pm_unavailable, terminal_okx_unavailable) = self
                .terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::TickSizeChanged,
                    reap_polymarket_adapter::PmPublicSessionFault::TickSizeChanged,
                    reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                    local_wall_now_ns,
                    monotonic_now_ns,
                );
            return Err(PmPublicLaneEnactError::Fault {
                delivery,
                source: fault_error,
                purged_queued_deliveries,
                terminal_pm_unavailable,
                terminal_okx_unavailable,
            });
        }
        if let Err(fault_error) = self.roles.enact_terminal_tick_size_aged(
            source,
            authority_id,
            connection,
            ordering,
            received_clock,
            monotonic_now_ns,
            old,
            new,
            &mut self.pm_reducer,
        ) {
            let purged_queued_deliveries = self.public_lane.purge_public_route(
                authority_id,
                source,
                connection,
                ordering.connection_epoch(),
            );
            let (terminal_pm_unavailable, terminal_okx_unavailable) = self
                .terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::TickSizeChanged,
                    reap_polymarket_adapter::PmPublicSessionFault::TickSizeChanged,
                    reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                    local_wall_now_ns,
                    monotonic_now_ns,
                );
            return Err(PmPublicLaneEnactError::Fault {
                delivery,
                source: fault_error,
                purged_queued_deliveries,
                terminal_pm_unavailable,
                terminal_okx_unavailable,
            });
        }
        let purged_queued_deliveries = self.public_lane.purge_public_route(
            authority_id,
            source,
            connection,
            ordering.connection_epoch(),
        );
        let (terminal_pm_unavailable, terminal_okx_unavailable) = self
            .terminalize_with_receive_evidence(
                PmPublicCaptureTerminalCause::TickSizeChanged,
                reap_polymarket_adapter::PmPublicSessionFault::TickSizeChanged,
                reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                local_wall_now_ns,
                monotonic_now_ns,
            );
        Err(PmPublicLaneEnactError::TickSizeChanged {
            delivery,
            action,
            old,
            new,
            purged_queued_deliveries,
            terminal_pm_unavailable,
            terminal_okx_unavailable,
        })
    }

    #[allow(
        clippy::result_large_err,
        reason = "fail-closed admission must return the exact move-only routed delivery and unavailable evidence without a hidden allocation"
    )]
    pub async fn enact_pm_metadata_lane_failure(
        &mut self,
        failure: PmPublicLaneAdmissionError<PmPublicMetadataDelivery>,
        local_wall_now_ns: u64,
        monotonic_now_ns: u64,
    ) -> Result<
        PmPublicLaneFaultEnactment<reap_polymarket_adapter::PmPublicSessionFault>,
        PmPublicLaneEnactError<PmPublicMetadataDelivery>,
    > {
        if self.has_pending_pm_lane_fault() && self.pending_pm_metadata_lane_fault.is_none() {
            return Err(PmPublicLaneEnactError::PendingBookFaultBlocksMutation { failure });
        }
        if self.artifact_terminal() && self.pending_pm_metadata_lane_fault.is_none() {
            return Err(PmPublicLaneEnactError::RunTerminal { failure });
        }
        if self.has_pending_pm_book_reductions() {
            return Err(PmPublicLaneEnactError::PendingBookReductionBlocksMutation { failure });
        }
        if !matches!(failure, PmPublicLaneAdmissionError::Lane(_))
            || !self.pending_pm_metadata_lane_fault_matches(failure.delivery())
        {
            return Err(PmPublicLaneEnactError::PendingBookFaultMismatch { failure });
        }
        let pending_route = {
            let delivery = failure.delivery();
            let envelope = delivery.envelope();
            (
                delivery.authority_id(),
                envelope.source(),
                envelope.connection_id(),
                envelope.ordering().connection_epoch(),
            )
        };
        let result = self
            .enact_pm_lane_failure(
                failure,
                local_wall_now_ns,
                monotonic_now_ns,
                PendingPmOverflowAuthority::Metadata,
                |delivery| {
                    let envelope = delivery.envelope();
                    (
                        delivery.authority_id(),
                        envelope.source(),
                        envelope.connection_id(),
                        envelope.ordering(),
                        envelope.received_clock(),
                    )
                },
            )
            .await;
        if result.is_err() {
            if self.pm_reducer.pending_external_fault() == Some(PmExternalBookFault::Overflow) {
                if let Some(pending) = self.pending_pm_metadata_lane_fault.as_ref() {
                    let _ = self.pm_reducer.finalize_pending_external_fault(
                        pending.reducer_fault_authority(),
                        PmExternalBookFault::InvalidTransition,
                    );
                }
            } else if self.pm_reducer.readiness().is_ready() {
                let _ = self.roles.apply_pm_reducer_external_fault(
                    &mut self.pm_reducer,
                    PmExternalBookFault::InvalidTransition,
                    PmPublicReadinessReason::InvalidTransition,
                );
            }
            self.clear_pending_pm_book_reductions();
            if !self.artifact_terminal() {
                let _ = self.public_lane.purge_public_route(
                    pending_route.0,
                    pending_route.1,
                    pending_route.2,
                    pending_route.3,
                );
                let (pm_unavailable, okx_unavailable) = self.terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::Lane,
                    reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                    reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                    local_wall_now_ns,
                    monotonic_now_ns,
                );
                self.terminal_pm_unavailable = pm_unavailable;
                self.terminal_okx_unavailable = okx_unavailable;
            }
        }
        self.clear_pending_pm_metadata_lane_fault();
        match result {
            Ok(enacted) => {
                let fault = match self.admit_pm_unavailable_or_terminal(enacted.unavailable) {
                    Ok(fault) => fault,
                    Err(PmPublicCaptureRunError::NotificationAdmission(failure)) => {
                        return Err(PmPublicLaneEnactError::NotificationAdmission {
                            delivery: enacted.rejected_delivery,
                            failure,
                        });
                    }
                    Err(_) => unreachable!("notification admission has one typed failure"),
                };
                Ok(PmPublicLaneFaultEnactment {
                    rejected_ordering: enacted.rejected_ordering,
                    unavailable_fault: fault,
                    reducer_reason: enacted.reducer_reason,
                    purged_queued_deliveries: enacted.purged_queued_deliveries,
                })
            }
            Err(source) => Err(source),
        }
    }

    async fn enact_pm_lane_failure<D>(
        &mut self,
        failure: PmPublicLaneAdmissionError<D>,
        local_wall_now_ns: u64,
        monotonic_now_ns: u64,
        pending_overflow: PendingPmOverflowAuthority,
        evidence: impl FnOnce(
            &D,
        ) -> (
            crate::public_routes::PmPublicRouteAuthorityId,
            reap_pm_core::PmProductSource,
            reap_pm_core::PmConnectionId,
            reap_pm_core::EventOrdering,
            reap_pm_core::ReceivedEventClock,
        ),
    ) -> Result<EnactedFull<D, PmPublicUnavailableDelivery>, PmPublicLaneEnactError<D>> {
        if self.artifact_terminal() {
            return Err(PmPublicLaneEnactError::RunTerminal { failure });
        }
        let lane_failure = match failure {
            PmPublicLaneAdmissionError::RunTerminal { delivery } => {
                return Err(PmPublicLaneEnactError::RunTerminal {
                    failure: PmPublicLaneAdmissionError::RunTerminal { delivery },
                });
            }
            PmPublicLaneAdmissionError::PendingPmBookAuthority { delivery } => {
                return Err(PmPublicLaneEnactError::PendingBookReductionBlocksMutation {
                    failure: PmPublicLaneAdmissionError::PendingPmBookAuthority { delivery },
                });
            }
            PmPublicLaneAdmissionError::RouteAuthorityMismatch { delivery } => {
                return Err(PmPublicLaneEnactError::RouteAuthorityMismatch { delivery });
            }
            PmPublicLaneAdmissionError::RouteScopeMismatch { delivery } => {
                return Err(PmPublicLaneEnactError::RouteScopeMismatch { delivery });
            }
            PmPublicLaneAdmissionError::Lane(failure) => failure,
        };
        let lane_failure = self
            .public_lane
            .authenticate_lane_failure(lane_failure)
            .map_err(|failure| PmPublicLaneEnactError::LaneStateMismatch {
                failure: PmPublicLaneAdmissionError::Lane(failure),
            })?;
        let (delivery, action) = match lane_failure {
            PmAuthenticatedPublicLaneFailure::Full { delivery, action } => (delivery, action),
            PmAuthenticatedPublicLaneFailure::DuplicateKey { delivery } => {
                return Err(PmPublicLaneEnactError::DuplicateKey { delivery });
            }
        };
        if action != SaturationAction::InvalidateStreamAndResync {
            return Err(PmPublicLaneEnactError::UnexpectedAction { delivery, action });
        }
        let (authority_id, source, connection, ordering, received_clock) = evidence(&delivery);
        if !self.pm_lifecycle.accepts_live_input()
            || !self
                .roles
                .pm_delivery_is_current(authority_id, source, connection, ordering)
        {
            return Err(PmPublicLaneEnactError::RouteScopeMismatch { delivery });
        }
        let lifecycle = PmCaptureLifecycle::Disconnected {
            local_wall_receive_ns: local_wall_now_ns,
            reason: PmCaptureDisconnectReason::Overflow,
        };
        let route_source = source;
        if let Err(fault_error) = self.roles.preflight_pm_lane_fault(
            source,
            authority_id,
            connection,
            ordering,
            received_clock,
            local_wall_now_ns,
            monotonic_now_ns,
            &self.pm_reducer,
        ) {
            let purged_queued_deliveries = self.public_lane.purge_public_route(
                authority_id,
                route_source,
                connection,
                ordering.connection_epoch(),
            );
            let (terminal_pm_unavailable, terminal_okx_unavailable) = self
                .terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::Lane,
                    reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                    reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                    local_wall_now_ns,
                    monotonic_now_ns,
                );
            return Err(PmPublicLaneEnactError::Fault {
                delivery,
                source: fault_error,
                purged_queued_deliveries,
                terminal_pm_unavailable,
                terminal_okx_unavailable,
            });
        }
        let writer_preflight = self.writer.preflight_pm_lifecycle(
            ordering.connection_epoch(),
            monotonic_now_ns,
            lifecycle,
        );
        if let Err(source) = writer_preflight {
            let _ = self.roles.apply_pm_reducer_external_fault(
                &mut self.pm_reducer,
                PmExternalBookFault::InvalidTransition,
                PmPublicReadinessReason::InvalidTransition,
            );
            let purged_queued_deliveries = self.public_lane.purge_public_route(
                authority_id,
                route_source,
                connection,
                ordering.connection_epoch(),
            );
            let (terminal_pm_unavailable, terminal_okx_unavailable) = self
                .terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::CaptureWriter,
                    source.session_fault(),
                    source.okx_session_fault(),
                    local_wall_now_ns,
                    monotonic_now_ns,
                );
            return Err(PmPublicLaneEnactError::LifecycleWrite {
                delivery,
                action,
                source,
                purged_queued_deliveries,
                terminal_pm_unavailable,
                terminal_okx_unavailable,
            });
        }
        if let Err(source) = self
            .writer
            .record_lifecycle(ordering.connection_epoch(), monotonic_now_ns, lifecycle)
            .await
        {
            let _ = self.roles.apply_pm_reducer_external_fault(
                &mut self.pm_reducer,
                PmExternalBookFault::InvalidTransition,
                PmPublicReadinessReason::InvalidTransition,
            );
            let purged_queued_deliveries = self.public_lane.purge_public_route(
                authority_id,
                route_source,
                connection,
                ordering.connection_epoch(),
            );
            let (terminal_pm_unavailable, terminal_okx_unavailable) = self
                .terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::CaptureWriter,
                    source.session_fault(),
                    source.okx_session_fault(),
                    local_wall_now_ns,
                    monotonic_now_ns,
                );
            return Err(PmPublicLaneEnactError::LifecycleWrite {
                delivery,
                action,
                source,
                purged_queued_deliveries,
                terminal_pm_unavailable,
                terminal_okx_unavailable,
            });
        }
        self.pm_disconnected_epoch = Some(ordering.connection_epoch().value());
        self.pm_lifecycle = PublicLifecyclePhase::Disconnected;
        let pending_fault = match pending_overflow {
            PendingPmOverflowAuthority::Book => self
                .pending_pm_book_lane_fault
                .as_ref()
                .expect("exact PM book Full retains pending reducer authority")
                .reducer_fault_authority(),
            PendingPmOverflowAuthority::Metadata => self
                .pending_pm_metadata_lane_fault
                .as_ref()
                .expect("exact PM metadata Full retains pending reducer authority")
                .reducer_fault_authority(),
        };
        match self.roles.enact_pm_lane_fault(
            source,
            authority_id,
            connection,
            ordering,
            received_clock,
            local_wall_now_ns,
            monotonic_now_ns,
            reap_polymarket_adapter::PmPublicSessionFault::Overflow,
            PmExternalBookFault::Overflow,
            PmPublicReadinessReason::Overflow,
            &mut self.pm_reducer,
            Some(pending_fault),
        ) {
            Ok((unavailable, reducer_reason)) => {
                self.clear_pending_pm_book_reductions();
                let purged_queued_deliveries = self.public_lane.purge_public_route(
                    authority_id,
                    source,
                    connection,
                    ordering.connection_epoch(),
                );
                Ok(EnactedFull {
                    rejected_ordering: ordering,
                    rejected_delivery: delivery,
                    unavailable,
                    reducer_reason: Some(reducer_reason),
                    purged_queued_deliveries,
                })
            }
            Err(fault_error) => {
                let purged_queued_deliveries = self.public_lane.purge_public_route(
                    authority_id,
                    source,
                    connection,
                    ordering.connection_epoch(),
                );
                let (terminal_pm_unavailable, terminal_okx_unavailable) = self
                    .terminalize_with_receive_evidence(
                        PmPublicCaptureTerminalCause::Lane,
                        reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                        reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                        local_wall_now_ns,
                        monotonic_now_ns,
                    );
                Err(PmPublicLaneEnactError::Fault {
                    delivery,
                    source: fault_error,
                    purged_queued_deliveries,
                    terminal_pm_unavailable,
                    terminal_okx_unavailable,
                })
            }
        }
    }

    #[allow(
        clippy::result_large_err,
        reason = "fail-closed admission must return the exact move-only routed delivery and unavailable evidence without a hidden allocation"
    )]
    pub async fn enact_okx_reference_lane_failure(
        &mut self,
        failure: PmPublicLaneAdmissionError<OkxPublicReferenceDelivery>,
        local_wall_now_ns: u64,
        monotonic_now_ns: u64,
    ) -> Result<
        PmPublicLaneFaultEnactment<reap_okx_public_source::OkxPublicSessionFault>,
        PmPublicLaneEnactError<OkxPublicReferenceDelivery>,
    > {
        if self.has_pending_pm_lane_fault() && self.pending_okx_reference_lane_fault.is_none() {
            return Err(PmPublicLaneEnactError::PendingBookFaultBlocksMutation { failure });
        }
        if self.artifact_terminal() && self.pending_okx_reference_lane_fault.is_none() {
            return Err(PmPublicLaneEnactError::RunTerminal { failure });
        }
        if !matches!(failure, PmPublicLaneAdmissionError::Lane(_))
            || !self.pending_okx_reference_lane_fault_matches(failure.delivery())
        {
            return Err(PmPublicLaneEnactError::PendingBookFaultMismatch { failure });
        }
        if self.has_pending_pm_book_reductions() {
            return Err(PmPublicLaneEnactError::PendingBookReductionBlocksMutation { failure });
        }
        if self.artifact_terminal() {
            return Err(PmPublicLaneEnactError::RunTerminal { failure });
        }
        let lane_failure = match failure {
            PmPublicLaneAdmissionError::RunTerminal { delivery } => {
                return Err(PmPublicLaneEnactError::RunTerminal {
                    failure: PmPublicLaneAdmissionError::RunTerminal { delivery },
                });
            }
            PmPublicLaneAdmissionError::PendingPmBookAuthority { delivery } => {
                return Err(PmPublicLaneEnactError::PendingBookReductionBlocksMutation {
                    failure: PmPublicLaneAdmissionError::PendingPmBookAuthority { delivery },
                });
            }
            PmPublicLaneAdmissionError::RouteAuthorityMismatch { delivery } => {
                return Err(PmPublicLaneEnactError::RouteAuthorityMismatch { delivery });
            }
            PmPublicLaneAdmissionError::RouteScopeMismatch { delivery } => {
                return Err(PmPublicLaneEnactError::RouteScopeMismatch { delivery });
            }
            PmPublicLaneAdmissionError::Lane(failure) => failure,
        };
        let lane_failure = self
            .public_lane
            .authenticate_lane_failure(lane_failure)
            .map_err(|failure| PmPublicLaneEnactError::LaneStateMismatch {
                failure: PmPublicLaneAdmissionError::Lane(failure),
            })?;
        let (delivery, action) = match lane_failure {
            PmAuthenticatedPublicLaneFailure::Full { delivery, action } => (delivery, action),
            PmAuthenticatedPublicLaneFailure::DuplicateKey { delivery } => {
                return Err(PmPublicLaneEnactError::DuplicateKey { delivery });
            }
        };
        if action != SaturationAction::InvalidateStreamAndResync {
            self.terminalize_plain(PmPublicCaptureTerminalCause::InternalInvariant);
            self.clear_pending_okx_reference_lane_fault();
            return Err(PmPublicLaneEnactError::UnexpectedAction { delivery, action });
        }
        let envelope = delivery.envelope();
        let authority_id = delivery.authority_id();
        let source = envelope.source();
        let connection = envelope.connection_id();
        let ordering = envelope.ordering();
        let received_clock = envelope.received_clock();
        if !self.okx_lifecycle.accepts_live_input()
            || !self
                .roles
                .okx_delivery_is_current(authority_id, source, connection, ordering)
        {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lane);
            self.clear_pending_okx_reference_lane_fault();
            return Err(PmPublicLaneEnactError::RouteScopeMismatch { delivery });
        }
        let lifecycle = OkxCaptureLifecycle::Disconnected {
            local_wall_receive_ns: local_wall_now_ns,
            reason: OkxCaptureDisconnectReason::Overflow,
        };
        if let Err(fault_error) = self.roles.preflight_okx_lane_fault(
            source,
            authority_id,
            connection,
            ordering,
            received_clock,
            local_wall_now_ns,
            monotonic_now_ns,
        ) {
            let purged_queued_deliveries = self.public_lane.purge_public_route(
                authority_id,
                source,
                connection,
                ordering.connection_epoch(),
            );
            let (terminal_pm_unavailable, terminal_okx_unavailable) = self
                .terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::Lane,
                    reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                    reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                    local_wall_now_ns,
                    monotonic_now_ns,
                );
            self.clear_pending_okx_reference_lane_fault();
            return Err(PmPublicLaneEnactError::Fault {
                delivery,
                source: fault_error,
                purged_queued_deliveries,
                terminal_pm_unavailable,
                terminal_okx_unavailable,
            });
        }
        if let Err(write_error) = self.writer.preflight_okx_lifecycle(
            ordering.connection_epoch().value(),
            monotonic_now_ns,
            lifecycle,
        ) {
            let purged_queued_deliveries = self.public_lane.purge_public_route(
                authority_id,
                source,
                connection,
                ordering.connection_epoch(),
            );
            let (terminal_pm_unavailable, terminal_okx_unavailable) = self
                .terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::CaptureWriter,
                    write_error.session_fault(),
                    write_error.okx_session_fault(),
                    local_wall_now_ns,
                    monotonic_now_ns,
                );
            self.clear_pending_okx_reference_lane_fault();
            return Err(PmPublicLaneEnactError::LifecycleWrite {
                delivery,
                action,
                source: write_error,
                purged_queued_deliveries,
                terminal_pm_unavailable,
                terminal_okx_unavailable,
            });
        }
        if let Err(write_error) = self
            .writer
            .record_okx_lifecycle(
                ordering.connection_epoch().value(),
                monotonic_now_ns,
                lifecycle,
            )
            .await
        {
            let purged_queued_deliveries = self.public_lane.purge_public_route(
                authority_id,
                source,
                connection,
                ordering.connection_epoch(),
            );
            let (terminal_pm_unavailable, terminal_okx_unavailable) = self
                .terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::CaptureWriter,
                    write_error.session_fault(),
                    write_error.okx_session_fault(),
                    local_wall_now_ns,
                    monotonic_now_ns,
                );
            self.clear_pending_okx_reference_lane_fault();
            return Err(PmPublicLaneEnactError::LifecycleWrite {
                delivery,
                action,
                source: write_error,
                purged_queued_deliveries,
                terminal_pm_unavailable,
                terminal_okx_unavailable,
            });
        }
        self.okx_disconnected_epoch = Some(ordering.connection_epoch().value());
        self.okx_lifecycle = PublicLifecyclePhase::Disconnected;
        let result = self.roles.enact_okx_lane_fault(
            source,
            authority_id,
            connection,
            ordering,
            received_clock,
            local_wall_now_ns,
            monotonic_now_ns,
            reap_okx_public_source::OkxPublicSessionFault::Overflow,
        );
        match result {
            Ok(unavailable) => {
                let purged_queued_deliveries = self.public_lane.purge_public_route(
                    authority_id,
                    source,
                    connection,
                    ordering.connection_epoch(),
                );
                self.clear_pending_okx_reference_lane_fault();
                let fault = match self.admit_okx_unavailable_or_terminal(unavailable) {
                    Ok(fault) => fault,
                    Err(PmPublicCaptureRunError::NotificationAdmission(failure)) => {
                        return Err(PmPublicLaneEnactError::NotificationAdmission {
                            delivery,
                            failure,
                        });
                    }
                    Err(_) => unreachable!("notification admission has one typed failure"),
                };
                Ok(PmPublicLaneFaultEnactment {
                    rejected_ordering: ordering,
                    unavailable_fault: fault,
                    reducer_reason: None,
                    purged_queued_deliveries,
                })
            }
            Err(fault_error) => {
                let purged_queued_deliveries = self.public_lane.purge_public_route(
                    authority_id,
                    source,
                    connection,
                    ordering.connection_epoch(),
                );
                let (terminal_pm_unavailable, terminal_okx_unavailable) = self
                    .terminalize_with_receive_evidence(
                        PmPublicCaptureTerminalCause::Lane,
                        reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                        reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                        local_wall_now_ns,
                        monotonic_now_ns,
                    );
                self.clear_pending_okx_reference_lane_fault();
                Err(PmPublicLaneEnactError::Fault {
                    delivery,
                    source: fault_error,
                    purged_queued_deliveries,
                    terminal_pm_unavailable,
                    terminal_okx_unavailable,
                })
            }
        }
    }

    /// Enacts a Full proof for an unavailable occurrence that was already
    /// durably recorded and applied to the PM session and reducer.
    ///
    /// This consumes the exact lane proof, verifies the original typed fault
    /// and its already-applied reducer reason/counters, and purges the exact
    /// route. It deliberately emits no second lifecycle record or unavailable
    /// occurrence and does not reapply the reducer fault.
    #[allow(
        clippy::result_large_err,
        reason = "fail-closed admission must return the exact move-only routed delivery and terminal evidence without a hidden allocation"
    )]
    pub fn enact_pm_unavailable_lane_failure(
        &mut self,
        failure: PmPublicLaneAdmissionError<PmPublicUnavailableDelivery>,
        monotonic_now_ns: u64,
    ) -> Result<
        PmPublicLaneFaultEnactment<reap_polymarket_adapter::PmPublicSessionFault>,
        PmPublicLaneEnactError<PmPublicUnavailableDelivery>,
    > {
        if self.has_pending_pm_lane_fault() && self.pending_pm_unavailable_lane_fault.is_none() {
            return Err(PmPublicLaneEnactError::PendingBookFaultBlocksMutation { failure });
        }
        if self.artifact_terminal() && self.pending_pm_unavailable_lane_fault.is_none() {
            return Err(PmPublicLaneEnactError::RunTerminal { failure });
        }
        if !matches!(failure, PmPublicLaneAdmissionError::Lane(_))
            || !self.pending_pm_unavailable_lane_fault_matches(failure.delivery())
        {
            return Err(PmPublicLaneEnactError::PendingBookFaultMismatch { failure });
        }
        if self.has_pending_pm_book_reductions() {
            return Err(PmPublicLaneEnactError::PendingBookReductionBlocksMutation { failure });
        }
        if self.artifact_terminal() {
            return Err(PmPublicLaneEnactError::RunTerminal { failure });
        }
        let (delivery, action) = authenticate_full_failure(&self.public_lane, failure)?;
        if action != SaturationAction::InvalidateStreamAndResync {
            self.terminalize_plain(PmPublicCaptureTerminalCause::InternalInvariant);
            self.clear_pending_pm_unavailable_lane_fault();
            return Err(PmPublicLaneEnactError::UnexpectedAction { delivery, action });
        }
        let envelope = delivery.envelope();
        let authority_id = delivery.authority_id();
        let source = envelope.source();
        let connection = envelope.connection_id();
        let ordering = envelope.ordering();
        let received_clock = envelope.received_clock();
        let existing_fault = envelope.payload().fault();
        if !matches!(self.pm_lifecycle, PublicLifecyclePhase::Disconnected)
            || self.pm_disconnected_epoch != Some(ordering.connection_epoch().value())
            || !self.roles.pm_unavailable_delivery_is_current(
                authority_id,
                source,
                connection,
                ordering,
                existing_fault,
            )
        {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lane);
            self.clear_pending_pm_unavailable_lane_fault();
            return Err(PmPublicLaneEnactError::RouteScopeMismatch { delivery });
        }
        let (reducer_fault, expected_reason) = pm_unavailable_reducer_fault(existing_fault);
        match self.roles.enact_already_unavailable_pm_lane_fault(
            source,
            authority_id,
            connection,
            ordering,
            received_clock,
            monotonic_now_ns,
            existing_fault,
            reducer_fault,
            expected_reason,
            &mut self.pm_reducer,
        ) {
            Ok(reducer_reason) => {
                let purged_queued_deliveries = self.public_lane.purge_public_route(
                    authority_id,
                    source,
                    connection,
                    ordering.connection_epoch(),
                );
                self.clear_pending_pm_unavailable_lane_fault();
                let fault = match self.admit_pm_unavailable_or_terminal(delivery) {
                    Ok(fault) => fault,
                    Err(PmPublicCaptureRunError::NotificationAdmission(failure)) => {
                        return Err(PmPublicLaneEnactError::NotificationAdmissionTerminal {
                            failure,
                        });
                    }
                    Err(_) => unreachable!("notification admission has one typed failure"),
                };
                Ok(PmPublicLaneFaultEnactment {
                    rejected_ordering: ordering,
                    unavailable_fault: fault,
                    reducer_reason: Some(reducer_reason),
                    purged_queued_deliveries,
                })
            }
            Err(source_error) => {
                let purged_queued_deliveries = self.public_lane.purge_public_route(
                    authority_id,
                    source,
                    connection,
                    ordering.connection_epoch(),
                );
                let (terminal_pm_unavailable, terminal_okx_unavailable) = self
                    .terminalize_with_receive_evidence(
                        PmPublicCaptureTerminalCause::Lane,
                        reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                        reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                        received_clock.local_wall_receive_ns(),
                        monotonic_now_ns,
                    );
                self.clear_pending_pm_unavailable_lane_fault();
                Err(PmPublicLaneEnactError::Fault {
                    delivery,
                    source: source_error,
                    purged_queued_deliveries,
                    terminal_pm_unavailable,
                    terminal_okx_unavailable,
                })
            }
        }
    }

    /// Enacts a Full proof for an unavailable occurrence that was already
    /// durably recorded and applied to the OKX session.
    ///
    /// The exact original fault is retained on the rejected delivery. No
    /// second lifecycle record or unavailable occurrence is emitted.
    #[allow(
        clippy::result_large_err,
        reason = "fail-closed admission must return the exact move-only routed delivery and terminal evidence without a hidden allocation"
    )]
    pub fn enact_okx_unavailable_lane_failure(
        &mut self,
        failure: PmPublicLaneAdmissionError<OkxPublicUnavailableDelivery>,
        monotonic_now_ns: u64,
    ) -> Result<
        PmPublicLaneFaultEnactment<reap_okx_public_source::OkxPublicSessionFault>,
        PmPublicLaneEnactError<OkxPublicUnavailableDelivery>,
    > {
        if self.has_pending_pm_lane_fault() && self.pending_okx_unavailable_lane_fault.is_none() {
            return Err(PmPublicLaneEnactError::PendingBookFaultBlocksMutation { failure });
        }
        if self.artifact_terminal() && self.pending_okx_unavailable_lane_fault.is_none() {
            return Err(PmPublicLaneEnactError::RunTerminal { failure });
        }
        if !matches!(failure, PmPublicLaneAdmissionError::Lane(_))
            || !self.pending_okx_unavailable_lane_fault_matches(failure.delivery())
        {
            return Err(PmPublicLaneEnactError::PendingBookFaultMismatch { failure });
        }
        if self.has_pending_pm_book_reductions() {
            return Err(PmPublicLaneEnactError::PendingBookReductionBlocksMutation { failure });
        }
        if self.artifact_terminal() {
            return Err(PmPublicLaneEnactError::RunTerminal { failure });
        }
        let (delivery, action) = authenticate_full_failure(&self.public_lane, failure)?;
        if action != SaturationAction::InvalidateStreamAndResync {
            self.terminalize_plain(PmPublicCaptureTerminalCause::InternalInvariant);
            self.clear_pending_okx_unavailable_lane_fault();
            return Err(PmPublicLaneEnactError::UnexpectedAction { delivery, action });
        }
        let envelope = delivery.envelope();
        let authority_id = delivery.authority_id();
        let source = envelope.source();
        let connection = envelope.connection_id();
        let ordering = envelope.ordering();
        let received_clock = envelope.received_clock();
        let existing_fault = envelope.payload().fault();
        if !matches!(self.okx_lifecycle, PublicLifecyclePhase::Disconnected)
            || self.okx_disconnected_epoch != Some(ordering.connection_epoch().value())
            || !self.roles.okx_unavailable_delivery_is_current(
                authority_id,
                source,
                connection,
                ordering,
                existing_fault,
            )
        {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lane);
            self.clear_pending_okx_unavailable_lane_fault();
            return Err(PmPublicLaneEnactError::RouteScopeMismatch { delivery });
        }
        match self.roles.validate_already_unavailable_okx_lane_fault(
            source,
            authority_id,
            connection,
            ordering,
            received_clock,
            monotonic_now_ns,
            existing_fault,
        ) {
            Ok(()) => {
                let purged_queued_deliveries = self.public_lane.purge_public_route(
                    authority_id,
                    source,
                    connection,
                    ordering.connection_epoch(),
                );
                self.clear_pending_okx_unavailable_lane_fault();
                let fault = match self.admit_okx_unavailable_or_terminal(delivery) {
                    Ok(fault) => fault,
                    Err(PmPublicCaptureRunError::NotificationAdmission(failure)) => {
                        return Err(PmPublicLaneEnactError::NotificationAdmissionTerminal {
                            failure,
                        });
                    }
                    Err(_) => unreachable!("notification admission has one typed failure"),
                };
                Ok(PmPublicLaneFaultEnactment {
                    rejected_ordering: ordering,
                    unavailable_fault: fault,
                    reducer_reason: None,
                    purged_queued_deliveries,
                })
            }
            Err(source_error) => {
                let purged_queued_deliveries = self.public_lane.purge_public_route(
                    authority_id,
                    source,
                    connection,
                    ordering.connection_epoch(),
                );
                let (terminal_pm_unavailable, terminal_okx_unavailable) = self
                    .terminalize_with_receive_evidence(
                        PmPublicCaptureTerminalCause::Lane,
                        reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                        reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                        received_clock.local_wall_receive_ns(),
                        monotonic_now_ns,
                    );
                self.clear_pending_okx_unavailable_lane_fault();
                Err(PmPublicLaneEnactError::Fault {
                    delivery,
                    source: source_error,
                    purged_queued_deliveries,
                    terminal_pm_unavailable,
                    terminal_okx_unavailable,
                })
            }
        }
    }
}

fn authenticate_full_failure<D>(
    public_lane: &PmPublicLaneState,
    failure: PmPublicLaneAdmissionError<D>,
) -> Result<(D, SaturationAction), PmPublicLaneEnactError<D>> {
    let lane_failure = match failure {
        PmPublicLaneAdmissionError::RunTerminal { delivery } => {
            return Err(PmPublicLaneEnactError::RunTerminal {
                failure: PmPublicLaneAdmissionError::RunTerminal { delivery },
            });
        }
        PmPublicLaneAdmissionError::PendingPmBookAuthority { delivery } => {
            return Err(PmPublicLaneEnactError::PendingBookReductionBlocksMutation {
                failure: PmPublicLaneAdmissionError::PendingPmBookAuthority { delivery },
            });
        }
        PmPublicLaneAdmissionError::RouteAuthorityMismatch { delivery } => {
            return Err(PmPublicLaneEnactError::RouteAuthorityMismatch { delivery });
        }
        PmPublicLaneAdmissionError::RouteScopeMismatch { delivery } => {
            return Err(PmPublicLaneEnactError::RouteScopeMismatch { delivery });
        }
        PmPublicLaneAdmissionError::Lane(failure) => failure,
    };
    match public_lane
        .authenticate_lane_failure(lane_failure)
        .map_err(|failure| PmPublicLaneEnactError::LaneStateMismatch {
            failure: PmPublicLaneAdmissionError::Lane(failure),
        })? {
        PmAuthenticatedPublicLaneFailure::Full { delivery, action } => Ok((delivery, action)),
        PmAuthenticatedPublicLaneFailure::DuplicateKey { delivery } => {
            Err(PmPublicLaneEnactError::DuplicateKey { delivery })
        }
    }
}
