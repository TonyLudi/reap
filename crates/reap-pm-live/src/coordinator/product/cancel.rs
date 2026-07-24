use super::*;

/// Captured decision facts shared by scheduled and urgent-control cancels.
///
/// Service time is deliberately absent: replayed cancellation identity and
/// expiry derive from the occurrence that caused the decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PmCancelDecisionContext {
    local_action_sequence: u64,
    decision_wall_timestamp_ms: u64,
    decision_monotonic_ns: u64,
}

impl PmCancelDecisionContext {
    pub(super) const fn new(
        local_action_sequence: u64,
        decision_wall_timestamp_ms: u64,
        decision_monotonic_ns: u64,
    ) -> Self {
        Self {
            local_action_sequence,
            decision_wall_timestamp_ms,
            decision_monotonic_ns,
        }
    }

    const fn from_timer(timer: PmTimerInput) -> Self {
        Self::new(
            timer.local_action_sequence(),
            timer.decision_wall_timestamp_ms(),
            timer.decision_monotonic_ns(),
        )
    }
}

impl<M: PmQuoteModel> PmCoordinator<M> {
    pub(super) fn cancel_tracked(
        &mut self,
        timer: PmTimerInput,
        reason: PmJournalCancelReasonV1,
        control_reason: PmControlReason,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        let side = timer.key().side();
        let context = PmCancelDecisionContext::from_timer(timer);
        if let Some(order) = self.mutation.owned_cancel_candidate(side) {
            self.begin_cancel_with_context(
                order.client_order(),
                side,
                context,
                reason,
                control_reason,
                effects,
            )?;
        } else if let Some(tracked) = self.tracked_quotes[side_index(side)] {
            self.clear_tracked_quote(tracked.client_order);
        }
        Ok(())
    }

    pub(super) fn cancel_tracked_owned_orders(
        &mut self,
        timer: PmTimerInput,
        reason: PmJournalCancelReasonV1,
        control_reason: PmControlReason,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        self.cancel_owned_orders(
            PmCancelDecisionContext::from_timer(timer),
            reason,
            control_reason,
            effects,
        )
    }

    pub(super) fn cancel_owned_orders(
        &mut self,
        context: PmCancelDecisionContext,
        reason: PmJournalCancelReasonV1,
        control_reason: PmControlReason,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        for side in [PmOrderSide::Buy, PmOrderSide::Sell] {
            if let Some(order) = self.mutation.owned_cancel_candidate(side) {
                self.begin_cancel_with_context(
                    order.client_order(),
                    side,
                    context,
                    reason,
                    control_reason,
                    effects,
                )?;
            } else if let Some(tracked) = self.tracked_quotes[side_index(side)] {
                self.clear_tracked_quote(tracked.client_order);
            }
        }
        Ok(())
    }

    pub(super) fn begin_cancel(
        &mut self,
        client_order: PmClientOrderKey,
        side: PmOrderSide,
        timer: PmTimerInput,
        reason: PmJournalCancelReasonV1,
        control_reason: PmControlReason,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        self.begin_cancel_with_context(
            client_order,
            side,
            PmCancelDecisionContext::from_timer(timer),
            reason,
            control_reason,
            effects,
        )
    }

    fn begin_cancel_with_context(
        &mut self,
        client_order: PmClientOrderKey,
        side: PmOrderSide,
        context: PmCancelDecisionContext,
        reason: PmJournalCancelReasonV1,
        control_reason: PmControlReason,
        effects: &mut PmProductEffectBatch,
    ) -> Result<(), PmCoordinatorError> {
        let request = PmCancelMutationRequest::new(
            client_order,
            reason,
            salt_for(context.local_action_sequence, side)?,
            context.decision_wall_timestamp_ms,
            context.decision_monotonic_ns,
            context
                .decision_monotonic_ns
                .checked_add(self.decision.policy.approval_lifetime_ns)
                .ok_or(PmCoordinatorError::ClockOverflow)?,
        );
        match self.mutation.begin_cancel(request) {
            Ok(PmCancelMutationAdmission::JournalPending {
                client_order,
                venue_order,
            }) => {
                let projection = PmFakeCancelEffect::new(
                    self.account_scope,
                    self.instrument,
                    client_order,
                    venue_order,
                    PmFakeEffectStage::PreparedAfterDurability,
                );
                self.pending_correlations
                    .push(CopiedEffectCorrelation::Cancel(projection))?;
                effects.push(PmProductEffect::DurableRecord(PmDurableRecordEffect::new(
                    PmDurableRecordKind::CancelIntent,
                    Some(client_order),
                    context.local_action_sequence,
                )))?;
                effects.push(PmProductEffect::FailClosedHaltOrCancel(
                    PmFailClosedEffect::cancel(
                        self.account_scope,
                        self.instrument,
                        control_reason,
                        client_order,
                        cancel_reason(reason),
                    ),
                ))?;
                self.counters.durable_record_effects =
                    self.counters.durable_record_effects.saturating_add(1);
                Ok(())
            }
            Ok(
                PmCancelMutationAdmission::AlreadyPending { .. }
                | PmCancelMutationAdmission::AlreadyTerminal,
            ) => Ok(()),
            Err(PmMutationError::UnknownOwnedOrder) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}
