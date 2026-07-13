use std::collections::HashMap;

use reap_book::{BookReducer, BookStatus};
use reap_core::{
    AccountUpdate, BookAction, Channel, EventId, MarketEvent, NormalizedEvent, SequencedBookUpdate,
    Symbol, SystemEvent, SystemEventKind, TimeMs, Venue,
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
    PrivateOrder {
        account_id: Option<String>,
        update: PrivateOrderUpdate,
    },
    PrivateFill {
        account_id: Option<String>,
        fill: RemoteFill,
    },
    PrivateAccount {
        account_id: Option<String>,
        update: AccountUpdate,
    },
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
    pub sequence_resets: u64,
    pub same_sequence_updates: u64,
    pub normalized_events: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StreamHealth {
    pub stream: FeedStreamId,
    pub sequence_status: SequenceStatus,
    pub book_status: BookStatus,
    pub last_seq_id: Option<i64>,
    pub buffered_updates: usize,
    pub sequence_resets: u64,
    pub same_sequence_updates: u64,
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
        let account_id = parsed.account_id;

        match parsed.event {
            VenueEvent::Book(update) => self.process_book(venue, update),
            VenueEvent::Normalized(event) => {
                self.stats.normalized_events += 1;
                vec![FeedOutput::Event(event)]
            }
            VenueEvent::PrivateOrder(update) => {
                vec![FeedOutput::PrivateOrder { account_id, update }]
            }
            VenueEvent::PrivateFill(fill) => vec![FeedOutput::PrivateFill { account_id, fill }],
            VenueEvent::Account(update) => {
                self.stats.normalized_events += 1;
                vec![FeedOutput::PrivateAccount { account_id, update }]
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
                sequence_resets: state.sequence.reset_count(),
                same_sequence_updates: state.sequence.same_sequence_count(),
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
                    account_id: None,
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
        let reset_count = state.sequence.reset_count();
        let same_sequence_count = state.sequence.same_sequence_count();
        let outcome = state.sequence.on_update(update);
        self.stats.sequence_resets = self
            .stats
            .sequence_resets
            .saturating_add(state.sequence.reset_count().saturating_sub(reset_count));
        self.stats.same_sequence_updates = self.stats.same_sequence_updates.saturating_add(
            state
                .sequence
                .same_sequence_count()
                .saturating_sub(same_sequence_count),
        );
        match outcome {
            SequenceOutcome::Apply(updates) => {
                let received_prev = updates.last().map_or(0, |update| update.prev_seq_id);
                let received_seq = updates.last().map_or(0, |update| update.seq_id);
                for update in updates {
                    apply_book_update(&mut state.book, update);
                }
                if !state.book.is_ready() {
                    self.stats.gaps += 1;
                    state.sequence.require_recovery();
                    state.book.mark_gapped();
                    state.book.mark_recovering();
                    return vec![
                        FeedOutput::System(SystemEvent {
                            ts_ms,
                            kind: SystemEventKind::FeedGap,
                            venue: Some(venue),
                            account_id: None,
                            symbol: Some(stream_id.symbol.clone()),
                            reason: "book is empty, invalid, or crossed after sequenced update"
                                .to_string(),
                        }),
                        FeedOutput::System(SystemEvent {
                            ts_ms,
                            kind: SystemEventKind::BookRecoveryStarted,
                            venue: Some(venue),
                            account_id: None,
                            symbol: Some(stream_id.symbol.clone()),
                            reason: "request a fresh websocket snapshot".to_string(),
                        }),
                        FeedOutput::RecoveryRequired(RecoveryRequest {
                            stream: stream_id,
                            expected_prev: state.sequence.last_seq_id(),
                            received_prev,
                            received_seq,
                        }),
                    ];
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
                            account_id: None,
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
                        account_id: None,
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
                        account_id: None,
                        symbol: Some(stream_id.symbol.clone()),
                        reason: format!(
                            "expected prev sequence {expected_prev:?}, received {received_prev}"
                        ),
                    }),
                    FeedOutput::System(SystemEvent {
                        ts_ms,
                        kind: SystemEventKind::BookRecoveryStarted,
                        venue: Some(venue),
                        account_id: None,
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
                    account_id: None,
                    symbol: Some(stream_id.symbol),
                    reason,
                })]
            }
        }
    }
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
                    prev_seq_id: prev,
                    seq_id: seq,
                    ts_ms: seq.max(0) as u64,
                    raw_hash: bid.to_bits(),
                },
            },
            account_id: None,
            event: VenueEvent::Book(update),
        }
    }

    #[test]
    fn private_payload_reduction_does_not_infer_aggregate_stream_health() {
        let mut processor = FeedProcessor::new(16, 16);
        let outputs = processor.process(ParsedEvent {
            id: EventId {
                venue: Venue::Okx,
                channel: Channel::Account,
                symbol: Some("main".to_string()),
                key: EventKey::Timestamp(1),
            },
            account_id: Some("main".to_string()),
            event: VenueEvent::Account(AccountUpdate {
                ts_ms: 1,
                balances: Vec::new(),
                positions: Vec::new(),
                margins: Vec::new(),
            }),
        });

        assert_eq!(outputs.len(), 1);
        assert!(matches!(outputs[0], FeedOutput::PrivateAccount { .. }));
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

    #[test]
    fn crossed_book_forces_snapshot_recovery() {
        let mut processor = FeedProcessor::new(16, 16);
        let mut crossed = parsed(BookAction::Snapshot, -1, 10, 1.0);
        let VenueEvent::Book(update) = &mut crossed.event else {
            unreachable!();
        };
        update.bids[0].px = 102.0;

        let outputs = processor.process(crossed);
        assert!(
            outputs
                .iter()
                .any(|output| matches!(output, FeedOutput::RecoveryRequired(_)))
        );
        assert!(
            !outputs
                .iter()
                .any(|output| matches!(output, FeedOutput::Event(_)))
        );
        let health = processor.stream_health();
        assert_eq!(health[0].sequence_status, SequenceStatus::Recovering);
        assert_eq!(health[0].book_status, BookStatus::Recovering);

        let recovered = processor.process(parsed(BookAction::Snapshot, -1, 11, 1.0));
        assert!(
            recovered
                .iter()
                .any(|output| matches!(output, FeedOutput::Event(_)))
        );
        assert_eq!(processor.stats().gaps, 1);
    }

    #[test]
    fn maintenance_reset_and_same_sequence_heartbeat_remain_ready() {
        let mut processor = FeedProcessor::new(16, 16);
        processor.process(parsed(BookAction::Snapshot, -1, 10, 1.0));
        processor.process(parsed(BookAction::Update, 10, 15, 1.1));

        let reset = processor.process(parsed(BookAction::Update, 15, 3, 1.2));
        let heartbeat = processor.process(parsed(BookAction::Update, 3, 3, 1.2));
        let following = processor.process(parsed(BookAction::Update, 3, 5, 1.3));

        assert!(
            !reset
                .iter()
                .any(|output| matches!(output, FeedOutput::RecoveryRequired(_)))
        );
        assert!(
            heartbeat
                .iter()
                .any(|output| matches!(output, FeedOutput::Event(_)))
        );
        assert!(
            following
                .iter()
                .any(|output| matches!(output, FeedOutput::Event(_)))
        );
        assert_eq!(processor.stats().sequence_resets, 1);
        assert_eq!(processor.stats().same_sequence_updates, 1);
        assert_eq!(processor.stats().gaps, 0);
        let health = processor.stream_health();
        assert_eq!(health[0].sequence_status, SequenceStatus::Ready);
        assert_eq!(health[0].last_seq_id, Some(5));
    }

    #[test]
    fn conflicting_redundant_payload_forces_recovery() {
        let mut processor = FeedProcessor::new(16, 16);
        processor.process(parsed(BookAction::Snapshot, -1, 10, 1.0));
        processor.process(parsed(BookAction::Update, 10, 11, 1.1));

        let outputs = processor.process(parsed(BookAction::Update, 10, 11, 2.0));

        assert!(
            outputs
                .iter()
                .any(|output| matches!(output, FeedOutput::RecoveryRequired(_)))
        );
        assert_eq!(processor.stats().duplicates, 0);
        assert_eq!(processor.stats().gaps, 1);
        assert_eq!(
            processor.stream_health()[0].sequence_status,
            SequenceStatus::Recovering
        );
    }

    #[test]
    fn conflicting_redundant_snapshot_forces_recovery() {
        let mut processor = FeedProcessor::new(16, 16);
        processor.process(parsed(BookAction::Snapshot, -1, 10, 1.0));

        let outputs = processor.process(parsed(BookAction::Snapshot, -1, 10, 2.0));

        assert!(
            outputs
                .iter()
                .any(|output| matches!(output, FeedOutput::RecoveryRequired(_)))
        );
        assert_eq!(processor.stats().duplicates, 0);
        assert_eq!(processor.stats().gaps, 1);
        assert_eq!(
            processor.stream_health()[0].sequence_status,
            SequenceStatus::Recovering
        );
    }

    #[test]
    fn conflicting_replicas_during_gap_cannot_be_collapsed_on_snapshot() {
        let mut processor = FeedProcessor::new(16, 16);
        processor.process(parsed(BookAction::Snapshot, -1, 10, 1.0));
        processor.process(parsed(BookAction::Update, 11, 12, 1.1));
        processor.process(parsed(BookAction::Update, 11, 12, 2.0));

        let outputs = processor.process(parsed(BookAction::Snapshot, -1, 11, 1.5));

        assert!(
            outputs
                .iter()
                .any(|output| matches!(output, FeedOutput::RecoveryRequired(_)))
        );
        assert!(
            !outputs
                .iter()
                .any(|output| matches!(output, FeedOutput::Event(_)))
        );
        assert_eq!(processor.stats().duplicates, 0);
        assert_eq!(processor.stats().gaps, 1);
        assert_eq!(
            processor.stream_health()[0].sequence_status,
            SequenceStatus::Recovering
        );
    }
}
