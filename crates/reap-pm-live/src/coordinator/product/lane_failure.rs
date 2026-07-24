use reap_pm_state::{PmRefreshReason, PmRefreshRequired};
use reap_pm_strategy::PmQuoteModel;

use super::*;
use crate::lanes::{PmCompleteServiceError, SaturationAction};
use crate::schedule::PmScheduleError;

impl<M: PmQuoteModel> PmCoordinator<M> {
    #[cfg(test)]
    pub(crate) fn phase6_reach_schedule_full(
        &mut self,
        observed_ns: u64,
        decision_wall_timestamp_ms: u64,
    ) -> Option<(usize, SaturationAction)> {
        let configured = self.instrument;
        let base = self.account_scope;
        let mut attempts = 0;
        for ordinal in 0..2_048_u16 {
            let account = PmAccountScope::new(
                base.environment(),
                base.chain(),
                base.signer(),
                base.funder(),
                reap_pm_core::PmAccountHandle::from_ordinal(ordinal),
            );
            for side in [PmOrderSide::Buy, PmOrderSide::Sell] {
                attempts += 1;
                let key = PmScheduledActionKey::new(
                    account,
                    configured,
                    side,
                    PmScheduledActionKind::QuoteEvaluation,
                );
                self.lanes
                    .as_mut()?
                    .schedule(
                        key,
                        1_000_000_000 + u64::try_from(attempts).ok()?,
                        observed_ns,
                        decision_wall_timestamp_ms,
                    )
                    .ok()?;
            }
        }
        attempts += 1;
        let attempted = PmScheduledActionKey::new(
            PmAccountScope::new(
                base.environment(),
                base.chain(),
                base.signer(),
                base.funder(),
                reap_pm_core::PmAccountHandle::from_ordinal(4_000),
            ),
            configured,
            PmOrderSide::Buy,
            PmScheduledActionKind::Freshness,
        );
        self.pending_schedules
            .insert(
                attempted,
                2_000_000_000,
                observed_ns,
                decision_wall_timestamp_ms,
            )
            .ok()?;
        let PmCoordinatorError::Schedule(error) = self.flush_pending_schedules().err()? else {
            return None;
        };
        let action = error.saturation_action()?;
        Some((attempts, action))
    }

    #[cfg(test)]
    pub(crate) fn phase6_enact_next_schedule_failure_with_observer_clock(
        &mut self,
        schedule_observed_ns: u64,
        observer_service_ns: u64,
    ) -> Option<SaturationAction> {
        let mut lanes = self.lanes.take()?;
        let result = lanes.service_turn(schedule_observed_ns, self);
        self.lanes = Some(lanes);
        let error = result.err()?;
        let action = error.action();
        self.observe_complete_service_failure(&error, observer_service_ns);
        action
    }

    pub(super) fn observe_scheduler_action(
        &mut self,
        action: Option<SaturationAction>,
        reason: PmControlReason,
        monotonic_service_ns: Option<u64>,
    ) {
        if action == Some(SaturationAction::CoalesceTelemetry) {
            return;
        }
        self.mutation.invalidate_revisions();
        match action {
            Some(SaturationAction::HaltAccountAndRequireReconciliation) => {
                self.reconciliation_gate = true;
                self.reconciliation_recovered = false;
                if monotonic_service_ns.is_none_or(|monotonic_service_ns| {
                    self.require_complete_reconciliation_after_private_lane_failure(
                        monotonic_service_ns,
                    )
                    .is_err()
                }) {
                    self.mutation.halt_contract();
                    self.latch_halt(reason);
                }
            }
            Some(
                SaturationAction::KeepUnreadyAndRetry | SaturationAction::RetainPendingRefresh,
            ) => {
                self.reconciliation_gate = true;
                self.reconciliation_recovered = false;
            }
            Some(
                SaturationAction::InvalidateStreamAndResync
                | SaturationAction::InvalidateCaptureAndResync,
            ) => {}
            Some(SaturationAction::CoalesceTelemetry) => {}
            Some(
                SaturationAction::GlobalStop
                | SaturationAction::SuppressDispatchAndHaltQuotes
                | SaturationAction::RejectEffectAndHaltQuotes
                | SaturationAction::SuppressQuoteAndCancelOwned,
            )
            | None => {
                self.mutation.halt_contract();
                self.latch_halt(reason);
            }
        }
    }

    pub(super) fn observe_complete_service_failure(
        &mut self,
        error: &PmCompleteServiceError,
        monotonic_service_ns: u64,
    ) {
        if let PmCompleteServiceError::Schedule(error) = error {
            self.observe_schedule_failure(error, monotonic_service_ns);
            return;
        }
        self.observe_scheduler_action(
            error.action(),
            PmControlReason::SchedulerOverload,
            Some(monotonic_service_ns),
        );
    }

