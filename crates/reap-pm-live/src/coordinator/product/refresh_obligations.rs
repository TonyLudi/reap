use reap_pm_state::{
    MAX_PM_REFRESH_OBLIGATIONS, PmRefreshAdmission, PmRefreshCompletion, PmRefreshReason,
    PmRefreshTicket,
};
use reap_pm_strategy::PmQuoteModel;

use super::*;
use crate::lanes::{PmLaneKind, PmLanePolicy};

const RECONCILIATION_REQUEST_MAXIMUM_AGE_NS: u64 =
    match PmLanePolicy::for_lane(PmLaneKind::ReconciliationRequest).maximum_age_ns() {
        Some(maximum_age_ns) => maximum_age_ns,
        None => panic!("reconciliation requests declare a state-bearing maximum age"),
    };

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PmRefreshObligationMetrics {
    canonical_insertions: u64,
    total_pending: usize,
    total_in_flight: usize,
    ambiguous_order_pending: usize,
    ambiguous_order_in_flight: usize,
    oldest_in_flight_age_ns: u64,
    maximum_observed_age_ns: u64,
    retry_effects: u64,
    fill_observed_pending: usize,
    fill_observed_in_flight: usize,
    fill_observed_high_water: usize,
    fill_observed_admissions: u64,
    fill_observed_effects: u64,
    fill_observed_completions: u64,
    external_ingress_pending: usize,
    external_ingress_in_flight: usize,
    external_ingress_high_water: usize,
    external_ingress_admissions: u64,
    external_ingress_effects: u64,
    external_ingress_completions: u64,
    duplicate_or_superseded_admissions: u64,
}

impl PmRefreshObligationMetrics {
    #[must_use]
    pub const fn canonical_insertions(self) -> u64 {
        self.canonical_insertions
    }

    #[must_use]
    pub const fn total_pending(self) -> usize {
        self.total_pending
    }

    #[must_use]
    pub const fn total_in_flight(self) -> usize {
        self.total_in_flight
    }

    #[must_use]
    pub const fn ambiguous_order_pending(self) -> usize {
        self.ambiguous_order_pending
    }

    #[must_use]
    pub const fn ambiguous_order_in_flight(self) -> usize {
        self.ambiguous_order_in_flight
    }

    #[must_use]
    pub const fn oldest_in_flight_age_ns(self) -> u64 {
        self.oldest_in_flight_age_ns
    }

    #[must_use]
    pub const fn maximum_observed_age_ns(self) -> u64 {
        self.maximum_observed_age_ns
    }

    #[must_use]
    pub const fn retry_effects(self) -> u64 {
        self.retry_effects
    }

    #[must_use]
    pub const fn fill_observed_pending(self) -> usize {
        self.fill_observed_pending
    }

    #[must_use]
    pub const fn fill_observed_in_flight(self) -> usize {
        self.fill_observed_in_flight
    }

    #[must_use]
    pub const fn fill_observed_high_water(self) -> usize {
        self.fill_observed_high_water
    }

    #[must_use]
    pub const fn fill_observed_admissions(self) -> u64 {
        self.fill_observed_admissions
    }

    #[must_use]
    pub const fn fill_observed_effects(self) -> u64 {
        self.fill_observed_effects
    }

    #[must_use]
    pub const fn fill_observed_completions(self) -> u64 {
        self.fill_observed_completions
    }

    #[must_use]
    pub const fn external_ingress_pending(self) -> usize {
        self.external_ingress_pending
    }

    #[must_use]
    pub const fn external_ingress_in_flight(self) -> usize {
        self.external_ingress_in_flight
    }

    #[must_use]
    pub const fn external_ingress_high_water(self) -> usize {
        self.external_ingress_high_water
    }

    #[must_use]
    pub const fn external_ingress_admissions(self) -> u64 {
        self.external_ingress_admissions
    }

    #[must_use]
    pub const fn external_ingress_effects(self) -> u64 {
        self.external_ingress_effects
    }

