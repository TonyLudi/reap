use std::collections::{HashSet, VecDeque};

use reap_core::{BookAction, SequencedBookUpdate};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceStatus {
    Empty,
    Ready,
    Recovering,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SequenceOutcome {
    Apply(Vec<SequencedBookUpdate>),
    Duplicate,
    RecoveryRequired {
        expected_prev: Option<i64>,
        received_prev: i64,
        received_seq: i64,
    },
    Buffered,
    RecoveryFailed {
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct SequenceTracker {
    status: SequenceStatus,
    last_seq_id: Option<i64>,
    last_ts_ms: Option<u64>,
    buffered: VecDeque<SequencedBookUpdate>,
    max_buffered: usize,
    reset_count: u64,
    same_sequence_count: u64,
}

impl SequenceTracker {
    pub fn new(max_buffered: usize) -> Self {
        Self {
            status: SequenceStatus::Empty,
            last_seq_id: None,
            last_ts_ms: None,
            buffered: VecDeque::new(),
            max_buffered: max_buffered.max(1),
            reset_count: 0,
            same_sequence_count: 0,
        }
    }

    pub fn status(&self) -> SequenceStatus {
        self.status
    }

    pub fn last_seq_id(&self) -> Option<i64> {
        self.last_seq_id
    }

    pub fn buffered_len(&self) -> usize {
        self.buffered.len()
    }

    pub fn reset_count(&self) -> u64 {
        self.reset_count
    }

    pub fn same_sequence_count(&self) -> u64 {
        self.same_sequence_count
    }

    pub fn require_recovery(&mut self) {
        self.status = SequenceStatus::Recovering;
        self.buffered.clear();
    }

    pub fn on_update(&mut self, update: SequencedBookUpdate) -> SequenceOutcome {
        if update.action == BookAction::Snapshot {
            return self.on_snapshot(update);
        }

        match self.status {
            SequenceStatus::Empty => {
                let received_prev = update.prev_seq_id;
                let received_seq = update.seq_id;
                if let Err(reason) = self.buffer(update) {
                    return SequenceOutcome::RecoveryFailed { reason };
                }
                self.status = SequenceStatus::Recovering;
                SequenceOutcome::RecoveryRequired {
                    expected_prev: None,
                    received_prev,
                    received_seq,
                }
            }
            SequenceStatus::Ready => {
                let last = self.last_seq_id.expect("ready sequence must have last id");
                if update.prev_seq_id == last {
                    self.record_contiguous_transition(last, update.seq_id, update.ts_ms);
                    return SequenceOutcome::Apply(vec![update]);
                }
                let received_prev = update.prev_seq_id;
                let received_seq = update.seq_id;
                if let Err(reason) = self.buffer(update) {
                    return SequenceOutcome::RecoveryFailed { reason };
                }
                self.status = SequenceStatus::Recovering;
                SequenceOutcome::RecoveryRequired {
                    expected_prev: Some(last),
                    received_prev,
                    received_seq,
                }
            }
            SequenceStatus::Recovering => match self.buffer(update) {
                Ok(()) => SequenceOutcome::Buffered,
                Err(reason) => SequenceOutcome::RecoveryFailed { reason },
            },
        }
    }

    fn on_snapshot(&mut self, snapshot: SequencedBookUpdate) -> SequenceOutcome {
        let previous_seq_id = self.last_seq_id;
        if self.status == SequenceStatus::Ready
            && self
                .last_ts_ms
                .is_some_and(|last_ts_ms| snapshot.ts_ms < last_ts_ms)
        {
            return SequenceOutcome::Duplicate;
        }
        if previous_seq_id.is_some_and(|last| snapshot.seq_id < last) {
            self.reset_count = self.reset_count.saturating_add(1);
        }
        let snapshot_ts_ms = snapshot.ts_ms;
        self.last_seq_id = Some(snapshot.seq_id);
        self.last_ts_ms = Some(snapshot.ts_ms);
        let mut apply = vec![snapshot];
        let mut pending = self.buffered.drain(..).collect::<Vec<_>>();
        pending.retain(|update| update.ts_ms >= snapshot_ts_ms);
        let mut seen = HashSet::new();
        let conflicting_transition = pending.iter().find_map(|update| {
            (!seen.insert((update.prev_seq_id, update.seq_id, update.ts_ms)))
                .then_some((update.prev_seq_id, update.seq_id))
        });
        if let Some((received_prev, received_seq)) = conflicting_transition {
            let expected_prev = self.last_seq_id;
            self.buffered.extend(pending);
            self.status = SequenceStatus::Recovering;
            return SequenceOutcome::RecoveryRequired {
                expected_prev,
                received_prev,
                received_seq,
            };
        }

        loop {
            let last = self.last_seq_id.expect("snapshot set last id");
            let Some(index) = pending
                .iter()
                .enumerate()
                .filter(|(_, update)| update.prev_seq_id == last)
                .min_by_key(|(_, update)| (update.ts_ms, update.seq_id != last))
                .map(|(index, _)| index)
            else {
                break;
            };
            let update = pending.remove(index);
            self.record_contiguous_transition(last, update.seq_id, update.ts_ms);
            apply.push(update);
        }
        let last = self.last_seq_id.expect("snapshot set last id");
        if let Some(first) = pending.first() {
            let received_prev = first.prev_seq_id;
            let received_seq = first.seq_id;
            self.buffered.extend(pending);
            self.status = SequenceStatus::Recovering;
            return SequenceOutcome::RecoveryRequired {
                expected_prev: Some(last),
                received_prev,
                received_seq,
            };
        }
        self.status = SequenceStatus::Ready;
        SequenceOutcome::Apply(apply)
    }

    fn record_contiguous_transition(&mut self, previous: i64, next: i64, ts_ms: u64) {
        if next < previous {
            self.reset_count = self.reset_count.saturating_add(1);
        } else if next == previous {
            self.same_sequence_count = self.same_sequence_count.saturating_add(1);
        }
        self.last_seq_id = Some(next);
        self.last_ts_ms = Some(ts_ms);
    }

    fn buffer(&mut self, update: SequencedBookUpdate) -> Result<(), String> {
        if self.buffered.len() >= self.max_buffered {
            self.buffered.clear();
            self.status = SequenceStatus::Recovering;
            return Err(format!(
                "sequence recovery buffer exceeded {} updates",
                self.max_buffered
            ));
        }
        self.buffered.push_back(update);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use reap_core::Level;

    use super::*;

    fn update(action: BookAction, prev: i64, seq: i64) -> SequencedBookUpdate {
        update_at(action, prev, seq, seq.max(0) as u64)
    }

    fn update_at(action: BookAction, prev: i64, seq: i64, ts_ms: u64) -> SequencedBookUpdate {
        SequencedBookUpdate {
            action,
            symbol: "BTC-USDT".to_string(),
            ts_ms,
            prev_seq_id: prev,
            seq_id: seq,
            bids: vec![Level::new(100.0, seq as f64)],
            asks: vec![],
        }
    }

    #[test]
    fn contiguous_updates_apply_after_snapshot() {
        let mut tracker = SequenceTracker::new(8);
        assert!(matches!(
            tracker.on_update(update(BookAction::Snapshot, -1, 10)),
            SequenceOutcome::Apply(_)
        ));
        assert!(matches!(
            tracker.on_update(update(BookAction::Update, 10, 11)),
            SequenceOutcome::Apply(_)
        ));
        assert_eq!(tracker.last_seq_id(), Some(11));
    }

    #[test]
    fn same_sequence_heartbeat_is_contiguous_and_observable() {
        let mut tracker = SequenceTracker::new(8);
        tracker.on_update(update_at(BookAction::Snapshot, -1, 10, 100));

        let outcome = tracker.on_update(update_at(BookAction::Update, 10, 10, 101));

        assert!(matches!(outcome, SequenceOutcome::Apply(_)));
        assert_eq!(tracker.last_seq_id(), Some(10));
        assert_eq!(tracker.same_sequence_count(), 1);
        assert_eq!(tracker.reset_count(), 0);
    }

    #[test]
    fn downward_maintenance_reset_preserves_continuity() {
        let mut tracker = SequenceTracker::new(8);
        tracker.on_update(update_at(BookAction::Snapshot, -1, 10, 100));
        tracker.on_update(update_at(BookAction::Update, 10, 15, 101));

        let reset = tracker.on_update(update_at(BookAction::Update, 15, 3, 102));
        let following = tracker.on_update(update_at(BookAction::Update, 3, 5, 103));

        assert!(matches!(reset, SequenceOutcome::Apply(_)));
        assert!(matches!(following, SequenceOutcome::Apply(_)));
        assert_eq!(tracker.status(), SequenceStatus::Ready);
        assert_eq!(tracker.last_seq_id(), Some(5));
        assert_eq!(tracker.reset_count(), 1);
    }

    #[test]
    fn same_final_sequence_with_wrong_predecessor_requires_recovery() {
        let mut tracker = SequenceTracker::new(8);
        tracker.on_update(update_at(BookAction::Snapshot, -1, 10, 100));

        let outcome = tracker.on_update(update_at(BookAction::Update, 9, 10, 101));

        assert!(matches!(
            outcome,
            SequenceOutcome::RecoveryRequired {
                expected_prev: Some(10),
                received_prev: 9,
                received_seq: 10,
            }
        ));
        assert_eq!(tracker.status(), SequenceStatus::Recovering);
    }

    #[test]
    fn missed_reset_requires_and_recovers_from_a_lower_snapshot() {
        let mut tracker = SequenceTracker::new(8);
        tracker.on_update(update_at(BookAction::Snapshot, -1, 15, 100));

        assert!(matches!(
            tracker.on_update(update_at(BookAction::Update, 3, 5, 102)),
            SequenceOutcome::RecoveryRequired { .. }
        ));
        let outcome = tracker.on_update(update_at(BookAction::Snapshot, -1, 3, 101));

        let SequenceOutcome::Apply(applied) = outcome else {
            panic!("expected lower-epoch snapshot recovery");
        };
        assert_eq!(applied.len(), 2);
        assert_eq!(tracker.status(), SequenceStatus::Ready);
        assert_eq!(tracker.last_seq_id(), Some(5));
        assert_eq!(tracker.reset_count(), 1);
    }

    #[test]
    fn gap_buffers_until_snapshot_then_replays_contiguously() {
        let mut tracker = SequenceTracker::new(8);
        tracker.on_update(update(BookAction::Snapshot, -1, 10));
        assert!(matches!(
            tracker.on_update(update(BookAction::Update, 12, 13)),
            SequenceOutcome::RecoveryRequired { .. }
        ));
        let outcome = tracker.on_update(update(BookAction::Snapshot, -1, 12));

        let SequenceOutcome::Apply(applied) = outcome else {
            panic!("expected successful recovery");
        };
        assert_eq!(applied.len(), 2);
        assert_eq!(tracker.status(), SequenceStatus::Ready);
        assert_eq!(tracker.last_seq_id(), Some(13));
    }

    #[test]
    fn non_contiguous_buffer_stays_recovering() {
        let mut tracker = SequenceTracker::new(8);
        tracker.on_update(update(BookAction::Update, 20, 21));
        let outcome = tracker.on_update(update(BookAction::Snapshot, -1, 19));

        assert!(matches!(outcome, SequenceOutcome::RecoveryRequired { .. }));
        assert_eq!(tracker.status(), SequenceStatus::Recovering);
    }

    #[test]
    fn stale_snapshot_cannot_rewind_ready_stream() {
        let mut tracker = SequenceTracker::new(8);
        tracker.on_update(update(BookAction::Snapshot, -1, 10));
        tracker.on_update(update(BookAction::Update, 10, 11));

        assert_eq!(
            tracker.on_update(update(BookAction::Snapshot, -1, 9)),
            SequenceOutcome::Duplicate
        );
        assert_eq!(tracker.last_seq_id(), Some(11));
    }

    #[test]
    fn equal_timestamp_snapshot_can_reinitialize_one_source() {
        let mut tracker = SequenceTracker::new(8);
        tracker.on_update(update(BookAction::Snapshot, -1, 10));

        assert!(matches!(
            tracker.on_update(update(BookAction::Snapshot, -1, 10)),
            SequenceOutcome::Apply(_)
        ));
        assert_eq!(tracker.status(), SequenceStatus::Ready);
    }

    #[test]
    fn failed_snapshot_keeps_all_remaining_buffered_updates() {
        let mut tracker = SequenceTracker::new(8);
        tracker.on_update(update(BookAction::Snapshot, -1, 10));
        tracker.on_update(update(BookAction::Update, 12, 13));
        tracker.on_update(update(BookAction::Update, 13, 14));

        assert!(matches!(
            tracker.on_update(update(BookAction::Snapshot, -1, 11)),
            SequenceOutcome::RecoveryRequired { .. }
        ));
        assert_eq!(tracker.buffered_len(), 2);
    }

    #[test]
    fn conflicting_buffered_transition_stays_recovering() {
        let mut tracker = SequenceTracker::new(8);
        tracker.on_update(update_at(BookAction::Snapshot, -1, 10, 100));
        tracker.on_update(update_at(BookAction::Update, 11, 12, 102));
        let mut conflict = update_at(BookAction::Update, 11, 12, 102);
        conflict.bids[0].qty = 99.0;
        tracker.on_update(conflict);

        let outcome = tracker.on_update(update_at(BookAction::Snapshot, -1, 11, 101));

        assert!(matches!(outcome, SequenceOutcome::RecoveryRequired { .. }));
        assert_eq!(tracker.status(), SequenceStatus::Recovering);
        assert_eq!(tracker.buffered_len(), 2);
    }

    #[test]
    fn explicit_recovery_preserves_last_sequence_and_accepts_fresh_snapshot() {
        let mut tracker = SequenceTracker::new(8);
        tracker.on_update(update(BookAction::Snapshot, -1, 10));
        tracker.require_recovery();

        assert_eq!(tracker.status(), SequenceStatus::Recovering);
        assert_eq!(tracker.last_seq_id(), Some(10));
        assert!(matches!(
            tracker.on_update(update(BookAction::Snapshot, -1, 10)),
            SequenceOutcome::Apply(_)
        ));
        assert_eq!(tracker.status(), SequenceStatus::Ready);
    }
}