    pub(super) fn observe_schedule_failure(
        &mut self,
        error: &PmScheduleError,
        monotonic_service_ns: u64,
    ) {
        match error {
            PmScheduleError::Aged {
                pending,
                observed_ns,
                local_action_sequence,
                decision_wall_timestamp_ms,
                action: SaturationAction::SuppressQuoteAndCancelOwned,
                ..
            } => self.enact_schedule_safety_failure(
                Some(*pending),
                *observed_ns,
                *local_action_sequence,
                *decision_wall_timestamp_ms,
                monotonic_service_ns,
            ),
            PmScheduleError::Full {
                observed_ns,
                local_action_sequence,
                decision_wall_timestamp_ms,
                action: SaturationAction::SuppressQuoteAndCancelOwned,
                ..
            } => self.enact_schedule_safety_failure(
                None,
                *observed_ns,
                *local_action_sequence,
                *decision_wall_timestamp_ms,
                monotonic_service_ns,
            ),
            _ => self.observe_scheduler_action(
                error
                    .saturation_action()
                    .or(Some(SaturationAction::GlobalStop)),
                PmControlReason::SchedulerOverload,
                Some(monotonic_service_ns),
            ),
        }
    }

    fn enact_schedule_safety_failure(
        &mut self,
        aged_pending: Option<PmScheduledActionKey>,
        observed_ns: u64,
        local_action_sequence: u64,
        decision_wall_timestamp_ms: u64,
        monotonic_service_ns: u64,
    ) {
        self.mutation.invalidate_revisions();
        let mut effects = PmProductEffectBatch::new();
        let clock_matches = observed_ns == monotonic_service_ns;
        let cancel_represented = clock_matches
            && self
                .cancel_owned_orders(
                    super::cancel::PmCancelDecisionContext::new(
                        local_action_sequence,
                        decision_wall_timestamp_ms,
                        observed_ns,
                    ),
                    PmJournalCancelReasonV1::SafetyHalt,
                    PmControlReason::SchedulerOverload,
                    &mut effects,
                )
                .is_ok();
        let halt_represented = effects
            .push(PmProductEffect::FailClosedHaltOrCancel(
                PmFailClosedEffect::halt(
                    self.account_scope,
                    self.instrument,
                    PmControlReason::SchedulerOverload,
                ),
            ))
            .is_ok();
        let outputs_published = halt_represented && self.publish_effect_batch(effects).is_ok();
        // A rejected or aged schedule receives one bounded canonical cancel
        // attempt. An aged slot is consumed even when representation fails;
        // Full never entered the schedule. The fallback is a global halt, not
        // repeated cancel/halt emission on every turn.
        let schedule_consumed = aged_pending.is_none()
            || aged_pending.is_some_and(|pending| {
                self.lanes
                    .as_mut()
                    .is_some_and(|lanes| lanes.resolve_aged_schedule(pending))
            });
        if !cancel_represented || !outputs_published || !schedule_consumed {
            self.mutation.halt_contract();
        }
        self.latch_halt(PmControlReason::SchedulerOverload);
    }

    pub(super) fn latch_scheduler_failure(&mut self, reason: PmControlReason) {
        self.observe_scheduler_action(Some(SaturationAction::GlobalStop), reason, None);
    }

    pub(super) fn note_private_reduction_while_gated(&mut self) {
        if self.reconciliation_gate {
            self.reconciliation_recovered = false;
        }
    }

    pub(super) fn note_complete_reconciliation(&mut self) {
        if self.reconciliation_gate {
            self.reconciliation_recovered = true;
        }
    }

    pub(super) fn clear_reconciliation_gate_if_drained(&mut self) {
        let lanes_drained = self
            .lanes
            .as_deref()
            .is_some_and(PmCompleteInputLanes::private_and_reconciliation_empty);
        let external_ingress_refresh_pending = self
            .mutation
            .pending_refresh_count_for(PmRefreshReason::ExternalIngressFault)
            != 0
            || self
                .refresh_obligations
                .has_reason(PmRefreshReason::ExternalIngressFault);
        if self.reconciliation_gate
            && self.reconciliation_recovered
            && lanes_drained
            && self.retained_private_admission.is_none()
            && self.retained_reconciliation_admission.is_none()
            && !external_ingress_refresh_pending
        {
            self.reconciliation_gate = false;
            self.reconciliation_recovered = false;
        }
    }

    fn require_complete_reconciliation_after_private_lane_failure(
        &mut self,
        monotonic_service_ns: u64,
    ) -> Result<(), PmCoordinatorError> {
        let required = self
            .mutation
            .require_refresh(PmRefreshReason::ExternalIngressFault)?;
        if matches!(required, PmRefreshRequired::SupersededInFlight { .. }) {
            self.refresh_obligations.record_duplicate_or_superseded();
        }
        let mut effects = PmProductEffectBatch::new();
        effects.push(PmProductEffect::FailClosedHaltOrCancel(
            PmFailClosedEffect::halt(
                self.account_scope,
                self.instrument,
                PmControlReason::SchedulerOverload,
            ),
        ))?;
        let _admitted = self.admit_refresh_reason(
            PmRefreshReason::ExternalIngressFault,
            monotonic_service_ns,
            &mut effects,
        )?;
        self.publish_effect_batch(effects)?;
        Ok(())
    }
}
