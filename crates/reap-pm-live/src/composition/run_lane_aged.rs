use reap_pm_core::{EventOrdering, PmConnectionId, PmProductSource, ReceivedEventClock};

use super::*;
use crate::capture::PmCaptureWriteError;
use crate::capture_roles::PmPublicLaneFaultError;
use crate::lanes::PmAgedDeliveryEvidence;
use crate::public_routes::PmPublicRouteAuthorityId;

impl PmPublicCaptureRun {
    pub async fn enact_public_lane_aged(
        &mut self,
        failure: PmServiceTurnError,
        local_wall_now_ns: u64,
        monotonic_now_ns: u64,
    ) -> Result<PmPublicAgedLaneFaultEnactment, PmPublicAgedLaneEnactError> {
        if self.artifact_terminal() {
            return Err(PmPublicAgedLaneEnactError::RunTerminal { failure });
        }
        let pending_pm_aged = matches!(
            failure.public_aged_head(),
            Some(PmPublicAgedHead::PmMetadata | PmPublicAgedHead::PmBook)
        );
        let pending_other_aged = matches!(
            failure.public_aged_head(),
            Some(PmPublicAgedHead::OkxReference)
        );
        if pending_pm_aged {
            if self.has_pending_pm_lane_fault() && self.pending_pm_aged_lane_fault.is_none() {
                return Err(PmPublicAgedLaneEnactError::PendingBookFault { failure });
            }
            let Some(evidence) = failure.public_aged_evidence() else {
                return Err(PmPublicAgedLaneEnactError::InvalidFailure { failure });
            };
            if !self.pending_pm_aged_lane_fault_matches(evidence) {
                return Err(PmPublicAgedLaneEnactError::InvalidFailure { failure });
            }
        } else if pending_other_aged {
            if self.has_pending_pm_lane_fault() && self.pending_other_aged_lane_fault.is_none() {
                return Err(PmPublicAgedLaneEnactError::PendingBookFault { failure });
            }
            let Some(evidence) = failure.public_aged_evidence() else {
                return Err(PmPublicAgedLaneEnactError::InvalidFailure { failure });
            };
            if !self.pending_other_aged_lane_fault_matches(evidence) {
                return Err(PmPublicAgedLaneEnactError::InvalidFailure { failure });
            }
        } else if self.has_pending_pm_lane_fault() {
            return Err(PmPublicAgedLaneEnactError::PendingBookFault { failure });
        }
        let pending_reductions_block = self.has_pending_pm_book_reductions()
            && matches!(
                failure.public_aged_head(),
                Some(PmPublicAgedHead::OkxReference)
            );
        if pending_reductions_block {
            return Err(PmPublicAgedLaneEnactError::PendingBookReduction { failure });
        }
        let observed_now_ns = failure
            .public_aged_evidence()
            .map(PmAgedDeliveryEvidence::observed_now_ns);
        let received_clock = failure
            .public_aged_evidence()
            .map(PmAgedDeliveryEvidence::received_clock);
        let evidence = match self
            .public_lane
            .authenticate_aged_failure(failure, monotonic_now_ns)
        {
            Ok(evidence) => evidence,
            Err(failure) => {
                let retryable_clock_argument = observed_now_ns
                    .is_some_and(|observed_now_ns| monotonic_now_ns < observed_now_ns);
                if (pending_pm_aged || pending_other_aged) && !retryable_clock_argument {
                    if pending_pm_aged
                        && self.pm_reducer.pending_external_fault()
                            == Some(PmExternalBookFault::BacklogAged)
                        && let Some(pending) = self.pending_pm_aged_lane_fault.as_ref()
                    {
                        let _ = self.pm_reducer.finalize_pending_external_fault(
                            pending.reducer_fault_authority(),
                            PmExternalBookFault::InvalidTransition,
                        );
                    }
                    if let Some(clock) = received_clock {
                        let (pm_unavailable, okx_unavailable) = self
                            .terminalize_with_receive_evidence(
                                PmPublicCaptureTerminalCause::Lane,
                                reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                                reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                                clock.local_wall_receive_ns(),
                                monotonic_now_ns,
                            );
                        self.terminal_pm_unavailable = pm_unavailable;
                        self.terminal_okx_unavailable = okx_unavailable;
                    } else {
                        self.terminalize_plain(PmPublicCaptureTerminalCause::Lane);
                    }
                    if pending_pm_aged {
                        self.clear_pending_pm_aged_lane_fault();
                    }
                    if pending_other_aged {
                        self.clear_pending_other_aged_lane_fault();
                    }
                }
                return Err(PmPublicAgedLaneEnactError::InvalidFailure { failure });
            }
        };
        let authority_id = evidence.public_authority_id();
        let source = evidence.public_source();
        let head = evidence.public_head();
        if authority_id != self.roles.authority_id() {
            return Err(PmPublicAgedLaneEnactError::EvidenceMismatch { evidence });
        }
        let route = AgedRoute {
            authority_id,
            source,
            connection: evidence.connection(),
            ordering: evidence.ordering(),
            received_clock: evidence.received_clock(),
        };
        let result = match head {
            PmPublicAgedHead::PmMetadata | PmPublicAgedHead::PmBook => {
                self.enact_fresh_pm_lane_aged(evidence, route, local_wall_now_ns, monotonic_now_ns)
                    .await
            }
            PmPublicAgedHead::OkxReference => {
                self.enact_fresh_okx_lane_aged(evidence, route, local_wall_now_ns, monotonic_now_ns)
                    .await
            }
            PmPublicAgedHead::PmUnavailable(_) | PmPublicAgedHead::OkxUnavailable(_) => {
                Err(PmPublicAgedLaneEnactError::EvidenceMismatch { evidence })
            }
            PmPublicAgedHead::PmTickSizeChanged { old, new } => self.enact_tick_lane_aged(
                evidence,
                route,
                local_wall_now_ns,
                monotonic_now_ns,
                old,
                new,
            ),
        };
        if pending_pm_aged {
            if result.is_err()
                && self.pm_reducer.pending_external_fault()
                    == Some(PmExternalBookFault::BacklogAged)
                && let Some(pending) = self.pending_pm_aged_lane_fault.as_ref()
            {
                let _ = self.pm_reducer.finalize_pending_external_fault(
                    pending.reducer_fault_authority(),
                    PmExternalBookFault::InvalidTransition,
                );
            }
            self.clear_pending_pm_aged_lane_fault();
        }
        if pending_other_aged {
            self.clear_pending_other_aged_lane_fault();
        }
        result
    }

