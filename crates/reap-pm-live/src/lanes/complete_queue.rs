use reap_transport::DeliveryClockError;

use super::{
    Admission, BoundedHeap, PmCompleteIngress, PmCompleteInputSource, PmCompleteLaneItem,
    PmCompleteServiceKey, PmCompleteSourceKind, PmLaneKind, PmLaneMetrics, PmLanePolicy,
    SaturationAction,
};

/// Fixed-cardinality runtime evidence for one complete input lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmCompleteLaneMetrics {
    lane: PmLaneKind,
    queue: PmLaneMetrics,
    serviced: u64,
    age_faults: u64,
    maximum_observed_age_ns: u64,
}

impl PmCompleteLaneMetrics {
    pub(super) const fn new(
        lane: PmLaneKind,
        queue: PmLaneMetrics,
        serviced: u64,
        age_faults: u64,
        maximum_observed_age_ns: u64,
    ) -> Self {
        Self {
            lane,
            queue,
            serviced,
            age_faults,
            maximum_observed_age_ns,
        }
    }

    #[must_use]
    pub const fn lane(self) -> PmLaneKind {
        self.lane
    }

    #[must_use]
    pub const fn policy(self) -> PmLanePolicy {
        PmLanePolicy::for_lane(self.lane)
    }

    #[must_use]
    pub const fn queue(self) -> PmLaneMetrics {
        self.queue
    }

    #[must_use]
    pub const fn serviced(self) -> u64 {
        self.serviced
    }

    #[must_use]
    pub const fn age_faults(self) -> u64 {
        self.age_faults
    }

    #[must_use]
    pub const fn maximum_observed_age_ns(self) -> u64 {
        self.maximum_observed_age_ns
    }
}

#[derive(Debug)]
pub(crate) enum PmCompleteLaneEnqueueError<T> {
    WrongSource {
        input: T,
        expected: PmCompleteSourceKind,
        received: PmCompleteSourceKind,
    },
    Duplicate {
        input: T,
        key: PmCompleteServiceKey,
    },
    Full {
        input: T,
        lane: PmLaneKind,
        key: PmCompleteServiceKey,
        action: SaturationAction,
    },
}

impl<T> PmCompleteLaneEnqueueError<T> {
    pub(crate) fn into_input(self) -> T {
        match self {
            Self::WrongSource { input, .. }
            | Self::Duplicate { input, .. }
            | Self::Full { input, .. } => input,
        }
    }

    pub(crate) const fn action(&self) -> Option<SaturationAction> {
        match self {
            Self::Full {
                lane, key, action, ..
            } => {
                let _ = (*lane, *key);
                Some(*action)
            }
            Self::WrongSource {
                expected, received, ..
            } => {
                let _ = (*expected, *received);
                None
            }
            Self::Duplicate { key, .. } => {
                let _ = *key;
                None
            }
        }
    }
}

/// Atomic pre-admission/build failure for a completion whose payload does not
/// exist until a fixture edge is consumed.
///
/// Capacity, source, and duplicate checks run before `build`; because the
/// lane owner invokes the closure synchronously, a successful build cannot
/// subsequently discover a full lane.
#[derive(Debug)]
pub(crate) enum PmCompleteLaneBuildError<E> {
    WrongSource {
        expected: PmCompleteSourceKind,
        received: PmCompleteSourceKind,
    },
    Duplicate {
        key: PmCompleteServiceKey,
    },
    Full {
        lane: PmLaneKind,
        key: PmCompleteServiceKey,
        action: SaturationAction,
    },
    Build(E),
}

