use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use reap_pm_core::PmClientOrderKey;
use thiserror::Error;

use reap_pm_state::PmOwnedCancelIntent;

use super::{PmAuthorityRevisions, PreparedPmCancel, PreparedPmQuote};

pub(crate) const PM_MUTATION_DISPATCH_CAPACITY: usize = 256;
pub(crate) const PM_MUTATION_DISPATCH_MAX_AGE_NS: u64 = 250_000_000;

// Compatibility names retained only for the frozen Goal-F unit-test surface.
#[cfg(test)]
pub(crate) const PM_FAKE_EFFECT_CAPACITY: usize = PM_MUTATION_DISPATCH_CAPACITY;

/// One exact prepared backend-neutral mutation waiting for deterministic
/// owner-loop service.
///
/// Prepared authority remains move-only. The queue never exposes a command or
/// transport object; service can move the value only into the statically
/// composed fixture executor or authenticated worker.
#[derive(Debug)]
pub(crate) enum PmPreparedMutation {
    Quote {
        authority: PreparedPmQuote,
    },
    Cancel {
        authority: PreparedPmCancel,
        owned_intent: PmOwnedCancelIntent,
    },
    #[cfg(test)]
    Synthetic {
        kind: PmPreparedMutationKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmPreparedMutationKind {
    Quote,
    Cancel,
}

impl PmPreparedMutation {
    pub(crate) const fn kind(&self) -> PmPreparedMutationKind {
        match self {
            Self::Quote { .. } => PmPreparedMutationKind::Quote,
            Self::Cancel { .. } => PmPreparedMutationKind::Cancel,
            #[cfg(test)]
            Self::Synthetic { kind } => *kind,
        }
    }

    #[cfg(test)]
    const fn synthetic(kind: PmPreparedMutationKind) -> Self {
        Self::Synthetic { kind }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct PmMutationDispatchQueueMetrics {
    reservations: u64,
    released_before_journal: u64,
    committed_after_durability: u64,
    serviced: u64,
    retained_after_durable_failure: u64,
    invalidated_after_durability: u64,
    retained_after_commit_failure: u64,
    retained_after_age: u64,
    retained_after_suppression: u64,
    retained_after_revision_change: u64,
    aged_safety_services: u64,
    saturations: u64,
    age_faults: u64,
    clock_regressions: u64,
    high_water: u16,
    maximum_observed_age_ns: u64,
}

impl PmMutationDispatchQueueMetrics {
    pub(crate) const fn reservations(self) -> u64 {
        self.reservations
    }

    pub(crate) const fn released_before_journal(self) -> u64 {
        self.released_before_journal
    }

    pub(crate) const fn committed_after_durability(self) -> u64 {
        self.committed_after_durability
    }

    pub(crate) const fn serviced(self) -> u64 {
        self.serviced
    }

    pub(crate) const fn retained_after_durable_failure(self) -> u64 {
        self.retained_after_durable_failure
    }

    pub(crate) const fn invalidated_after_durability(self) -> u64 {
        self.invalidated_after_durability
    }

    pub(crate) const fn retained_after_commit_failure(self) -> u64 {
        self.retained_after_commit_failure
    }

    pub(crate) const fn retained_after_age(self) -> u64 {
        self.retained_after_age
    }

    pub(crate) const fn retained_after_suppression(self) -> u64 {
        self.retained_after_suppression
    }

    pub(crate) const fn retained_after_revision_change(self) -> u64 {
        self.retained_after_revision_change
    }

    pub(crate) const fn aged_safety_services(self) -> u64 {
        self.aged_safety_services
    }

    pub(crate) const fn saturations(self) -> u64 {
        self.saturations
    }

    pub(crate) const fn age_faults(self) -> u64 {
        self.age_faults
    }

    pub(crate) const fn clock_regressions(self) -> u64 {
        self.clock_regressions
    }

    pub(crate) const fn high_water(self) -> u16 {
        self.high_water
    }

    pub(crate) const fn maximum_observed_age_ns(self) -> u64 {
        self.maximum_observed_age_ns
    }
}

/// Copied observation of the bounded backend-neutral dispatch authority queue.
///
/// Counts expose queue pressure and fail-closed retention without exposing a
/// prepared command, permit, or mutation authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmEffectDispatchMetrics {
    capacity: usize,
    depth: usize,
    queued: usize,
    blocked: usize,
    retained: usize,
    quote_suppressed: bool,
    reservations: u64,
    released_before_journal: u64,
    committed_after_durability: u64,
    serviced: u64,
    retained_after_durable_failure: u64,
    invalidated_after_durability: u64,
    retained_after_commit_failure: u64,
    retained_after_age: u64,
    retained_after_suppression: u64,
    retained_after_revision_change: u64,
    aged_safety_services: u64,
    saturations: u64,
    age_faults: u64,
    clock_regressions: u64,
    high_water: u16,
    maximum_observed_age_ns: u64,
}

impl PmEffectDispatchMetrics {
    #[must_use]
    pub const fn capacity(self) -> usize {
        self.capacity
    }

    #[must_use]
    pub const fn depth(self) -> usize {
        self.depth
    }

    #[must_use]
    pub const fn queued(self) -> usize {
        self.queued
    }

    #[must_use]
    pub const fn blocked(self) -> usize {
        self.blocked
    }

    #[must_use]
    pub const fn retained(self) -> usize {
        self.retained
    }

    #[must_use]
    pub const fn quote_suppressed(self) -> bool {
        self.quote_suppressed
    }

    #[must_use]
    pub const fn reservations(self) -> u64 {
        self.reservations
    }

    #[must_use]
    pub const fn released_before_journal(self) -> u64 {
        self.released_before_journal
    }

    #[must_use]
    pub const fn committed_after_durability(self) -> u64 {
        self.committed_after_durability
    }

    #[must_use]
    pub const fn serviced(self) -> u64 {
        self.serviced
    }

    #[must_use]
    pub const fn retained_after_durable_failure(self) -> u64 {
        self.retained_after_durable_failure
    }

    #[must_use]
    pub const fn invalidated_after_durability(self) -> u64 {
        self.invalidated_after_durability
    }

    #[must_use]
    pub const fn retained_after_commit_failure(self) -> u64 {
        self.retained_after_commit_failure
    }

    #[must_use]
    pub const fn retained_after_age(self) -> u64 {
        self.retained_after_age
    }

    #[must_use]
    pub const fn retained_after_suppression(self) -> u64 {
        self.retained_after_suppression
    }

    #[must_use]
    pub const fn retained_after_revision_change(self) -> u64 {
        self.retained_after_revision_change
    }

    #[must_use]
    pub const fn aged_safety_services(self) -> u64 {
        self.aged_safety_services
    }

    #[must_use]
    pub const fn saturations(self) -> u64 {
        self.saturations
    }

    #[must_use]
    pub const fn age_faults(self) -> u64 {
        self.age_faults
    }

    #[must_use]
    pub const fn clock_regressions(self) -> u64 {
        self.clock_regressions
    }

    #[must_use]
    pub const fn high_water(self) -> u16 {
        self.high_water
    }

    #[must_use]
    pub const fn maximum_observed_age_ns(self) -> u64 {
        self.maximum_observed_age_ns
    }
}

/// Move-only capacity held before a mutation intent is journaled.
///
/// A permit belongs to one queue incarnation. It is released only when the
/// intent was not accepted by storage, converted into a queued effect after a
/// durable acknowledgement, or explicitly retained fail-closed after the
/// accepted record lost its acknowledgement.
#[derive(Debug)]
pub(crate) struct PmMutationDispatchPermit {
    owner: u64,
    ordinal: u64,
}

#[derive(Debug)]
pub(crate) struct PmMutationDispatchQueue {
    owner: u64,
    next_ordinal: u64,
    reserved: Vec<u64>,
    queued: VecDeque<PmQueuedMutation>,
    blocked: VecDeque<PmQueuedMutation>,
    metrics: PmMutationDispatchQueueMetrics,
    quote_suppressed: bool,
}

impl PmMutationDispatchQueue {
    pub(crate) fn new() -> Result<Self, PmMutationDispatchQueueError> {
        static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);
        let owner = NEXT_OWNER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| PmMutationDispatchQueueError::OwnerIdentityExhausted)?;
        Ok(Self {
            owner,
            next_ordinal: 1,
            reserved: Vec::with_capacity(PM_MUTATION_DISPATCH_CAPACITY),
            queued: VecDeque::with_capacity(PM_MUTATION_DISPATCH_CAPACITY),
            blocked: VecDeque::with_capacity(PM_MUTATION_DISPATCH_CAPACITY),
            metrics: PmMutationDispatchQueueMetrics::default(),
            quote_suppressed: false,
        })
    }

    pub(crate) fn try_reserve(
        &mut self,
    ) -> Result<PmMutationDispatchPermit, PmMutationDispatchQueueError> {
        if self.depth() >= PM_MUTATION_DISPATCH_CAPACITY {
            self.metrics.saturations = self.metrics.saturations.saturating_add(1);
            self.quote_suppressed = true;
            return Err(PmMutationDispatchQueueError::Full);
        }
        let ordinal = self.next_ordinal;
        self.next_ordinal = ordinal
            .checked_add(1)
            .ok_or(PmMutationDispatchQueueError::PermitIdentityExhausted)?;
        self.reserved.push(ordinal);
        self.metrics.reservations = self.metrics.reservations.saturating_add(1);
        self.note_high_water();
        Ok(PmMutationDispatchPermit {
            owner: self.owner,
            ordinal,
        })
    }

    pub(crate) fn release_before_journal(
        &mut self,
        permit: PmMutationDispatchPermit,
    ) -> Result<(), PmMutationDispatchQueueError> {
        self.remove_reservation(permit)?;
        self.metrics.released_before_journal =
            self.metrics.released_before_journal.saturating_add(1);
        Ok(())
    }

    pub(crate) fn retain_after_durable_failure(
        &mut self,
        permit: PmMutationDispatchPermit,
    ) -> Result<(), PmMutationDispatchQueueError> {
        self.validate_permit(&permit)?;
        self.metrics.retained_after_durable_failure = self
            .metrics
            .retained_after_durable_failure
            .saturating_add(1);
        self.quote_suppressed = true;
        // The ordinal intentionally remains in `reserved`: an accepted intent
        // with no durable acknowledgement must keep its effect capacity bound.
        Ok(())
    }

    /// Releases an intent permit only after its durable acknowledgement when
    /// the still-local quote authority was invalidated before backend dispatch.
    pub(crate) fn invalidate_after_durability(
        &mut self,
        permit: PmMutationDispatchPermit,
    ) -> Result<(), PmMutationDispatchQueueError> {
        self.remove_reservation(permit)?;
        self.metrics.invalidated_after_durability =
            self.metrics.invalidated_after_durability.saturating_add(1);
        Ok(())
    }

    pub(crate) fn commit(
        &mut self,
        permit: PmMutationDispatchPermit,
        effect: PmPreparedMutation,
        enqueued_monotonic_ns: u64,
    ) -> Result<(), PmMutationDispatchQueueError> {
        self.validate_permit(&permit)?;
        if self.depth() > PM_MUTATION_DISPATCH_CAPACITY {
            self.metrics.retained_after_commit_failure =
                self.metrics.retained_after_commit_failure.saturating_add(1);
            self.quote_suppressed = true;
            return Err(PmMutationDispatchQueueError::InvariantCapacity);
        }

        // From this point onward the durable effect, rather than a raw
        // reservation ordinal, accounts for the capacity. Even malformed
        // service-clock evidence is retained in bounded quarantine.
        self.remove_reservation(permit)?;
        let queued = PmQueuedMutation {
            effect,
            enqueued_monotonic_ns,
        };
        if enqueued_monotonic_ns == 0 {
            self.blocked.push_back(queued);
            self.metrics.retained_after_commit_failure =
                self.metrics.retained_after_commit_failure.saturating_add(1);
            self.quote_suppressed = true;
            self.note_high_water();
            return Err(PmMutationDispatchQueueError::InvalidMonotonicTime);
        }
        self.queued.push_back(queued);
        self.metrics.committed_after_durability =
            self.metrics.committed_after_durability.saturating_add(1);
        self.note_high_water();
        Ok(())
    }

    pub(crate) fn pop_at(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<Option<PmPreparedMutation>, PmMutationDispatchQueueError> {
        if self.preflight_front(monotonic_now_ns)?.is_none() {
            return Ok(None);
        }
        Ok(self.pop_front_effect())
    }

    /// Services only a quote whose approval remains current at the final
    /// backend-dispatch boundary.
    ///
    /// Revision change and exact approval expiry retain the move-only
    /// authority in bounded quarantine. No command reaches any backend.
    pub(crate) fn pop_quote_at(
        &mut self,
        monotonic_now_ns: u64,
        current_revisions: Option<PmAuthorityRevisions>,
    ) -> Result<Option<PmPreparedMutation>, PmMutationDispatchQueueError> {
        let Some(kind) = self.preflight_front(monotonic_now_ns)? else {
            return Ok(None);
        };
        if kind != PmPreparedMutationKind::Quote {
            return Err(PmMutationDispatchQueueError::EffectKindMismatch);
        }
        let current = self
            .queued
            .front()
            .is_some_and(|front| match &front.effect {
                PmPreparedMutation::Quote { authority } => {
                    current_revisions == Some(authority.revisions())
                        && monotonic_now_ns < authority.expires_at_monotonic_ns()
                }
                PmPreparedMutation::Cancel { .. } => false,
                #[cfg(test)]
                PmPreparedMutation::Synthetic { kind } => {
                    *kind == PmPreparedMutationKind::Quote && current_revisions.is_some()
                }
            });
        if !current {
            self.quarantine_front_quote();
            self.metrics.retained_after_revision_change = self
                .metrics
                .retained_after_revision_change
                .saturating_add(1);
            return Err(PmMutationDispatchQueueError::QuoteAuthorityInvalidated);
        }
        Ok(self.pop_front_effect())
    }

    /// Retains the head quote without dispatch when the statically selected
    /// authenticated place worker cannot accept another operation.
    ///
    /// Cancels must remain serviceable while placement is suppressed. Moving
    /// only the head quote into the existing bounded blocked queue exposes a
    /// following cancel without cloning or discarding prepared authority.
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn quarantine_front_quote_for_authenticated_backpressure(
        &mut self,
    ) -> Result<(), PmMutationDispatchQueueError> {
        self.suppress_front_quote_without_dispatch()
    }

    /// Retains a prepared quote during controlled shutdown without
    /// classifying the intentional no-send transition as backend saturation.
    #[cfg(any(test, feature = "loopback-evidence"))]
    pub(crate) fn suppress_front_quote_for_shutdown(
        &mut self,
    ) -> Result<(), PmMutationDispatchQueueError> {
        self.suppress_front_quote_without_dispatch()
    }

    #[cfg(any(test, feature = "loopback-evidence"))]
    fn suppress_front_quote_without_dispatch(
        &mut self,
    ) -> Result<(), PmMutationDispatchQueueError> {
        if self.next_kind() != Some(PmPreparedMutationKind::Quote) {
            return Err(PmMutationDispatchQueueError::EffectKindMismatch);
        }
        self.quote_suppressed = true;
        self.quarantine_front_quote();
        self.metrics.retained_after_suppression =
            self.metrics.retained_after_suppression.saturating_add(1);
        Ok(())
    }

    fn preflight_front(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<Option<PmPreparedMutationKind>, PmMutationDispatchQueueError> {
        let Some((kind, enqueued_monotonic_ns)) = self
            .queued
            .front()
            .map(|front| (front.effect.kind(), front.enqueued_monotonic_ns))
        else {
            return Ok(None);
        };
        let Some(age_ns) = monotonic_now_ns.checked_sub(enqueued_monotonic_ns) else {
            self.metrics.clock_regressions = self.metrics.clock_regressions.saturating_add(1);
            self.quote_suppressed = true;
            return Err(PmMutationDispatchQueueError::ClockRegression);
        };
        self.metrics.maximum_observed_age_ns = self.metrics.maximum_observed_age_ns.max(age_ns);
        if age_ns > PM_MUTATION_DISPATCH_MAX_AGE_NS {
            self.metrics.age_faults = self.metrics.age_faults.saturating_add(1);
            self.quote_suppressed = true;
            if kind == PmPreparedMutationKind::Quote {
                self.quarantine_front_quote();
                self.metrics.retained_after_age = self.metrics.retained_after_age.saturating_add(1);
                return Err(PmMutationDispatchQueueError::AgeExceeded);
            }
            self.metrics.aged_safety_services = self.metrics.aged_safety_services.saturating_add(1);
        } else if self.quote_suppressed && kind == PmPreparedMutationKind::Quote {
            self.quarantine_front_quote();
            self.metrics.retained_after_suppression =
                self.metrics.retained_after_suppression.saturating_add(1);
            return Err(PmMutationDispatchQueueError::QuoteSuppressed);
        }
        Ok(Some(kind))
    }

    fn pop_front_effect(&mut self) -> Option<PmPreparedMutation> {
        let effect = self
            .queued
            .pop_front()
            .expect("front effect remains present after age preflight")
            .effect;
        self.metrics.serviced = self.metrics.serviced.saturating_add(1);
        Some(effect)
    }

    pub(crate) fn next_kind(&self) -> Option<PmPreparedMutationKind> {
        self.queued.front().map(|queued| queued.effect.kind())
    }

    pub(crate) fn contains_prepared_quote(&self, client_order: PmClientOrderKey) -> bool {
        self.queued
            .iter()
            .chain(&self.blocked)
            .any(|queued| match &queued.effect {
                PmPreparedMutation::Quote { authority } => authority.client_order() == client_order,
                PmPreparedMutation::Cancel { .. } => false,
                #[cfg(test)]
                PmPreparedMutation::Synthetic { .. } => false,
            })
    }

    pub(crate) fn invalidate_prepared_quote(
        &mut self,
        client_order: PmClientOrderKey,
    ) -> Result<(), PmMutationDispatchQueueError> {
        if let Some(index) = self.queued.iter().position(|queued| {
            matches!(
                &queued.effect,
                PmPreparedMutation::Quote { authority }
                    if authority.client_order() == client_order
            )
        }) {
            let removed = self
                .queued
                .remove(index)
                .expect("located prepared quote remains queued");
            debug_assert_eq!(removed.effect.kind(), PmPreparedMutationKind::Quote);
        } else if let Some(index) = self.blocked.iter().position(|queued| {
            matches!(
                &queued.effect,
                PmPreparedMutation::Quote { authority }
                    if authority.client_order() == client_order
            )
        }) {
            let removed = self
                .blocked
                .remove(index)
                .expect("located prepared quote remains blocked");
            debug_assert_eq!(removed.effect.kind(), PmPreparedMutationKind::Quote);
        } else {
            return Err(PmMutationDispatchQueueError::UnknownPreparedQuote);
        }
        self.metrics.invalidated_after_durability =
            self.metrics.invalidated_after_durability.saturating_add(1);
        Ok(())
    }

    pub(crate) const fn quote_suppressed(&self) -> bool {
        self.quote_suppressed
    }

    pub(crate) fn depth(&self) -> usize {
        self.reserved.len() + self.queued.len() + self.blocked.len()
    }

    pub(crate) fn queued_len(&self) -> usize {
        self.queued.len()
    }

    pub(crate) fn blocked_len(&self) -> usize {
        self.blocked.len()
    }

    pub(crate) fn retained_permits(&self) -> usize {
        self.reserved.len() + self.blocked.len()
    }

    pub(crate) const fn metrics(&self) -> PmMutationDispatchQueueMetrics {
        self.metrics
    }

    pub(crate) fn projection(&self) -> PmEffectDispatchMetrics {
        let metrics = self.metrics();
        PmEffectDispatchMetrics {
            capacity: PM_MUTATION_DISPATCH_CAPACITY,
            depth: self.depth(),
            queued: self.queued_len(),
            blocked: self.blocked_len(),
            retained: self.retained_permits(),
            quote_suppressed: self.quote_suppressed(),
            reservations: metrics.reservations(),
            released_before_journal: metrics.released_before_journal(),
            committed_after_durability: metrics.committed_after_durability(),
            serviced: metrics.serviced(),
            retained_after_durable_failure: metrics.retained_after_durable_failure(),
            invalidated_after_durability: metrics.invalidated_after_durability(),
            retained_after_commit_failure: metrics.retained_after_commit_failure(),
            retained_after_age: metrics.retained_after_age(),
            retained_after_suppression: metrics.retained_after_suppression(),
            retained_after_revision_change: metrics.retained_after_revision_change(),
            aged_safety_services: metrics.aged_safety_services(),
            saturations: metrics.saturations(),
            age_faults: metrics.age_faults(),
            clock_regressions: metrics.clock_regressions(),
            high_water: metrics.high_water(),
            maximum_observed_age_ns: metrics.maximum_observed_age_ns(),
        }
    }

    pub(crate) fn reserved_capacity_bytes(&self) -> usize {
        self.reserved.capacity() * std::mem::size_of::<u64>()
            + self.queued.capacity() * std::mem::size_of::<PmQueuedMutation>()
            + self.blocked.capacity() * std::mem::size_of::<PmQueuedMutation>()
    }

    fn validate_permit(
        &self,
        permit: &PmMutationDispatchPermit,
    ) -> Result<(), PmMutationDispatchQueueError> {
        if permit.owner != self.owner {
            return Err(PmMutationDispatchQueueError::WrongOwner);
        }
        if self.reserved.binary_search(&permit.ordinal).is_err() {
            return Err(PmMutationDispatchQueueError::UnknownPermit);
        }
        Ok(())
    }

    fn remove_reservation(
        &mut self,
        permit: PmMutationDispatchPermit,
    ) -> Result<(), PmMutationDispatchQueueError> {
        self.validate_permit(&permit)?;
        let index = self
            .reserved
            .binary_search(&permit.ordinal)
            .expect("validated permit remains reserved");
        self.reserved.remove(index);
        Ok(())
    }

    fn note_high_water(&mut self) {
        let depth = u16::try_from(self.depth()).expect("fixed effect capacity fits u16");
        self.metrics.high_water = self.metrics.high_water.max(depth);
    }

    fn quarantine_front_quote(&mut self) {
        let effect = self
            .queued
            .pop_front()
            .expect("front quote remains present for bounded quarantine");
        debug_assert_eq!(effect.effect.kind(), PmPreparedMutationKind::Quote);
        self.blocked.push_back(effect);
    }
}

/// Compatibility name for Goal F fake-backend metrics.
pub type PmFakeEffectMetrics = PmEffectDispatchMetrics;

#[derive(Debug)]
struct PmQueuedMutation {
    effect: PmPreparedMutation,
    enqueued_monotonic_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum PmMutationDispatchQueueError {
    #[error("PM mutation-dispatch queue owner identity is exhausted")]
    OwnerIdentityExhausted,
    #[error("PM mutation-dispatch permit identity is exhausted")]
    PermitIdentityExhausted,
    #[error("PM mutation-dispatch queue is full")]
    Full,
    #[error("PM mutation-dispatch permit belongs to another queue")]
    WrongOwner,
    #[error("PM mutation-dispatch permit is no longer outstanding")]
    UnknownPermit,
    #[error("PM prepared quote is not retained by this mutation-dispatch queue")]
    UnknownPreparedQuote,
    #[error("PM mutation-dispatch queue violated its fixed-capacity invariant")]
    InvariantCapacity,
    #[error("PM mutation-dispatch enqueue requires nonzero monotonic time")]
    InvalidMonotonicTime,
    #[error("PM mutation-dispatch service clock regressed")]
    ClockRegression,
    #[error("PM prepared quote exceeded its maximum mutation-dispatch queue age")]
    AgeExceeded,
    #[error("PM prepared quote is suppressed")]
    QuoteSuppressed,
    #[error("PM prepared quote approval changed or expired before dispatch")]
    QuoteAuthorityInvalidated,
    #[error("PM mutation kind does not match the requested dispatch service")]
    EffectKindMismatch,
}

#[cfg(test)]
pub(crate) type PmFakeEffectQueueError = PmMutationDispatchQueueError;

#[cfg(test)]
pub(crate) struct Phase6FakeEffectAllocationProbe {
    queue: PmMutationDispatchQueue,
}

#[cfg(test)]
impl Phase6FakeEffectAllocationProbe {
    pub(crate) fn new() -> Result<Self, PmMutationDispatchQueueError> {
        Ok(Self {
            queue: PmMutationDispatchQueue::new()?,
        })
    }

    pub(crate) fn attempt(
        &mut self,
        monotonic_ns: u64,
    ) -> Result<(), PmMutationDispatchQueueError> {
        let permit = self.queue.try_reserve()?;
        self.queue.commit(
            permit,
            PmPreparedMutation::synthetic(PmPreparedMutationKind::Quote),
            monotonic_ns,
        )
    }

    pub(crate) fn metrics(&self) -> PmFakeEffectMetrics {
        self.queue.projection()
    }
}

#[cfg(test)]
mod tests {
    use reap_pm_core::SnapshotRevision;

    use super::*;

    fn commit_synthetic(
        queue: &mut PmMutationDispatchQueue,
        kind: PmPreparedMutationKind,
        enqueued_monotonic_ns: u64,
    ) -> Result<(), PmMutationDispatchQueueError> {
        let permit = queue.try_reserve()?;
        queue.commit(
            permit,
            PmPreparedMutation::synthetic(kind),
            enqueued_monotonic_ns,
        )
    }

    fn expect_synthetic(effect: Option<PmPreparedMutation>, expected: PmPreparedMutationKind) {
        let Some(PmPreparedMutation::Synthetic { kind }) = effect else {
            panic!("expected one synthetic fake effect");
        };
        assert_eq!(kind, expected);
    }

    fn revisions() -> PmAuthorityRevisions {
        PmAuthorityRevisions::new(SnapshotRevision::new(1), SnapshotRevision::new(2), 3, 4, 5)
            .unwrap()
    }

    #[test]
    fn final_quote_dispatch_requires_current_revision_evidence() {
        let mut invalidated = PmMutationDispatchQueue::new().unwrap();
        commit_synthetic(&mut invalidated, PmPreparedMutationKind::Quote, 100).unwrap();
        assert_eq!(
            invalidated.pop_quote_at(101, None).unwrap_err(),
            PmMutationDispatchQueueError::QuoteAuthorityInvalidated
        );
        assert_eq!(invalidated.queued_len(), 0);
        assert_eq!(invalidated.blocked_len(), 1);
        assert_eq!(invalidated.metrics().serviced(), 0);
        assert_eq!(invalidated.metrics().retained_after_revision_change(), 1);

        let mut current = PmMutationDispatchQueue::new().unwrap();
        commit_synthetic(&mut current, PmPreparedMutationKind::Quote, 100).unwrap();
        expect_synthetic(
            current.pop_quote_at(101, Some(revisions())).unwrap(),
            PmPreparedMutationKind::Quote,
        );
        assert_eq!(current.metrics().serviced(), 1);
    }

    #[test]
    fn reservations_are_bounded_owner_scoped_and_fail_closed() {
        let mut queue = PmMutationDispatchQueue::new().unwrap();
        let mut permits = Vec::with_capacity(PM_MUTATION_DISPATCH_CAPACITY);
        for _ in 0..PM_MUTATION_DISPATCH_CAPACITY {
            permits.push(queue.try_reserve().unwrap());
        }

        assert_eq!(queue.depth(), PM_MUTATION_DISPATCH_CAPACITY);
        assert_eq!(queue.metrics().high_water(), 256);
        assert_eq!(
            queue.try_reserve().unwrap_err(),
            PmMutationDispatchQueueError::Full
        );
        assert!(queue.quote_suppressed());
        assert_eq!(queue.metrics().saturations(), 1);

        queue
            .release_before_journal(permits.pop().unwrap())
            .unwrap();
        assert_eq!(queue.depth(), PM_MUTATION_DISPATCH_CAPACITY - 1);
        assert_eq!(queue.metrics().released_before_journal(), 1);
    }

    #[test]
    fn every_permit_transition_remains_explicitly_accounted() {
        let mut released = PmMutationDispatchQueue::new().unwrap();
        let permit = released.try_reserve().unwrap();
        released.release_before_journal(permit).unwrap();
        assert_eq!(released.depth(), 0);
        assert_eq!(released.metrics().reservations(), 1);
        assert_eq!(released.metrics().released_before_journal(), 1);

        let mut serviced = PmMutationDispatchQueue::new().unwrap();
        commit_synthetic(&mut serviced, PmPreparedMutationKind::Cancel, 100).unwrap();
        expect_synthetic(
            serviced.pop_at(101).unwrap(),
            PmPreparedMutationKind::Cancel,
        );
        assert_eq!(serviced.depth(), 0);
        assert_eq!(serviced.metrics().reservations(), 1);
        assert_eq!(serviced.metrics().committed_after_durability(), 1);
        assert_eq!(serviced.metrics().serviced(), 1);

        let mut invalid_clock = PmMutationDispatchQueue::new().unwrap();
        assert_eq!(
            commit_synthetic(&mut invalid_clock, PmPreparedMutationKind::Quote, 0).unwrap_err(),
            PmMutationDispatchQueueError::InvalidMonotonicTime
        );
        assert_eq!(invalid_clock.depth(), 1);
        assert_eq!(invalid_clock.queued_len(), 0);
        assert_eq!(invalid_clock.blocked_len(), 1);
        assert_eq!(invalid_clock.retained_permits(), 1);
        assert_eq!(invalid_clock.metrics().reservations(), 1);
        assert_eq!(invalid_clock.metrics().committed_after_durability(), 0);
        assert_eq!(invalid_clock.metrics().retained_after_commit_failure(), 1);
        assert!(invalid_clock.quote_suppressed());
    }

    #[test]
    fn accepted_intent_without_ack_keeps_its_effect_permit_bound() {
        let mut queue = PmMutationDispatchQueue::new().unwrap();
        let permit = queue.try_reserve().unwrap();
        queue.retain_after_durable_failure(permit).unwrap();

        assert_eq!(queue.retained_permits(), 1);
        assert_eq!(queue.queued_len(), 0);
        assert_eq!(queue.metrics().retained_after_durable_failure(), 1);
        assert!(queue.quote_suppressed());
    }

    #[test]
    fn a_sibling_queue_cannot_consume_an_effect_permit() {
        let mut first = PmMutationDispatchQueue::new().unwrap();
        let mut second = PmMutationDispatchQueue::new().unwrap();
        let permit = first.try_reserve().unwrap();

        assert_eq!(
            second.release_before_journal(permit).unwrap_err(),
            PmMutationDispatchQueueError::WrongOwner
        );
        assert_eq!(first.retained_permits(), 1);
        assert_eq!(second.retained_permits(), 0);
    }

    #[test]
    fn aged_quote_is_quarantined_and_can_never_dispatch() {
        let mut queue = PmMutationDispatchQueue::new().unwrap();
        commit_synthetic(&mut queue, PmPreparedMutationKind::Quote, 100).unwrap();

        let observed_age = PM_MUTATION_DISPATCH_MAX_AGE_NS + 1;
        assert_eq!(
            queue.pop_at(100 + observed_age).unwrap_err(),
            PmMutationDispatchQueueError::AgeExceeded
        );
        assert!(queue.quote_suppressed());
        assert_eq!(queue.depth(), 1);
        assert_eq!(queue.queued_len(), 0);
        assert_eq!(queue.blocked_len(), 1);
        assert_eq!(queue.retained_permits(), 1);
        assert_eq!(queue.metrics().age_faults(), 1);
        assert_eq!(queue.metrics().retained_after_age(), 1);
        assert_eq!(queue.metrics().serviced(), 0);
        assert_eq!(queue.metrics().maximum_observed_age_ns(), observed_age);
        assert!(queue.pop_at(100 + observed_age + 1).unwrap().is_none());
    }

    #[test]
    fn quote_at_the_exact_age_limit_is_still_serviceable() {
        let mut queue = PmMutationDispatchQueue::new().unwrap();
        commit_synthetic(&mut queue, PmPreparedMutationKind::Quote, 100).unwrap();

        expect_synthetic(
            queue.pop_at(100 + PM_MUTATION_DISPATCH_MAX_AGE_NS).unwrap(),
            PmPreparedMutationKind::Quote,
        );
        assert_eq!(
            queue.metrics().maximum_observed_age_ns(),
            PM_MUTATION_DISPATCH_MAX_AGE_NS
        );
        assert_eq!(queue.metrics().age_faults(), 0);
        assert_eq!(queue.metrics().serviced(), 1);
        assert!(!queue.quote_suppressed());
    }

    #[test]
    fn aged_owned_cancel_remains_serviceable_behind_a_quarantined_quote() {
        let mut queue = PmMutationDispatchQueue::new().unwrap();
        commit_synthetic(&mut queue, PmPreparedMutationKind::Quote, 100).unwrap();
        commit_synthetic(&mut queue, PmPreparedMutationKind::Cancel, 101).unwrap();

        let service_time = 102 + PM_MUTATION_DISPATCH_MAX_AGE_NS;
        assert_eq!(
            queue.pop_at(service_time).unwrap_err(),
            PmMutationDispatchQueueError::AgeExceeded
        );
        assert_eq!(queue.next_kind(), Some(PmPreparedMutationKind::Cancel));
        expect_synthetic(
            queue.pop_at(service_time).unwrap(),
            PmPreparedMutationKind::Cancel,
        );

        assert!(queue.quote_suppressed());
        assert_eq!(queue.depth(), 1);
        assert_eq!(queue.queued_len(), 0);
        assert_eq!(queue.blocked_len(), 1);
        assert_eq!(queue.metrics().age_faults(), 2);
        assert_eq!(queue.metrics().aged_safety_services(), 1);
        assert_eq!(queue.metrics().serviced(), 1);
    }

    #[test]
    fn authenticated_place_backpressure_retains_quote_and_exposes_cancel() {
        let mut queue = PmMutationDispatchQueue::new().unwrap();
        commit_synthetic(&mut queue, PmPreparedMutationKind::Quote, 100).unwrap();
        commit_synthetic(&mut queue, PmPreparedMutationKind::Cancel, 101).unwrap();

        queue
            .quarantine_front_quote_for_authenticated_backpressure()
            .unwrap();

        assert_eq!(queue.next_kind(), Some(PmPreparedMutationKind::Cancel));
        expect_synthetic(queue.pop_at(102).unwrap(), PmPreparedMutationKind::Cancel);
        assert_eq!(queue.blocked_len(), 1);
        assert_eq!(queue.queued_len(), 0);
        assert_eq!(queue.metrics().serviced(), 1);
        assert_eq!(queue.metrics().retained_after_suppression(), 1);
        assert!(queue.quote_suppressed());
    }

    #[test]
    fn clock_regression_retains_front_and_safety_cancel_can_retry() {
        let mut queue = PmMutationDispatchQueue::new().unwrap();
        commit_synthetic(&mut queue, PmPreparedMutationKind::Cancel, 100).unwrap();

        assert_eq!(
            queue.pop_at(99).unwrap_err(),
            PmMutationDispatchQueueError::ClockRegression
        );
        assert_eq!(queue.depth(), 1);
        assert_eq!(queue.queued_len(), 1);
        assert_eq!(queue.blocked_len(), 0);
        assert_eq!(queue.metrics().clock_regressions(), 1);
        assert_eq!(queue.metrics().maximum_observed_age_ns(), 0);
        assert_eq!(queue.metrics().serviced(), 0);
        assert!(queue.quote_suppressed());

        expect_synthetic(queue.pop_at(101).unwrap(), PmPreparedMutationKind::Cancel);
        assert_eq!(queue.depth(), 0);
        assert_eq!(queue.metrics().serviced(), 1);
    }

    #[test]
    fn clock_regression_makes_a_quote_permanently_non_dispatchable() {
        let mut queue = PmMutationDispatchQueue::new().unwrap();
        commit_synthetic(&mut queue, PmPreparedMutationKind::Quote, 100).unwrap();

        assert_eq!(
            queue.pop_at(99).unwrap_err(),
            PmMutationDispatchQueueError::ClockRegression
        );
        assert_eq!(
            queue.pop_at(101).unwrap_err(),
            PmMutationDispatchQueueError::QuoteSuppressed
        );
        assert_eq!(queue.depth(), 1);
        assert_eq!(queue.queued_len(), 0);
        assert_eq!(queue.blocked_len(), 1);
        assert_eq!(queue.metrics().retained_after_suppression(), 1);
        assert_eq!(queue.metrics().serviced(), 0);
    }

    #[test]
    fn phase6_fake_effect_row_is_257_attempts_after_256_durable_records() {
        let mut queue = PmMutationDispatchQueue::new().unwrap();
        let reserved_capacity_bytes = queue.reserved_capacity_bytes();
        for ordinal in 0..PM_MUTATION_DISPATCH_CAPACITY {
            commit_synthetic(
                &mut queue,
                PmPreparedMutationKind::Quote,
                100 + u64::try_from(ordinal).unwrap(),
            )
            .unwrap();
        }

        assert_eq!(queue.depth(), PM_MUTATION_DISPATCH_CAPACITY);
        assert_eq!(queue.queued_len(), PM_MUTATION_DISPATCH_CAPACITY);
        assert_eq!(queue.metrics().high_water(), 256);
        assert_eq!(queue.metrics().committed_after_durability(), 256);
        let committed_before_rejection = queue.metrics().committed_after_durability();
        assert_eq!(
            queue.try_reserve().unwrap_err(),
            PmMutationDispatchQueueError::Full
        );
        assert_eq!(
            queue.metrics().committed_after_durability(),
            committed_before_rejection,
            "the 257th attempt is rejected before any record can claim dispatch"
        );
        assert_eq!(queue.metrics().serviced(), 0);
        assert!(queue.quote_suppressed());

        for _ in 0..PM_MUTATION_DISPATCH_CAPACITY {
            assert_eq!(
                queue.pop_at(1_000).unwrap_err(),
                PmMutationDispatchQueueError::QuoteSuppressed
            );
        }
        assert_eq!(queue.depth(), PM_MUTATION_DISPATCH_CAPACITY);
        assert_eq!(queue.queued_len(), 0);
        assert_eq!(queue.blocked_len(), PM_MUTATION_DISPATCH_CAPACITY);
        assert_eq!(queue.metrics().reservations(), 256);
        assert_eq!(queue.metrics().committed_after_durability(), 256);
        assert_eq!(queue.metrics().saturations(), 1);
        assert_eq!(queue.metrics().retained_after_suppression(), 256);
        assert_eq!(queue.metrics().serviced(), 0);
        assert_eq!(queue.reserved_capacity_bytes(), reserved_capacity_bytes);
    }
}