    #[must_use]
    pub const fn external_ingress_completions(self) -> u64 {
        self.external_ingress_completions
    }

    #[must_use]
    pub const fn duplicate_or_superseded_admissions(self) -> u64 {
        self.duplicate_or_superseded_admissions
    }
}

#[derive(Clone, Copy)]
struct RetainedRefresh {
    ticket: PmRefreshTicket,
    admitted_at_ns: u64,
}

pub(super) struct PmRefreshObligations {
    tickets: [Option<RetainedRefresh>; MAX_PM_REFRESH_OBLIGATIONS],
    len: usize,
    oldest_in_flight_age_ns: u64,
    maximum_observed_age_ns: u64,
    retry_effects: u64,
    fill_observed_high_water: usize,
    fill_observed_admissions: u64,
    fill_observed_effects: u64,
    fill_observed_completions: u64,
    external_ingress_high_water: usize,
    external_ingress_admissions: u64,
    external_ingress_effects: u64,
    external_ingress_completions: u64,
    duplicate_or_superseded_admissions: u64,
}

impl PmRefreshObligations {
    pub(super) const fn new() -> Self {
        Self {
            tickets: [None; MAX_PM_REFRESH_OBLIGATIONS],
            len: 0,
            oldest_in_flight_age_ns: 0,
            maximum_observed_age_ns: 0,
            retry_effects: 0,
            fill_observed_high_water: 0,
            fill_observed_admissions: 0,
            fill_observed_effects: 0,
            fill_observed_completions: 0,
            external_ingress_high_water: 0,
            external_ingress_admissions: 0,
            external_ingress_effects: 0,
            external_ingress_completions: 0,
            duplicate_or_superseded_admissions: 0,
        }
    }

    fn ensure_can_retain(&self, ticket: PmRefreshTicket) -> Result<(), PmCoordinatorError> {
        if self
            .tickets
            .iter()
            .flatten()
            .any(|current| current.ticket == ticket)
        {
            return Ok(());
        }
        if self.len == MAX_PM_REFRESH_OBLIGATIONS {
            Err(PmCoordinatorError::RefreshRetentionSaturated)
        } else {
            Ok(())
        }
    }

    fn retain(
        &mut self,
        ticket: PmRefreshTicket,
        admitted_at_ns: u64,
    ) -> Result<(), PmCoordinatorError> {
        self.ensure_can_retain(ticket)?;
        if self
            .tickets
            .iter()
            .flatten()
            .any(|current| current.ticket == ticket)
        {
            return Ok(());
        }
        let slot = self
            .tickets
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(PmCoordinatorError::RefreshRetentionSaturated)?;
        *slot = Some(RetainedRefresh {
            ticket,
            admitted_at_ns,
        });
        self.len += 1;
        if self.len == 1 {
            self.oldest_in_flight_age_ns = 0;
        }
        if ticket.key().reason() == PmRefreshReason::FillObserved {
            self.fill_observed_admissions = self.fill_observed_admissions.saturating_add(1);
            self.fill_observed_high_water = self
                .fill_observed_high_water
                .max(self.count(PmRefreshReason::FillObserved));
        } else if ticket.key().reason() == PmRefreshReason::ExternalIngressFault {
            self.external_ingress_admissions = self.external_ingress_admissions.saturating_add(1);
            self.external_ingress_high_water = self
                .external_ingress_high_water
                .max(self.count(PmRefreshReason::ExternalIngressFault));
        }
        Ok(())
    }

    fn first(&self, reason: PmRefreshReason) -> Option<PmRefreshTicket> {
        self.tickets
            .iter()
            .flatten()
            .map(|retained| retained.ticket)
            .find(|ticket| ticket.key().reason() == reason)
    }

    fn contains(&self, ticket: PmRefreshTicket) -> bool {
        self.tickets
            .iter()
            .flatten()
            .any(|retained| retained.ticket == ticket)
    }

