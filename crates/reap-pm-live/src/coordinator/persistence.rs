use std::collections::VecDeque;

use reap_pm_core::{PmClientOrderKey, PmVenueOrderKey};
use reap_pm_state::{PmOwnedCancelIntent, PmOwnedIntentId};
use thiserror::Error;

use super::effect_queue::PmFakeEffectPermit;
use super::{ReservedPmCancel, ReservedPmQuote};
use crate::journal::{
    PmCancelIntentDurablyAcknowledged, PmCancelIntentReceiptPoll, PmJournalAcknowledged,
    PmJournalReceiptPoll, PmPendingCancelIntent, PmPendingJournalRecord, PmPendingQuoteIntent,
    PmQuoteIntentDurablyAcknowledged, PmQuoteIntentReceiptPoll,
};

pub(crate) const PM_PENDING_PERSISTENCE_CAPACITY: usize = 1_024;
pub(crate) const PM_PENDING_PERSISTENCE_MAX_AGE_NS: u64 = 1_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmPersistenceIntentIdentity {
    Quote {
        intent: PmOwnedIntentId,
        client_order: PmClientOrderKey,
    },
    Cancel {
        client_order: PmClientOrderKey,
        venue_order: PmVenueOrderKey,
    },
}

pub(crate) enum PmPendingPersistence {
    QuoteIntent {
        reserved: ReservedPmQuote,
        effect_permit: PmFakeEffectPermit,
        receipt: PmPendingQuoteIntent,
        enqueued_monotonic_ns: u64,
    },
    CancelIntent {
        reserved: ReservedPmCancel,
        owned_intent: PmOwnedCancelIntent,
        effect_permit: PmFakeEffectPermit,
        receipt: PmPendingCancelIntent,
        enqueued_monotonic_ns: u64,
    },
    Fact {
        receipt: PmPendingJournalRecord,
        enqueued_monotonic_ns: u64,
    },
}

impl std::fmt::Debug for PmPendingPersistence {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::QuoteIntent { .. } => "PmPendingPersistence::QuoteIntent(..)",
            Self::CancelIntent { .. } => "PmPendingPersistence::CancelIntent(..)",
            Self::Fact { .. } => "PmPendingPersistence::Fact(..)",
        })
    }
}

impl PmPendingPersistence {
    const fn enqueued_monotonic_ns(&self) -> u64 {
        match self {
            Self::QuoteIntent {
                enqueued_monotonic_ns,
                ..
            }
            | Self::CancelIntent {
                enqueued_monotonic_ns,
                ..
            }
            | Self::Fact {
                enqueued_monotonic_ns,
                ..
            } => *enqueued_monotonic_ns,
        }
    }

    const fn intent_identity(&self) -> Option<PmPersistenceIntentIdentity> {
        match self {
            Self::QuoteIntent { reserved, .. } => Some(PmPersistenceIntentIdentity::Quote {
                intent: reserved.intent(),
                client_order: reserved.client_order(),
            }),
            Self::CancelIntent { owned_intent, .. } => Some(PmPersistenceIntentIdentity::Cancel {
                client_order: owned_intent.client_order(),
                venue_order: owned_intent.venue_order(),
            }),
            Self::Fact { .. } => None,
        }
    }
}

