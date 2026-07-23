use super::*;

impl PmPublicCaptureRun {
    /// Services the bounded public lane through its active owning Run. If the exact
    /// PM public head is stale, the bound reducer becomes synchronously
    /// unavailable before the evidenced age failure is returned.
    pub fn service_lane_turn<C: PmPublicLaneService>(
        &mut self,
        now_ns: u64,
        consumer: &mut C,
    ) -> Result<usize, PmServiceTurnError> {
        if self.artifact_terminal()
            || self.has_pending_pm_lane_fault()
            || self.has_pending_pm_book_reductions()
        {
            return Err(PmServiceTurnError::PublicRunUnavailable);
        }
        let result = self.public_lane.service_turn(now_ns, consumer);
        let Some(evidence) = result
            .as_ref()
            .err()
            .and_then(|failure| failure.public_aged_evidence())
        else {
            return result;
        };
        if !self.public_aged_evidence_is_current(evidence) {
            let _ = self.public_lane.purge_public_route(
                evidence.public_authority_id(),
                evidence.public_source(),
                evidence.connection(),
                evidence.ordering().connection_epoch(),
            );
            self.terminalize_plain(PmPublicCaptureTerminalCause::InternalInvariant);
            return result;
        }

        match evidence.public_head() {
            PmPublicAgedHead::PmMetadata | PmPublicAgedHead::PmBook => {
                let epoch = evidence.ordering().connection_epoch();
                match self
                    .pm_reducer
                    .begin_pending_external_fault(epoch, PmExternalBookFault::BacklogAged)
                {
                    Ok(reducer_fault_authority) => {
                        let pending =
                            PendingPmAgedLaneFault::new(evidence, reducer_fault_authority);
                        if let Err(pending) = self.register_pending_pm_aged_lane_fault(pending) {
                            let _ = self.pm_reducer.finalize_pending_external_fault(
                                pending.reducer_fault_authority(),
                                PmExternalBookFault::InvalidTransition,
                            );
                            self.terminalize_plain(PmPublicCaptureTerminalCause::InternalInvariant);
                        }
                    }
                    Err(_) => {
                        self.terminalize_plain(PmPublicCaptureTerminalCause::InternalInvariant);
                    }
                }
            }
            PmPublicAgedHead::OkxReference => {
                self.pending_other_aged_lane_fault = Some(PendingOtherAgedLaneFault::new(evidence));
            }
            PmPublicAgedHead::PmUnavailable(_) | PmPublicAgedHead::OkxUnavailable(_) => {
                self.terminalize_plain(PmPublicCaptureTerminalCause::InternalInvariant);
            }
            PmPublicAgedHead::PmTickSizeChanged { .. } => {
                // Tick changes terminalize during capture, before they can
                // become a serviceable public head. Reaching this branch means
                // the Run and lane escaped their terminal cleanup contract.
                self.terminalize_plain(PmPublicCaptureTerminalCause::TickSizeChanged);
            }
        }
        result
    }

    fn public_aged_evidence_is_current(&self, evidence: &PmAgedDeliveryEvidence) -> bool {
        let authority_id = evidence.public_authority_id();
        let source = evidence.public_source();
        let head = evidence.public_head();
        let connection = evidence.connection();
        let ordering = evidence.ordering();

        match head {
            PmPublicAgedHead::PmMetadata | PmPublicAgedHead::PmBook => {
                self.pm_lifecycle.accepts_live_input()
                    && self
                        .roles
                        .pm_delivery_is_current(authority_id, source, connection, ordering)
            }
            PmPublicAgedHead::OkxReference => {
                self.okx_lifecycle.accepts_live_input()
                    && self.roles.okx_delivery_is_current(
                        authority_id,
                        source,
                        connection,
                        ordering,
                    )
            }
            PmPublicAgedHead::PmUnavailable(fault) => {
                matches!(self.pm_lifecycle, PublicLifecyclePhase::Disconnected)
                    && self.pm_disconnected_epoch == Some(ordering.connection_epoch().value())
                    && self.roles.pm_unavailable_delivery_is_current(
                        authority_id,
                        source,
                        connection,
                        ordering,
                        fault,
                    )
            }
            PmPublicAgedHead::OkxUnavailable(fault) => {
                matches!(self.okx_lifecycle, PublicLifecyclePhase::Disconnected)
                    && self.okx_disconnected_epoch == Some(ordering.connection_epoch().value())
                    && self.roles.okx_unavailable_delivery_is_current(
                        authority_id,
                        source,
                        connection,
                        ordering,
                        fault,
                    )
            }
            PmPublicAgedHead::PmTickSizeChanged { .. } => self
                .roles
                .pm_terminal_tick_delivery_is_current(authority_id, source, connection, ordering),
        }
    }
}
