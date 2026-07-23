use super::*;

impl PmPublicCaptureRun {
    pub(super) async fn record_pm_disconnected_session(
        &mut self,
        local_wall_receive_ns: u64,
        monotonic_ns: u64,
    ) -> Result<PmPublicUnavailableDelivery, PmPublicCaptureRunError> {
        self.ensure_active()?;
        if !self.pm_lifecycle.accepts_live_input() {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            return Err(PmPublicCaptureRunError::InvalidLifecyclePhase);
        }
        let epoch = self.roles.pm_epoch();
        if let Err(source) = self
            .roles
            .preflight_pm_invalidation(local_wall_receive_ns, monotonic_ns)
        {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            return Err(source.into());
        }
        if let Err(source) = self.writer.preflight_pm_lifecycle(
            reap_pm_core::ConnectionEpoch::new(epoch),
            monotonic_ns,
            PmCaptureLifecycle::Disconnected {
                local_wall_receive_ns,
                reason: PmCaptureDisconnectReason::Disconnect,
            },
        ) {
            self.terminalize_plain(PmPublicCaptureTerminalCause::CaptureWriter);
            return Err(source.into());
        }
        if let Err(source) = self
            .writer
            .record_lifecycle(
                reap_pm_core::ConnectionEpoch::new(epoch),
                monotonic_ns,
                PmCaptureLifecycle::Disconnected {
                    local_wall_receive_ns,
                    reason: PmCaptureDisconnectReason::Disconnect,
                },
            )
            .await
        {
            self.terminalize_plain(PmPublicCaptureTerminalCause::CaptureWriter);
            return Err(source.into());
        }
        let unavailable = match self.roles.invalidate_and_route_pm(
            reap_polymarket_adapter::PmPublicSessionFault::Disconnect,
            local_wall_receive_ns,
            monotonic_ns,
        ) {
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
        self.pm_disconnected_epoch = Some(epoch);
        self.pm_lifecycle = PublicLifecyclePhase::Disconnected;
        let _ = self.public_lane.purge_public_route(
            self.roles.authority_id(),
            self.pm_route.source(),
            self.pm_route.connection(),
            reap_pm_core::ConnectionEpoch::new(epoch),
        );
        Ok(unavailable)
    }

    /// Records an explicit disconnect and admits its must-deliver unavailable
    /// occurrence before returning a copied fault fact.
    pub async fn record_okx_disconnected(
        &mut self,
        local_wall_receive_ns: u64,
        monotonic_ns: u64,
    ) -> Result<reap_okx_public_source::OkxPublicSessionFault, PmPublicCaptureRunError> {
        let delivery = self
            .record_okx_disconnected_routed(local_wall_receive_ns, monotonic_ns)
            .await?;
        self.admit_okx_unavailable_or_terminal(delivery)
    }

    async fn record_okx_disconnected_routed(
        &mut self,
        local_wall_receive_ns: u64,
        monotonic_ns: u64,
    ) -> Result<OkxPublicUnavailableDelivery, PmPublicCaptureRunError> {
        self.ensure_active()?;
        self.ensure_no_pending_pm_book_reductions()?;
        if !self.okx_lifecycle.accepts_live_input() {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            return Err(PmPublicCaptureRunError::InvalidLifecyclePhase);
        }
        let epoch = self.roles.okx_epoch();
        if let Err(source) = self
            .roles
            .preflight_okx_invalidation(local_wall_receive_ns, monotonic_ns)
        {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            return Err(source.into());
        }
        if let Err(source) = self.writer.preflight_okx_lifecycle(
            epoch,
            monotonic_ns,
            OkxCaptureLifecycle::Disconnected {
                local_wall_receive_ns,
                reason: OkxCaptureDisconnectReason::Disconnect,
            },
        ) {
            self.terminalize_plain(PmPublicCaptureTerminalCause::CaptureWriter);
            return Err(source.into());
        }
        if let Err(source) = self
            .writer
            .record_okx_lifecycle(
                epoch,
                monotonic_ns,
                OkxCaptureLifecycle::Disconnected {
                    local_wall_receive_ns,
                    reason: OkxCaptureDisconnectReason::Disconnect,
                },
            )
            .await
        {
            self.terminalize_plain(PmPublicCaptureTerminalCause::CaptureWriter);
            return Err(source.into());
        }
        let unavailable = match self.roles.invalidate_and_route_okx(
            reap_okx_public_source::OkxPublicSessionFault::Disconnect,
            local_wall_receive_ns,
            monotonic_ns,
        ) {
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
        self.okx_disconnected_epoch = Some(epoch);
        self.okx_lifecycle = PublicLifecyclePhase::Disconnected;
        let _ = self.public_lane.purge_public_route(
            self.roles.authority_id(),
            self.okx_route.source(),
            self.okx_route.connection(),
            reap_pm_core::ConnectionEpoch::new(epoch),
        );
        Ok(unavailable)
    }

    pub(super) async fn record_pm_reconnect_scheduled_session(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<Duration, PmPublicCaptureRunError> {
        self.ensure_active()?;
        if !self.pm_lifecycle.accepts_reconnect()
            || self.pm_disconnected_epoch != Some(self.roles.pm_epoch())
        {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            return Err(PmPublicCaptureRunError::InvalidLifecyclePhase);
        }
        let pending = self.pending_pm_public_route_depth();
        if pending != 0 {
            return Err(PmPublicCaptureRunError::PendingPmPublicRouteReconnect {
                epoch: self.roles.pm_epoch(),
                pending,
            });
        }
        let (prior_epoch, next_epoch, delay) = match self.roles.preview_pm_failure() {
            Ok(schedule) => schedule,
            Err(source) => {
                self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
                return Err(source.into());
            }
        };
        let delay_ns = match u64::try_from(delay.as_nanos()) {
            Ok(delay_ns) => delay_ns,
            Err(_) => {
                self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
                return Err(PmPublicCaptureRunError::ReconnectDelayOverflow);
            }
        };
        let event = PmCaptureLifecycle::ReconnectScheduled {
            next_epoch: reap_pm_core::ConnectionEpoch::new(next_epoch),
            delay_ns,
        };
        if let Err(source) = self.writer.preflight_pm_lifecycle(
            reap_pm_core::ConnectionEpoch::new(prior_epoch),
            monotonic_ns,
            event,
        ) {
            self.terminalize_plain(PmPublicCaptureTerminalCause::CaptureWriter);
            return Err(source.into());
        }
        if let Err(source) = self
            .writer
            .record_lifecycle(
                reap_pm_core::ConnectionEpoch::new(prior_epoch),
                monotonic_ns,
                event,
            )
            .await
        {
            self.terminalize_plain(PmPublicCaptureTerminalCause::CaptureWriter);
            return Err(source.into());
        }
        let applied = match self.roles.after_pm_failure() {
            Ok(applied) => applied,
            Err(source) => {
                self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
                return Err(source.into());
            }
        };
        if applied != (prior_epoch, next_epoch, delay) {
            self.terminalize_plain(PmPublicCaptureTerminalCause::InternalInvariant);
            return Err(PmPublicCaptureRunError::ReconnectTransitionMismatch);
        }
        self.pm_disconnected_epoch = None;
        self.pm_lifecycle = PublicLifecyclePhase::AwaitingConnection;
        Ok(delay)
    }

    pub async fn record_okx_reconnect_scheduled(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<Duration, PmPublicCaptureRunError> {
        self.ensure_active()?;
        self.ensure_no_pending_pm_book_reductions()?;
        if !self.okx_lifecycle.accepts_reconnect()
            || self.okx_disconnected_epoch != Some(self.roles.okx_epoch())
        {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            return Err(PmPublicCaptureRunError::InvalidLifecyclePhase);
        }
        let pending = self.pending_okx_public_route_depth();
        if pending != 0 {
            return Err(PmPublicCaptureRunError::PendingOkxPublicRouteReconnect {
                epoch: self.roles.okx_epoch(),
                pending,
            });
        }
        let (prior_epoch, next_epoch, delay) = match self.roles.preview_okx_failure() {
            Ok(schedule) => schedule,
            Err(source) => {
                self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
                return Err(source.into());
            }
        };
        let delay_ns = match u64::try_from(delay.as_nanos()) {
            Ok(delay_ns) => delay_ns,
            Err(_) => {
                self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
                return Err(PmPublicCaptureRunError::ReconnectDelayOverflow);
            }
        };
        let event = OkxCaptureLifecycle::ReconnectScheduled {
            next_epoch,
            delay_ns,
        };
        if let Err(source) = self
            .writer
            .preflight_okx_lifecycle(prior_epoch, monotonic_ns, event)
        {
            self.terminalize_plain(PmPublicCaptureTerminalCause::CaptureWriter);
            return Err(source.into());
        }
        if let Err(source) = self
            .writer
            .record_okx_lifecycle(prior_epoch, monotonic_ns, event)
            .await
        {
            self.terminalize_plain(PmPublicCaptureTerminalCause::CaptureWriter);
            return Err(source.into());
        }
        let applied = match self.roles.after_okx_failure() {
            Ok(applied) => applied,
            Err(source) => {
                self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
                return Err(source.into());
            }
        };
        if applied != (prior_epoch, next_epoch, delay) {
            self.terminalize_plain(PmPublicCaptureTerminalCause::InternalInvariant);
            return Err(PmPublicCaptureRunError::ReconnectTransitionMismatch);
        }
        self.okx_disconnected_epoch = None;
        self.okx_lifecycle = PublicLifecyclePhase::AwaitingConnection;
        Ok(delay)
    }

    /// Commits the exact snapshot before admitting that same route capability
    /// to the public lane.
    pub fn commit_then_enqueue_pm_snapshot(
        &mut self,
        delivery: PmPublicBookDelivery,
        flow: PmPublicSnapshotFlow,
    ) -> Result<(), PmPublicBookPipelineError> {
        let delivery = self
            .commit_pm_snapshot_owned(delivery, flow)
            .map_err(PmPublicBookPipelineError::Reduce)?;
        self.enqueue_reduced_pm_book(delivery)
            .map_err(PmPublicBookPipelineError::Lane)
    }

    fn commit_pm_snapshot_owned(
        &mut self,
        delivery: PmPublicBookDelivery,
        flow: PmPublicSnapshotFlow,
    ) -> Result<PmPublicBookDelivery, PmPublicCaptureRunError> {
        if self.artifact_terminal() {
            return Err(PmPublicCaptureRunError::PmSnapshotCommitRunTerminal { delivery, flow });
        }
        if self.has_pending_pm_lane_fault() {
            return Err(PmPublicCaptureRunError::PmSnapshotCommitPendingLaneFault {
                delivery,
                flow,
            });
        }
        if !self.pm_lifecycle.accepts_live_input() {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            return Err(PmPublicCaptureRunError::PmSnapshotCommitInvalidPhase { delivery, flow });
        }
        if !self.pending_pm_book_matches(&delivery, PendingPmBookKind::Snapshot) {
            return Err(PmPublicCaptureRunError::PmSnapshotReductionOrderMismatch {
                delivery,
                flow,
            });
        }
        let clock = delivery.envelope().received_clock();
        match self
            .roles
            .commit_pm_snapshot(delivery, flow, &mut self.pm_reducer)
        {
            Ok(delivery) => {
                self.consume_pending_pm_book();
                Ok(delivery)
            }
            Err(PmCaptureSnapshotCommitFailure::Commit {
                source,
                unavailable,
            }) => {
                self.clear_pending_pm_book_reductions();
                let (pm_terminal, okx_terminal) = self.terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::SnapshotReducer,
                    reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                    reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                    clock.local_wall_receive_ns(),
                    clock.monotonic_receive_ns(),
                );
                self.terminal_pm_unavailable = pm_terminal;
                self.terminal_okx_unavailable = okx_terminal;
                Err(PmPublicCaptureRunError::PmSnapshotCommit {
                    source,
                    unavailable,
                })
            }
            Err(PmCaptureSnapshotCommitFailure::Route(source)) => {
                self.clear_pending_pm_book_reductions();
                let (pm_terminal, okx_terminal) = self.terminalize_with_receive_evidence(
                    PmPublicCaptureTerminalCause::Route,
                    reap_polymarket_adapter::PmPublicSessionFault::InvalidTransition,
                    reap_okx_public_source::OkxPublicSessionFault::InvalidTransition,
                    clock.local_wall_receive_ns(),
                    clock.monotonic_receive_ns(),
                );
                self.terminal_pm_unavailable = pm_terminal;
                self.terminal_okx_unavailable = okx_terminal;
                Err(source.into())
            }
        }
    }

    pub async fn record_freshness_timer(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<PmPublicFreshnessTimerOutcome, PmPublicCaptureRunError> {
        self.ensure_active()?;
        self.ensure_no_pending_pm_book_reductions()?;
        if !self.pm_lifecycle.accepts_live_input() {
            self.terminalize_plain(PmPublicCaptureTerminalCause::Lifecycle);
            return Err(PmPublicCaptureRunError::InvalidLifecyclePhase);
        }
        if let Err(source) = self.roles.preflight_pm_reducer_freshness(&self.pm_reducer) {
            return Err(self.terminalize_reducer_sync_failure(source, 0, monotonic_ns, None));
        }
        if let Err(source) = self.writer.preflight_freshness_timer(monotonic_ns) {
            self.fail_reducer_after_terminal_write_error();
            self.terminalize_plain(PmPublicCaptureTerminalCause::CaptureWriter);
            return Err(source.into());
        }
        if let Err(source) = self.writer.record_freshness_timer(monotonic_ns).await {
            self.fail_reducer_after_terminal_write_error();
            self.terminalize_plain(PmPublicCaptureTerminalCause::CaptureWriter);
            return Err(source.into());
        }
        self.roles
            .apply_pm_reducer_freshness(&mut self.pm_reducer, monotonic_ns)
            .map_err(|source| self.terminalize_reducer_sync_failure(source, 0, monotonic_ns, None))
    }

    pub async fn finish(self) -> Result<PmPublicCaptureOutcome, PmPublicCaptureRunError> {
        let terminal_cause = self.terminal_cause;
        let pending_pm_book_reductions = self.pending_pm_book_reductions.len();
        let pending_pm_book_lane_fault = self.has_pending_pm_lane_fault();
        let queued_public_deliveries = self.public_lane.metrics().depth();
        let terminal_tick_cleanup = self.terminal_tick_cleanup;
        let terminal_notification_admission_failure = self.terminal_notification_admission_failure;
        let public_consumer_transfer_poisoned = self.public_lane.consumer_transfer_poisoned();
        let shutdown = self.writer.finish().await;
        if terminal_cause == Some(PmPublicCaptureTerminalCause::TickSizeChanged)
            && terminal_tick_cleanup == PmPublicTerminalTickCleanupStatus::Pending
        {
            let shutdown_error = match shutdown {
                Ok(shutdown) => shutdown.failure.map(PmPublicCaptureShutdownError::Writer),
                Err(source) => Some(PmPublicCaptureShutdownError::Capture(source)),
            };
            return Err(PmPublicCaptureRunError::TerminalTickCleanupIncomplete {
                cleanup_status: terminal_tick_cleanup,
                shutdown_error,
            });
        }
        if let Some(failure) = terminal_notification_admission_failure {
            let shutdown_error = match shutdown {
                Ok(shutdown) => shutdown.failure.map(PmPublicCaptureShutdownError::Writer),
                Err(source) => Some(PmPublicCaptureShutdownError::Capture(source)),
            };
            return Err(
                PmPublicCaptureRunError::NotificationAdmissionTerminalFinish {
                    failure,
                    shutdown_error,
                },
            );
        }
        if public_consumer_transfer_poisoned {
            let shutdown_error = match shutdown {
                Ok(shutdown) => shutdown.failure.map(PmPublicCaptureShutdownError::Writer),
                Err(source) => Some(PmPublicCaptureShutdownError::Capture(source)),
            };
            return Err(
                PmPublicCaptureRunError::PublicConsumerTransferPoisonedFinish { shutdown_error },
            );
        }
        if pending_pm_book_lane_fault {
            let shutdown_error = match shutdown {
                Ok(shutdown) => shutdown.failure.map(PmPublicCaptureShutdownError::Writer),
                Err(source) => Some(PmPublicCaptureShutdownError::Capture(source)),
            };
            return Err(PmPublicCaptureRunError::PendingPmBookLaneFaultFinish { shutdown_error });
        }
        if pending_pm_book_reductions != 0 {
            let shutdown_error = match shutdown {
                Ok(shutdown) => shutdown.failure.map(PmPublicCaptureShutdownError::Writer),
                Err(source) => Some(PmPublicCaptureShutdownError::Capture(source)),
            };
            return Err(PmPublicCaptureRunError::PendingPmBookReductionFinish {
                pending: pending_pm_book_reductions,
                shutdown_error,
            });
        }
        if let Some(cause) = terminal_cause {
            let shutdown_error = match shutdown {
                Ok(shutdown) => shutdown.failure.map(PmPublicCaptureShutdownError::Writer),
                Err(source) => Some(PmPublicCaptureShutdownError::Capture(source)),
            };
            return Err(PmPublicCaptureRunError::TerminalFinish {
                cause,
                shutdown_error,
            });
        }
        if queued_public_deliveries != 0 {
            let shutdown_error = match shutdown {
                Ok(shutdown) => shutdown.failure.map(PmPublicCaptureShutdownError::Writer),
                Err(source) => Some(PmPublicCaptureShutdownError::Capture(source)),
            };
            return Err(PmPublicCaptureRunError::QueuedPublicLaneFinish {
                pending: queued_public_deliveries,
                shutdown_error,
            });
        }
        let shutdown = shutdown?;
        if let Some(source) = shutdown.failure {
            return Err(PmPublicCaptureRunError::WriterShutdown(source));
        }
        let verification = verify_pm_public_capture(&self.path, &self.header)?;
        if shutdown.stats.sha256 != verification.artifact_sha256
            || shutdown.stats.records != verification.records
            || shutdown.stats.bytes != verification.bytes
        {
            return Err(PmPublicCaptureRunError::WriterEvidenceMismatch);
        }
        let projection = replay_pm_public_capture(&self.path, &self.header)?;
        Ok(PmPublicCaptureOutcome {
            path: self.path,
            header: self.header,
            writer_max_queue_bytes: shutdown.stats.max_queue_bytes,
            writer_max_reserved_bytes: shutdown.stats.max_reserved_bytes,
            verification,
            projection,
        })
    }
}
