use std::collections::HashMap;

use reap_book::{BookReducer, BookStatus};
use reap_core::{
    AccountUpdate, BookAction, Channel, ConnId, EventId, MarketEvent, NormalizedEvent, OkxVenue,
    OrderBook, SequencedBookUpdate, Symbol, SystemEvent, SystemEventKind, TimeMs,
};
use reap_venue::{ParsedEvent, PrivateOrderUpdate, RemoteFill, VenueEvent};

use crate::{DedupDecision, Deduplicator, SequenceOutcome, SequenceStatus, SequenceTracker};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FeedStreamId {
    pub venue: OkxVenue,
    pub channel: Channel,
    pub symbol: Symbol,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryRequest {
    pub stream: FeedStreamId,
    pub source_conn_id: Option<ConnId>,
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
    source_dedup: Deduplicator,
    max_sequence_buffer: usize,
    streams: HashMap<FeedStreamId, BookStream>,
    stats: ProcessorStats,
}

impl FeedProcessor {
    pub fn new(dedup_capacity_per_stream: usize, max_sequence_buffer: usize) -> Self {
        Self {
            dedup: Deduplicator::new(dedup_capacity_per_stream),
            source_dedup: Deduplicator::new(max_sequence_buffer),
            max_sequence_buffer: max_sequence_buffer.max(1),
            streams: HashMap::new(),
            stats: ProcessorStats::default(),
        }
    }

    #[cfg(test)]
    fn process(&mut self, parsed: ParsedEvent) -> Vec<FeedOutput> {
        self.process_from(&ConnId::new("default"), parsed)
    }

    pub fn process_from(&mut self, source: &ConnId, parsed: ParsedEvent) -> Vec<FeedOutput> {
        self.stats.parsed += 1;
        let globally_duplicate = self.dedup.check(&parsed.id) == DedupDecision::Duplicate;
        if globally_duplicate {
            self.stats.duplicates += 1;
        } else {
            self.stats.accepted += 1;
        }
        let venue = parsed.id.venue;
        let account_id = parsed.account_id;

        match parsed.event {
            VenueEvent::Book(update) => {
                let recovery_snapshot = update.action == BookAction::Snapshot
                    && self
                        .streams
                        .get(&FeedStreamId {
                            venue,
                            channel: Channel::Books,
                            symbol: update.symbol.clone(),
                        })
                        .is_some_and(|state| {
                            state.aggregate_recovering
                                || state
                                    .sources
                                    .get(source)
                                    .is_some_and(|source| !source.is_ready())
                        });
                if self.source_dedup.check_source(&parsed.id, source) == DedupDecision::Duplicate
                    && !recovery_snapshot
                {
                    return vec![FeedOutput::Duplicate(parsed.id)];
                }
                let mut outputs = self.process_book(venue, source, update);
                if globally_duplicate {
                    outputs.push(FeedOutput::Duplicate(parsed.id));
                }
                outputs
            }
            _ if globally_duplicate => vec![FeedOutput::Duplicate(parsed.id)],
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
                sequence_status: state.sequence_status(),
                book_status: state.published.status(),
                last_seq_id: state.published_version.map(|version| version.seq_id),
                buffered_updates: state
                    .sources
                    .values()
                    .map(|source| source.sequence.buffered_len())
                    .sum(),
                sequence_resets: state
                    .sources
                    .values()
                    .map(|source| source.sequence.reset_count())
                    .sum(),
                same_sequence_updates: state
                    .sources
                    .values()
                    .map(|source| source.sequence.same_sequence_count())
                    .sum(),
                best_bid: state
                    .published
                    .best(reap_core::Side::Buy)
                    .map(|level| level.px),
                best_ask: state
                    .published
                    .best(reap_core::Side::Sell)
                    .map(|level| level.px),
            })
            .collect::<Vec<_>>();
        health.sort_by(|left, right| left.stream.symbol.cmp(&right.stream.symbol));
        health
    }

    pub fn ready_books(&self) -> Vec<reap_core::OrderBook> {
        let mut books = self
            .streams
            .values()
            .filter(|state| state.is_ready())
            .filter_map(|state| state.published.book().cloned())
            .collect::<Vec<_>>();
        books.sort_by(|left, right| left.symbol.cmp(&right.symbol));
        books
    }

    pub fn mark_stale(&mut self, now_ms: TimeMs, max_age_ms: TimeMs) -> Vec<SystemEvent> {
        self.streams
            .iter_mut()
            .filter_map(|(stream, state)| {
                let before = state.published.status();
                let after = state.published.mark_stale_if_older_than(now_ms, max_age_ms);
                if before != BookStatus::Stale && after == BookStatus::Stale {
                    state.aggregate_recovering = true;
                }
                (before != BookStatus::Stale && after == BookStatus::Stale).then(|| SystemEvent {
                    ts_ms: now_ms,
                    kind: SystemEventKind::FeedStale,
                    venue: Some(stream.venue.into()),
                    account_id: None,
                    symbol: Some(stream.symbol.clone()),
                    reason: format!("book exceeded max age of {max_age_ms}ms"),
                })
            })
            .collect()
    }
    fn process_book(
        &mut self,
        venue: OkxVenue,
        source: &ConnId,
        update: SequencedBookUpdate,
    ) -> Vec<FeedOutput> {
        let stream_id = FeedStreamId {
            venue,
            channel: Channel::Books,
            symbol: update.symbol.clone(),
        };
        let state = self
            .streams
            .entry(stream_id.clone())
            .or_insert_with(|| BookStream::new(update.symbol.clone()));
        process_source_book(
            &mut self.stats,
            self.max_sequence_buffer,
            state,
            stream_id,
            source,
            update,
        )
    }
}

