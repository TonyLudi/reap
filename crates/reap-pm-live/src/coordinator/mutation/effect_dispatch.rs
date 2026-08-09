//! Backend-neutral prepared dispatch handoff and durable live-result bridge.
//!
//! This keeps the sole mutation owner focused on lifecycle/risk orchestration:
//! fixed fixture execution remains an explicit caller-owned capability, while
//! authenticated completions cross the second durability barrier here.

use super::*;

impl PmMutationOwner {
    pub(crate) fn next_effect_kind(&self) -> Option<PmPreparedMutationKind> {
        self.effects.next_kind()
    }

    /// Retains a durable head quote when the authenticated place worker has
    /// no admission capacity. Placement remains halted, while a following
    /// safety cancel becomes the queue head and stays serviceable.
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn quarantine_next_place_for_authenticated_backpressure(
        &mut self,
    ) -> Result<(), PmMutationError> {
        self.effects
            .quarantine_front_quote_for_authenticated_backpressure()?;
        self.halt = Some(PmMutationHalt::FakeEffectSaturated);
        Ok(())
    }

    /// Retains a durable unsent quote when the product has entered controlled
    /// shutdown. The coordinator's shutdown halt is already authoritative;
    /// this transition exists only to expose a following exact-owned cancel.
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn suppress_next_place_for_shutdown(&mut self) -> Result<(), PmMutationError> {
        self.effects.suppress_front_quote_for_shutdown()?;
        Ok(())
    }

    /// Takes one already-Goal-F-durable place dispatch out of the coordinator.
    pub(crate) fn take_next_place_dispatch(
        &mut self,
        monotonic_service_ns: u64,
    ) -> Result<PmPreparedPlaceDispatch, PmMutationError> {
        if self.next_effect_kind() != Some(PmPreparedMutationKind::Quote) {
            return Err(PmMutationError::EffectKindMismatch);
        }
        let effect = match self
            .effects
            .pop_quote_at(monotonic_service_ns, self.current_revisions)
        {
            Ok(effect) => effect,
            Err(error) => {
                self.halt = Some(
                    if error == PmMutationDispatchQueueError::QuoteAuthorityInvalidated {
                        PmMutationHalt::PreparationFailed
                    } else {
                        PmMutationHalt::FakeEffectSaturated
                    },
                );
                return Err(error.into());
            }
        };
        let Some(PmPreparedMutation::Quote { authority }) = effect else {
            return Err(PmMutationError::EffectKindMismatch);
        };
        Ok(take_prepared_place_dispatch(
            authority,
            self.scope.account_scope(),
            self.account_signature_profile,
            self.private.instrument(),
            self.instrument_id,
        )?)
    }

    /// Takes one already-Goal-F-durable exact-owned cancel dispatch and keeps
    /// its move-only lifecycle proof attached to the backend handoff.
    pub(crate) fn take_next_cancel_dispatch(
        &mut self,
        monotonic_service_ns: u64,
    ) -> Result<(PmPreparedCancelDispatch, PmOwnedCancelIntent), PmMutationError> {
        if self.next_effect_kind() != Some(PmPreparedMutationKind::Cancel) {
            return Err(PmMutationError::EffectKindMismatch);
        }
        let effect = match self.effects.pop_at(monotonic_service_ns) {
            Ok(effect) => effect,
            Err(error) => {
                self.halt = Some(PmMutationHalt::FakeEffectSaturated);
                return Err(error.into());
            }
        };
        let Some(PmPreparedMutation::Cancel {
            authority,
            owned_intent,
        }) = effect
        else {
            return Err(PmMutationError::EffectKindMismatch);
        };
        let dispatch = take_prepared_cancel_dispatch(
            authority,
            self.scope.account_scope(),
            self.account_signature_profile,
            self.private.instrument(),
            self.instrument_id,
        )?;
        Ok((dispatch, owned_intent))
    }

    #[cfg(test)]
    pub(crate) fn execute_next_quote(
        &mut self,
        executor: &PmFixtureEffectExecutor,
        script: PmFakePlaceScript,
        monotonic_service_ns: u64,
    ) -> Result<(), PmMutationError> {
        let result = self.execute_next_quote_to_result(executor, script, monotonic_service_ns)?;
        self.reduce_serviced_fake_place(result, monotonic_service_ns)
    }

    pub(crate) fn execute_next_quote_to_result(
        &mut self,
        executor: &PmFixtureEffectExecutor,
        script: PmFakePlaceScript,
        monotonic_service_ns: u64,
    ) -> Result<PmFakePlaceResult, PmMutationError> {
        let dispatch = self.take_next_place_dispatch(monotonic_service_ns)?;
        Ok(executor.execute_place_fixture(dispatch, script)?)
    }

    pub(crate) fn reduce_serviced_fake_place(
        &mut self,
        result: PmFakePlaceResult,
        monotonic_service_ns: u64,
    ) -> Result<(), PmMutationError> {
        super::super::reduction::reduce_fake_place(self, result, monotonic_service_ns)
    }

    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn begin_serviced_live_place(
        &mut self,
        completion: Box<super::super::live_completion::PmLivePlaceCompletion>,
        monotonic_service_ns: u64,
    ) -> Result<(), PmMutationError> {
        self.begin_live_bridge(PmPendingLiveBridge::Place(completion), monotonic_service_ns)
    }