impl<E> PmCompleteLaneBuildError<E> {
    pub(crate) const fn action(&self) -> Option<SaturationAction> {
        match self {
            Self::Full {
                lane, key, action, ..
            } => {
                let _ = (*lane, *key);
                Some(*action)
            }
            Self::WrongSource { expected, received } => {
                let _ = (*expected, *received);
                None
            }
            Self::Duplicate { key } => {
                let _ = *key;
                None
            }
            Self::Build(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PmCompleteLaneAgeFault {
    lane: PmLaneKind,
    key: PmCompleteServiceKey,
    observed_age_ns: u64,
    maximum_age_ns: u64,
    action: SaturationAction,
}

impl PmCompleteLaneAgeFault {
    pub(crate) const fn lane(self) -> PmLaneKind {
        self.lane
    }

    pub(crate) const fn key(self) -> PmCompleteServiceKey {
        self.key
    }

    pub(crate) const fn observed_age_ns(self) -> u64 {
        self.observed_age_ns
    }

    pub(crate) const fn maximum_age_ns(self) -> u64 {
        self.maximum_age_ns
    }

    pub(crate) const fn action(self) -> SaturationAction {
        self.action
    }
}

pub(crate) struct PmCompleteLane<T> {
    lane: PmLaneKind,
    queue: BoundedHeap<PmCompleteServiceKey, PmCompleteLaneItem<T>>,
    serviced: u64,
    age_faults: u64,
    maximum_observed_age_ns: u64,
    aged_head: Option<PmCompleteServiceKey>,
}

impl<T> PmCompleteLane<T> {
    pub(crate) fn new(lane: PmLaneKind) -> Self {
        debug_assert!(lane.service_priority_rank().is_some());
        debug_assert_ne!(lane, PmLaneKind::Public);
        debug_assert_ne!(lane, PmLaneKind::Scheduled);
        Self {
            lane,
            queue: BoundedHeap::new(lane),
            serviced: 0,
            age_faults: 0,
            maximum_observed_age_ns: 0,
            aged_head: None,
        }
    }

    pub(crate) fn enqueue(
        &mut self,
        ingress: PmCompleteIngress,
        input: T,
        variant_rank: u8,
        expected_source: PmCompleteSourceKind,
    ) -> Result<(), PmCompleteLaneEnqueueError<T>> {
        let received_source = source_kind(ingress.source());
        if received_source != expected_source {
            return Err(PmCompleteLaneEnqueueError::WrongSource {
                input,
                expected: expected_source,
                received: received_source,
            });
        }
        let key = PmCompleteServiceKey::derived(ingress, variant_rank);
        match self.queue.prepare(key) {
            Admission::Insert | Admission::Coalesced => {
                self.queue
                    .insert(key, PmCompleteLaneItem::new(key, ingress, input));
                Ok(())
            }
            Admission::Duplicate => Err(PmCompleteLaneEnqueueError::Duplicate { input, key }),
            Admission::Full(action) => Err(PmCompleteLaneEnqueueError::Full {
                input,
                lane: self.lane,
                key,
                action,
            }),
        }
    }

    pub(crate) fn enqueue_built<E>(
        &mut self,
        ingress: PmCompleteIngress,
        variant_rank: u8,
        expected_source: PmCompleteSourceKind,
        build: impl FnOnce() -> Result<T, E>,
    ) -> Result<(), PmCompleteLaneBuildError<E>> {
        let received_source = source_kind(ingress.source());
        if received_source != expected_source {
            return Err(PmCompleteLaneBuildError::WrongSource {
                expected: expected_source,
                received: received_source,
            });
        }
        let key = PmCompleteServiceKey::derived(ingress, variant_rank);
        match self.queue.prepare(key) {
            Admission::Insert => {}
            Admission::Coalesced => {
                unreachable!("atomic built inputs are never admitted to a coalescing lane")
            }
            Admission::Duplicate => {
                return Err(PmCompleteLaneBuildError::Duplicate { key });
            }
            Admission::Full(action) => {
                return Err(PmCompleteLaneBuildError::Full {
                    lane: self.lane,
                    key,
                    action,
                });
            }
        }
        let input = build().map_err(PmCompleteLaneBuildError::Build)?;
        self.queue
            .insert(key, PmCompleteLaneItem::new(key, ingress, input));
        Ok(())
    }

    pub(crate) fn check_age(
        &mut self,
        monotonic_now_ns: u64,
    ) -> Result<(), PmCompleteLaneCheckError> {
        let Some(head) = self.queue.peek() else {
            self.aged_head = None;
            return Ok(());
        };
        let age = head
            .value
            .queue_age_ns(monotonic_now_ns)
            .map_err(PmCompleteLaneCheckError::Clock)?;
        self.maximum_observed_age_ns = self.maximum_observed_age_ns.max(age);
        let Some(maximum_age_ns) = self.queue.policy().maximum_age_ns() else {
            return Ok(());
        };
        if age <= maximum_age_ns {
            self.aged_head = None;
            return Ok(());
        }
        let key = head.value.key();
        if self.aged_head != Some(key) {
            self.age_faults = self.age_faults.saturating_add(1);
            self.aged_head = Some(key);
        }
        Err(PmCompleteLaneCheckError::Aged(PmCompleteLaneAgeFault {
            lane: self.lane,
            key,
            observed_age_ns: age,
            maximum_age_ns,
            action: self.queue.policy().saturation_action(),
        }))
    }

    pub(crate) fn pop(&mut self) -> Option<PmCompleteLaneItem<T>> {
        let item = self.queue.pop()?.value;
        self.serviced = self.serviced.saturating_add(1);
        self.aged_head = None;
        Some(item)
    }

    pub(crate) fn len(&self) -> usize {
        self.queue.len()
    }

    pub(crate) const fn lane(&self) -> PmLaneKind {
        self.lane
    }

    pub(crate) fn metrics(&self) -> PmCompleteLaneMetrics {
        PmCompleteLaneMetrics::new(
            self.lane,
            self.queue.metrics(),
            self.serviced,
            self.age_faults,
            self.maximum_observed_age_ns,
        )
    }

    pub(crate) fn reserved_capacity_bytes(&self) -> usize {
        self.queue.reserved_capacity_bytes()
    }
}

#[derive(Debug)]
pub(crate) enum PmCompleteLaneCheckError {
    Clock(DeliveryClockError),
    EventClock(reap_pm_core::EnvelopeError),
    Aged(PmCompleteLaneAgeFault),
}

const fn source_kind(source: PmCompleteInputSource) -> PmCompleteSourceKind {
    match source {
        PmCompleteInputSource::Product(PmProductSource::OkxReference { .. }) => {
            PmCompleteSourceKind::OkxReference
        }
        PmCompleteInputSource::Product(PmProductSource::PolymarketMarket { .. }) => {
            PmCompleteSourceKind::PolymarketMarket
        }
        PmCompleteInputSource::Product(PmProductSource::PolymarketAccount { .. }) => {
            PmCompleteSourceKind::PolymarketAccount
        }
        PmCompleteInputSource::Internal(_) => PmCompleteSourceKind::InternalSignal,
    }
}

use reap_pm_core::PmProductSource;