pub(crate) enum PmPersistencePoll {
    Empty,
    Pending,
    QuoteAcknowledged {
        reserved: ReservedPmQuote,
        effect_permit: PmFakeEffectPermit,
        acknowledgement: PmQuoteIntentDurablyAcknowledged,
    },
    CancelAcknowledged {
        reserved: ReservedPmCancel,
        owned_intent: PmOwnedCancelIntent,
        effect_permit: PmFakeEffectPermit,
        acknowledgement: PmCancelIntentDurablyAcknowledged,
    },
    FactAcknowledged(PmJournalAcknowledged),
    IntentFailed {
        identity: PmPersistenceIntentIdentity,
        effect_permit: PmFakeEffectPermit,
        reason: PmPersistenceFailure,
    },
    FactFailed(PmPersistenceFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct PmPersistenceQueueMetrics {
    admitted: u64,
    acknowledged: u64,
    durability_failures: u64,
    closed_failures: u64,
    saturations: u64,
    age_faults: u64,
    high_water: u16,
    maximum_observed_age_ns: u64,
}

impl PmPersistenceQueueMetrics {
    pub(crate) const fn admitted(self) -> u64 {
        self.admitted
    }

    pub(crate) const fn acknowledged(self) -> u64 {
        self.acknowledged
    }

    pub(crate) const fn durability_failures(self) -> u64 {
        self.durability_failures
    }

    pub(crate) const fn closed_failures(self) -> u64 {
        self.closed_failures
    }

    pub(crate) const fn saturations(self) -> u64 {
        self.saturations
    }

    pub(crate) const fn age_faults(self) -> u64 {
        self.age_faults
    }

    pub(crate) const fn high_water(self) -> u16 {
        self.high_water
    }

    pub(crate) const fn maximum_observed_age_ns(self) -> u64 {
        self.maximum_observed_age_ns
    }
}

/// Allocation-free observation of the bounded durable-acknowledgement queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmPersistenceMetrics {
    capacity: usize,
    depth: usize,
    admitted: u64,
    acknowledged: u64,
    durability_failures: u64,
    closed_failures: u64,
    saturations: u64,
    age_faults: u64,
    high_water: u16,
    maximum_observed_age_ns: u64,
    globally_stopped: bool,
}

impl PmPersistenceMetrics {
    #[must_use]
    pub const fn capacity(self) -> usize {
        self.capacity
    }

    #[must_use]
    pub const fn depth(self) -> usize {
        self.depth
    }

    #[must_use]
    pub const fn admitted(self) -> u64 {
        self.admitted
    }

    #[must_use]
    pub const fn acknowledged(self) -> u64 {
        self.acknowledged
    }

    #[must_use]
    pub const fn durability_failures(self) -> u64 {
        self.durability_failures
    }

    #[must_use]
    pub const fn closed_failures(self) -> u64 {
        self.closed_failures
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
    pub const fn high_water(self) -> u16 {
        self.high_water
    }

    #[must_use]
    pub const fn maximum_observed_age_ns(self) -> u64 {
        self.maximum_observed_age_ns
    }

    #[must_use]
    pub const fn globally_stopped(self) -> bool {
        self.globally_stopped
    }
}

#[derive(Debug)]
pub(crate) struct PmPersistenceQueue {
    entries: VecDeque<PmPendingPersistence>,
    metrics: PmPersistenceQueueMetrics,
    globally_stopped: bool,
}

impl PmPersistenceQueue {
    pub(crate) fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(PM_PENDING_PERSISTENCE_CAPACITY),
            metrics: PmPersistenceQueueMetrics::default(),
            globally_stopped: false,
        }
    }

    pub(crate) fn ensure_capacity(&mut self, additional: usize) -> Result<(), PmPersistenceError> {
        if self.entries.len().saturating_add(additional) > PM_PENDING_PERSISTENCE_CAPACITY {
            self.metrics.saturations = self.metrics.saturations.saturating_add(1);
            self.globally_stopped = true;
            Err(PmPersistenceError::Full)
        } else {
            Ok(())
        }
    }

    pub(crate) fn push(&mut self, pending: PmPendingPersistence) -> Result<(), PmPersistenceError> {
        self.ensure_capacity(1)?;
        if pending.enqueued_monotonic_ns() == 0 {
            self.globally_stopped = true;
            return Err(PmPersistenceError::InvalidMonotonicTime);
        }
        self.entries.push_back(pending);
        self.metrics.admitted = self.metrics.admitted.saturating_add(1);
        let depth = u16::try_from(self.entries.len()).expect("fixed queue capacity fits u16");
        self.metrics.high_water = self.metrics.high_water.max(depth);
        Ok(())
    }