fn process_source_book(
    stats: &mut ProcessorStats,
    max_sequence_buffer: usize,
    state: &mut BookStream,
    stream_id: FeedStreamId,
    source: &ConnId,
    update: SequencedBookUpdate,
) -> Vec<FeedOutput> {
    let aggregate_was_ready = state.is_ready();
    let aggregate_was_recovering = state.aggregate_recovering;
    let ts_ms = update.ts_ms;
    let source_state = state
        .sources
        .entry(source.clone())
        .or_insert_with(|| SourceBookStream::new(update.symbol.clone(), max_sequence_buffer));
    let source_was_recovering = source_state.sequence.status() == SequenceStatus::Recovering;
    let reset_count = source_state.sequence.reset_count();
    let same_sequence_count = source_state.sequence.same_sequence_count();
    let outcome = source_state.sequence.on_update(update);
    stats.sequence_resets = stats.sequence_resets.saturating_add(
        source_state
            .sequence
            .reset_count()
            .saturating_sub(reset_count),
    );
    stats.same_sequence_updates = stats.same_sequence_updates.saturating_add(
        source_state
            .sequence
            .same_sequence_count()
            .saturating_sub(same_sequence_count),
    );

    match outcome {
        SequenceOutcome::Apply(updates) => {
            let version = BookVersion::from(
                updates
                    .last()
                    .expect("an applied sequence must contain at least one update"),
            );
            let source_state = state
                .sources
                .get_mut(source)
                .expect("source state was inserted before sequence processing");
            for update in updates {
                apply_book_update(&mut source_state.book, update);
            }
            if !source_state.book.is_ready() {
                source_state.sequence.require_recovery();
                source_state.book.mark_gapped();
                source_state.book.mark_recovering();
                if !source_was_recovering {
                    stats.gaps = stats.gaps.saturating_add(1);
                }
                let expected_prev = source_state.sequence.last_seq_id();
                return source_recovery_outputs(
                    state,
                    &stream_id,
                    SourceRecovery {
                        source_conn_id: source.clone(),
                        ts_ms,
                        expected_prev,
                        received_prev: version.prev_seq_id,
                        received_seq: version.seq_id,
                        reason: "book is empty, invalid, or crossed after sequenced update"
                            .to_string(),
                        failed: false,
                    },
                );
            }
            if source_was_recovering {
                stats.recoveries = stats.recoveries.saturating_add(1);
            }
            let candidate = source_state
                .book
                .book()
                .cloned()
                .expect("ready source book must contain a book");
            match state.publish_candidate(version, candidate) {
                PublishDecision::Published => {
                    let mut outputs = Vec::new();
                    if !aggregate_was_ready {
                        outputs.push(FeedOutput::System(SystemEvent {
                            ts_ms,
                            kind: SystemEventKind::FeedRecovered,
                            venue: Some(stream_id.venue.into()),
                            account_id: None,
                            symbol: Some(stream_id.symbol.clone()),
                            reason: if aggregate_was_recovering {
                                "a source snapshot restored a valid canonical book".to_string()
                            } else {
                                "initial source snapshot established a canonical book".to_string()
                            },
                        }));
                    }
                    outputs.push(FeedOutput::System(SystemEvent {
                        ts_ms,
                        kind: SystemEventKind::FeedHeartbeat,
                        venue: Some(stream_id.venue.into()),
                        account_id: None,
                        symbol: Some(stream_id.symbol.clone()),
                        reason: "source-sequenced canonical book update accepted".to_string(),
                    }));
                    if let Some(book) = state.published.book().cloned() {
                        stats.normalized_events = stats.normalized_events.saturating_add(1);
                        outputs.push(FeedOutput::Event(NormalizedEvent::from(
                            MarketEvent::Depth(book),
                        )));
                    }
                    outputs
                }
                PublishDecision::Ignored => Vec::new(),
                PublishDecision::Conflict => {
                    if !state.aggregate_recovering {
                        stats.gaps = stats.gaps.saturating_add(1);
                    }
                    replica_conflict_outputs(state, &stream_id, ts_ms, version)
                }
            }
        }
        SequenceOutcome::Duplicate | SequenceOutcome::Buffered => Vec::new(),
        SequenceOutcome::RecoveryRequired {
            expected_prev,
            received_prev,
            received_seq,
        } => {
            if !source_was_recovering {
                stats.gaps = stats.gaps.saturating_add(1);
            }
            let source_state = state
                .sources
                .get_mut(source)
                .expect("source state was inserted before sequence processing");
            source_state.book.mark_gapped();
            source_state.book.mark_recovering();
            source_recovery_outputs(
                state,
                &stream_id,
                SourceRecovery {
                    source_conn_id: source.clone(),
                    ts_ms,
                    expected_prev,
                    received_prev,
                    received_seq,
                    reason: format!(
                        "source expected prev sequence {expected_prev:?}, received {received_prev}"
                    ),
                    failed: false,
                },
            )
        }
        SequenceOutcome::RecoveryFailed { reason } => {
            stats.recovery_failures = stats.recovery_failures.saturating_add(1);
            let source_state = state
                .sources
                .get_mut(source)
                .expect("source state was inserted before sequence processing");
            source_state.sequence.require_recovery();
            source_state.book.mark_recovering();
            let expected_prev = source_state.sequence.last_seq_id();
            source_recovery_outputs(
                state,
                &stream_id,
                SourceRecovery {
                    source_conn_id: source.clone(),
                    ts_ms,
                    expected_prev,
                    received_prev: 0,
                    received_seq: 0,
                    reason,
                    failed: true,
                },
            )
        }
    }
}

