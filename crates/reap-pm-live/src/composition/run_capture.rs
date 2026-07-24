use super::*;

impl PmPublicCaptureRun {
    pub async fn capture_pm_public(
        &mut self,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
        raw: &[u8],
    ) -> Result<PmPublicCaptureBatch, PmPublicCaptureRunError> {
        self.ensure_active()?;
        self.ensure_no_pending_pm_book_reductions()?;
        if !self.pm_lifecycle.accepts_live_input() {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            return Err(PmPublicCaptureRunError::InvalidLifecyclePhase);
        }
        let epoch = self.roles.pm_epoch();
        let ingress = match self.pm_raw_ingress.next(epoch) {
            Ok(ingress) => ingress,
            Err(source) => {
                self.terminalize_plain(PmPublicCaptureTerminalCause::IngressSessionClassification);
                return Err(source);
            }
        };
        if let Err(source) = self
            .writer
            .capture_raw_before_parse(
                reap_pm_core::ConnectionEpoch::new(epoch),
                IngressSequence::new(ingress),
                local_wall_receive_ns,
                monotonic_receive_ns,
                raw,
            )
            .await
        {
            return Err(self.reject_pm_capture_write(
                source,
                local_wall_receive_ns,
                monotonic_receive_ns,
            ));
        }
        self.pm_raw_ingress.commit(epoch, ingress);
        match self
            .roles
            .classify_and_route_pm(raw, local_wall_receive_ns, monotonic_receive_ns)
        {
            Ok(batch) => {
                self.register_pm_book_reductions(&batch)?;
                let terminal_tick_clock = batch.books().iter().find_map(|delivery| {
                    matches!(
                        delivery.envelope().payload().update(),
                        reap_pm_core::PmBookUpdate::TickSizeChanged { .. }
                    )
                    .then(|| delivery.envelope().received_clock())
                });
                if let Some(clock) = terminal_tick_clock {
                    let (pm_unavailable, okx_unavailable) = self.terminalize_with_receive_evidence(
                        PmPublicCaptureTerminalCause::TickSizeChanged,
                        reap_polymarket_adapter::PmPublicSessionFault::TickSizeChanged,
                        reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                        clock.local_wall_receive_ns(),
                        clock.monotonic_receive_ns(),
                    );
                    // The PM session already issued its one exact unavailable
                    // occurrence into `batch`; terminalizing the sibling OKX
                    // session must not synthesize a second PM occurrence.
                    self.terminal_pm_unavailable = pm_unavailable;
                    self.terminal_okx_unavailable = okx_unavailable;
                }
                Ok(batch)
            }
            Err(PmCaptureRoleIngressError::PmSession(source)) => {
                let (unavailable, okx_unavailable) = self.terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::IngressSessionClassification,
                    reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                    reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                    local_wall_receive_ns,
                    monotonic_receive_ns,
                );
                self.terminal_okx_unavailable = okx_unavailable;
                Err(PmPublicCaptureRunError::PmClassify {
                    source,
                    unavailable,
                })
            }
            Err(PmCaptureRoleIngressError::Route(source)) => {
                let (pm_unavailable, okx_unavailable) = self.terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::Route,
                    reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                    reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                    local_wall_receive_ns,
                    monotonic_receive_ns,
                );
                self.terminal_pm_unavailable = pm_unavailable;
                self.terminal_okx_unavailable = okx_unavailable;
                Err(source.into())
            }
            Err(
                PmCaptureRoleIngressError::OkxSession(_)
                | PmCaptureRoleIngressError::OkxRawNotUtf8 { .. },
            ) => {
                self.terminalize_plain(PmPublicCaptureTerminalCause::InternalInvariant);
                Err(PmPublicCaptureRunError::InternalRoleMismatch)
            }
        }
    }

    fn reject_pm_capture_write(
        &mut self,
        source: PmCaptureWriteError,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> PmPublicCaptureRunError {
        let (unavailable, okx_unavailable) = self.terminalize_with_receive_evidence(
            PmPublicCaptureTerminalCause::CaptureWriter,
            source.session_fault(),
            source.okx_session_fault(),
            local_wall_receive_ns,
            monotonic_receive_ns,
        );
        self.terminal_pm_unavailable = unavailable;
        self.terminal_okx_unavailable = okx_unavailable;
        PmPublicCaptureRunError::PmCaptureRejected {
            source,
            unavailable: self.terminal_pm_unavailable.take(),
        }
    }

    #[cfg(test)]
    pub(crate) fn phase6_reject_pm_capture_write_for_evidence(
        &mut self,
        source: PmCaptureWriteError,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
    ) -> PmPublicCaptureRunError {
        self.reject_pm_capture_write(source, local_wall_receive_ns, monotonic_receive_ns)
    }

    /// Captures, authenticates, and admits an OKX reference without exposing
    /// an unqueued route delivery.
    pub async fn capture_okx_public(
        &mut self,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
        raw: &[u8],
    ) -> Result<OkxPublicCaptureEvent, PmPublicDataPipelineError<OkxPublicReferenceDelivery>> {
        let event = self
            .capture_okx_public_routed(local_wall_receive_ns, monotonic_receive_ns, raw)
            .await
            .map_err(PmPublicDataPipelineError::Run)?;
        match event {
            OkxCaptureRoleEvent::SubscriptionAcknowledged(evidence) => {
                Ok(OkxPublicCaptureEvent::SubscriptionAcknowledged(evidence))
            }
            OkxCaptureRoleEvent::Heartbeat(evidence) => {
                Ok(OkxPublicCaptureEvent::Heartbeat(evidence))
            }
            OkxCaptureRoleEvent::Control(evidence) => Ok(OkxPublicCaptureEvent::Control(evidence)),
            OkxCaptureRoleEvent::Reference(delivery) => {
                self.enqueue_okx_reference(delivery)
                    .map_err(PmPublicDataPipelineError::Lane)?;
                Ok(OkxPublicCaptureEvent::ReferenceEnqueued)
            }
        }
    }

    async fn capture_okx_public_routed(
        &mut self,
        local_wall_receive_ns: u64,
        monotonic_receive_ns: u64,
        raw: &[u8],
    ) -> Result<OkxCaptureRoleEvent, PmPublicCaptureRunError> {
        self.ensure_active()?;
        self.ensure_no_pending_pm_book_reductions()?;
        if !self.okx_lifecycle.accepts_live_input() {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            return Err(PmPublicCaptureRunError::InvalidLifecyclePhase);
        }
        let epoch = self.roles.okx_epoch();
        let ingress = match self.okx_raw_ingress.next(epoch) {
            Ok(ingress) => ingress,
            Err(source) => {
                self.terminalize_plain(PmPublicCaptureTerminalCause::IngressSessionClassification);
                return Err(source);
            }
        };
        let raw_hash = match self
            .writer
            .capture_okx_raw_before_parse(
                epoch,
                ingress,
                local_wall_receive_ns,
                monotonic_receive_ns,
                raw,
            )
            .await
        {
            Ok(raw_hash) => raw_hash,
            Err(source) => {
                let (pm_unavailable, unavailable) = self.terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::CaptureWriter,
                    source.session_fault(),
                    source.okx_session_fault(),
                    local_wall_receive_ns,
                    monotonic_receive_ns,
                );
                self.terminal_pm_unavailable = pm_unavailable;
                return Err(PmPublicCaptureRunError::OkxCaptureRejected {
                    source,
                    unavailable,
                });
            }
        };
        self.okx_raw_ingress.commit(epoch, ingress);
        match self.roles.classify_and_route_okx(
            raw,
            local_wall_receive_ns,
            monotonic_receive_ns,
            raw_hash,
        ) {
            Ok(event) => Ok(event),
            Err(PmCaptureRoleIngressError::OkxSession(source)) => {
                let (pm_unavailable, unavailable) = self.terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::IngressSessionClassification,
                    reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                    reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                    local_wall_receive_ns,
                    monotonic_receive_ns,
                );
                self.terminal_pm_unavailable = pm_unavailable;
                Err(PmPublicCaptureRunError::OkxClassify {
                    source,
                    unavailable,
                })
            }
            Err(PmCaptureRoleIngressError::OkxRawNotUtf8 { unavailable }) => {
                let (pm_unavailable, _) = self.terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::IngressSessionClassification,
                    reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                    reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                    local_wall_receive_ns,
                    monotonic_receive_ns,
                );
                self.terminal_pm_unavailable = pm_unavailable;
                Err(PmPublicCaptureRunError::OkxRawNotUtf8 { unavailable })
            }
            Err(PmCaptureRoleIngressError::Route(source)) => {
                let (pm_unavailable, okx_unavailable) = self.terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::Route,
                    reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                    reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                    local_wall_receive_ns,
                    monotonic_receive_ns,
                );
                self.terminal_pm_unavailable = pm_unavailable;
                self.terminal_okx_unavailable = okx_unavailable;
                Err(source.into())
            }
            Err(PmCaptureRoleIngressError::PmSession(_)) => {
                self.terminalize_plain(PmPublicCaptureTerminalCause::InternalInvariant);
                Err(PmPublicCaptureRunError::InternalRoleMismatch)
            }
        }
    }
}
