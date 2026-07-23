use super::*;

/// Whether a terminal tick-size delivery still owes its exact product-state
/// invalidation before the sealed capture Run may be discarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmPublicTerminalTickCleanupStatus {
    NotRequired,
    Pending,
    Applied,
}

/// Failure to apply the one exact terminal tick delivery to product state.
///
/// Every rejection returns the move-only delivery without exposing the
/// Run-owned reducer.
#[derive(Debug, Error)]
#[allow(
    clippy::large_enum_variant,
    reason = "terminal authentication failures retain exact move-only routed evidence without heap allocation"
)]
pub enum PmPublicTerminalTickApplyError {
    #[error("capture run was not sealed by a PM tick-size change")]
    RunCauseMismatch { delivery: PmPublicBookDelivery },
    #[error("routed PM delivery is not the terminal tick-size change")]
    DeliveryIsNotTickSizeChange { delivery: PmPublicBookDelivery },
    #[error("terminal PM tick cleanup is not pending (current status: {cleanup_status:?})")]
    CleanupStatusMismatch {
        delivery: PmPublicBookDelivery,
        cleanup_status: PmPublicTerminalTickCleanupStatus,
    },
    #[error("terminal PM tick delivery is not the exact pending cleanup obligation")]
    PendingDeliveryMismatch { delivery: PmPublicBookDelivery },
    #[error("terminal PM tick-size delivery failed active-run authentication: {source}")]
    Authentication {
        delivery: PmPublicBookDelivery,
        #[source]
        source: crate::capture_roles::PmPublicLaneFaultError,
    },
}

impl PmPublicTerminalTickApplyError {
    pub const fn delivery(&self) -> &PmPublicBookDelivery {
        match self {
            Self::RunCauseMismatch { delivery }
            | Self::DeliveryIsNotTickSizeChange { delivery }
            | Self::CleanupStatusMismatch { delivery, .. }
            | Self::PendingDeliveryMismatch { delivery }
            | Self::Authentication { delivery, .. } => delivery,
        }
    }

    pub fn into_delivery(self) -> PmPublicBookDelivery {
        match self {
            Self::RunCauseMismatch { delivery }
            | Self::DeliveryIsNotTickSizeChange { delivery }
            | Self::CleanupStatusMismatch { delivery, .. }
            | Self::PendingDeliveryMismatch { delivery }
            | Self::Authentication { delivery, .. } => delivery,
        }
    }
}

impl PmPublicCaptureRun {
    /// Applies the exact old/new tick carried by the terminal routed delivery
    /// to the reducer bound to this capture root.
    ///
    /// This is the sole post-terminal mutation allowed by the run. It cannot
    /// reopen capture or protocol flow; it only consumes authenticated terminal
    /// evidence so product state records metadata drift before rotation.
    #[allow(
        clippy::result_large_err,
        reason = "authentication failure returns the exact move-only routed delivery"
    )]
    pub fn apply_terminal_tick_invalidation(
        &mut self,
        delivery: PmPublicBookDelivery,
    ) -> Result<PmPublicReadinessReason, PmPublicTerminalTickApplyError> {
        if self.terminal_cause != Some(PmPublicCaptureTerminalCause::TickSizeChanged) {
            return Err(PmPublicTerminalTickApplyError::RunCauseMismatch { delivery });
        }
        if self.terminal_tick_cleanup != PmPublicTerminalTickCleanupStatus::Pending {
            return Err(PmPublicTerminalTickApplyError::CleanupStatusMismatch {
                delivery,
                cleanup_status: self.terminal_tick_cleanup,
            });
        }
        let envelope = delivery.envelope();
        let reap_pm_core::PmBookUpdate::TickSizeChanged { old, new } = envelope.payload().update()
        else {
            return Err(PmPublicTerminalTickApplyError::DeliveryIsNotTickSizeChange { delivery });
        };
        let (old, new) = (*old, *new);
        if !self.pending_pm_book_matches(&delivery, PendingPmBookKind::TickSizeChanged) {
            return Err(PmPublicTerminalTickApplyError::PendingDeliveryMismatch { delivery });
        }
        let authority_id = delivery.authority_id();
        let source = envelope.source();
        let connection = envelope.connection_id();
        let ordering = envelope.ordering();
        let received_clock = envelope.received_clock();
        let reason = self
            .roles
            .enact_terminal_tick_size_aged(
                source,
                authority_id,
                connection,
                ordering,
                received_clock,
                received_clock.monotonic_receive_ns(),
                old,
                new,
                &mut self.pm_reducer,
            )
            .map_err(|source| PmPublicTerminalTickApplyError::Authentication {
                delivery,
                source,
            })?;
        self.consume_pending_pm_book();
        self.terminal_tick_cleanup = PmPublicTerminalTickCleanupStatus::Applied;
        Ok(reason)
    }

    #[must_use]
    pub const fn terminal_tick_cleanup_status(&self) -> PmPublicTerminalTickCleanupStatus {
        self.terminal_tick_cleanup
    }
}