struct SourceRecovery {
    source_conn_id: ConnId,
    ts_ms: TimeMs,
    expected_prev: Option<i64>,
    received_prev: i64,
    received_seq: i64,
    reason: String,
    failed: bool,
}

fn source_recovery_outputs(
    state: &mut BookStream,
    stream_id: &FeedStreamId,
    recovery: SourceRecovery,
) -> Vec<FeedOutput> {
    let request = FeedOutput::RecoveryRequired(RecoveryRequest {
        stream: stream_id.clone(),
        source_conn_id: Some(recovery.source_conn_id.clone()),
        expected_prev: recovery.expected_prev,
        received_prev: recovery.received_prev,
        received_seq: recovery.received_seq,
    });
    if state.is_ready() {
        return vec![request];
    }
    state.aggregate_recovering = true;
    state.published.mark_gapped();
    state.published.mark_recovering();
    let kind = if recovery.failed {
        SystemEventKind::BookRecoveryFailed
    } else {
        SystemEventKind::FeedGap
    };
    let mut outputs = vec![FeedOutput::System(SystemEvent {
        ts_ms: recovery.ts_ms,
        kind,
        venue: Some(stream_id.venue.into()),
        account_id: None,
        symbol: Some(stream_id.symbol.clone()),
        reason: format!("source {}: {}", recovery.source_conn_id, recovery.reason),
    })];
    if !recovery.failed {
        outputs.push(FeedOutput::System(SystemEvent {
            ts_ms: recovery.ts_ms,
            kind: SystemEventKind::BookRecoveryStarted,
            venue: Some(stream_id.venue.into()),
            account_id: None,
            symbol: Some(stream_id.symbol.clone()),
            reason: format!(
                "request a fresh websocket snapshot from source {}",
                recovery.source_conn_id
            ),
        }));
    }
    outputs.push(request);
    outputs
}

