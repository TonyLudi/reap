use std::collections::{HashMap, HashSet, VecDeque};

use reap_core::{Channel, EventId, EventKey, Symbol, Venue};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupDecision {
    Accepted,
    Duplicate,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DedupStats {
    pub accepted: u64,
    pub duplicates: u64,
    pub evicted: u64,
}

#[derive(Debug)]
pub struct Deduplicator {
    capacity_per_stream: usize,
    streams: HashMap<StreamKey, RecentKeys>,
    stats: DedupStats,
}

impl Deduplicator {
    pub fn new(capacity_per_stream: usize) -> Self {
        Self {
            capacity_per_stream: capacity_per_stream.max(1),
            streams: HashMap::new(),
            stats: DedupStats::default(),
        }
    }

    pub fn check(&mut self, event_id: &EventId) -> DedupDecision {
        let stream = StreamKey::from(event_id);
        let recent = self.streams.entry(stream).or_default();
        if recent.set.contains(&event_id.key) {
            self.stats.duplicates += 1;
            return DedupDecision::Duplicate;
        }
        recent.set.insert(event_id.key.clone());
        recent.order.push_back(event_id.key.clone());
        self.stats.accepted += 1;
        if recent.order.len() > self.capacity_per_stream
            && let Some(expired) = recent.order.pop_front()
        {
            recent.set.remove(&expired);
            self.stats.evicted += 1;
        }
        DedupDecision::Accepted
    }

    pub fn stats(&self) -> &DedupStats {
        &self.stats
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct StreamKey {
    venue: Venue,
    channel: Channel,
    symbol: Option<Symbol>,
}

impl From<&EventId> for StreamKey {
    fn from(value: &EventId) -> Self {
        Self {
            venue: value.venue,
            channel: value.channel.clone(),
            symbol: value.symbol.clone(),
        }
    }
}

#[derive(Debug, Default)]
struct RecentKeys {
    order: VecDeque<EventKey>,
    set: HashSet<EventKey>,
}

#[cfg(test)]
mod tests {
    use reap_core::BookAction;

    use super::*;

    fn id(prev_seq_id: i64, seq_id: i64, ts_ms: u64, raw_hash: u64) -> EventId {
        EventId {
            venue: Venue::Okx,
            channel: Channel::Books,
            symbol: Some("BTC-USDT".to_string()),
            key: EventKey::BookSequence {
                action: BookAction::Update,
                prev_seq_id,
                seq_id,
                ts_ms,
                raw_hash,
            },
        }
    }

    #[test]
    fn duplicate_ids_from_redundant_connections_are_rejected() {
        let mut dedup = Deduplicator::new(4);
        assert_eq!(dedup.check(&id(0, 1, 100, 7)), DedupDecision::Accepted);
        assert_eq!(dedup.check(&id(0, 1, 100, 7)), DedupDecision::Duplicate);
        assert_eq!(dedup.stats().duplicates, 1);
    }

    #[test]
    fn conflicting_payload_for_the_same_transition_is_not_suppressed() {
        let mut dedup = Deduplicator::new(4);

        assert_eq!(dedup.check(&id(0, 1, 100, 7)), DedupDecision::Accepted);
        assert_eq!(dedup.check(&id(0, 1, 100, 8)), DedupDecision::Accepted);
    }

    #[test]
    fn reset_epochs_and_same_sequence_heartbeats_are_not_false_duplicates() {
        let mut dedup = Deduplicator::new(8);

        assert_eq!(dedup.check(&id(2, 3, 100, 7)), DedupDecision::Accepted);
        assert_eq!(dedup.check(&id(15, 3, 200, 8)), DedupDecision::Accepted);
        assert_eq!(dedup.check(&id(3, 3, 201, 9)), DedupDecision::Accepted);
        assert_eq!(dedup.check(&id(3, 3, 201, 9)), DedupDecision::Duplicate);
    }

    #[test]
    fn cache_is_bounded_per_channel_symbol() {
        let mut dedup = Deduplicator::new(2);
        dedup.check(&id(0, 1, 1, 1));
        dedup.check(&id(1, 2, 2, 2));
        dedup.check(&id(2, 3, 3, 3));

        assert_eq!(dedup.stats().evicted, 1);
        assert_eq!(dedup.check(&id(0, 1, 1, 1)), DedupDecision::Accepted);
    }
}
