use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use reap_core::{Channel, MarketEvent, NormalizedEvent, OrderBook};
use reap_feed::{FeedOutput, FeedProcessor, RawCapture};
use reap_venue::{VenueAdapter, VenueEvent, okx::OkxAdapter};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{CaptureConfig, CaptureError, digest_hex, is_book_channel};

const ANALYSIS_FORMAT_VERSION: u16 = 3;
const DISTRIBUTION_SAMPLE_CAPACITY: usize = 8_192;
const MAX_REPORTED_ERRORS: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureDistribution {
    pub count: u64,
    pub sample_count: usize,
    pub min: Option<f64>,
    pub mean: Option<f64>,
    pub p50: Option<f64>,
    pub p95: Option<f64>,
    pub p99: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureBookMetrics {
    pub samples: u64,
    pub spread_bps: CaptureDistribution,
    pub absolute_mid_move_bps: CaptureDistribution,
    pub bid_levels: CaptureDistribution,
    pub ask_levels: CaptureDistribution,
    pub best_bid_qty: CaptureDistribution,
    pub best_ask_qty: CaptureDistribution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureTradeMetrics {
    pub samples: u64,
    pub quantity: CaptureDistribution,
    pub price_times_quantity: CaptureDistribution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureStreamAnalysis {
    pub channel: String,
    pub symbol: String,
    pub raw_frames: u64,
    pub data_frames: u64,
    pub source_connections: Vec<String>,
    pub accepted_events: u64,
    pub duplicate_events: u64,
    pub normalized_events: u64,
    pub first_recv_ts_ns: Option<u64>,
    pub last_recv_ts_ns: Option<u64>,
    pub first_exchange_ts_ms: Option<u64>,
    pub last_exchange_ts_ms: Option<u64>,
    pub receive_timestamp_regressions: u64,
    pub exchange_timestamp_regressions: u64,
    pub negative_receive_delay_count: u64,
    pub receive_delay_ms: CaptureDistribution,
    pub receive_interval_ms: CaptureDistribution,
    pub exchange_interval_ms: CaptureDistribution,
    pub book: Option<CaptureBookMetrics>,
    pub trade: Option<CaptureTradeMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureStreamCoverage {
    pub channel: String,
    pub symbol: String,
    pub expected_connections: usize,
    pub observed_connections: usize,
    pub raw_frames: u64,
    pub data_frames: u64,
    pub accepted_events: u64,
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureStreamIdentity {
    pub channel: String,
    pub symbol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureAnalysisBookHealth {
    pub channel: String,
    pub symbol: String,
    pub sequence_status: String,
    pub book_status: String,
    pub last_seq_id: Option<i64>,
    pub buffered_updates: usize,
    pub sequence_resets: u64,
    pub same_sequence_updates: u64,
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureAnalysisError {
    pub line: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureAnalysisReport {
    pub format_version: u16,
    pub source_path: Option<PathBuf>,
    pub config_fingerprint: String,
    pub sha256: String,
    pub bytes: u64,
    pub lines: u64,
    pub ignored_lines: u64,
    pub capture_sessions: Vec<String>,
    pub first_recv_ts_ns: Option<u64>,
    pub last_recv_ts_ns: Option<u64>,
    pub duration_ms: Option<f64>,
    pub parsed_events: u64,
    pub accepted_events: u64,
    pub duplicate_events: u64,
    pub normalized_events: u64,
    pub reconstructed_normalized_records: u64,
    pub reconstructed_normalized_bytes: u64,
    pub reconstructed_normalized_sha256: String,
    pub gaps: u64,
    pub recoveries: u64,
    pub recovery_failures: u64,
    pub sequence_resets: u64,
    pub same_sequence_updates: u64,
    pub receive_timestamp_regressions: u64,
    pub exchange_timestamp_regressions: u64,
    pub unrecovered_book_streams: usize,
    pub error_count: u64,
    pub errors: Vec<CaptureAnalysisError>,
    pub expected_streams: Vec<CaptureStreamCoverage>,
    pub unexpected_data_streams: Vec<CaptureStreamIdentity>,
    pub streams: Vec<CaptureStreamAnalysis>,
    pub books: Vec<CaptureAnalysisBookHealth>,
    pub integrity_healthy: bool,
}

pub fn analyze_capture_path(
    path: impl AsRef<Path>,
    config: &CaptureConfig,
) -> Result<CaptureAnalysisReport, CaptureError> {
    let path = path.as_ref();
    let file = File::open(path)?;
    let mut effective_config = config.clone();
    effective_config.output.raw_path = path.to_path_buf();
    let mut report = analyze_capture(file, &effective_config)?;
    report.source_path = Some(path.to_path_buf());
    Ok(report)
}

pub fn analyze_capture<R: Read>(
    reader: R,
    config: &CaptureConfig,
) -> Result<CaptureAnalysisReport, CaptureError> {
    config.ensure_valid()?;
    let mut analyzer = CaptureAnalyzer::new(config)?;
    let mut reader = BufReader::new(reader);
    let mut buffer = Vec::new();
    let mut physical_line = 0_usize;

    loop {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer)?;
        if read == 0 {
            break;
        }
        physical_line = physical_line.saturating_add(1);
        analyzer.bytes = analyzer.bytes.saturating_add(read as u64);
        analyzer.hasher.update(&buffer);
        let line = trim_ascii_whitespace(&buffer);
        if line.is_empty() || line.first() == Some(&b'#') {
            analyzer.ignored_lines = analyzer.ignored_lines.saturating_add(1);
            continue;
        }
        analyzer.lines = analyzer.lines.saturating_add(1);
        match serde_json::from_slice::<RawCapture>(line) {
            Ok(capture) => analyzer.on_capture(physical_line, capture),
            Err(error) => analyzer.record_error(physical_line, error.to_string()),
        }
    }

    Ok(analyzer.finish())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct StreamKey {
    channel: String,
    symbol: String,
}

struct CaptureAnalyzer<'a> {
    config: &'a CaptureConfig,
    config_fingerprint: String,
    adapter: OkxAdapter,
    processor: FeedProcessor,
    streams: BTreeMap<StreamKey, StreamAccumulator>,
    sessions: BTreeSet<String>,
    first_recv_ts_ns: Option<u64>,
    last_recv_ts_ns: Option<u64>,
    lines: u64,
    ignored_lines: u64,
    bytes: u64,
    hasher: Sha256,
    reconstructed_normalized_records: u64,
    reconstructed_normalized_bytes: u64,
    reconstructed_normalized_hasher: Sha256,
    error_count: u64,
    errors: Vec<CaptureAnalysisError>,
}

impl<'a> CaptureAnalyzer<'a> {
    fn new(config: &'a CaptureConfig) -> Result<Self, CaptureError> {
        let streams = config
            .subscriptions
            .iter()
            .map(|subscription| {
                (
                    StreamKey {
                        channel: subscription.channel.trim().to_string(),
                        symbol: subscription.symbol.trim().to_string(),
                    },
                    StreamAccumulator::default(),
                )
            })
            .collect();
        Ok(Self {
            config,
            config_fingerprint: config.fingerprint()?,
            adapter: OkxAdapter::new(&config.venue.public_ws_url, &config.venue.public_ws_url),
            processor: FeedProcessor::new(
                config.runtime.dedup_capacity_per_stream,
                config.runtime.max_sequence_buffer,
            ),
            streams,
            sessions: BTreeSet::new(),
            first_recv_ts_ns: None,
            last_recv_ts_ns: None,
            lines: 0,
            ignored_lines: 0,
            bytes: 0,
            hasher: Sha256::new(),
            reconstructed_normalized_records: 0,
            reconstructed_normalized_bytes: 0,
            reconstructed_normalized_hasher: Sha256::new(),
            error_count: 0,
            errors: Vec::new(),
        })
    }

    fn on_capture(&mut self, line: usize, capture: RawCapture) {
        let session = capture
            .capture_session_id
            .as_deref()
            .unwrap_or("legacy:unspecified")
            .to_string();
        if self.sessions.insert(session) && self.sessions.len() > 1 {
            self.record_error(line, "capture contains more than one process session");
        }

        let recv_ts_ns = capture.recv_ts_ns;
        self.first_recv_ts_ns = Some(
            self.first_recv_ts_ns
                .map_or(recv_ts_ns, |current| current.min(recv_ts_ns)),
        );
        self.last_recv_ts_ns = Some(
            self.last_recv_ts_ns
                .map_or(recv_ts_ns, |current| current.max(recv_ts_ns)),
        );

        let stream = capture_stream_key(&capture);
        let data_frame = is_data_frame(&capture);
        if let Some(key) = &stream {
            self.streams.entry(key.clone()).or_default().on_frame(
                &capture.conn_id.0,
                recv_ts_ns,
                data_frame,
            );
        } else if data_frame {
            self.record_error(line, "data frame has no channel/symbol stream identity");
        }

        let envelope = match capture.into_envelope() {
            Ok(envelope) => envelope,
            Err(error) => {
                self.record_error(line, error.to_string());
                return;
            }
        };
        let parsed_events = match self.adapter.parse(&envelope) {
            Ok(parsed_events) => parsed_events,
            Err(error) => {
                self.record_error(line, error.to_string());
                return;
            }
        };

        for parsed in parsed_events {
            let key = stream.clone().or_else(|| parsed_stream_key(&parsed));
            let exchange_ts_ms = venue_event_ts_ms(&parsed.event);
            if let Some(key) = &key {
                self.streams
                    .entry(key.clone())
                    .or_default()
                    .on_wire_event(recv_ts_ns, exchange_ts_ms);
            }

            let before = self.processor.stats().clone();
            let outputs = self.processor.process(parsed);
            let after = self.processor.stats().clone();
            if let Some(key) = &key {
                let stream = self.streams.entry(key.clone()).or_default();
                let accepted = after.accepted.saturating_sub(before.accepted);
                let duplicates = after.duplicates.saturating_sub(before.duplicates);
                stream.accepted_events = stream.accepted_events.saturating_add(accepted);
                stream.duplicate_events = stream.duplicate_events.saturating_add(duplicates);
            }
            for output in outputs {
                match output {
                    FeedOutput::Event(event) => {
                        self.record_reconstructed_normalized(line, &event);
                        if let Some(key) = &key {
                            let stream = self.streams.entry(key.clone()).or_default();
                            stream.normalized_events = stream.normalized_events.saturating_add(1);
                            stream.on_canonical_event(event.ts_ms());
                            stream.on_normalized_event(&event);
                        }
                    }
                    FeedOutput::System(event) => {
                        self.record_reconstructed_normalized(line, &NormalizedEvent::System(event))
                    }
                    FeedOutput::Duplicate(_)
                    | FeedOutput::RecoveryRequired(_)
                    | FeedOutput::PrivateOrder { .. }
                    | FeedOutput::PrivateFill { .. }
                    | FeedOutput::PrivateAccount { .. } => {}
                }
            }
        }
    }

    fn record_reconstructed_normalized(&mut self, line: usize, event: &NormalizedEvent) {
        match serde_json::to_vec(event) {
            Ok(mut bytes) => {
                bytes.push(b'\n');
                self.reconstructed_normalized_records =
                    self.reconstructed_normalized_records.saturating_add(1);
                self.reconstructed_normalized_bytes = self
                    .reconstructed_normalized_bytes
                    .saturating_add(bytes.len() as u64);
                self.reconstructed_normalized_hasher.update(bytes);
            }
            Err(error) => self.record_error(
                line,
                format!("failed to reconstruct normalized capture output: {error}"),
            ),
        }
    }

    fn record_error(&mut self, line: usize, message: impl Into<String>) {
        self.error_count = self.error_count.saturating_add(1);
        if self.errors.len() < MAX_REPORTED_ERRORS {
            self.errors.push(CaptureAnalysisError {
                line,
                message: message.into(),
            });
        }
    }

    fn finish(self) -> CaptureAnalysisReport {
        let processor = self.processor.stats().clone();
        let health_by_symbol = self
            .processor
            .stream_health()
            .into_iter()
            .map(|health| (health.stream.symbol.clone(), health))
            .collect::<HashMap<_, _>>();

        let expected_streams = self
            .config
            .subscriptions
            .iter()
            .map(|expected| {
                let key = StreamKey {
                    channel: expected.channel.trim().to_string(),
                    symbol: expected.symbol.trim().to_string(),
                };
                let stream = self.streams.get(&key);
                let observed_connections = stream.map_or(0, |stream| stream.sources.len());
                let raw_frames = stream.map_or(0, |stream| stream.raw_frames);
                let data_frames = stream.map_or(0, |stream| stream.data_frames);
                let accepted_events = stream.map_or(0, |stream| stream.accepted_events);
                CaptureStreamCoverage {
                    channel: key.channel,
                    symbol: key.symbol,
                    expected_connections: expected.connections,
                    observed_connections,
                    raw_frames,
                    data_frames,
                    accepted_events,
                    complete: observed_connections == expected.connections
                        && data_frames > 0
                        && accepted_events > 0,
                }
            })
            .collect::<Vec<_>>();

        let expected_keys = self
            .config
            .subscriptions
            .iter()
            .map(|subscription| StreamKey {
                channel: subscription.channel.trim().to_string(),
                symbol: subscription.symbol.trim().to_string(),
            })
            .collect::<BTreeSet<_>>();
        let unexpected_data_streams = self
            .streams
            .iter()
            .filter(|(key, stream)| stream.data_frames > 0 && !expected_keys.contains(*key))
            .map(|(key, _)| CaptureStreamIdentity {
                channel: key.channel.clone(),
                symbol: key.symbol.clone(),
            })
            .collect::<Vec<_>>();

        let books = self
            .config
            .subscriptions
            .iter()
            .filter(|subscription| is_book_channel(subscription.channel.trim()))
            .map(|expected| {
                let channel = expected.channel.trim().to_string();
                let symbol = expected.symbol.trim().to_string();
                match health_by_symbol.get(&symbol) {
                    Some(health) => CaptureAnalysisBookHealth {
                        channel,
                        symbol,
                        sequence_status: lower_debug(health.sequence_status),
                        book_status: lower_debug(health.book_status),
                        last_seq_id: health.last_seq_id,
                        buffered_updates: health.buffered_updates,
                        sequence_resets: health.sequence_resets,
                        same_sequence_updates: health.same_sequence_updates,
                        best_bid: health.best_bid,
                        best_ask: health.best_ask,
                    },
                    None => CaptureAnalysisBookHealth {
                        channel,
                        symbol,
                        sequence_status: "awaiting_snapshot".to_string(),
                        book_status: "empty".to_string(),
                        last_seq_id: None,
                        buffered_updates: 0,
                        sequence_resets: 0,
                        same_sequence_updates: 0,
                        best_bid: None,
                        best_ask: None,
                    },
                }
            })
            .collect::<Vec<_>>();
        let unrecovered_book_streams = books
            .iter()
            .filter(|book| book.sequence_status != "ready" || book.book_status != "ready")
            .count();
        let coverage_complete = expected_streams.iter().all(|stream| stream.complete);
        let receive_timestamp_regressions = self
            .streams
            .values()
            .map(|stream| stream.receive_timestamp_regressions)
            .sum();
        let exchange_timestamp_regressions = self
            .streams
            .values()
            .map(|stream| stream.exchange_timestamp_regressions)
            .sum();
        let sessions = self.sessions.into_iter().collect::<Vec<_>>();
        let duration_ms = match (self.first_recv_ts_ns, self.last_recv_ts_ns) {
            (Some(first), Some(last)) => Some(last.saturating_sub(first) as f64 / 1_000_000.0),
            _ => None,
        };
        let integrity_healthy = self.lines > 0
            && self.ignored_lines == 0
            && sessions.len() == 1
            && self.error_count == 0
            && processor.gaps == 0
            && processor.recovery_failures == 0
            && receive_timestamp_regressions == 0
            && unrecovered_book_streams == 0
            && coverage_complete
            && unexpected_data_streams.is_empty();
        let streams = self
            .streams
            .into_iter()
            .map(|(key, stream)| stream.finish(key))
            .collect();

        CaptureAnalysisReport {
            format_version: ANALYSIS_FORMAT_VERSION,
            source_path: None,
            config_fingerprint: self.config_fingerprint,
            sha256: digest_hex(self.hasher.finalize()),
            bytes: self.bytes,
            lines: self.lines,
            ignored_lines: self.ignored_lines,
            capture_sessions: sessions,
            first_recv_ts_ns: self.first_recv_ts_ns,
            last_recv_ts_ns: self.last_recv_ts_ns,
            duration_ms,
            parsed_events: processor.parsed,
            accepted_events: processor.accepted,
            duplicate_events: processor.duplicates,
            normalized_events: processor.normalized_events,
            reconstructed_normalized_records: self.reconstructed_normalized_records,
            reconstructed_normalized_bytes: self.reconstructed_normalized_bytes,
            reconstructed_normalized_sha256: digest_hex(
                self.reconstructed_normalized_hasher.finalize(),
            ),
            gaps: processor.gaps,
            recoveries: processor.recoveries,
            recovery_failures: processor.recovery_failures,
            sequence_resets: processor.sequence_resets,
            same_sequence_updates: processor.same_sequence_updates,
            receive_timestamp_regressions,
            exchange_timestamp_regressions,
            unrecovered_book_streams,
            error_count: self.error_count,
            errors: self.errors,
            expected_streams,
            unexpected_data_streams,
            streams,
            books,
            integrity_healthy,
        }
    }
}

#[derive(Default)]
struct StreamAccumulator {
    raw_frames: u64,
    data_frames: u64,
    sources: BTreeSet<String>,
    accepted_events: u64,
    duplicate_events: u64,
    normalized_events: u64,
    first_recv_ts_ns: Option<u64>,
    last_recv_ts_ns: Option<u64>,
    last_recv_by_connection: HashMap<String, u64>,
    first_exchange_ts_ms: Option<u64>,
    last_exchange_ts_ms: Option<u64>,
    previous_exchange_ts_ms: Option<u64>,
    receive_timestamp_regressions: u64,
    exchange_timestamp_regressions: u64,
    negative_receive_delay_count: u64,
    receive_delay_ms: DistributionAccumulator,
    receive_interval_ms: DistributionAccumulator,
    exchange_interval_ms: DistributionAccumulator,
    book_samples: u64,
    previous_mid: Option<f64>,
    spread_bps: DistributionAccumulator,
    absolute_mid_move_bps: DistributionAccumulator,
    bid_levels: DistributionAccumulator,
    ask_levels: DistributionAccumulator,
    best_bid_qty: DistributionAccumulator,
    best_ask_qty: DistributionAccumulator,
    trade_samples: u64,
    trade_quantity: DistributionAccumulator,
    trade_price_times_quantity: DistributionAccumulator,
}

impl StreamAccumulator {
    fn on_frame(&mut self, connection: &str, recv_ts_ns: u64, data_frame: bool) {
        self.raw_frames = self.raw_frames.saturating_add(1);
        if !data_frame {
            return;
        }
        self.data_frames = self.data_frames.saturating_add(1);
        self.sources.insert(connection.to_string());
        self.first_recv_ts_ns = Some(
            self.first_recv_ts_ns
                .map_or(recv_ts_ns, |current| current.min(recv_ts_ns)),
        );
        self.last_recv_ts_ns = Some(
            self.last_recv_ts_ns
                .map_or(recv_ts_ns, |current| current.max(recv_ts_ns)),
        );
        if let Some(previous) = self
            .last_recv_by_connection
            .insert(connection.to_string(), recv_ts_ns)
        {
            if recv_ts_ns < previous {
                self.receive_timestamp_regressions =
                    self.receive_timestamp_regressions.saturating_add(1);
            } else {
                self.receive_interval_ms
                    .record(recv_ts_ns.saturating_sub(previous) as f64 / 1_000_000.0);
            }
        }
    }

    fn on_wire_event(&mut self, recv_ts_ns: u64, exchange_ts_ms: u64) {
        if exchange_ts_ms == 0 {
            return;
        }
        let delay_ms = recv_ts_ns as f64 / 1_000_000.0 - exchange_ts_ms as f64;
        if delay_ms < 0.0 {
            self.negative_receive_delay_count = self.negative_receive_delay_count.saturating_add(1);
        }
        self.receive_delay_ms.record(delay_ms);
    }

    fn on_canonical_event(&mut self, exchange_ts_ms: u64) {
        if exchange_ts_ms == 0 {
            return;
        }
        self.first_exchange_ts_ms = Some(
            self.first_exchange_ts_ms
                .map_or(exchange_ts_ms, |current| current.min(exchange_ts_ms)),
        );
        self.last_exchange_ts_ms = Some(
            self.last_exchange_ts_ms
                .map_or(exchange_ts_ms, |current| current.max(exchange_ts_ms)),
        );
        if let Some(previous) = self.previous_exchange_ts_ms.replace(exchange_ts_ms) {
            if exchange_ts_ms < previous {
                self.exchange_timestamp_regressions =
                    self.exchange_timestamp_regressions.saturating_add(1);
            } else {
                self.exchange_interval_ms
                    .record(exchange_ts_ms.saturating_sub(previous) as f64);
            }
        }
    }

    fn on_normalized_event(&mut self, event: &NormalizedEvent) {
        let NormalizedEvent::Market(event) = event else {
            return;
        };
        match event {
            MarketEvent::Depth(book) => self.on_book(book),
            MarketEvent::Trade { price, qty, .. } => {
                self.trade_samples = self.trade_samples.saturating_add(1);
                self.trade_quantity.record(*qty);
                self.trade_price_times_quantity.record(*price * *qty);
            }
            MarketEvent::IndexPrice { .. }
            | MarketEvent::FundingRate { .. }
            | MarketEvent::BurstSignal { .. }
            | MarketEvent::PriceLimits { .. } => {}
        }
    }

    fn on_book(&mut self, book: &OrderBook) {
        self.book_samples = self.book_samples.saturating_add(1);
        self.bid_levels.record(book.bids.len() as f64);
        self.ask_levels.record(book.asks.len() as f64);
        if let Some(best_bid) = book.best_bid() {
            self.best_bid_qty.record(best_bid.qty);
        }
        if let Some(best_ask) = book.best_ask() {
            self.best_ask_qty.record(best_ask.qty);
        }
        if let Some(mid) = book.mid().filter(|mid| mid.is_finite() && *mid > 0.0) {
            if let (Some(best_bid), Some(best_ask)) = (book.best_bid(), book.best_ask()) {
                self.spread_bps
                    .record((best_ask.px - best_bid.px) / mid * 10_000.0);
            }
            if let Some(previous) = self.previous_mid.filter(|previous| *previous > 0.0) {
                self.absolute_mid_move_bps
                    .record((mid - previous).abs() / previous * 10_000.0);
            }
            self.previous_mid = Some(mid);
        }
    }

    fn finish(self, key: StreamKey) -> CaptureStreamAnalysis {
        let book = is_book_channel(&key.channel).then(|| CaptureBookMetrics {
            samples: self.book_samples,
            spread_bps: self.spread_bps.finish(),
            absolute_mid_move_bps: self.absolute_mid_move_bps.finish(),
            bid_levels: self.bid_levels.finish(),
            ask_levels: self.ask_levels.finish(),
            best_bid_qty: self.best_bid_qty.finish(),
            best_ask_qty: self.best_ask_qty.finish(),
        });
        let trade =
            matches!(key.channel.as_str(), "trades" | "trades-all").then(|| CaptureTradeMetrics {
                samples: self.trade_samples,
                quantity: self.trade_quantity.finish(),
                price_times_quantity: self.trade_price_times_quantity.finish(),
            });
        CaptureStreamAnalysis {
            channel: key.channel,
            symbol: key.symbol,
            raw_frames: self.raw_frames,
            data_frames: self.data_frames,
            source_connections: self.sources.into_iter().collect(),
            accepted_events: self.accepted_events,
            duplicate_events: self.duplicate_events,
            normalized_events: self.normalized_events,
            first_recv_ts_ns: self.first_recv_ts_ns,
            last_recv_ts_ns: self.last_recv_ts_ns,
            first_exchange_ts_ms: self.first_exchange_ts_ms,
            last_exchange_ts_ms: self.last_exchange_ts_ms,
            receive_timestamp_regressions: self.receive_timestamp_regressions,
            exchange_timestamp_regressions: self.exchange_timestamp_regressions,
            negative_receive_delay_count: self.negative_receive_delay_count,
            receive_delay_ms: self.receive_delay_ms.finish(),
            receive_interval_ms: self.receive_interval_ms.finish(),
            exchange_interval_ms: self.exchange_interval_ms.finish(),
            book,
            trade,
        }
    }
}

struct DistributionAccumulator {
    count: u64,
    sum: f64,
    min: Option<f64>,
    max: Option<f64>,
    samples: Vec<f64>,
    random_state: u64,
}

impl Default for DistributionAccumulator {
    fn default() -> Self {
        Self {
            count: 0,
            sum: 0.0,
            min: None,
            max: None,
            samples: Vec::new(),
            random_state: 0x9e37_79b9_7f4a_7c15,
        }
    }
}

impl DistributionAccumulator {
    fn record(&mut self, value: f64) {
        if !value.is_finite() {
            return;
        }
        self.count = self.count.saturating_add(1);
        self.sum += value;
        self.min = Some(self.min.map_or(value, |current| current.min(value)));
        self.max = Some(self.max.map_or(value, |current| current.max(value)));
        if self.samples.len() < DISTRIBUTION_SAMPLE_CAPACITY {
            self.samples.push(value);
            return;
        }
        let candidate = self.next_random() % self.count;
        if candidate < DISTRIBUTION_SAMPLE_CAPACITY as u64 {
            self.samples[candidate as usize] = value;
        }
    }

    fn next_random(&mut self) -> u64 {
        self.random_state = self.random_state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.random_state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn finish(mut self) -> CaptureDistribution {
        self.samples.sort_by(f64::total_cmp);
        CaptureDistribution {
            count: self.count,
            sample_count: self.samples.len(),
            min: self.min,
            mean: (self.count > 0).then(|| self.sum / self.count as f64),
            p50: quantile(&self.samples, 0.50),
            p95: quantile(&self.samples, 0.95),
            p99: quantile(&self.samples, 0.99),
            max: self.max,
        }
    }
}

fn quantile(sorted: &[f64], probability: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let index = ((sorted.len() - 1) as f64 * probability).round() as usize;
    sorted.get(index).copied()
}

fn capture_stream_key(capture: &RawCapture) -> Option<StreamKey> {
    let arg = capture.payload.get("arg").and_then(|arg| arg.as_object());
    let channel = arg
        .and_then(|arg| arg.get("channel"))
        .and_then(|channel| channel.as_str())
        .map(str::to_string)
        .or_else(|| channel_name(&capture.channel));
    let symbol = arg
        .and_then(|arg| arg.get("instId"))
        .and_then(|symbol| symbol.as_str())
        .filter(|symbol| !symbol.is_empty())
        .map(str::to_string)
        .or_else(|| capture.symbol.clone())
        .or_else(|| payload_symbol(&capture.payload));
    Some(StreamKey {
        channel: channel?,
        symbol: symbol?,
    })
}

fn parsed_stream_key(parsed: &reap_venue::ParsedEvent) -> Option<StreamKey> {
    Some(StreamKey {
        channel: channel_name(&parsed.id.channel)?,
        symbol: parsed.id.symbol.clone()?,
    })
}

fn channel_name(channel: &Channel) -> Option<String> {
    Some(match channel {
        Channel::Books => "books".to_string(),
        Channel::Trades => "trades".to_string(),
        Channel::Orders => "orders".to_string(),
        Channel::Fills => "fills".to_string(),
        Channel::Account => "account".to_string(),
        Channel::Positions => "positions".to_string(),
        Channel::Custom(channel) if !channel.is_empty() => channel.clone(),
        Channel::Custom(_) => return None,
    })
}

fn payload_symbol(payload: &serde_json::Value) -> Option<String> {
    payload
        .get("data")?
        .as_array()?
        .first()?
        .get("instId")?
        .as_str()
        .filter(|symbol| !symbol.is_empty())
        .map(str::to_string)
}

fn is_data_frame(capture: &RawCapture) -> bool {
    capture.payload.get("event").is_none()
        && capture
            .payload
            .get("arg")
            .is_some_and(serde_json::Value::is_object)
        && capture
            .payload
            .get("data")
            .is_some_and(serde_json::Value::is_array)
}

fn venue_event_ts_ms(event: &VenueEvent) -> u64 {
    match event {
        VenueEvent::Book(update) => update.ts_ms,
        VenueEvent::Normalized(event) => event.ts_ms(),
        VenueEvent::PrivateOrder(update) => update.ts_ms,
        VenueEvent::PrivateFill(fill) => fill.ts_ms,
        VenueEvent::Account(update) => update.ts_ms,
    }
}

fn trim_ascii_whitespace(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |index| index + 1);
    &bytes[start..end]
}

fn lower_debug(value: impl std::fmt::Debug) -> String {
    format!("{value:?}").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CaptureOutputConfig, CapturePriority, CaptureRuntimeConfig, CaptureSubscriptionConfig,
        CaptureVenueConfig, sha256_hex,
    };

    fn config(include_trades: bool) -> CaptureConfig {
        let mut subscriptions = vec![CaptureSubscriptionConfig {
            channel: "books".to_string(),
            symbol: "BTC-USDT".to_string(),
            connections: 2,
            priority: CapturePriority::Critical,
        }];
        if include_trades {
            subscriptions.push(CaptureSubscriptionConfig {
                channel: "trades".to_string(),
                symbol: "BTC-USDT".to_string(),
                connections: 2,
                priority: CapturePriority::High,
            });
        }
        CaptureConfig {
            venue: CaptureVenueConfig::default(),
            runtime: CaptureRuntimeConfig::default(),
            output: CaptureOutputConfig::default(),
            subscriptions,
        }
    }

    #[test]
    fn analyzer_reports_redundancy_integrity_and_book_distributions() {
        let fixture = include_bytes!("../../../fixtures/raw/okx/depth-reset.jsonl");
        let report = analyze_capture(fixture.as_slice(), &config(false)).unwrap();

        assert!(report.integrity_healthy, "{report:#?}");
        assert_eq!(report.lines, 7);
        assert_eq!(report.capture_sessions, ["reset-session"]);
        assert_eq!(report.duplicate_events, 3);
        assert_eq!(report.expected_streams.len(), 1);
        assert_eq!(report.expected_streams[0].observed_connections, 2);
        assert!(report.expected_streams[0].complete);
        assert_eq!(report.sha256, sha256_hex(fixture));
        assert_eq!(report.sha256.len(), 64);
        let book = report.streams[0].book.as_ref().unwrap();
        assert!(book.samples > 0);
        assert!(book.spread_bps.count > 0);
        assert_eq!(report.books[0].sequence_status, "ready");
        assert_eq!(report.books[0].book_status, "ready");
    }

    #[test]
    fn analyzer_fails_integrity_when_a_configured_stream_is_absent() {
        let fixture = include_bytes!("../../../fixtures/raw/okx/depth-reset.jsonl");
        let report = analyze_capture(fixture.as_slice(), &config(true)).unwrap();

        assert!(!report.integrity_healthy);
        let trades = report
            .expected_streams
            .iter()
            .find(|stream| stream.channel == "trades")
            .unwrap();
        assert!(!trades.complete);
        assert_eq!(trades.observed_connections, 0);
    }

    #[test]
    fn analyzer_rejects_non_writer_blank_or_comment_records() {
        let fixture = include_str!("../../../fixtures/raw/okx/depth-reset.jsonl");
        let input = format!("# capture artifacts do not admit comments\n{fixture}\n");

        let report = analyze_capture(input.as_bytes(), &config(false)).unwrap();

        assert!(!report.integrity_healthy);
        assert_eq!(report.ignored_lines, 2);
    }

    #[test]
    fn analyzer_rejects_replica_count_and_unexpected_stream_mismatches() {
        let fixture = include_str!("../../../fixtures/raw/okx/depth-reset.jsonl");
        let mut expected = config(false);
        expected.subscriptions[0].connections = 1;
        let count_report = analyze_capture(fixture.as_bytes(), &expected).unwrap();
        assert!(!count_report.integrity_healthy);
        assert!(!count_report.expected_streams[0].complete);

        let first_line = fixture.lines().next().unwrap();
        let mut extra: RawCapture = serde_json::from_str(first_line).unwrap();
        extra.symbol = Some("ETH-USDT".to_string());
        extra.payload["arg"]["instId"] = serde_json::Value::String("ETH-USDT".to_string());
        let input = format!("{fixture}{}\n", serde_json::to_string(&extra).unwrap());
        let extra_report = analyze_capture(input.as_bytes(), &config(false)).unwrap();
        assert!(!extra_report.integrity_healthy);
        assert_eq!(extra_report.unexpected_data_streams.len(), 1);
        assert_eq!(extra_report.unexpected_data_streams[0].symbol, "ETH-USDT");
    }

    #[test]
    fn path_analysis_fingerprints_the_effective_raw_path_override() {
        let fixture = include_bytes!("../../../fixtures/raw/okx/depth-reset.jsonl");
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("capture.jsonl");
        std::fs::write(&path, fixture).unwrap();
        let mut expected = config(false);
        expected.output.raw_path = path.clone();

        let report = analyze_capture_path(&path, &config(false)).unwrap();

        assert_eq!(report.config_fingerprint, expected.fingerprint().unwrap());
        assert_eq!(report.source_path.as_deref(), Some(path.as_path()));
    }

    #[test]
    fn distribution_uses_bounded_samples_and_retains_full_bounds() {
        let mut distribution = DistributionAccumulator::default();
        for value in 0..20_000 {
            distribution.record(value as f64);
        }
        let report = distribution.finish();

        assert_eq!(report.count, 20_000);
        assert_eq!(report.sample_count, DISTRIBUTION_SAMPLE_CAPACITY);
        assert_eq!(report.min, Some(0.0));
        assert_eq!(report.max, Some(19_999.0));
        assert!(report.p50.is_some_and(|value| value > 8_000.0));
        assert!(report.p50.is_some_and(|value| value < 12_000.0));
        assert!(report.p99.is_some_and(|value| value > 18_000.0));
    }
}
