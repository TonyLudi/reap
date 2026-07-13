use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use reap_book::BookStatus;
use reap_core::{
    Channel, FeedPriority, NormalizedEvent, RawEnvelope, Subscription, SystemEventKind, Venue,
};
use reap_feed::{
    ConnectionStatus, ConnectionStatusKind, FeedOutput, FeedProcessor, RawCapture, ReconnectPolicy,
    RecoveryRequest, SequenceStatus, no_bootstrap, partition_subscriptions, spawn_supervised_feed,
};
use reap_venue::{VenueAdapter, VenueError, okx::OkxAdapter};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureConfig {
    #[serde(default)]
    pub venue: CaptureVenueConfig,
    #[serde(default)]
    pub runtime: CaptureRuntimeConfig,
    #[serde(default)]
    pub output: CaptureOutputConfig,
    pub subscriptions: Vec<CaptureSubscriptionConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureVenueConfig {
    pub public_ws_url: String,
}

impl Default for CaptureVenueConfig {
    fn default() -> Self {
        Self {
            public_ws_url: "wss://ws.okx.com:8443/ws/v5/public".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureRuntimeConfig {
    pub feed_channel_capacity: usize,
    pub writer_channel_capacity: usize,
    pub dedup_capacity_per_stream: usize,
    pub max_sequence_buffer: usize,
    pub max_subscriptions_per_socket: usize,
    pub health_interval_ms: u64,
    pub max_book_age_ms: u64,
}

impl Default for CaptureRuntimeConfig {
    fn default() -> Self {
        Self {
            feed_channel_capacity: 65_536,
            writer_channel_capacity: 65_536,
            dedup_capacity_per_stream: 100_000,
            max_sequence_buffer: 4_096,
            max_subscriptions_per_socket: 100,
            health_interval_ms: 1_000,
            max_book_age_ms: 5_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureOutputConfig {
    pub raw_path: PathBuf,
    pub normalized_path: Option<PathBuf>,
    pub flush_every_records: usize,
    pub fsync_every_records: usize,
}

impl Default for CaptureOutputConfig {
    fn default() -> Self {
        Self {
            raw_path: PathBuf::from("var/reap/capture/okx-raw.jsonl"),
            normalized_path: None,
            flush_every_records: 1_024,
            fsync_every_records: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureSubscriptionConfig {
    pub channel: String,
    pub symbol: String,
    #[serde(default = "default_connections")]
    pub connections: usize,
    #[serde(default)]
    pub priority: CapturePriority,
}

fn default_connections() -> usize {
    2
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapturePriority {
    #[default]
    Critical,
    High,
    Normal,
    Low,
}

impl From<CapturePriority> for FeedPriority {
    fn from(value: CapturePriority) -> Self {
        match value {
            CapturePriority::Critical => Self::Critical,
            CapturePriority::High => Self::High,
            CapturePriority::Normal => Self::Normal,
            CapturePriority::Low => Self::Low,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureValidation {
    pub valid: bool,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CaptureRunOptions {
    pub run_duration: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureStopReason {
    DurationElapsed,
    OperatorSignal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureBookHealth {
    pub symbol: String,
    pub sequence_status: String,
    pub book_status: String,
    pub last_seq_id: Option<i64>,
    pub buffered_updates: usize,
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureRunReport {
    pub format_version: u16,
    pub capture_session_id: String,
    pub stop_reason: CaptureStopReason,
    pub elapsed_ms: u64,
    pub raw_path: PathBuf,
    pub normalized_path: Option<PathBuf>,
    pub raw_records: u64,
    pub normalized_records: u64,
    pub raw_bytes: u64,
    pub normalized_bytes: u64,
    pub max_raw_queue_depth: usize,
    pub max_normalized_queue_depth: usize,
    pub parsed_events: u64,
    pub accepted_events: u64,
    pub duplicates: u64,
    pub gaps: u64,
    pub recoveries: u64,
    pub recovery_failures: u64,
    pub recovery_requests: u64,
    pub missing_recovery_routes: u64,
    pub parse_errors: u64,
    pub stale_book_events: u64,
    pub connection_disconnects: u64,
    pub expected_connections: usize,
    pub ready_connections_at_stop: usize,
    pub reached_all_connections_ready: bool,
    pub books: Vec<CaptureBookHealth>,
    pub clean_capture: bool,
}

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("failed to read capture config {path}: {source}")]
    ReadConfig {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse capture config: {0}")]
    ParseConfig(#[from] toml::de::Error),
    #[error("capture configuration is invalid: {0}")]
    InvalidConfig(String),
    #[error("failed to partition capture subscriptions: {0}")]
    Partition(#[from] reap_feed::PartitionError),
    #[error("venue adapter failed: {0}")]
    Venue(#[from] VenueError),
    #[error("capture IO failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("capture serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("{0} capture writer closed unexpectedly")]
    WriterClosed(&'static str),
    #[error("capture writer task failed: {0}")]
    WriterJoin(#[from] tokio::task::JoinError),
    #[error("capture feed channel closed unexpectedly")]
    FeedClosed,
}

impl CaptureConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, CaptureError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| CaptureError::ReadConfig {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_toml(&text)
    }

    pub fn from_toml(text: &str) -> Result<Self, CaptureError> {
        let config: Self = toml::from_str(text)?;
        config.ensure_valid()?;
        Ok(config)
    }

    pub fn ensure_valid(&self) -> Result<(), CaptureError> {
        let validation = self.validate();
        if validation.valid {
            Ok(())
        } else {
            Err(CaptureError::InvalidConfig(validation.errors.join("; ")))
        }
    }

    pub fn validate(&self) -> CaptureValidation {
        let mut errors = Vec::new();
        validate_ws_url(&self.venue.public_ws_url, &mut errors);
        for (name, value) in [
            (
                "runtime.feed_channel_capacity",
                self.runtime.feed_channel_capacity,
            ),
            (
                "runtime.writer_channel_capacity",
                self.runtime.writer_channel_capacity,
            ),
            (
                "runtime.dedup_capacity_per_stream",
                self.runtime.dedup_capacity_per_stream,
            ),
            (
                "runtime.max_sequence_buffer",
                self.runtime.max_sequence_buffer,
            ),
            (
                "runtime.max_subscriptions_per_socket",
                self.runtime.max_subscriptions_per_socket,
            ),
        ] {
            if value == 0 {
                errors.push(format!("{name} must be positive"));
            }
        }
        if self.runtime.health_interval_ms == 0 {
            errors.push("runtime.health_interval_ms must be positive".to_string());
        }
        if self.runtime.max_book_age_ms == 0 {
            errors.push("runtime.max_book_age_ms must be positive".to_string());
        }
        if self.output.raw_path.as_os_str().is_empty() {
            errors.push("output.raw_path must not be empty".to_string());
        }
        if let Some(normalized_path) = &self.output.normalized_path {
            if normalized_path.as_os_str().is_empty() {
                errors.push("output.normalized_path must not be empty when set".to_string());
            }
            if self.output.raw_path == *normalized_path {
                errors.push("raw and normalized output paths must differ".to_string());
            }
        }
        if self.output.flush_every_records == 0 {
            errors.push("output.flush_every_records must be positive".to_string());
        }
        if self.subscriptions.is_empty() {
            errors.push("at least one public subscription is required".to_string());
        }

        let mut seen = HashSet::new();
        let mut book_symbols = HashSet::new();
        let mut has_book = false;
        for subscription in &self.subscriptions {
            let channel = subscription.channel.trim();
            let symbol = subscription.symbol.trim();
            if !supported_public_channel(channel) {
                errors.push(format!(
                    "unsupported public capture channel {}",
                    subscription.channel
                ));
            }
            if symbol.is_empty() {
                errors.push(format!("capture channel {channel} requires a symbol"));
            }
            if subscription.connections == 0 {
                errors.push(format!(
                    "capture subscription {channel}/{symbol} connections must be positive"
                ));
            }
            if !seen.insert((channel.to_string(), symbol.to_string())) {
                errors.push(format!("duplicate capture subscription {channel}/{symbol}"));
            }
            if is_book_channel(channel) {
                has_book = true;
                if !book_symbols.insert(symbol.to_string()) {
                    errors.push(format!(
                        "capture symbol {symbol} must use exactly one order-book channel"
                    ));
                }
            }
        }
        if !has_book {
            errors.push("at least one order-book subscription is required".to_string());
        }

        errors.sort();
        errors.dedup();
        CaptureValidation {
            valid: errors.is_empty(),
            errors,
        }
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        self.subscriptions
            .iter()
            .map(CaptureSubscriptionConfig::subscription)
            .collect()
    }

    fn expected_book_symbols(&self) -> HashSet<String> {
        self.subscriptions
            .iter()
            .filter(|subscription| is_book_channel(subscription.channel.trim()))
            .map(|subscription| subscription.symbol.trim().to_string())
            .collect()
    }
}

impl CaptureSubscriptionConfig {
    fn subscription(&self) -> Subscription {
        let channel = match self.channel.trim() {
            "books" => Channel::Books,
            "trades" => Channel::Trades,
            channel => Channel::Custom(channel.to_string()),
        };
        let mut subscription = Subscription::public(
            Venue::Okx,
            channel,
            self.symbol.trim(),
            self.priority.into(),
        );
        subscription.connections = self.connections;
        subscription
    }
}

fn supported_public_channel(channel: &str) -> bool {
    matches!(
        channel,
        "books"
            | "books-l2-tbt"
            | "books50-l2-tbt"
            | "trades"
            | "trades-all"
            | "funding-rate"
            | "index-tickers"
            | "price-limit"
            | "mark-price"
    )
}

fn is_book_channel(channel: &str) -> bool {
    matches!(channel, "books" | "books-l2-tbt" | "books50-l2-tbt")
}

fn validate_ws_url(value: &str, errors: &mut Vec<String>) {
    match Url::parse(value) {
        Ok(url) if matches!(url.scheme(), "ws" | "wss") => {}
        Ok(url) => errors.push(format!(
            "venue.public_ws_url scheme {} is not ws or wss",
            url.scheme()
        )),
        Err(error) => errors.push(format!("venue.public_ws_url is invalid: {error}")),
    }
}

pub async fn run_capture_path(
    path: impl AsRef<Path>,
    options: CaptureRunOptions,
) -> Result<CaptureRunReport, CaptureError> {
    let config = CaptureConfig::load(path)?;
    run_capture(config, options).await
}

pub async fn run_capture(
    config: CaptureConfig,
    options: CaptureRunOptions,
) -> Result<CaptureRunReport, CaptureError> {
    config.ensure_valid()?;
    if options
        .run_duration
        .is_some_and(|duration| duration.is_zero())
    {
        return Err(CaptureError::InvalidConfig(
            "capture duration must be positive".to_string(),
        ));
    }

    let subscriptions = config.subscriptions();
    let plans =
        partition_subscriptions(&subscriptions, config.runtime.max_subscriptions_per_socket)?;
    let expected_connections = plans
        .iter()
        .map(|plan| plan.conn_id.clone())
        .collect::<HashSet<_>>();
    let adapter: Arc<dyn VenueAdapter> = Arc::new(OkxAdapter::new(
        &config.venue.public_ws_url,
        &config.venue.public_ws_url,
    ));
    let mut feed = spawn_supervised_feed(
        Arc::clone(&adapter),
        plans,
        no_bootstrap(),
        config.runtime.feed_channel_capacity,
        ReconnectPolicy::default(),
    );
    let mut raw_rx = feed.take_raw();
    let mut status_rx = feed.take_status();

    let raw_writer = JsonlWriter::start(
        "raw",
        config.output.raw_path.clone(),
        config.runtime.writer_channel_capacity,
        config.output.flush_every_records,
        config.output.fsync_every_records,
    )
    .await?;
    let normalized_writer = match &config.output.normalized_path {
        Some(path) => Some(
            JsonlWriter::start(
                "normalized",
                path.clone(),
                config.runtime.writer_channel_capacity,
                config.output.flush_every_records,
                config.output.fsync_every_records,
            )
            .await?,
        ),
        None => None,
    };

    let started = Instant::now();
    let capture_session_id = format!("{:x}-{:x}", reap_feed::unix_time_ns(), std::process::id());
    let mut state = CaptureState::new(
        config.runtime.dedup_capacity_per_stream,
        config.runtime.max_sequence_buffer,
        expected_connections,
        config.expected_book_symbols(),
        capture_session_id,
    );
    let loop_result = run_capture_loop(
        &mut state,
        &adapter,
        &feed,
        &mut raw_rx,
        &mut status_rx,
        &raw_writer,
        normalized_writer.as_ref(),
        &config.runtime,
        options.run_duration,
    )
    .await;

    let (_, drain_result) = tokio::join!(
        feed.shutdown(),
        drain_capture_channels(
            &mut state,
            adapter.as_ref(),
            &mut raw_rx,
            &mut status_rx,
            &raw_writer,
            normalized_writer.as_ref(),
        )
    );
    drain_result?;
    let raw_stats_result = raw_writer.shutdown().await;
    let normalized_stats_result = match normalized_writer {
        Some(writer) => writer.shutdown().await,
        None => Ok(JsonlWriterStats::default()),
    };
    let stop_reason = loop_result?;
    let raw_stats = raw_stats_result?;
    let normalized_stats = normalized_stats_result?;
    Ok(state.report(
        stop_reason,
        elapsed_ms(&started),
        &config,
        raw_stats,
        normalized_stats,
    ))
}

async fn drain_capture_channels(
    state: &mut CaptureState,
    adapter: &dyn VenueAdapter,
    raw_rx: &mut mpsc::Receiver<RawEnvelope>,
    status_rx: &mut mpsc::Receiver<ConnectionStatus>,
    raw_writer: &JsonlWriter<RawCapture>,
    normalized_writer: Option<&JsonlWriter<NormalizedEvent>>,
) -> Result<(), CaptureError> {
    let mut raw_closed = false;
    let mut status_closed = false;
    while !raw_closed || !status_closed {
        tokio::select! {
            envelope = raw_rx.recv(), if !raw_closed => match envelope {
                Some(envelope) => {
                    let recoveries = state
                        .on_envelope(adapter, envelope, raw_writer, normalized_writer)
                        .await?;
                    state.recovery_requests = state
                        .recovery_requests
                        .saturating_add(recoveries.len() as u64);
                }
                None => raw_closed = true,
            },
            status = status_rx.recv(), if !status_closed => match status {
                Some(status) => state.on_status(status),
                None => status_closed = true,
            },
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_capture_loop(
    state: &mut CaptureState,
    adapter: &Arc<dyn VenueAdapter>,
    feed: &reap_feed::SupervisedFeed,
    raw_rx: &mut mpsc::Receiver<RawEnvelope>,
    status_rx: &mut mpsc::Receiver<ConnectionStatus>,
    raw_writer: &JsonlWriter<RawCapture>,
    normalized_writer: Option<&JsonlWriter<NormalizedEvent>>,
    runtime: &CaptureRuntimeConfig,
    run_duration: Option<Duration>,
) -> Result<CaptureStopReason, CaptureError> {
    let mut health = tokio::time::interval(Duration::from_millis(runtime.health_interval_ms));
    health.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    let duration_elapsed = async move {
        match run_duration {
            Some(duration) => tokio::time::sleep(duration).await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(duration_elapsed);

    loop {
        tokio::select! {
            signal = &mut shutdown => {
                signal?;
                return Ok(CaptureStopReason::OperatorSignal);
            }
            _ = &mut duration_elapsed => {
                return Ok(CaptureStopReason::DurationElapsed);
            }
            status = status_rx.recv() => {
                let status = status.ok_or(CaptureError::FeedClosed)?;
                state.on_status(status);
            }
            envelope = raw_rx.recv() => {
                let envelope = envelope.ok_or(CaptureError::FeedClosed)?;
                let recoveries = state
                    .on_envelope(adapter.as_ref(), envelope, raw_writer, normalized_writer)
                    .await?;
                for request in recoveries {
                    state.recovery_requests += 1;
                    if feed.request_recovery(&request) == 0 {
                        state.missing_recovery_routes += 1;
                    }
                }
            }
            _ = health.tick() => {
                let now_ms = unix_time_ms();
                for event in state.processor.mark_stale(now_ms, runtime.max_book_age_ms) {
                    state.stale_book_events += 1;
                    if let Some(symbol) = event.symbol {
                        let request = RecoveryRequest {
                            stream: reap_feed::FeedStreamId {
                                venue: event.venue.unwrap_or(Venue::Okx),
                                channel: Channel::Books,
                                symbol,
                            },
                            expected_prev: None,
                            received_prev: 0,
                            received_seq: 0,
                        };
                        state.recovery_requests += 1;
                        if feed.request_recovery(&request) == 0 {
                            state.missing_recovery_routes += 1;
                        }
                    }
                }
            }
        }
    }
}

struct CaptureState {
    processor: FeedProcessor,
    capture_session_id: String,
    expected_connections: HashSet<reap_core::ConnId>,
    expected_book_symbols: HashSet<String>,
    ready_connections: HashSet<reap_core::ConnId>,
    reached_all_connections_ready: bool,
    parse_errors: u64,
    stale_book_events: u64,
    connection_disconnects: u64,
    recovery_requests: u64,
    missing_recovery_routes: u64,
}

impl CaptureState {
    fn new(
        dedup_capacity_per_stream: usize,
        max_sequence_buffer: usize,
        expected_connections: HashSet<reap_core::ConnId>,
        expected_book_symbols: HashSet<String>,
        capture_session_id: String,
    ) -> Self {
        Self {
            processor: FeedProcessor::new(dedup_capacity_per_stream, max_sequence_buffer),
            capture_session_id,
            expected_connections,
            expected_book_symbols,
            ready_connections: HashSet::new(),
            reached_all_connections_ready: false,
            parse_errors: 0,
            stale_book_events: 0,
            connection_disconnects: 0,
            recovery_requests: 0,
            missing_recovery_routes: 0,
        }
    }

    fn on_status(&mut self, status: ConnectionStatus) {
        match status.kind {
            ConnectionStatusKind::Ready | ConnectionStatusKind::Heartbeat => {
                self.ready_connections.insert(status.conn_id);
                self.reached_all_connections_ready |=
                    self.expected_connections.is_subset(&self.ready_connections);
            }
            ConnectionStatusKind::Disconnected => {
                self.connection_disconnects += 1;
                self.ready_connections.remove(&status.conn_id);
            }
        }
    }

    async fn on_envelope(
        &mut self,
        adapter: &dyn VenueAdapter,
        envelope: RawEnvelope,
        raw_writer: &JsonlWriter<RawCapture>,
        normalized_writer: Option<&JsonlWriter<NormalizedEvent>>,
    ) -> Result<Vec<RecoveryRequest>, CaptureError> {
        raw_writer
            .send(raw_capture(&envelope, &self.capture_session_id))
            .await?;
        let parsed = match adapter.parse(&envelope) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.parse_errors += 1;
                tracing::warn!(?error, conn_id = %envelope.conn_id, "capture payload parse failed");
                return Ok(Vec::new());
            }
        };

        let mut recoveries = Vec::new();
        for parsed in parsed {
            for output in self.processor.process(parsed) {
                match output {
                    FeedOutput::Event(event) => {
                        if let Some(writer) = normalized_writer {
                            writer.send(event).await?;
                        }
                    }
                    FeedOutput::RecoveryRequired(request) => recoveries.push(request),
                    FeedOutput::System(event) => {
                        if event.kind == SystemEventKind::BookRecoveryFailed {
                            tracing::warn!(?event, "capture book recovery failed");
                        }
                        if let Some(writer) = normalized_writer {
                            writer.send(NormalizedEvent::System(event)).await?;
                        }
                    }
                    FeedOutput::Duplicate(_)
                    | FeedOutput::PrivateOrder { .. }
                    | FeedOutput::PrivateFill { .. }
                    | FeedOutput::PrivateAccount { .. } => {}
                }
            }
        }
        Ok(recoveries)
    }

    fn report(
        &self,
        stop_reason: CaptureStopReason,
        elapsed_ms: u64,
        config: &CaptureConfig,
        raw: JsonlWriterStats,
        normalized: JsonlWriterStats,
    ) -> CaptureRunReport {
        let processor = self.processor.stats();
        let stream_health = self.processor.stream_health();
        let stream_health = stream_health
            .into_iter()
            .map(|health| (health.stream.symbol.clone(), health))
            .collect::<HashMap<_, _>>();
        let all_books_ready = !self.expected_book_symbols.is_empty()
            && self.expected_book_symbols.iter().all(|symbol| {
                stream_health.get(symbol).is_some_and(|health| {
                    health.sequence_status == SequenceStatus::Ready
                        && health.book_status == BookStatus::Ready
                })
            });
        let mut expected_book_symbols = self
            .expected_book_symbols
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        expected_book_symbols.sort();
        let books = expected_book_symbols
            .into_iter()
            .map(|symbol| match stream_health.get(&symbol) {
                Some(health) => CaptureBookHealth {
                    symbol,
                    sequence_status: format!("{:?}", health.sequence_status).to_lowercase(),
                    book_status: format!("{:?}", health.book_status).to_lowercase(),
                    last_seq_id: health.last_seq_id,
                    buffered_updates: health.buffered_updates,
                    best_bid: health.best_bid,
                    best_ask: health.best_ask,
                },
                None => CaptureBookHealth {
                    symbol,
                    sequence_status: "awaiting_snapshot".to_string(),
                    book_status: "empty".to_string(),
                    last_seq_id: None,
                    buffered_updates: 0,
                    best_bid: None,
                    best_ask: None,
                },
            })
            .collect::<Vec<_>>();
        let all_connections_ready = self.expected_connections.is_subset(&self.ready_connections);
        let clean_capture = stop_reason == CaptureStopReason::DurationElapsed
            && self.reached_all_connections_ready
            && all_connections_ready
            && all_books_ready
            && raw.records > 0
            && (config.output.normalized_path.is_none() || normalized.records > 0)
            && self.parse_errors == 0
            && self.stale_book_events == 0
            && self.recovery_requests == 0
            && self.missing_recovery_routes == 0
            && processor.gaps == 0
            && processor.recovery_failures == 0;

        CaptureRunReport {
            format_version: 1,
            capture_session_id: self.capture_session_id.clone(),
            stop_reason,
            elapsed_ms,
            raw_path: config.output.raw_path.clone(),
            normalized_path: config.output.normalized_path.clone(),
            raw_records: raw.records,
            normalized_records: normalized.records,
            raw_bytes: raw.bytes,
            normalized_bytes: normalized.bytes,
            max_raw_queue_depth: raw.max_queue_depth,
            max_normalized_queue_depth: normalized.max_queue_depth,
            parsed_events: processor.parsed,
            accepted_events: processor.accepted,
            duplicates: processor.duplicates,
            gaps: processor.gaps,
            recoveries: processor.recoveries,
            recovery_failures: processor.recovery_failures,
            recovery_requests: self.recovery_requests,
            missing_recovery_routes: self.missing_recovery_routes,
            parse_errors: self.parse_errors,
            stale_book_events: self.stale_book_events,
            connection_disconnects: self.connection_disconnects,
            expected_connections: self.expected_connections.len(),
            ready_connections_at_stop: self.ready_connections.len(),
            reached_all_connections_ready: self.reached_all_connections_ready,
            books,
            clean_capture,
        }
    }
}

fn raw_capture(envelope: &RawEnvelope, capture_session_id: &str) -> RawCapture {
    RawCapture {
        capture_session_id: Some(capture_session_id.to_string()),
        venue: envelope.venue,
        conn_id: envelope.conn_id.clone(),
        channel: envelope.channel.clone(),
        symbol: envelope.symbol.clone(),
        recv_ts_ns: envelope.recv_ts_ns,
        raw_hash: Some(envelope.raw_hash),
        payload: serde_json::from_str(&envelope.payload)
            .unwrap_or_else(|_| Value::String(envelope.payload.clone())),
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct JsonlWriterStats {
    records: u64,
    bytes: u64,
    max_queue_depth: usize,
}

struct JsonlWriter<T> {
    name: &'static str,
    sender: Option<mpsc::Sender<T>>,
    task: JoinHandle<Result<(), CaptureError>>,
    queued: Arc<AtomicUsize>,
    max_queue_depth: Arc<AtomicUsize>,
    records: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
}

impl<T> JsonlWriter<T>
where
    T: Serialize + Send + 'static,
{
    async fn start(
        name: &'static str,
        path: PathBuf,
        capacity: usize,
        flush_every_records: usize,
        fsync_every_records: usize,
    ) -> Result<Self, CaptureError> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await?;
        let (sender, receiver) = mpsc::channel(capacity.max(1));
        let queued = Arc::new(AtomicUsize::new(0));
        let max_queue_depth = Arc::new(AtomicUsize::new(0));
        let records = Arc::new(AtomicU64::new(0));
        let bytes = Arc::new(AtomicU64::new(0));
        let task = tokio::spawn(run_jsonl_writer(
            file,
            receiver,
            flush_every_records,
            fsync_every_records,
            Arc::clone(&queued),
            Arc::clone(&records),
            Arc::clone(&bytes),
        ));
        Ok(Self {
            name,
            sender: Some(sender),
            task,
            queued,
            max_queue_depth,
            records,
            bytes,
        })
    }

    async fn send(&self, value: T) -> Result<(), CaptureError> {
        let depth = self.queued.fetch_add(1, Ordering::Relaxed) + 1;
        self.max_queue_depth.fetch_max(depth, Ordering::Relaxed);
        let Some(sender) = &self.sender else {
            self.queued.fetch_sub(1, Ordering::Relaxed);
            return Err(CaptureError::WriterClosed(self.name));
        };
        if sender.send(value).await.is_err() {
            self.queued.fetch_sub(1, Ordering::Relaxed);
            return Err(CaptureError::WriterClosed(self.name));
        }
        Ok(())
    }

    async fn shutdown(mut self) -> Result<JsonlWriterStats, CaptureError> {
        drop(self.sender.take());
        self.task.await??;
        Ok(JsonlWriterStats {
            records: self.records.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            max_queue_depth: self.max_queue_depth.load(Ordering::Relaxed),
        })
    }
}

async fn run_jsonl_writer<T>(
    file: tokio::fs::File,
    mut receiver: mpsc::Receiver<T>,
    flush_every_records: usize,
    fsync_every_records: usize,
    queued: Arc<AtomicUsize>,
    records: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
) -> Result<(), CaptureError>
where
    T: Serialize,
{
    let flush_every_records = flush_every_records.max(1);
    let mut writer = BufWriter::new(file);
    let mut since_flush = 0_usize;
    let mut since_sync = 0_usize;
    while let Some(value) = receiver.recv().await {
        let mut line = serde_json::to_vec(&value)?;
        line.push(b'\n');
        writer.write_all(&line).await?;
        queued.fetch_sub(1, Ordering::Relaxed);
        records.fetch_add(1, Ordering::Relaxed);
        bytes.fetch_add(line.len() as u64, Ordering::Relaxed);
        since_flush += 1;
        since_sync += 1;
        if since_flush >= flush_every_records {
            writer.flush().await?;
            since_flush = 0;
        }
        if fsync_every_records > 0 && since_sync >= fsync_every_records {
            writer.flush().await?;
            writer.get_ref().sync_data().await?;
            since_sync = 0;
        }
    }
    writer.flush().await?;
    if fsync_every_records > 0 && since_sync > 0 {
        writer.get_ref().sync_data().await?;
    }
    Ok(())
}

fn unix_time_ms() -> u64 {
    reap_feed::unix_time_ns() / 1_000_000
}

fn elapsed_ms(started: &Instant) -> u64 {
    started.elapsed().as_millis().min(u64::MAX as u128) as u64
}

#[cfg(unix)]
async fn shutdown_signal() -> Result<(), std::io::Error> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() -> Result<(), std::io::Error> {
    tokio::signal::ctrl_c().await
}

#[cfg(test)]
mod tests {
    use reap_core::{Level, MarketEvent, OrderBook};

    use super::*;

    #[test]
    fn example_config_is_valid_and_public_only() {
        let config =
            CaptureConfig::from_toml(include_str!("../../../examples/capture-okx-public.toml"))
                .unwrap();
        assert!(config.validate().valid);
        assert!(
            config
                .subscriptions()
                .iter()
                .all(|subscription| !subscription.channel.is_private())
        );
        assert!(
            config
                .subscriptions()
                .iter()
                .all(|subscription| subscription.connections == 2)
        );
    }

    #[test]
    fn config_rejects_private_duplicate_and_missing_book_subscriptions() {
        let config = CaptureConfig {
            venue: CaptureVenueConfig::default(),
            runtime: CaptureRuntimeConfig::default(),
            output: CaptureOutputConfig::default(),
            subscriptions: vec![
                CaptureSubscriptionConfig {
                    channel: "orders".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    connections: 1,
                    priority: CapturePriority::Critical,
                },
                CaptureSubscriptionConfig {
                    channel: "orders".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    connections: 1,
                    priority: CapturePriority::Critical,
                },
            ],
        };
        let validation = config.validate();
        assert!(!validation.valid);
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("unsupported public"))
        );
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("duplicate capture"))
        );
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("order-book"))
        );
    }

    #[test]
    fn config_rejects_multiple_book_channels_for_one_symbol() {
        let config = CaptureConfig {
            venue: CaptureVenueConfig::default(),
            runtime: CaptureRuntimeConfig::default(),
            output: CaptureOutputConfig::default(),
            subscriptions: vec![
                CaptureSubscriptionConfig {
                    channel: "books".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    connections: 1,
                    priority: CapturePriority::Critical,
                },
                CaptureSubscriptionConfig {
                    channel: "books-l2-tbt".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    connections: 1,
                    priority: CapturePriority::Critical,
                },
            ],
        };

        let validation = config.validate();

        assert!(!validation.valid);
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("exactly one order-book channel"))
        );
    }

    #[test]
    fn clean_report_requires_every_configured_book_snapshot() {
        let config = CaptureConfig {
            venue: CaptureVenueConfig::default(),
            runtime: CaptureRuntimeConfig::default(),
            output: CaptureOutputConfig::default(),
            subscriptions: vec![
                CaptureSubscriptionConfig {
                    channel: "books".to_string(),
                    symbol: "BTC-USDT".to_string(),
                    connections: 1,
                    priority: CapturePriority::Critical,
                },
                CaptureSubscriptionConfig {
                    channel: "books".to_string(),
                    symbol: "ETH-USDT".to_string(),
                    connections: 1,
                    priority: CapturePriority::Critical,
                },
            ],
        };
        let mut state = CaptureState::new(
            16,
            16,
            HashSet::new(),
            config.expected_book_symbols(),
            "test-session".to_string(),
        );
        state.reached_all_connections_ready = true;
        let first_line = include_str!("../../../fixtures/raw/okx/depth-gap.jsonl")
            .lines()
            .next()
            .unwrap();
        let capture: RawCapture = serde_json::from_str(first_line).unwrap();
        let adapter = OkxAdapter::default();
        for parsed in adapter.parse(&capture.into_envelope().unwrap()).unwrap() {
            let _ = state.processor.process(parsed);
        }

        let report = state.report(
            CaptureStopReason::DurationElapsed,
            1,
            &config,
            JsonlWriterStats {
                records: 1,
                bytes: 1,
                max_queue_depth: 1,
            },
            JsonlWriterStats::default(),
        );

        assert!(!report.clean_capture);
        assert_eq!(report.books.len(), 2);
        assert_eq!(report.books[1].symbol, "ETH-USDT");
        assert_eq!(report.books[1].sequence_status, "awaiting_snapshot");
    }

    #[tokio::test]
    async fn normalized_writer_emits_backtest_compatible_jsonl() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("normalized.jsonl");
        let writer = JsonlWriter::start("test", path.clone(), 4, 1, 0)
            .await
            .unwrap();
        let event = NormalizedEvent::from(MarketEvent::Depth(OrderBook {
            symbol: "BTC-USDT".to_string(),
            ts_ms: 1,
            bids: vec![Level::new(100.0, 1.0)],
            asks: vec![Level::new(101.0, 1.0)],
        }));
        writer.send(event).await.unwrap();
        let stats = writer.shutdown().await.unwrap();
        assert_eq!(stats.records, 1);

        let text = std::fs::read_to_string(path).unwrap();
        let decoded: NormalizedEvent = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(decoded.ts_ms(), 1);
    }

    #[tokio::test]
    async fn shutdown_drain_persists_queued_feed_frames() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let writer = JsonlWriter::start("raw", path, 4, 1_000, 1_000)
            .await
            .unwrap();
        let first_line = include_str!("../../../fixtures/raw/okx/depth-gap.jsonl")
            .lines()
            .next()
            .unwrap();
        let capture: RawCapture = serde_json::from_str(first_line).unwrap();
        let (raw_tx, mut raw_rx) = mpsc::channel(1);
        raw_tx.send(capture.into_envelope().unwrap()).await.unwrap();
        drop(raw_tx);
        let (status_tx, mut status_rx) = mpsc::channel(1);
        drop(status_tx);
        let mut state = CaptureState::new(
            16,
            16,
            HashSet::new(),
            HashSet::new(),
            "test-session".to_string(),
        );

        drain_capture_channels(
            &mut state,
            &OkxAdapter::default(),
            &mut raw_rx,
            &mut status_rx,
            &writer,
            None,
        )
        .await
        .unwrap();
        let stats = writer.shutdown().await.unwrap();

        assert_eq!(stats.records, 1);
        assert_eq!(state.processor.stats().accepted, 1);
    }
}
