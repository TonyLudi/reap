use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};
use std::hash::Hash;

use super::policy::{PmLaneKind, PmLaneMetrics, PmLanePolicy, SaturationAction};

#[derive(Debug)]
pub(super) struct HeapEntry<K, T> {
    pub(super) key: K,
    pub(super) value: T,
}

impl<K: Ord, T> PartialEq for HeapEntry<K, T> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl<K: Ord, T> Eq for HeapEntry<K, T> {}

impl<K: Ord, T> PartialOrd for HeapEntry<K, T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: Ord, T> Ord for HeapEntry<K, T> {
    fn cmp(&self, other: &Self) -> Ordering {
        other.key.cmp(&self.key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Admission {
    Insert,
    Coalesced,
    Duplicate,
    Full(SaturationAction),
}

#[derive(Debug)]
pub(super) struct BoundedHeap<K, T> {
    policy: PmLanePolicy,
    heap: BinaryHeap<HeapEntry<K, T>>,
    keys: HashSet<K>,
    high_water: usize,
    rejected_full: u64,
    coalesced: u64,
}

impl<K: Copy + Eq + Hash + Ord, T> BoundedHeap<K, T> {
    pub(super) fn new(lane: PmLaneKind) -> Self {
        let policy = PmLanePolicy::for_lane(lane);
        Self {
            policy,
            heap: BinaryHeap::with_capacity(policy.capacity()),
            keys: HashSet::with_capacity(policy.capacity()),
            high_water: 0,
            rejected_full: 0,
            coalesced: 0,
        }
    }

    pub(super) fn prepare(&mut self, key: K) -> Admission {
        if self.keys.contains(&key) {
            return Admission::Duplicate;
        }
        if self.heap.len() < self.policy.capacity() {
            return Admission::Insert;
        }
        if self.policy.saturation_action() == SaturationAction::CoalesceTelemetry {
            let discarded = self.heap.pop().expect("full heap is nonempty");
            assert!(self.keys.remove(&discarded.key));
            increment_counter(&mut self.coalesced);
            Admission::Coalesced
        } else {
            increment_counter(&mut self.rejected_full);
            Admission::Full(self.policy.saturation_action())
        }
    }

    pub(super) fn insert(&mut self, key: K, value: T) {
        assert!(self.keys.insert(key));
        self.heap.push(HeapEntry { key, value });
        self.high_water = self.high_water.max(self.heap.len());
    }

    pub(super) fn peek(&self) -> Option<&HeapEntry<K, T>> {
        self.heap.peek()
    }

    pub(super) fn pop(&mut self) -> Option<HeapEntry<K, T>> {
        let entry = self.heap.pop()?;
        assert!(self.keys.remove(&entry.key));
        Some(entry)
    }

    pub(super) fn len(&self) -> usize {
        self.heap.len()
    }

    pub(super) const fn policy(&self) -> PmLanePolicy {
        self.policy
    }

    pub(super) fn metrics(&self) -> PmLaneMetrics {
        PmLaneMetrics::new(
            self.heap.len(),
            self.high_water,
            self.rejected_full,
            self.coalesced,
        )
    }
}

fn increment_counter(counter: &mut u64) {
    *counter = counter
        .checked_add(1)
        .expect("lane observability counter overflow");
}