    fn remove(&mut self, ticket: PmRefreshTicket) -> bool {
        let Some(slot) = self
            .tickets
            .iter_mut()
            .find(|slot| slot.is_some_and(|retained| retained.ticket == ticket))
        else {
            return false;
        };
        *slot = None;
        self.len -= 1;
        if self.len == 0 {
            self.oldest_in_flight_age_ns = 0;
        }
        true
    }

    fn count(&self, reason: PmRefreshReason) -> usize {
        self.tickets
            .iter()
            .flatten()
            .filter(|retained| retained.ticket.key().reason() == reason)
            .count()
    }

    fn observe_age_and_expired_index(
        &mut self,
        monotonic_service_ns: u64,
    ) -> Result<Option<usize>, PmCoordinatorError> {
        if self.len == 0 {
            self.oldest_in_flight_age_ns = 0;
            return Ok(None);
        }
        let mut oldest_age = 0;
        let mut expired = None;
        for (index, retained) in self.tickets.iter().enumerate() {
            let Some(retained) = retained else {
                continue;
            };
            let age = monotonic_service_ns
                .checked_sub(retained.admitted_at_ns)
                .ok_or(PmCoordinatorError::ClockRegression)?;
            oldest_age = oldest_age.max(age);
            if expired.is_none() && age > RECONCILIATION_REQUEST_MAXIMUM_AGE_NS {
                expired = Some(index);
            }
        }
        self.oldest_in_flight_age_ns = oldest_age;
        self.maximum_observed_age_ns = self.maximum_observed_age_ns.max(oldest_age);
        Ok(expired)
    }

    fn record_retry(&mut self, index: usize, monotonic_service_ns: u64) {
        self.tickets[index]
            .as_mut()
            .expect("expired refresh slot remains retained")
            .admitted_at_ns = monotonic_service_ns;
        self.retry_effects = self.retry_effects.saturating_add(1);
        self.oldest_in_flight_age_ns = self
            .tickets
            .iter()
            .flatten()
            .map(|retained| monotonic_service_ns.saturating_sub(retained.admitted_at_ns))
            .max()
            .unwrap_or(0);
    }

    pub(super) fn has_reason(&self, reason: PmRefreshReason) -> bool {
        self.count(reason) != 0
    }

    fn record_effect(&mut self, reason: PmRefreshReason) {
        if reason == PmRefreshReason::FillObserved {
            self.fill_observed_effects = self.fill_observed_effects.saturating_add(1);
        } else if reason == PmRefreshReason::ExternalIngressFault {
            self.external_ingress_effects = self.external_ingress_effects.saturating_add(1);
        }
    }

    fn record_completion(&mut self, reason: PmRefreshReason) {
        if reason == PmRefreshReason::FillObserved {
            self.fill_observed_completions = self.fill_observed_completions.saturating_add(1);
        } else if reason == PmRefreshReason::ExternalIngressFault {
            self.external_ingress_completions = self.external_ingress_completions.saturating_add(1);
        }
    }

    pub(super) fn record_duplicate_or_superseded(&mut self) {
        self.duplicate_or_superseded_admissions =
            self.duplicate_or_superseded_admissions.saturating_add(1);
    }