fn replica_conflict_outputs(
    state: &mut BookStream,
    stream_id: &FeedStreamId,
    ts_ms: TimeMs,
    version: BookVersion,
) -> Vec<FeedOutput> {
    state.aggregate_recovering = true;
    state.published.mark_gapped();
    state.published.mark_recovering();
    let mut sources = state.sources.keys().cloned().collect::<Vec<_>>();
    sources.sort_by(|left, right| left.0.cmp(&right.0));
    for source in state.sources.values_mut() {
        source.sequence.require_recovery();
        source.book.mark_recovering();
    }
    let mut outputs = vec![
        FeedOutput::System(SystemEvent {
            ts_ms,
            kind: SystemEventKind::FeedGap,
            venue: Some(stream_id.venue.into()),
            account_id: None,
            symbol: Some(stream_id.symbol.clone()),
            reason: format!(
                "replica books disagree at exchange timestamp {} sequence {}",
                version.ts_ms, version.seq_id
            ),
        }),
        FeedOutput::System(SystemEvent {
            ts_ms,
            kind: SystemEventKind::BookRecoveryStarted,
            venue: Some(stream_id.venue.into()),
            account_id: None,
            symbol: Some(stream_id.symbol.clone()),
            reason: "request fresh snapshots from every conflicting source".to_string(),
        }),
    ];
    outputs.extend(sources.into_iter().map(|source_conn_id| {
        FeedOutput::RecoveryRequired(RecoveryRequest {
            stream: stream_id.clone(),
            source_conn_id: Some(source_conn_id),
            expected_prev: state.published_version.map(|current| current.seq_id),
            received_prev: version.prev_seq_id,
            received_seq: version.seq_id,
        })
    }));
    outputs
}

#[derive(Debug)]
struct BookStream {
    sources: HashMap<ConnId, SourceBookStream>,
    published: BookReducer,
    published_version: Option<BookVersion>,
    aggregate_recovering: bool,
}

impl BookStream {
    fn new(symbol: Symbol) -> Self {
        Self {
            sources: HashMap::new(),
            published: BookReducer::new(symbol),
            published_version: None,
            aggregate_recovering: false,
        }
    }

    fn is_ready(&self) -> bool {
        !self.aggregate_recovering
            && self.published.is_ready()
            && self.sources.values().any(SourceBookStream::is_ready)
    }

    fn sequence_status(&self) -> SequenceStatus {
        if self.sources.is_empty() {
            SequenceStatus::Empty
        } else if self.is_ready() {
            SequenceStatus::Ready
        } else {
            SequenceStatus::Recovering
        }
    }

    fn publish_candidate(
        &mut self,
        candidate_version: BookVersion,
        candidate: OrderBook,
    ) -> PublishDecision {
        if self.aggregate_recovering || self.published_version.is_none() {
            self.replace_published(candidate_version, candidate);
            return PublishDecision::Published;
        }
        let current_version = self
            .published_version
            .expect("a published version was checked above");
        if candidate_version.ts_ms < current_version.ts_ms {
            return PublishDecision::Ignored;
        }
        if candidate_version.ts_ms > current_version.ts_ms {
            self.replace_published(candidate_version, candidate);
            return PublishDecision::Published;
        }
        if candidate_version.seq_id == current_version.seq_id {
            return if self.published.book() == Some(&candidate) {
                PublishDecision::Ignored
            } else {
                PublishDecision::Conflict
            };
        }
        if candidate_version.prev_seq_id == current_version.seq_id {
            self.replace_published(candidate_version, candidate);
            return PublishDecision::Published;
        }
        if current_version.prev_seq_id == candidate_version.seq_id {
            return PublishDecision::Ignored;
        }
        PublishDecision::Conflict
    }

    fn replace_published(&mut self, version: BookVersion, book: OrderBook) {
        self.published.apply_snapshot(book);
        self.published_version = Some(version);
        self.aggregate_recovering = false;
    }
}

#[derive(Debug)]
struct SourceBookStream {
    sequence: SequenceTracker,
    book: BookReducer,
}

impl SourceBookStream {
    fn new(symbol: Symbol, max_sequence_buffer: usize) -> Self {
        Self {
            sequence: SequenceTracker::new(max_sequence_buffer),
            book: BookReducer::new(symbol),
        }
    }