    #[cfg(test)]
    pub(crate) fn execute_next_cancel(
        &mut self,
        executor: &PmFixtureEffectExecutor,
        script: PmFakeCancelScript,
        monotonic_service_ns: u64,
    ) -> Result<(), PmMutationError> {
        let result = self.execute_next_cancel_to_result(executor, script, monotonic_service_ns)?;
        self.reduce_serviced_fake_cancel(result, monotonic_service_ns)
    }

    pub(crate) fn execute_next_cancel_to_result(
        &mut self,
        executor: &PmFixtureEffectExecutor,
        script: PmFakeCancelScript,
        monotonic_service_ns: u64,
    ) -> Result<PmPendingFakeCancelResult, PmMutationError> {
        let (dispatch, owned_intent) = self.take_next_cancel_dispatch(monotonic_service_ns)?;
        let result = executor.execute_cancel_fixture(dispatch, script)?;
        Ok(PmPendingFakeCancelResult {
            intent: owned_intent,
            result,
        })
    }

    pub(crate) fn reduce_serviced_fake_cancel(
        &mut self,
        pending: PmPendingFakeCancelResult,
        monotonic_service_ns: u64,
    ) -> Result<(), PmMutationError> {
        let (intent, result) = pending.into_parts();
        super::super::reduction::reduce_fake_cancel(self, intent, result, monotonic_service_ns)
    }

    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn begin_serviced_live_cancel(
        &mut self,
        completion: super::super::live_completion::PmLiveCancelCompletion,
        monotonic_service_ns: u64,
    ) -> Result<(), PmMutationError> {
        self.begin_live_bridge(
            PmPendingLiveBridge::Cancel(completion),
            monotonic_service_ns,
        )
    }

    #[cfg(any(test, feature = "loopback-evidence"))]
    fn begin_live_bridge(
        &mut self,
        completion: PmPendingLiveBridge,
        monotonic_now_ns: u64,
    ) -> Result<(), PmMutationError> {
        if self.applied_live_bridges.len() >= 2 {
            self.quarantine_live_bridge(completion);
            return Err(PmMutationError::AuthenticatedBridgeCompletionSaturated);
        }
        if let Err(error) = Self::validate_persistence_time(monotonic_now_ns) {
            self.quarantine_live_bridge(completion);
            return Err(error);
        }
        let preflight = match &completion {
            PmPendingLiveBridge::Place(completion) => {
                super::super::authenticated_reduction::preflight_live_place(self, completion)
            }
            PmPendingLiveBridge::Cancel(completion) => {
                super::super::authenticated_reduction::preflight_live_cancel(self, completion)
            }
        };
        if let Err(error) = preflight {
            let _ = self.enter_terminal_safety(error.safety_reason(), monotonic_now_ns);
            self.quarantine_live_bridge(completion);
            return Err(error.into());
        }
        if let Err(error) = self.ensure_fact_capacity(1) {
            self.quarantine_live_bridge(completion);
            return Err(error);
        }
        let record = match &completion {
            PmPendingLiveBridge::Place(completion) => PmJournalRecordV1::AuthenticatedResult(
                crate::journal::PmJournalAuthenticatedResultV1::Place(completion.result()),
            ),
            PmPendingLiveBridge::Cancel(completion) => PmJournalRecordV1::AuthenticatedResult(
                crate::journal::PmJournalAuthenticatedResultV1::Cancel(completion.result()),
            ),
        };
        let pending = match self.journal.try_record(record) {
            Ok(pending) => pending,
            Err(error) => {
                self.halt = Some(PmMutationHalt::JournalAdmissionFailed);
                self.quarantine_live_bridge(completion);
                return Err(error.into());
            }
        };
        let correlation = self.counters.fact_records.saturating_add(1);
        if let Err((error, rejected)) =
            self.persistence
                .push_retaining(PmPendingPersistence::LiveBridge {
                    receipt: pending,
                    completion,
                    correlation,
                    enqueued_monotonic_ns: monotonic_now_ns,
                })
        {
            let PmPendingPersistence::LiveBridge { completion, .. } = rejected else {
                unreachable!("live bridge admission returns the same pending variant")
            };
            self.halt = Some(PmMutationHalt::InternalInvariant);
            self.quarantine_live_bridge(completion);
            return Err(error.into());
        }
        self.counters.fact_records = correlation;
        Ok(())
    }

    pub(super) fn quarantine_live_bridge(&mut self, completion: PmPendingLiveBridge) {
        if self.quarantined_live_bridges.len() < 2 {
            self.quarantined_live_bridges.push_back(completion);
        } else {
            self.halt = Some(PmMutationHalt::InternalInvariant);
        }
    }

    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(in crate::coordinator) fn take_applied_live_bridge(
        &mut self,
    ) -> Option<PmAuthenticatedBridgeApplied> {
        self.applied_live_bridges.pop_front()
    }

    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(in crate::coordinator) fn take_failed_live_bridge(
        &mut self,
    ) -> Option<PmAuthenticatedBridgeFailure> {
        self.failed_live_bridges.pop_front()
    }

    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(in crate::coordinator) fn take_failed_goal_f_write(
        &mut self,
    ) -> Option<PmGoalFWriterFailure> {
        self.failed_goal_f_write.take()
    }
}