    pub(crate) fn poll_one(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<PmPersistencePoll, PmPersistenceError> {
        let Some(enqueued) = self
            .entries
            .front()
            .map(PmPendingPersistence::enqueued_monotonic_ns)
        else {
            return Ok(PmPersistencePoll::Empty);
        };
        let Some(age) = monotonic_now_ns.checked_sub(enqueued) else {
            self.globally_stopped = true;
            return Err(PmPersistenceError::ClockRegression);
        };
        self.metrics.maximum_observed_age_ns = self.metrics.maximum_observed_age_ns.max(age);
        if age > PM_PENDING_PERSISTENCE_MAX_AGE_NS {
            self.metrics.age_faults = self.metrics.age_faults.saturating_add(1);
            self.globally_stopped = true;
            let pending = self
                .entries
                .pop_front()
                .expect("front entry remains present after age check");
            let identity = pending.intent_identity();
            return Ok(match pending {
                PmPendingPersistence::QuoteIntent { effect_permit, .. }
                | PmPendingPersistence::CancelIntent { effect_permit, .. } => {
                    PmPersistencePoll::IntentFailed {
                        identity: identity.expect("intent pending value has exact identity"),
                        effect_permit,
                        reason: PmPersistenceFailure::AgeExceeded,
                    }
                }
                PmPendingPersistence::Fact { .. } => {
                    PmPersistencePoll::FactFailed(PmPersistenceFailure::AgeExceeded)
                }
            });
        }

        let pending = self
            .entries
            .pop_front()
            .expect("front entry remains present after age check");
        Ok(match pending {
            PmPendingPersistence::QuoteIntent {
                reserved,
                effect_permit,
                receipt,
                enqueued_monotonic_ns,
            } => {
                let identity = PmPersistenceIntentIdentity::Quote {
                    intent: reserved.intent(),
                    client_order: reserved.client_order(),
                };
                match receipt.poll() {
                    PmQuoteIntentReceiptPoll::Pending(receipt) => {
                        self.entries.push_back(PmPendingPersistence::QuoteIntent {
                            reserved,
                            effect_permit,
                            receipt,
                            enqueued_monotonic_ns,
                        });
                        PmPersistencePoll::Pending
                    }
                    PmQuoteIntentReceiptPoll::Acknowledged(acknowledgement) => {
                        self.metrics.acknowledged = self.metrics.acknowledged.saturating_add(1);
                        PmPersistencePoll::QuoteAcknowledged {
                            reserved,
                            effect_permit,
                            acknowledgement,
                        }
                    }
                    PmQuoteIntentReceiptPoll::Failed(message) => {
                        self.metrics.durability_failures =
                            self.metrics.durability_failures.saturating_add(1);
                        self.globally_stopped = true;
                        PmPersistencePoll::IntentFailed {
                            identity,
                            effect_permit,
                            reason: PmPersistenceFailure::Durability(message),
                        }
                    }
                    PmQuoteIntentReceiptPoll::Closed => {
                        self.metrics.closed_failures =
                            self.metrics.closed_failures.saturating_add(1);
                        self.globally_stopped = true;
                        PmPersistencePoll::IntentFailed {
                            identity,
                            effect_permit,
                            reason: PmPersistenceFailure::Closed,
                        }
                    }
                }
            }
            PmPendingPersistence::CancelIntent {
                reserved,
                owned_intent,
                effect_permit,
                receipt,
                enqueued_monotonic_ns,
            } => {
                let identity = PmPersistenceIntentIdentity::Cancel {
                    client_order: owned_intent.client_order(),
                    venue_order: owned_intent.venue_order(),
                };
                match receipt.poll() {
                    PmCancelIntentReceiptPoll::Pending(receipt) => {
                        self.entries.push_back(PmPendingPersistence::CancelIntent {
                            reserved,
                            owned_intent,
                            effect_permit,
                            receipt,
                            enqueued_monotonic_ns,
                        });
                        PmPersistencePoll::Pending
                    }
                    PmCancelIntentReceiptPoll::Acknowledged(acknowledgement) => {
                        self.metrics.acknowledged = self.metrics.acknowledged.saturating_add(1);
                        PmPersistencePoll::CancelAcknowledged {
                            reserved,
                            owned_intent,
                            effect_permit,
                            acknowledgement,
                        }
                    }
                    PmCancelIntentReceiptPoll::Failed(message) => {
                        self.metrics.durability_failures =
                            self.metrics.durability_failures.saturating_add(1);
                        self.globally_stopped = true;
                        PmPersistencePoll::IntentFailed {
                            identity,
                            effect_permit,
                            reason: PmPersistenceFailure::Durability(message),
                        }
                    }
                    PmCancelIntentReceiptPoll::Closed => {
                        self.metrics.closed_failures =
                            self.metrics.closed_failures.saturating_add(1);
                        self.globally_stopped = true;
                        PmPersistencePoll::IntentFailed {
                            identity,
                            effect_permit,
                            reason: PmPersistenceFailure::Closed,
                        }
                    }
                }
            }
            PmPendingPersistence::Fact {
                receipt,
                enqueued_monotonic_ns,
            } => match receipt.poll() {
                PmJournalReceiptPoll::Pending(receipt) => {
                    self.entries.push_back(PmPendingPersistence::Fact {
                        receipt,
                        enqueued_monotonic_ns,
                    });
                    PmPersistencePoll::Pending
                }
                PmJournalReceiptPoll::Acknowledged(acknowledgement) => {
                    self.metrics.acknowledged = self.metrics.acknowledged.saturating_add(1);
                    PmPersistencePoll::FactAcknowledged(acknowledgement)
                }
                PmJournalReceiptPoll::Failed(message) => {
                    self.metrics.durability_failures =
                        self.metrics.durability_failures.saturating_add(1);
                    self.globally_stopped = true;
                    PmPersistencePoll::FactFailed(PmPersistenceFailure::Durability(message))
                }
                PmJournalReceiptPoll::Closed => {
                    self.metrics.closed_failures = self.metrics.closed_failures.saturating_add(1);
                    self.globally_stopped = true;
                    PmPersistencePoll::FactFailed(PmPersistenceFailure::Closed)
                }
            },
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) const fn globally_stopped(&self) -> bool {
        self.globally_stopped
    }

    pub(crate) const fn metrics(&self) -> PmPersistenceQueueMetrics {
        self.metrics
    }

    pub(crate) fn projection(&self) -> PmPersistenceMetrics {
        let metrics = self.metrics();
        PmPersistenceMetrics {
            capacity: PM_PENDING_PERSISTENCE_CAPACITY,
            depth: self.len(),
            admitted: metrics.admitted(),
            acknowledged: metrics.acknowledged(),
            durability_failures: metrics.durability_failures(),
            closed_failures: metrics.closed_failures(),
            saturations: metrics.saturations(),
            age_faults: metrics.age_faults(),
            high_water: metrics.high_water(),
            maximum_observed_age_ns: metrics.maximum_observed_age_ns(),
            globally_stopped: self.globally_stopped(),
        }
    }

    pub(crate) fn reserved_capacity_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<PmPendingPersistence>()
    }
}

#[derive(Debug, Error)]
pub(crate) enum PmPersistenceFailure {
    #[error("durable PM write failed: {0}")]
    Durability(String),
    #[error("durable PM writer closed before acknowledgement")]
    Closed,
    #[error("pending PM persistence exceeded its maximum service age")]
    AgeExceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum PmPersistenceError {
    #[error("PM persistence queue is full")]
    Full,
    #[error("PM persistence ingress requires nonzero monotonic time")]
    InvalidMonotonicTime,
    #[error("PM persistence service clock regressed")]
    ClockRegression,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_preflight_is_exact_and_latches_global_stop() {
        let mut queue = PmPersistenceQueue::new();
        queue
            .ensure_capacity(PM_PENDING_PERSISTENCE_CAPACITY)
            .unwrap();
        assert_eq!(
            queue
                .ensure_capacity(PM_PENDING_PERSISTENCE_CAPACITY + 1)
                .unwrap_err(),
            PmPersistenceError::Full
        );
        assert!(queue.globally_stopped());
        assert_eq!(queue.metrics().saturations(), 1);
        assert_eq!(queue.len(), 0);
    }
}