    fn is_ready(&self) -> bool {
        self.sequence.status() == SequenceStatus::Ready && self.book.is_ready()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BookVersion {
    ts_ms: TimeMs,
    prev_seq_id: i64,
    seq_id: i64,
}

impl From<&SequencedBookUpdate> for BookVersion {
    fn from(update: &SequencedBookUpdate) -> Self {
        Self {
            ts_ms: update.ts_ms,
            prev_seq_id: update.prev_seq_id,
            seq_id: update.seq_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublishDecision {
    Published,
    Ignored,
    Conflict,
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
        parsed_at(action, prev, seq, seq.max(0) as u64, bid)
    }

    fn parsed_at(action: BookAction, prev: i64, seq: i64, ts_ms: TimeMs, bid: f64) -> ParsedEvent {
        let update = SequencedBookUpdate {
            action,
            symbol: "BTC-USDT".to_string(),
            ts_ms,
            prev_seq_id: prev,
            seq_id: seq,
            bids: vec![Level::new(100.0, bid)],
            asks: vec![Level::new(101.0, 1.0)],
        };
        ParsedEvent {
            id: EventId {
                venue: OkxVenue,
                channel: Channel::Books,
                symbol: Some("BTC-USDT".to_string()),
                key: EventKey::BookSequence {
                    action,
                    prev_seq_id: prev,
                    seq_id: seq,
                    ts_ms,
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
                venue: OkxVenue,
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
        processor.process(parsed_at(BookAction::Snapshot, -1, 10, 100, 1.0));
        processor.process(parsed_at(BookAction::Update, 10, 15, 101, 1.1));

        let reset = processor.process(parsed_at(BookAction::Update, 15, 3, 102, 1.2));
        let heartbeat = processor.process(parsed_at(BookAction::Update, 3, 3, 103, 1.2));
        let following = processor.process(parsed_at(BookAction::Update, 3, 5, 104, 1.3));

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
    fn conflicting_payload_on_one_source_forces_recovery() {
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
    fn conflicting_snapshot_on_one_source_forces_recovery() {
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
    fn conflicting_buffered_payloads_on_one_source_cannot_be_collapsed() {
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

    #[test]
    fn equivalent_redundant_startup_race_does_not_create_a_false_gap() {
        let mut processor = FeedProcessor::new(16, 16);
        let first = ConnId::new("book-r0");
        let second = ConnId::new("book-r1");

        processor.process_from(&first, parsed_at(BookAction::Snapshot, -1, 10, 100, 1.0));
        let leading_snapshot =
            processor.process_from(&second, parsed_at(BookAction::Snapshot, -1, 12, 200, 2.0));
        let lagging_delta =
            processor.process_from(&first, parsed_at(BookAction::Update, 10, 12, 200, 2.0));

        assert!(
            leading_snapshot
                .iter()
                .any(|output| matches!(output, FeedOutput::Event(_)))
        );
        assert!(
            !lagging_delta
                .iter()
                .any(|output| matches!(output, FeedOutput::RecoveryRequired(_)))
        );
        assert_eq!(processor.stats().gaps, 0);
        assert_eq!(processor.stats().recoveries, 0);
        let health = processor.stream_health();
        assert_eq!(health[0].sequence_status, SequenceStatus::Ready);
        assert_eq!(health[0].last_seq_id, Some(12));
        assert_eq!(processor.ready_books()[0].bids[0].qty, 2.0);
    }

    #[test]
    fn equivalent_global_duplicate_initializes_each_source_sequence() {
        let mut processor = FeedProcessor::new(16, 16);
        let first = ConnId::new("book-r0");
        let second = ConnId::new("book-r1");
        let snapshot = parsed_at(BookAction::Snapshot, -1, 10, 100, 1.0);

        processor.process_from(&first, snapshot.clone());
        let duplicate = processor.process_from(&second, snapshot);
        let update =
            processor.process_from(&second, parsed_at(BookAction::Update, 10, 11, 101, 1.5));

        assert!(
            duplicate
                .iter()
                .any(|output| matches!(output, FeedOutput::Duplicate(_)))
        );
        assert!(
            update
                .iter()
                .any(|output| matches!(output, FeedOutput::Event(_)))
        );
        assert!(
            !update
                .iter()
                .any(|output| matches!(output, FeedOutput::RecoveryRequired(_)))
        );
        assert_eq!(processor.stats().duplicates, 1);
        assert_eq!(processor.stats().gaps, 0);
        assert_eq!(processor.stream_health()[0].last_seq_id, Some(11));
    }

    #[test]
    fn one_source_gap_requests_scoped_recovery_without_degrading_canonical_book() {
        let mut processor = FeedProcessor::new(16, 16);
        let first = ConnId::new("book-r0");
        let second = ConnId::new("book-r1");
        let snapshot = parsed_at(BookAction::Snapshot, -1, 10, 100, 1.0);
        processor.process_from(&first, snapshot.clone());
        processor.process_from(&second, snapshot);

        let outputs =
            processor.process_from(&first, parsed_at(BookAction::Update, 11, 12, 102, 2.0));

        let requests = outputs
            .iter()
            .filter_map(|output| match output {
                FeedOutput::RecoveryRequired(request) => Some(request),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].source_conn_id.as_ref(), Some(&first));
        assert!(!outputs.iter().any(|output| matches!(
            output,
            FeedOutput::System(SystemEvent {
                kind: SystemEventKind::FeedGap,
                ..
            })
        )));
        assert_eq!(processor.stats().gaps, 1);
        assert_eq!(
            processor.stream_health()[0].sequence_status,
            SequenceStatus::Ready
        );
    }

    #[test]
    fn healthy_replica_continues_publishing_while_other_source_recovers() {
        let mut processor = FeedProcessor::new(16, 16);
        let first = ConnId::new("book-r0");
        let second = ConnId::new("book-r1");
        let snapshot = parsed_at(BookAction::Snapshot, -1, 10, 100, 1.0);
        processor.process_from(&first, snapshot.clone());
        processor.process_from(&second, snapshot);
        processor.process_from(&first, parsed_at(BookAction::Update, 11, 12, 102, 2.0));

        let outputs =
            processor.process_from(&second, parsed_at(BookAction::Update, 10, 11, 103, 1.5));

        assert!(
            outputs
                .iter()
                .any(|output| matches!(output, FeedOutput::Event(_)))
        );
        assert!(
            !outputs
                .iter()
                .any(|output| matches!(output, FeedOutput::RecoveryRequired(_)))
        );
        assert_eq!(processor.stream_health()[0].last_seq_id, Some(11));
        assert_eq!(processor.stream_health()[0].book_status, BookStatus::Ready);
    }

    #[test]
    fn conflicting_redundant_snapshots_fail_closed_and_recover_every_source() {
        let mut processor = FeedProcessor::new(16, 16);
        let first = ConnId::new("book-r0");
        let second = ConnId::new("book-r1");
        processor.process_from(&first, parsed_at(BookAction::Snapshot, -1, 10, 100, 1.0));

        let outputs =
            processor.process_from(&second, parsed_at(BookAction::Snapshot, -1, 10, 100, 2.0));

        let mut recovered_sources = outputs
            .iter()
            .filter_map(|output| match output {
                FeedOutput::RecoveryRequired(request) => request.source_conn_id.clone(),
                _ => None,
            })
            .collect::<Vec<_>>();
        recovered_sources.sort_by(|left, right| left.0.cmp(&right.0));
        assert_eq!(recovered_sources, vec![first, second]);
        assert!(
            !outputs
                .iter()
                .any(|output| matches!(output, FeedOutput::Event(_)))
        );
        assert_eq!(processor.stats().gaps, 1);
        assert_eq!(
            processor.stream_health()[0].sequence_status,
            SequenceStatus::Recovering
        );
        assert!(processor.ready_books().is_empty());
    }

    #[test]
    fn fresh_unchanged_snapshot_recovers_a_stale_canonical_book() {
        let mut processor = FeedProcessor::new(16, 16);
        let source = ConnId::new("book-r0");
        let snapshot = parsed_at(BookAction::Snapshot, -1, 10, 100, 1.0);
        processor.process_from(&source, snapshot.clone());
        assert_eq!(processor.mark_stale(200, 50).len(), 1);

        let outputs = processor.process_from(&source, snapshot);

        assert!(
            outputs
                .iter()
                .any(|output| matches!(output, FeedOutput::Event(_)))
        );
        assert_eq!(processor.stream_health()[0].book_status, BookStatus::Ready);
    }
}