    async fn enact_fresh_pm_lane_aged(
        &mut self,
        evidence: PmAgedDeliveryEvidence,
        route: AgedRoute,
        local_wall_now_ns: u64,
        monotonic_now_ns: u64,
    ) -> Result<PmPublicAgedLaneFaultEnactment, PmPublicAgedLaneEnactError> {
        if !self.pm_lifecycle.accepts_live_input()
            || !self.roles.pm_delivery_is_current(
                route.authority_id,
                route.source,
                route.connection,
                route.ordering,
            )
        {
            return Err(self.terminal_aged_fault(
                evidence,
                route,
                local_wall_now_ns,
                monotonic_now_ns,
                true,
                PmPublicLaneFaultError::EvidenceMismatch,
            ));
        }
        if let Err(source) = self.roles.preflight_pm_lane_fault(
            route.source,
            route.authority_id,
            route.connection,
            route.ordering,
            route.received_clock,
            local_wall_now_ns,
            monotonic_now_ns,
            &self.pm_reducer,
        ) {
            return Err(self.terminal_aged_fault(
                evidence,
                route,
                local_wall_now_ns,
                monotonic_now_ns,
                true,
                source,
            ));
        }
        let lifecycle = PmCaptureLifecycle::Disconnected {
            local_wall_receive_ns: local_wall_now_ns,
            reason: PmCaptureDisconnectReason::Stale,
        };
        if let Err(source) = self.writer.preflight_pm_lifecycle(
            route.ordering.connection_epoch(),
            monotonic_now_ns,
            lifecycle,
        ) {
            return Err(self.terminal_aged_write(
                evidence,
                route,
                local_wall_now_ns,
                monotonic_now_ns,
                source,
                true,
            ));
        }
        if let Err(source) = self
            .writer
            .record_lifecycle(
                route.ordering.connection_epoch(),
                monotonic_now_ns,
                lifecycle,
            )
            .await
        {
            return Err(self.terminal_aged_write(
                evidence,
                route,
                local_wall_now_ns,
                monotonic_now_ns,
                source,
                true,
            ));
        }
        self.pm_disconnected_epoch = Some(route.ordering.connection_epoch().value());
        self.pm_lifecycle = PublicLifecyclePhase::Disconnected;
        match self.roles.enact_pm_lane_fault(
            route.source,
            route.authority_id,
            route.connection,
            route.ordering,
            route.received_clock,
            local_wall_now_ns,
            monotonic_now_ns,
            reap_polymarket_adapter::PmPublicSessionFault::Stale,
            PmExternalBookFault::BacklogAged,
            PmPublicReadinessReason::BookStale,
            &mut self.pm_reducer,
            Some(
                self.pending_pm_aged_lane_fault
                    .as_ref()
                    .expect("exact PM aged proof retains pending reducer authority")
                    .reducer_fault_authority(),
            ),
        ) {
            Ok((unavailable, reducer_reason)) => {
                self.clear_pending_pm_book_reductions();
                let purged_queued_deliveries = purge_aged_route(&mut self.public_lane, route);
                self.clear_pending_pm_aged_lane_fault();
                let fault = match self.admit_pm_unavailable_or_terminal(unavailable) {
                    Ok(fault) => fault,
                    Err(PmPublicCaptureRunError::NotificationAdmission(failure)) => {
                        return Err(PmPublicAgedLaneEnactError::NotificationAdmission {
                            evidence,
                            failure,
                        });
                    }
                    Err(_) => unreachable!("notification admission has one typed failure"),
                };
                Ok(PmPublicAgedLaneFaultEnactment::Polymarket {
                    unavailable_fault: fault,
                    reducer_reason,
                    purged_queued_deliveries,
                })
            }
            Err(source) => Err(self.terminal_aged_fault(
                evidence,
                route,
                local_wall_now_ns,
                monotonic_now_ns,
                true,
                source,
            )),
        }
    }

