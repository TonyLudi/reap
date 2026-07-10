use std::collections::HashMap;

use reap_book::{BookReducer, BookStatus};
use reap_core::{
    BookAction, Channel, EventId, MarketEvent, NormalizedEvent, SequencedBookUpdate, Symbol,
    SystemEvent, SystemEventKind, TimeMs, Venue,
};
use reap_venue::{ParsedEvent, PrivateOrderUpdate, RemoteFill, VenueEvent};

use crate::{DedupDecision, Deduplicator, SequenceOutcome, SequenceStatus, SequenceTracker};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FeedStreamId {
    pub venue: Venue,
    pub channel: Channel,
    pub symbol: Symbol,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryRequest {
    pub stream: FeedStreamId,
    pub expected_prev: Option<i64>,
    pub received_prev: i64,
    pub received_seq: i64,
}

#[derive(Debug, Clone)]
pub enum FeedOutput {
    Event(NormalizedEvent),
    PrivateOrder(PrivateOrderUpdate),
    PrivateFill(RemoteFill),
    Duplicate(EventId),
    RecoveryRequired(RecoveryRequest),
    System(SystemEvent),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessorStats {
    pub parsed: u64,
    pub accepted: u64,
    pub duplicates: u64,
    pub gaps: u64,
    pub recoveries: u64,
    pub recovery_failures: u64,
    pub normalized_events: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StreamHealth {
    pub stream: FeedStreamId,
    pub sequence_status: SequenceStatus,
    pub book_status: BookStatus,
    pub last_seq_id: Option<i64>,
    pub buffered_updates: usize,
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
}

#[derive(Debug)]
pub struct FeedProcessor {
    dedup: Deduplicator,
    max_sequence_buffer: usize,
    streams: HashMap<FeedStreamId, BookStream>,
    stats: ProcessorStats,
}

impl FeedProcessor {
    pub fn new(dedup_capacity_per_stream: usize, max_sequence_buffer: usize) -> Self {
        Self {
            dedup: Deduplicator::new(dedup_capacity_per_stream),
            max_sequence_buffer: max_sequence_buffer.max(1),
            streams: HashMap::new(),
            stats: ProcessorStats::default(),
        }
    }

    pub fn process(&mut self, parsed: ParsedEvent) -> Vec<FeedOutput> {
        self.stats.parsed += 1;
        if self.dedup.check(&parsed.id) == DedupDecision::Duplicate {
            self.stats.duplicates += 1;
            return vec![FeedOutput::Duplicate(parsed.id)];
        }
        self.stats.accepted += 1;
        let venue = parsed.id.venue;

        match parsed.event {
            VenueEvent::Book(update) => self.process_book(venue, update),
            VenueEvent::Normalized(event) => {
                self.stats.normalized_events += 1;
                vec![FeedOutput::Event(event)]
            }
            VenueEvent::PrivateOrder(update) => vec![
                private_heartbeat(venue, update.ts_ms),
                FeedOutput::PrivateOrder(update),
            ],
            VenueEvent::PrivateFill(fill) => vec![
                private_heartbeat(venue, fill.ts_ms),
                FeedOutput::PrivateFill(fill),
            ],
            VenueEvent::Account(update) => {
                self.stats.normalized_events += 1;
                vec![
                    private_heartbeat(venue, update.ts_ms),
                    FeedOutput::Event(NormalizedEvent::Account(update)),
                ]
            }
        }
    }

    pub fn stats(&self) -> &ProcessorStats {
        &self.stats
    }

    pub fn stream_health(&self) -> Vec<StreamHealth> {
        let mut health = self
            .streams
            .iter()
            .map(|(stream, state)| StreamHealth {
                stream: stream.clone(),
                sequence_status: state.sequence.status(),
                book_status: state.book.status(),
                last_seq_id: state.sequence.last_seq_id(),
                buffered_updates: state.sequence.buffered_len(),
                best_bid: state.book.best(reap_core::Side::Buy).map(|level| level.px),
                best_ask: state.book.best(reap_core::Side::Sell).map(|level| level.px),
            })
            .collect::<Vec<_>>();
        health.sort_by(|left, right| left.stream.symbol.cmp(&right.stream.symbol));
        health
    }

    pub fn mark_stale(&mut self, now_ms: TimeMs, max_age_ms: TimeMs) -> Vec<SystemEvent> {
        self.streams
            .iter_mut()
            .filter_map(|(stream, state)| {
                let before = state.book.status();
                let after = state.book.mark_stale_if_older_than(now_ms, max_age_ms);
                (before != BookStatus::Stale && after == BookStatus::Stale).then(|| SystemEvent {
                    ts_ms: now_ms,
                    kind: SystemEventKind::FeedStale,
                    venue: Some(stream.venue),
                    symbol: Some(stream.symbol.clone()),
                    reason: format!("book exceeded max age of {max_age_ms}ms"),
                })
            })
            .collect()
    }

    fn process_book(&mut self, venue: Venue, update: SequencedBookUpdate) -> Vec<FeedOutput> {
        let stream_id = FeedStreamId {
            venue,
            channel: Channel::Books,
            symbol: update.symbol.clone(),
        };
        let state = self
            .streams
            .entry(stream_id.clone())
            .or_insert_with(|| BookStream {
                sequence: SequenceTracker::new(self.max_sequence_buffer),
                book: BookReducer::new(update.symbol.clone()),
            });
        let was_recovering = state.sequence.status() == SequenceStatus::Recovering;
        let was_ready = state.sequence.status() == SequenceStatus::Ready;
        let ts_ms = update.ts_ms;
        match state.sequence.on_update(update) {
            SequenceOutcome::Apply(updates) => {
                for update in updates {
                    apply_book_update(&mut state.book, update);
                }
                if state.book.is_ready() {
                    let mut outputs = Vec::new();
                    if !was_ready {
                        if was_recovering {
                            self.stats.recoveries += 1;
                        }
                        outputs.push(FeedOutput::System(SystemEvent {
                            ts_ms,
                            kind: SystemEventKind::FeedRecovered,
                            venue: Some(venue),
                            symbol: Some(stream_id.symbol.clone()),
                            reason: if was_recovering {
                                "snapshot and buffered deltas are contiguous".to_string()
                            } else {
                                "initial book snapshot is ready".to_string()
                            },
                        }));
                    }
                    outputs.push(FeedOutput::System(SystemEvent {
                        ts_ms,
                        kind: SystemEventKind::FeedHeartbeat,
                        venue: Some(venue),
                        symbol: Some(stream_id.symbol.clone()),
                        reason: "sequenced book update accepted".to_string(),
                    }));
                    if let Some(book) = state.book.book().cloned() {
                        self.stats.normalized_events += 1;
                        outputs.push(FeedOutput::Event(NormalizedEvent::from(
                            MarketEvent::Depth(book),
                        )));
                    }
                    outputs
                } else {
                    Vec::new()
                }
            }
            SequenceOutcome::Duplicate | SequenceOutcome::Buffered => Vec::new(),
            SequenceOutcome::RecoveryRequired {
                expected_prev,
                received_prev,
                received_seq,
            } => {
                if !was_recovering {
                    self.stats.gaps += 1;
                }
                state.book.mark_gapped();
                state.book.mark_recovering();
                vec![
                    FeedOutput::System(SystemEvent {
                        ts_ms,
                        kind: SystemEventKind::FeedGap,
                        venue: Some(venue),
                        symbol: Some(stream_id.symbol.clone()),
                        reason: format!(
                            "expected prev sequence {expected_prev:?}, received {received_prev}"
                        ),
                    }),
                    FeedOutput::System(SystemEvent {
                        ts_ms,
                        kind: SystemEventKind::BookRecoveryStarted,
                        venue: Some(venue),
                        symbol: Some(stream_id.symbol.clone()),
                        reason: "request a fresh websocket snapshot".to_string(),
                    }),
                    FeedOutput::RecoveryRequired(RecoveryRequest {
                        stream: stream_id,
                        expected_prev,
                        received_prev,
                        received_seq,
                    }),
                ]
            }
            SequenceOutcome::RecoveryFailed { reason } => {
                self.stats.recovery_failures += 1;
                state.book.mark_recovering();
                vec![FeedOutput::System(SystemEvent {
                    ts_ms,
                    kind: SystemEventKind::BookRecoveryFailed,
                    venue: Some(venue),
                    symbol: Some(stream_id.symbol),
                    reason,
                })]
            }
        }
    }
}

fn private_heartbeat(venue: Venue, ts_ms: TimeMs) -> FeedOutput {
    FeedOutput::System(SystemEvent {
        ts_ms,
        kind: SystemEventKind::PrivateStreamHeartbeat,
        venue: Some(venue),
        symbol: None,
        reason: "private websocket event accepted".to_string(),
    })
}

#[derive(Debug)]
struct BookStream {
    sequence: SequenceTracker,
    book: BookReducer,
}

fn apply_book_update(book: &mut BookReducer, update: SequencedBookUpdate) {
    match update.action {
        BookAction::Snapshot => {
            book.apply_snapshot(update.as_book());
        }
        BookAction::Update => {
            book.apply_delta(update.ts_ms, &update.bids, &update.asks);
        }
    }
}

#[cfg(test)]
mod tests {
    use reap_core::{BookAction, EventKey, Level};
    use reap_venue::VenueEvent;

    use super::*;

    fn parsed(action: BookAction, prev: i64, seq: i64, bid: f64) -> ParsedEvent {
        let update = SequencedBookUpdate {
            action,
            symbol: "BTC-USDT".to_string(),
            ts_ms: seq as u64,
            prev_seq_id: prev,
            seq_id: seq,
            bids: vec![Level::new(100.0, bid)],
            asks: vec![Level::new(101.0, 1.0)],
        };
        ParsedEvent {
            id: EventId {
                venue: Venue::Okx,
                channel: Channel::Books,
                symbol: Some("BTC-USDT".to_string()),
                key: EventKey::BookSequence {
                    action,
                    seq_id: seq,
                },
            },
            event: VenueEvent::Book(update),
        }
    }

    #[test]
    fn gap_suppresses_books_until_snapshot_recovery() {
        let mut processor = FeedProcessor::new(16, 16);
        let first = processor.process(parsed(BookAction::Snapshot, -1, 10, 1.0));
        assert!(
            first
                .iter()
                .any(|output| matches!(output, FeedOutput::Event(_)))
        );

        let gap = processor.process(parsed(BookAction::Update, 11, 12, 2.0));
        assert!(
            gap.iter()
                .any(|output| matches!(output, FeedOutput::RecoveryRequired(_)))
        );
        assert!(
            !gap.iter()
                .any(|output| matches!(output, FeedOutput::Event(_)))
        );

        let recovered = processor.process(parsed(BookAction::Snapshot, -1, 11, 1.5));
        assert!(
            recovered
                .iter()
                .any(|output| matches!(output, FeedOutput::Event(_)))
        );
        let health = processor.stream_health();
        assert_eq!(health[0].sequence_status, SequenceStatus::Ready);
        assert_eq!(health[0].book_status, BookStatus::Ready);
        assert_eq!(health[0].last_seq_id, Some(12));
    }
}
