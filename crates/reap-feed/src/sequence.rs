use std::collections::VecDeque;

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
    buffered: VecDeque<SequencedBookUpdate>,
    max_buffered: usize,
}

impl SequenceTracker {
    pub fn new(max_buffered: usize) -> Self {
        Self {
            status: SequenceStatus::Empty,
            last_seq_id: None,
            buffered: VecDeque::new(),
            max_buffered: max_buffered.max(1),
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
                if update.seq_id <= last {
                    return SequenceOutcome::Duplicate;
                }
                if update.prev_seq_id == last {
                    self.last_seq_id = Some(update.seq_id);
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
        if self.last_seq_id.is_some_and(|last| snapshot.seq_id < last) {
            return SequenceOutcome::Duplicate;
        }
        if self.status == SequenceStatus::Ready && self.last_seq_id == Some(snapshot.seq_id) {
            return SequenceOutcome::Duplicate;
        }
        self.last_seq_id = Some(snapshot.seq_id);
        let mut apply = vec![snapshot];
        let mut pending = self.buffered.drain(..).collect::<Vec<_>>();
        pending.sort_by_key(|update| update.seq_id);
        pending.dedup_by_key(|update| update.seq_id);
        pending.retain(|update| update.seq_id > self.last_seq_id.unwrap_or(i64::MIN));

        let mut pending = pending.into_iter();
        while let Some(update) = pending.next() {
            let last = self.last_seq_id.expect("snapshot set last id");
            if update.prev_seq_id != last {
                self.buffered.push_back(update);
                self.buffered.extend(pending);
                self.status = SequenceStatus::Recovering;
                return SequenceOutcome::RecoveryRequired {
                    expected_prev: Some(last),
                    received_prev: self.buffered[0].prev_seq_id,
                    received_seq: self.buffered[0].seq_id,
                };
            }
            self.last_seq_id = Some(update.seq_id);
            apply.push(update);
        }
        self.status = SequenceStatus::Ready;
        SequenceOutcome::Apply(apply)
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
        SequencedBookUpdate {
            action,
            symbol: "BTC-USDT".to_string(),
            ts_ms: seq as u64,
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
}
