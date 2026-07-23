use super::*;

impl PmCaptureRoles {
    pub(crate) fn preflight_pm_reducer_freshness(
        &self,
        reducer: &PmBookReducer,
    ) -> Result<(), PmPublicReducerSyncError> {
        self.validate_pm_reducer_identity(reducer)?;
        let readiness = reducer.readiness();
        if self.pm.requires_reconnect()
            || !self.pm.protocol_flow_open()
            || reducer.connection_epoch() != Some(self.pm.connection_epoch())
            || !readiness.is_ready()
            || readiness.metadata_revision() != Some(self.pm.metadata_revision())
            || readiness.snapshot_revision() != self.pm.current_snapshot_revision()
        {
            return Err(PmPublicReducerSyncError::SessionReducerStateMismatch);
        }
        Ok(())
    }

    pub(crate) fn apply_pm_reducer_freshness(
        &self,
        reducer: &mut PmBookReducer,
        monotonic_ns: u64,
    ) -> Result<PmPublicFreshnessTimerOutcome, PmPublicReducerSyncError> {
        self.preflight_pm_reducer_freshness(reducer)?;
        let before = reducer.counters();
        let outcome = match reducer.check_freshness(monotonic_ns) {
            Ok(PmBookTransition::FreshnessConfirmed) => PmPublicFreshnessTimerOutcome::Confirmed,
            Ok(_) => return Err(PmPublicReducerSyncError::ReducerTransitionMismatch),
            Err(
                reason @ (PmPublicReadinessReason::MetadataStale
                | PmPublicReadinessReason::BookStale),
            ) => PmPublicFreshnessTimerOutcome::Unavailable { reason },
            Err(reason) => return Err(PmPublicReducerSyncError::Reducer(reason)),
        };
        let after = reducer.counters();
        let counters_match = match outcome {
            PmPublicFreshnessTimerOutcome::Confirmed => {
                after.freshness_checks == before.freshness_checks.saturating_add(1)
                    && after.freshness_confirmed == before.freshness_confirmed.saturating_add(1)
                    && after.stale_invalidations == before.stale_invalidations
                    && after.invalidations == before.invalidations
            }
            PmPublicFreshnessTimerOutcome::Unavailable { reason } => {
                reducer.readiness().reason() == Some(reason)
                    && after.freshness_checks == before.freshness_checks.saturating_add(1)
                    && after.freshness_confirmed == before.freshness_confirmed
                    && after.stale_invalidations == before.stale_invalidations.saturating_add(1)
                    && after.invalidations == before.invalidations.saturating_add(1)
                    && after.unavailable_transitions
                        == before.unavailable_transitions.saturating_add(1)
            }
        };
        if !counters_match {
            return Err(PmPublicReducerSyncError::ReducerTransitionMismatch);
        }
        Ok(outcome)
    }
}