    fn projection(
        &self,
        canonical_insertions: u64,
        total_pending: usize,
        fill_observed_waiting: usize,
        ambiguous_order_waiting: usize,
        external_ingress_waiting: usize,
    ) -> PmRefreshObligationMetrics {
        let fill_observed_in_flight = self.count(PmRefreshReason::FillObserved);
        let ambiguous_order_in_flight = self.count(PmRefreshReason::AmbiguousOrder);
        let external_ingress_in_flight = self.count(PmRefreshReason::ExternalIngressFault);
        PmRefreshObligationMetrics {
            canonical_insertions,
            total_pending,
            total_in_flight: self.len,
            ambiguous_order_pending: ambiguous_order_waiting + ambiguous_order_in_flight,
            ambiguous_order_in_flight,
            oldest_in_flight_age_ns: self.oldest_in_flight_age_ns,
            maximum_observed_age_ns: self.maximum_observed_age_ns,
            retry_effects: self.retry_effects,
            fill_observed_pending: fill_observed_waiting + fill_observed_in_flight,
            fill_observed_in_flight,
            fill_observed_high_water: self.fill_observed_high_water,
            fill_observed_admissions: self.fill_observed_admissions,
            fill_observed_effects: self.fill_observed_effects,
            fill_observed_completions: self.fill_observed_completions,
            external_ingress_pending: external_ingress_waiting + external_ingress_in_flight,
            external_ingress_in_flight,
            external_ingress_high_water: self.external_ingress_high_water,
            external_ingress_admissions: self.external_ingress_admissions,
            external_ingress_effects: self.external_ingress_effects,
            external_ingress_completions: self.external_ingress_completions,
            duplicate_or_superseded_admissions: self.duplicate_or_superseded_admissions,
        }
    }
}

#[cfg(test)]
pub(crate) struct Phase6RefreshAllocationProbe {
    obligations: PmRefreshObligations,
}

#[cfg(test)]
impl Phase6RefreshAllocationProbe {
    pub(crate) const fn new() -> Self {
        Self {
            obligations: PmRefreshObligations::new(),
        }
    }

    pub(crate) fn retain(
        &mut self,
        ticket: PmRefreshTicket,
        admitted_at_ns: u64,
    ) -> Result<(), PmCoordinatorError> {
        self.obligations.retain(ticket, admitted_at_ns)
    }

    pub(crate) const fn len(&self) -> usize {
        self.obligations.len
    }
}

impl<M: PmQuoteModel> PmCoordinator<M> {
    pub(crate) fn refresh_obligation_metrics(&self) -> PmRefreshObligationMetrics {
        let counters = self.mutation.refresh_counters();
        let canonical_insertions = counters
            .requirements()
            .saturating_sub(counters.deduplicated())
            .saturating_sub(counters.superseded_in_flight())
            .saturating_sub(counters.saturations());
        self.refresh_obligations.projection(
            canonical_insertions,
            self.mutation.refresh_obligation_count(),
            self.mutation
                .pending_refresh_count_for(PmRefreshReason::FillObserved),
            self.mutation
                .pending_refresh_count_for(PmRefreshReason::AmbiguousOrder),
            self.mutation
                .pending_refresh_count_for(PmRefreshReason::ExternalIngressFault),
        )
    }

    pub(super) fn admit_refresh_reason(
        &mut self,
        reason: PmRefreshReason,
        monotonic_service_ns: u64,
        effects: &mut PmProductEffectBatch,
    ) -> Result<bool, PmCoordinatorError> {
        let Some(ticket) = self.mutation.pending_refresh(reason) else {
            return Ok(false);
        };
        self.admit_refresh_ticket(ticket, monotonic_service_ns, effects)
    }

    pub(super) fn admit_next_refresh(
        &mut self,
        monotonic_service_ns: u64,
        effects: &mut PmProductEffectBatch,
    ) -> Result<bool, PmCoordinatorError> {
        let Some(ticket) = self.mutation.next_pending_refresh() else {
            return Ok(false);
        };
        self.admit_refresh_ticket(ticket, monotonic_service_ns, effects)
    }

