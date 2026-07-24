use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use reap_pm_core::PmClientOrderKey;
use thiserror::Error;

use reap_pm_state::PmOwnedCancelIntent;

use super::{PmAuthorityRevisions, PreparedPmCancel, PreparedPmQuote};

pub(crate) const PM_FAKE_EFFECT_CAPACITY: usize = 256;
pub(crate) const PM_FAKE_EFFECT_MAX_AGE_NS: u64 = 250_000_000;

/// One exact prepared fake mutation waiting for deterministic owner-loop
/// service.
///
/// Prepared authority remains move-only. The queue never exposes a command or
/// transport object; service can only move the value into the crate-private
/// fake execution role.
#[derive(Debug)]
pub(crate) enum PmPreparedFakeEffect {
    Quote {
        authority: PreparedPmQuote,
    },
    Cancel {
        authority: PreparedPmCancel,
        owned_intent: PmOwnedCancelIntent,
    },
    #[cfg(test)]
    Synthetic {
        kind: PmPreparedFakeEffectKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmPreparedFakeEffectKind {
    Quote,
    Cancel,
}

impl PmPreparedFakeEffect {
    pub(crate) const fn kind(&self) -> PmPreparedFakeEffectKind {
        match self {
            Self::Quote { .. } => PmPreparedFakeEffectKind::Quote,
            Self::Cancel { .. } => PmPreparedFakeEffectKind::Cancel,
            #[cfg(test)]
            Self::Synthetic { kind } => *kind,
        }
    }

    #[cfg(test)]
    const fn synthetic(kind: PmPreparedFakeEffectKind) -> Self {
        Self::Synthetic { kind }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct PmFakeEffectQueueMetrics {
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

impl PmFakeEffectQueueMetrics {
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

/// Copied observation of the bounded fixture-effect authority queue.
///
/// Counts expose queue pressure and fail-closed retention without exposing a
/// prepared command, permit, or mutation authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmFakeEffectMetrics {
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

impl PmFakeEffectMetrics {
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
pub(crate) struct PmFakeEffectPermit {
    owner: u64,
    ordinal: u64,
}

#[derive(Debug)]
pub(crate) struct PmFakeEffectQueue {
    owner: u64,
    next_ordinal: u64,
    reserved: Vec<u64>,
    queued: VecDeque<PmQueuedFakeEffect>,
    blocked: VecDeque<PmQueuedFakeEffect>,
    metrics: PmFakeEffectQueueMetrics,
    quote_suppressed: bool,
}

impl PmFakeEffectQueue {
    pub(crate) fn new() -> Result<Self, PmFakeEffectQueueError> {
        static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);
        let owner = NEXT_OWNER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| PmFakeEffectQueueError::OwnerIdentityExhausted)?;
        Ok(Self {
            owner,
            next_ordinal: 1,
            reserved: Vec::with_capacity(PM_FAKE_EFFECT_CAPACITY),
            queued: VecDeque::with_capacity(PM_FAKE_EFFECT_CAPACITY),
            blocked: VecDeque::with_capacity(PM_FAKE_EFFECT_CAPACITY),
            metrics: PmFakeEffectQueueMetrics::default(),
            quote_suppressed: false,
        })
    }

    pub(crate) fn try_reserve(&mut self) -> Result<PmFakeEffectPermit, PmFakeEffectQueueError> {
        if self.depth() >= PM_FAKE_EFFECT_CAPACITY {
            self.metrics.saturations = self.metrics.saturations.saturating_add(1);
            self.quote_suppressed = true;
            return Err(PmFakeEffectQueueError::Full);
        }
        let ordinal = self.next_ordinal;
        self.next_ordinal = ordinal
            .checked_add(1)
            .ok_or(PmFakeEffectQueueError::PermitIdentityExhausted)?;
        self.reserved.push(ordinal);
        self.metrics.reservations = self.metrics.reservations.saturating_add(1);
        self.note_high_water();
        Ok(PmFakeEffectPermit {
            owner: self.owner,
            ordinal,
        })
    }

    pub(crate) fn release_before_journal(
        &mut self,
        permit: PmFakeEffectPermit,
    ) -> Result<(), PmFakeEffectQueueError> {
        self.remove_reservation(permit)?;
        self.metrics.released_before_journal =
            self.metrics.released_before_journal.saturating_add(1);
        Ok(())
    }

    pub(crate) fn retain_after_durable_failure(
        &mut self,
        permit: PmFakeEffectPermit,
    ) -> Result<(), PmFakeEffectQueueError> {
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
    /// the still-local quote authority was invalidated before fake dispatch.
    pub(crate) fn invalidate_after_durability(
        &mut self,
        permit: PmFakeEffectPermit,
    ) -> Result<(), PmFakeEffectQueueError> {
        self.remove_reservation(permit)?;
        self.metrics.invalidated_after_durability =
            self.metrics.invalidated_after_durability.saturating_add(1);
        Ok(())
    }

    pub(crate) fn commit(
        &mut self,
        permit: PmFakeEffectPermit,
        effect: PmPreparedFakeEffect,
        enqueued_monotonic_ns: u64,
    ) -> Result<(), PmFakeEffectQueueError> {
        self.validate_permit(&permit)?;
        if self.depth() > PM_FAKE_EFFECT_CAPACITY {
            self.metrics.retained_after_commit_failure =
                self.metrics.retained_after_commit_failure.saturating_add(1);
            self.quote_suppressed = true;
            return Err(PmFakeEffectQueueError::InvariantCapacity);
        }

        // From this point onward the durable effect, rather than a raw
        // reservation ordinal, accounts for the capacity. Even malformed
        // service-clock evidence is retained in bounded quarantine.
        self.remove_reservation(permit)?;
        let queued = PmQueuedFakeEffect {
            effect,
            enqueued_monotonic_ns,
        };
        if enqueued_monotonic_ns == 0 {
            self.blocked.push_back(queued);
            self.metrics.retained_after_commit_failure =
                self.metrics.retained_after_commit_failure.saturating_add(1);
            self.quote_suppressed = true;
            self.note_high_water();
            return Err(PmFakeEffectQueueError::InvalidMonotonicTime);
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
    ) -> Result<Option<PmPreparedFakeEffect>, PmFakeEffectQueueError> {
        if self.preflight_front(monotonic_now_ns)?.is_none() {
            return Ok(None);
        }
        Ok(self.pop_front_effect())
    }

    /// Services only a quote whose approval remains current at the final
    /// fixture-dispatch boundary.
    ///
    /// Revision change and exact approval expiry retain the move-only
    /// authority in bounded quarantine. No command reaches the fake transport.
    pub(crate) fn pop_quote_at(
        &mut self,
        monotonic_now_ns: u64,
        current_revisions: Option<PmAuthorityRevisions>,
    ) -> Result<Option<PmPreparedFakeEffect>, PmFakeEffectQueueError> {
        let Some(kind) = self.preflight_front(monotonic_now_ns)? else {
            return Ok(None);
        };
        if kind != PmPreparedFakeEffectKind::Quote {
            return Err(PmFakeEffectQueueError::EffectKindMismatch);
        }
        let current = self
            .queued
            .front()
            .is_some_and(|front| match &front.effect {
                PmPreparedFakeEffect::Quote { authority } => {
                    current_revisions == Some(authority.revisions())
                        && monotonic_now_ns < authority.expires_at_monotonic_ns()
                }
                PmPreparedFakeEffect::Cancel { .. } => false,
                #[cfg(test)]
                PmPreparedFakeEffect::Synthetic { kind } => {
                    *kind == PmPreparedFakeEffectKind::Quote && current_revisions.is_some()
                }
            });
        if !current {
            self.quarantine_front_quote();
            self.metrics.retained_after_revision_change = self
                .metrics
                .retained_after_revision_change
                .saturating_add(1);
            return Err(PmFakeEffectQueueError::QuoteAuthorityInvalidated);
        }
        Ok(self.pop_front_effect())
    }

    fn preflight_front(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<Option<PmPreparedFakeEffectKind>, PmFakeEffectQueueError> {
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
            return Err(PmFakeEffectQueueError::ClockRegression);
        };
        self.metrics.maximum_observed_age_ns = self.metrics.maximum_observed_age_ns.max(age_ns);
        if age_ns > PM_FAKE_EFFECT_MAX_AGE_NS {
            self.metrics.age_faults = self.metrics.age_faults.saturating_add(1);
            self.quote_suppressed = true;
            if kind == PmPreparedFakeEffectKind::Quote {
                self.quarantine_front_quote();
                self.metrics.retained_after_age = self.metrics.retained_after_age.saturating_add(1);
                return Err(PmFakeEffectQueueError::AgeExceeded);
            }
            self.metrics.aged_safety_services = self.metrics.aged_safety_services.saturating_add(1);
        } else if self.quote_suppressed && kind == PmPreparedFakeEffectKind::Quote {
            self.quarantine_front_quote();
            self.metrics.retained_after_suppression =
                self.metrics.retained_after_suppression.saturating_add(1);
            return Err(PmFakeEffectQueueError::QuoteSuppressed);
        }
        Ok(Some(kind))
    }

    fn pop_front_effect(&mut self) -> Option<PmPreparedFakeEffect> {
        let effect = self
            .queued
            .pop_front()
            .expect("front effect remains present after age preflight")
            .effect;
        self.metrics.serviced = self.metrics.serviced.saturating_add(1);
        Some(effect)
    }

    pub(crate) fn next_kind(&self) -> Option<PmPreparedFakeEffectKind> {
        self.queued.front().map(|queued| queued.effect.kind())
    }

    pub(crate) fn contains_prepared_quote(&self, client_order: PmClientOrderKey) -> bool {
        self.queued
            .iter()
            .chain(&self.blocked)
            .any(|queued| match &queued.effect {
                PmPreparedFakeEffect::Quote { authority } => {
                    authority.client_order() == client_order
                }
                PmPreparedFakeEffect::Cancel { .. } => false,
                #[cfg(test)]
                PmPreparedFakeEffect::Synthetic { .. } => false,
            })
    }

    pub(crate) fn invalidate_prepared_quote(
        &mut self,
        client_order: PmClientOrderKey,
    ) -> Result<(), PmFakeEffectQueueError> {
        if let Some(index) = self.queued.iter().position(|queued| {
            matches!(
                &queued.effect,
                PmPreparedFakeEffect::Quote { authority }
                    if authority.client_order() == client_order
            )
        }) {
            let removed = self
                .queued
                .remove(index)
                .expect("located prepared quote remains queued");
            debug_assert_eq!(removed.effect.kind(), PmPreparedFakeEffectKind::Quote);
        } else if let Some(index) = self.blocked.iter().position(|queued| {
            matches!(
                &queued.effect,
                PmPreparedFakeEffect::Quote { authority }
                    if authority.client_order() == client_order
            )
        }) {
            let removed = self
                .blocked
                .remove(index)
                .expect("located prepared quote remains blocked");
            debug_assert_eq!(removed.effect.kind(), PmPreparedFakeEffectKind::Quote);
        } else {
            return Err(PmFakeEffectQueueError::UnknownPreparedQuote);
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

    pub(crate) const fn metrics(&self) -> PmFakeEffectQueueMetrics {
        self.metrics
    }

    pub(crate) fn projection(&self) -> PmFakeEffectMetrics {
        let metrics = self.metrics();
        PmFakeEffectMetrics {
            capacity: PM_FAKE_EFFECT_CAPACITY,
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
            + self.queued.capacity() * std::mem::size_of::<PmQueuedFakeEffect>()
            + self.blocked.capacity() * std::mem::size_of::<PmQueuedFakeEffect>()
    }

    fn validate_permit(&self, permit: &PmFakeEffectPermit) -> Result<(), PmFakeEffectQueueError> {
        if permit.owner != self.owner {
            return Err(PmFakeEffectQueueError::WrongOwner);
        }
        if self.reserved.binary_search(&permit.ordinal).is_err() {
            return Err(PmFakeEffectQueueError::UnknownPermit);
        }
        Ok(())
    }

    fn remove_reservation(
        &mut self,
        permit: PmFakeEffectPermit,
    ) -> Result<(), PmFakeEffectQueueError> {
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
        debug_assert_eq!(effect.effect.kind(), PmPreparedFakeEffectKind::Quote);
        self.blocked.push_back(effect);
    }
}

#[derive(Debug)]
struct PmQueuedFakeEffect {
    effect: PmPreparedFakeEffect,
    enqueued_monotonic_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum PmFakeEffectQueueError {
    #[error("PM fake-effect queue owner identity is exhausted")]
    OwnerIdentityExhausted,
    #[error("PM fake-effect permit identity is exhausted")]
    PermitIdentityExhausted,
    #[error("PM fake-effect queue is full")]
    Full,
    #[error("PM fake-effect permit belongs to another queue")]
    WrongOwner,
    #[error("PM fake-effect permit is no longer outstanding")]
    UnknownPermit,
    #[error("PM prepared fake quote is not retained by this queue")]
    UnknownPreparedQuote,
    #[error("PM fake-effect queue violated its fixed-capacity invariant")]
    InvariantCapacity,
    #[error("PM fake-effect enqueue requires nonzero monotonic time")]
    InvalidMonotonicTime,
    #[error("PM fake-effect service clock regressed")]
    ClockRegression,
    #[error("PM prepared fake quote exceeded its maximum queue age")]
    AgeExceeded,
    #[error("PM prepared fake quote is suppressed")]
    QuoteSuppressed,
    #[error("PM prepared fake quote approval changed or expired before dispatch")]
    QuoteAuthorityInvalidated,
    #[error("PM fake-effect kind does not match the requested service")]
    EffectKindMismatch,
}

#[cfg(test)]
pub(crate) struct Phase6FakeEffectAllocationProbe {
    queue: PmFakeEffectQueue,
}

#[cfg(test)]
impl Phase6FakeEffectAllocationProbe {
    pub(crate) fn new() -> Result<Self, PmFakeEffectQueueError> {
        Ok(Self {
            queue: PmFakeEffectQueue::new()?,
        })
    }

    pub(crate) fn attempt(&mut self, monotonic_ns: u64) -> Result<(), PmFakeEffectQueueError> {
        let permit = self.queue.try_reserve()?;
        self.queue.commit(
            permit,
            PmPreparedFakeEffect::synthetic(PmPreparedFakeEffectKind::Quote),
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
        queue: &mut PmFakeEffectQueue,
        kind: PmPreparedFakeEffectKind,
        enqueued_monotonic_ns: u64,
    ) -> Result<(), PmFakeEffectQueueError> {
        let permit = queue.try_reserve()?;
        queue.commit(
            permit,
            PmPreparedFakeEffect::synthetic(kind),
            enqueued_monotonic_ns,
        )
    }

    fn expect_synthetic(effect: Option<PmPreparedFakeEffect>, expected: PmPreparedFakeEffectKind) {
        let Some(PmPreparedFakeEffect::Synthetic { kind }) = effect else {
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
        let mut invalidated = PmFakeEffectQueue::new().unwrap();
        commit_synthetic(&mut invalidated, PmPreparedFakeEffectKind::Quote, 100).unwrap();
        assert_eq!(
            invalidated.pop_quote_at(101, None).unwrap_err(),
            PmFakeEffectQueueError::QuoteAuthorityInvalidated
        );
        assert_eq!(invalidated.queued_len(), 0);
        assert_eq!(invalidated.blocked_len(), 1);
        assert_eq!(invalidated.metrics().serviced(), 0);
        assert_eq!(invalidated.metrics().retained_after_revision_change(), 1);

        let mut current = PmFakeEffectQueue::new().unwrap();
        commit_synthetic(&mut current, PmPreparedFakeEffectKind::Quote, 100).unwrap();
        expect_synthetic(
            current.pop_quote_at(101, Some(revisions())).unwrap(),
            PmPreparedFakeEffectKind::Quote,
        );
        assert_eq!(current.metrics().serviced(), 1);
    }

    #[test]
    fn reservations_are_bounded_owner_scoped_and_fail_closed() {
        let mut queue = PmFakeEffectQueue::new().unwrap();
        let mut permits = Vec::with_capacity(PM_FAKE_EFFECT_CAPACITY);
        for _ in 0..PM_FAKE_EFFECT_CAPACITY {
            permits.push(queue.try_reserve().unwrap());
        }

        assert_eq!(queue.depth(), PM_FAKE_EFFECT_CAPACITY);
        assert_eq!(queue.metrics().high_water(), 256);
        assert_eq!(
            queue.try_reserve().unwrap_err(),
            PmFakeEffectQueueError::Full
        );
        assert!(queue.quote_suppressed());
        assert_eq!(queue.metrics().saturations(), 1);

        queue
            .release_before_journal(permits.pop().unwrap())
            .unwrap();
        assert_eq!(queue.depth(), PM_FAKE_EFFECT_CAPACITY - 1);
        assert_eq!(queue.metrics().released_before_journal(), 1);
    }

    #[test]
    fn every_permit_transition_remains_explicitly_accounted() {
        let mut released = PmFakeEffectQueue::new().unwrap();
        let permit = released.try_reserve().unwrap();
        released.release_before_journal(permit).unwrap();
        assert_eq!(released.depth(), 0);
        assert_eq!(released.metrics().reservations(), 1);
        assert_eq!(released.metrics().released_before_journal(), 1);

        let mut serviced = PmFakeEffectQueue::new().unwrap();
        commit_synthetic(&mut serviced, PmPreparedFakeEffectKind::Cancel, 100).unwrap();
        expect_synthetic(
            serviced.pop_at(101).unwrap(),
            PmPreparedFakeEffectKind::Cancel,
        );
        assert_eq!(serviced.depth(), 0);
        assert_eq!(serviced.metrics().reservations(), 1);
        assert_eq!(serviced.metrics().committed_after_durability(), 1);
        assert_eq!(serviced.metrics().serviced(), 1);

        let mut invalid_clock = PmFakeEffectQueue::new().unwrap();
        assert_eq!(
            commit_synthetic(&mut invalid_clock, PmPreparedFakeEffectKind::Quote, 0).unwrap_err(),
            PmFakeEffectQueueError::InvalidMonotonicTime
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
        let mut queue = PmFakeEffectQueue::new().unwrap();
        let permit = queue.try_reserve().unwrap();
        queue.retain_after_durable_failure(permit).unwrap();

        assert_eq!(queue.retained_permits(), 1);
        assert_eq!(queue.queued_len(), 0);
        assert_eq!(queue.metrics().retained_after_durable_failure(), 1);
        assert!(queue.quote_suppressed());
    }

    #[test]
    fn a_sibling_queue_cannot_consume_an_effect_permit() {
        let mut first = PmFakeEffectQueue::new().unwrap();
        let mut second = PmFakeEffectQueue::new().unwrap();
        let permit = first.try_reserve().unwrap();

        assert_eq!(
            second.release_before_journal(permit).unwrap_err(),
            PmFakeEffectQueueError::WrongOwner
        );
        assert_eq!(first.retained_permits(), 1);
        assert_eq!(second.retained_permits(), 0);
    }

    #[test]
    fn aged_quote_is_quarantined_and_can_never_dispatch() {
        let mut queue = PmFakeEffectQueue::new().unwrap();
        commit_synthetic(&mut queue, PmPreparedFakeEffectKind::Quote, 100).unwrap();

        let observed_age = PM_FAKE_EFFECT_MAX_AGE_NS + 1;
        assert_eq!(
            queue.pop_at(100 + observed_age).unwrap_err(),
            PmFakeEffectQueueError::AgeExceeded
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
        let mut queue = PmFakeEffectQueue::new().unwrap();
        commit_synthetic(&mut queue, PmPreparedFakeEffectKind::Quote, 100).unwrap();

        expect_synthetic(
            queue.pop_at(100 + PM_FAKE_EFFECT_MAX_AGE_NS).unwrap(),
            PmPreparedFakeEffectKind::Quote,
        );
        assert_eq!(
            queue.metrics().maximum_observed_age_ns(),
            PM_FAKE_EFFECT_MAX_AGE_NS
        );
        assert_eq!(queue.metrics().age_faults(), 0);
        assert_eq!(queue.metrics().serviced(), 1);
        assert!(!queue.quote_suppressed());
    }

    #[test]
    fn aged_owned_cancel_remains_serviceable_behind_a_quarantined_quote() {
        let mut queue = PmFakeEffectQueue::new().unwrap();
        commit_synthetic(&mut queue, PmPreparedFakeEffectKind::Quote, 100).unwrap();
        commit_synthetic(&mut queue, PmPreparedFakeEffectKind::Cancel, 101).unwrap();

        let service_time = 102 + PM_FAKE_EFFECT_MAX_AGE_NS;
        assert_eq!(
            queue.pop_at(service_time).unwrap_err(),
            PmFakeEffectQueueError::AgeExceeded
        );
        assert_eq!(queue.next_kind(), Some(PmPreparedFakeEffectKind::Cancel));
        expect_synthetic(
            queue.pop_at(service_time).unwrap(),
            PmPreparedFakeEffectKind::Cancel,
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
    fn clock_regression_retains_front_and_safety_cancel_can_retry() {
        let mut queue = PmFakeEffectQueue::new().unwrap();
        commit_synthetic(&mut queue, PmPreparedFakeEffectKind::Cancel, 100).unwrap();

        assert_eq!(
            queue.pop_at(99).unwrap_err(),
            PmFakeEffectQueueError::ClockRegression
        );
        assert_eq!(queue.depth(), 1);
        assert_eq!(queue.queued_len(), 1);
        assert_eq!(queue.blocked_len(), 0);
        assert_eq!(queue.metrics().clock_regressions(), 1);
        assert_eq!(queue.metrics().maximum_observed_age_ns(), 0);
        assert_eq!(queue.metrics().serviced(), 0);
        assert!(queue.quote_suppressed());

        expect_synthetic(queue.pop_at(101).unwrap(), PmPreparedFakeEffectKind::Cancel);
        assert_eq!(queue.depth(), 0);
        assert_eq!(queue.metrics().serviced(), 1);
    }

    #[test]
    fn clock_regression_makes_a_quote_permanently_non_dispatchable() {
        let mut queue = PmFakeEffectQueue::new().unwrap();
        commit_synthetic(&mut queue, PmPreparedFakeEffectKind::Quote, 100).unwrap();

        assert_eq!(
            queue.pop_at(99).unwrap_err(),
            PmFakeEffectQueueError::ClockRegression
        );
        assert_eq!(
            queue.pop_at(101).unwrap_err(),
            PmFakeEffectQueueError::QuoteSuppressed
        );
        assert_eq!(queue.depth(), 1);
        assert_eq!(queue.queued_len(), 0);
        assert_eq!(queue.blocked_len(), 1);
        assert_eq!(queue.metrics().retained_after_suppression(), 1);
        assert_eq!(queue.metrics().serviced(), 0);
    }

    #[test]
    fn phase6_fake_effect_row_is_257_attempts_after_256_durable_records() {
        let mut queue = PmFakeEffectQueue::new().unwrap();
        let reserved_capacity_bytes = queue.reserved_capacity_bytes();
        for ordinal in 0..PM_FAKE_EFFECT_CAPACITY {
            commit_synthetic(
                &mut queue,
                PmPreparedFakeEffectKind::Quote,
                100 + u64::try_from(ordinal).unwrap(),
            )
            .unwrap();
        }

        assert_eq!(queue.depth(), PM_FAKE_EFFECT_CAPACITY);
        assert_eq!(queue.queued_len(), PM_FAKE_EFFECT_CAPACITY);
        assert_eq!(queue.metrics().high_water(), 256);
        assert_eq!(queue.metrics().committed_after_durability(), 256);
        let committed_before_rejection = queue.metrics().committed_after_durability();
        assert_eq!(
            queue.try_reserve().unwrap_err(),
            PmFakeEffectQueueError::Full
        );
        assert_eq!(
            queue.metrics().committed_after_durability(),
            committed_before_rejection,
            "the 257th attempt is rejected before any record can claim dispatch"
        );
        assert_eq!(queue.metrics().serviced(), 0);
        assert!(queue.quote_suppressed());

        for _ in 0..PM_FAKE_EFFECT_CAPACITY {
            assert_eq!(
                queue.pop_at(1_000).unwrap_err(),
                PmFakeEffectQueueError::QuoteSuppressed
            );
        }
        assert_eq!(queue.depth(), PM_FAKE_EFFECT_CAPACITY);
        assert_eq!(queue.queued_len(), 0);
        assert_eq!(queue.blocked_len(), PM_FAKE_EFFECT_CAPACITY);
        assert_eq!(queue.metrics().reservations(), 256);
        assert_eq!(queue.metrics().committed_after_durability(), 256);
        assert_eq!(queue.metrics().saturations(), 1);
        assert_eq!(queue.metrics().retained_after_suppression(), 256);
        assert_eq!(queue.metrics().serviced(), 0);
        assert_eq!(queue.reserved_capacity_bytes(), reserved_capacity_bytes);
    }
}