    async fn enact_fresh_okx_lane_aged(
        &mut self,
        evidence: PmAgedDeliveryEvidence,
        route: AgedRoute,
        local_wall_now_ns: u64,
        monotonic_now_ns: u64,
    ) -> Result<PmPublicAgedLaneFaultEnactment, PmPublicAgedLaneEnactError> {
        if !self.okx_lifecycle.accepts_live_input()
            || !self.roles.okx_delivery_is_current(
                route.authority_id,
                route.source,
                route.connection,
                route.ordering,
            )
        {
            return Err(self.terminal_aged_fault(
                evidence,
                route,
                local_wall_now_ns,
                monotonic_now_ns,
                false,
                PmPublicLaneFaultError::EvidenceMismatch,
            ));
        }
        if let Err(source) = self.roles.preflight_okx_lane_fault(
            route.source,
            route.authority_id,
            route.connection,
            route.ordering,
            route.received_clock,
            local_wall_now_ns,
            monotonic_now_ns,
        ) {
            return Err(self.terminal_aged_fault(
                evidence,
                route,
                local_wall_now_ns,
                monotonic_now_ns,
                false,
                source,
            ));
        }
        let lifecycle = OkxCaptureLifecycle::Disconnected {
            local_wall_receive_ns: local_wall_now_ns,
            reason: OkxCaptureDisconnectReason::Stale,
        };
        if let Err(source) = self.writer.preflight_okx_lifecycle(
            route.ordering.connection_epoch().value(),
            monotonic_now_ns,
            lifecycle,
        ) {
            return Err(self.terminal_aged_write(
                evidence,
                route,
                local_wall_now_ns,
                monotonic_now_ns,
                source,
                false,
            ));
        }
        if let Err(source) = self
            .writer
            .record_okx_lifecycle(
                route.ordering.connection_epoch().value(),
                monotonic_now_ns,
                lifecycle,
            )
            .await
        {
            return Err(self.terminal_aged_write(
                evidence,
                route,
                local_wall_now_ns,
                monotonic_now_ns,
                source,
                false,
            ));
        }
        self.okx_disconnected_epoch = Some(route.ordering.connection_epoch().value());
        self.okx_lifecycle = PublicLifecyclePhase::Disconnected;
        match self.roles.enact_okx_lane_fault(
            route.source,
            route.authority_id,
            route.connection,
            route.ordering,
            route.received_clock,
            local_wall_now_ns,
            monotonic_now_ns,
            reap_okx_public_source::OkxPublicSessionFault::Stale,
        ) {
            Ok(unavailable) => {
                let purged_queued_deliveries = purge_aged_route(&mut self.public_lane, route);
                self.clear_pending_other_aged_lane_fault();
                let fault = match self.admit_okx_unavailable_or_terminal(unavailable) {
                    Ok(fault) => fault,
                    Err(PmPublicCaptureRunError::NotificationAdmission(failure)) => {
                        return Err(PmPublicAgedLaneEnactError::NotificationAdmission {
                            evidence,
                            failure,
                        });
                    }
                    Err(_) => unreachable!("notification admission has one typed failure"),
                };
                Ok(PmPublicAgedLaneFaultEnactment::Okx {
                    unavailable_fault: fault,
                    purged_queued_deliveries,
                })
            }
            Err(source) => Err(self.terminal_aged_fault(
                evidence,
                route,
                local_wall_now_ns,
                monotonic_now_ns,
                false,
                source,
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn enact_tick_lane_aged(
        &mut self,
        evidence: PmAgedDeliveryEvidence,
        route: AgedRoute,
        local_wall_now_ns: u64,
        monotonic_now_ns: u64,
        old: reap_pm_core::PmTick,
        new: reap_pm_core::PmTick,
    ) -> Result<PmPublicAgedLaneFaultEnactment, PmPublicAgedLaneEnactError> {
        if let Err(source) = self.roles.preflight_terminal_tick_size_aged(
            route.source,
            route.authority_id,
            route.connection,
            route.ordering,
            route.received_clock,
            monotonic_now_ns,
            &self.pm_reducer,
        ) {
            return Err(self.terminal_aged_fault(
                evidence,
                route,
                local_wall_now_ns,
                monotonic_now_ns,
                true,
                source,
            ));
        }
        if let Err(source) = self.roles.enact_terminal_tick_size_aged(
            route.source,
            route.authority_id,
            route.connection,
            route.ordering,
            route.received_clock,
            monotonic_now_ns,
            old,
            new,
            &mut self.pm_reducer,
        ) {
            return Err(self.terminal_aged_fault(
                evidence,
                route,
                local_wall_now_ns,
                monotonic_now_ns,
                true,
                source,
            ));
        }
        let purged_queued_deliveries = purge_aged_route(&mut self.public_lane, route);
        let (terminal_pm_unavailable, terminal_okx_unavailable) = self
            .terminalize_with_receive_evidence(
                PmPublicCaptureTerminalCause::TickSizeChanged,
                reap_polymarket_adapter::PmPublicSessionFault::TickSizeChanged,
                reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                local_wall_now_ns,
                monotonic_now_ns,
            );
        Err(PmPublicAgedLaneEnactError::TickSizeChanged {
            evidence,
            old,
            new,
            purged_queued_deliveries,
            terminal_pm_unavailable,
            terminal_okx_unavailable,
        })
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "terminalization retains exact lane, route, clock, writer, and reducer-coupling evidence"
    )]
    fn terminal_aged_write(
        &mut self,
        evidence: PmAgedDeliveryEvidence,
        route: AgedRoute,
        local_wall_now_ns: u64,
        monotonic_now_ns: u64,
        source: PmCaptureWriteError,
        invalidate_pm_reducer: bool,
    ) -> PmPublicAgedLaneEnactError {
        if invalidate_pm_reducer && self.pm_reducer.readiness().is_ready() {
            let _ = self.roles.apply_pm_reducer_external_fault(
                &mut self.pm_reducer,
                PmExternalBookFault::InvalidTransition,
                PmPublicReadinessReason::InvalidTransition,
            );
        }
        if invalidate_pm_reducer {
            self.clear_pending_pm_book_reductions();
        }
        let purged_queued_deliveries = purge_aged_route(&mut self.public_lane, route);
        let (terminal_pm_unavailable, terminal_okx_unavailable) = self
            .terminalize_with_receive_evidence(
                PmPublicCaptureTerminalCause::CaptureWriter,
                source.session_fault(),
                source.okx_session_fault(),
                local_wall_now_ns,
                monotonic_now_ns,
            );
        PmPublicAgedLaneEnactError::LifecycleWrite {
            evidence,
            source,
            purged_queued_deliveries,
            terminal_pm_unavailable,
            terminal_okx_unavailable,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn terminal_aged_fault(
        &mut self,
        evidence: PmAgedDeliveryEvidence,
        route: AgedRoute,
        local_wall_now_ns: u64,
        monotonic_now_ns: u64,
        invalidate_pm_reducer: bool,
        source: PmPublicLaneFaultError,
    ) -> PmPublicAgedLaneEnactError {
        if invalidate_pm_reducer && self.pm_reducer.readiness().is_ready() {
            let _ = self.roles.apply_pm_reducer_external_fault(
                &mut self.pm_reducer,
                PmExternalBookFault::InvalidTransition,
                PmPublicReadinessReason::InvalidTransition,
            );
        }
        if invalidate_pm_reducer {
            self.clear_pending_pm_book_reductions();
        }
        let purged_queued_deliveries = purge_aged_route(&mut self.public_lane, route);
        let (terminal_pm_unavailable, terminal_okx_unavailable) = self
            .terminalize_with_receive_evidence(
                PmPublicCaptureTerminalCause::Lane,
                reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                local_wall_now_ns,
                monotonic_now_ns,
            );
        PmPublicAgedLaneEnactError::Fault {
            evidence,
            source,
            purged_queued_deliveries,
            terminal_pm_unavailable,
            terminal_okx_unavailable,
        }
    }
}

#[derive(Clone, Copy)]
struct AgedRoute {
    authority_id: PmPublicRouteAuthorityId,
    source: PmProductSource,
    connection: PmConnectionId,
    ordering: EventOrdering,
    received_clock: ReceivedEventClock,
}

fn purge_aged_route(public_lane: &mut PmPublicLaneState, route: AgedRoute) -> usize {
    public_lane.purge_public_route(
        route.authority_id,
        route.source,
        route.connection,
        route.ordering.connection_epoch(),
    )
}