    fn admit_refresh_ticket(
        &mut self,
        ticket: PmRefreshTicket,
        monotonic_service_ns: u64,
        effects: &mut PmProductEffectBatch,
    ) -> Result<bool, PmCoordinatorError> {
        self.refresh_obligations.ensure_can_retain(ticket)?;
        match self.mutation.mark_refresh_admitted(ticket)? {
            PmRefreshAdmission::Admitted(admitted) if admitted == ticket => {
                self.refresh_obligations
                    .retain(ticket, monotonic_service_ns)?;
                effects.push(PmProductEffect::ReconciliationRefresh(
                    PmRefreshEffect::new(
                        self.account_scope,
                        self.instrument,
                        PmRefreshEffectKind::CompleteReconciliation,
                    ),
                ))?;
                self.refresh_obligations
                    .record_effect(ticket.key().reason());
                self.counters.refresh_effects = self.counters.refresh_effects.saturating_add(1);
                Ok(true)
            }
            PmRefreshAdmission::AlreadyInFlight(in_flight)
                if in_flight.key() == ticket.key()
                    && self.refresh_obligations.contains(in_flight) =>
            {
                self.refresh_obligations.record_duplicate_or_superseded();
                Ok(false)
            }
            PmRefreshAdmission::Admitted(_)
            | PmRefreshAdmission::AlreadyInFlight(_)
            | PmRefreshAdmission::NotRequired(_)
            | PmRefreshAdmission::Stale(_) => Err(PmCoordinatorError::RefreshAdmissionMismatch),
        }
    }

    pub(super) fn complete_refresh_reason(
        &mut self,
        reason: PmRefreshReason,
    ) -> Result<bool, PmCoordinatorError> {
        let Some(ticket) = self.refresh_obligations.first(reason) else {
            return Ok(false);
        };
        match self.mutation.complete_refresh(ticket)? {
            PmRefreshCompletion::Cleared(completed)
            | PmRefreshCompletion::NewerRequirementRetained { completed, .. }
                if completed == ticket =>
            {
                if !self.refresh_obligations.remove(ticket) {
                    return Err(PmCoordinatorError::RefreshAdmissionMismatch);
                }
                self.refresh_obligations.record_completion(reason);
                Ok(true)
            }
            PmRefreshCompletion::Cleared(_)
            | PmRefreshCompletion::NewerRequirementRetained { .. }
            | PmRefreshCompletion::Stale(_) => Err(PmCoordinatorError::RefreshAdmissionMismatch),
        }
    }

    pub(super) fn complete_reconciled_refresh_reason(
        &mut self,
        reason: PmRefreshReason,
        monotonic_service_ns: u64,
        effects: &mut PmProductEffectBatch,
    ) -> Result<bool, PmCoordinatorError> {
        if !self.complete_refresh_reason(reason)? {
            return Ok(false);
        }
        let _newer = self.admit_refresh_reason(reason, monotonic_service_ns, effects)?;
        Ok(true)
    }

    pub(super) fn retry_expired_refresh(
        &mut self,
        monotonic_service_ns: u64,
    ) -> Result<bool, PmCoordinatorError> {
        let expired = match self
            .refresh_obligations
            .observe_age_and_expired_index(monotonic_service_ns)
        {
            Ok(expired) => expired,
            Err(error) => {
                self.latch_scheduler_failure(PmControlReason::ContractViolation);
                return Err(error);
            }
        };
        let Some(index) = expired else {
            return Ok(false);
        };
        let mut effects = PmProductEffectBatch::new();
        effects.push(PmProductEffect::ReconciliationRefresh(
            PmRefreshEffect::new(
                self.account_scope,
                self.instrument,
                PmRefreshEffectKind::CompleteReconciliation,
            ),
        ))?;
        self.publish_effect_batch(effects)?;
        self.refresh_obligations
            .record_retry(index, monotonic_service_ns);
        self.counters.refresh_effects = self.counters.refresh_effects.saturating_add(1);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::PmRefreshObligations;

    #[test]
    fn empty_age_observation_keeps_age_metrics_stable_without_a_ticket_scan() {
        let mut obligations = PmRefreshObligations::new();
        obligations.oldest_in_flight_age_ns = 7;
        obligations.maximum_observed_age_ns = 11;

        assert_eq!(
            obligations
                .observe_age_and_expired_index(1)
                .expect("an empty refresh set cannot regress a retained ticket clock"),
            None
        );
        assert_eq!(obligations.oldest_in_flight_age_ns, 0);
        assert_eq!(obligations.maximum_observed_age_ns, 11);
    }
}
