mod analysis;
mod verification;

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use reap_book::BookStatus;
use reap_core::{
    Channel, FeedPriority, NormalizedEvent, PINNED_JAVA_REVISION, RawEnvelope, Subscription,
    SystemEventKind, Venue,
};
use reap_feed::{
    ConnectionAttemptPacer, ConnectionStatus, ConnectionStatusKind,
    DEFAULT_OKX_CONNECTION_ATTEMPT_PACER_PATH, FeedOutput, FeedProcessor,
    OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS, RawCapture, ReconnectPolicy, RecoveryRequest,
    SequenceStatus, no_bootstrap, partition_subscriptions, spawn_supervised_feed,
};
pub use reap_telemetry::{HostGuardConfig, HostHealthSnapshot};
use reap_telemetry::{
    HostGuardRuntime, HostGuardStats, HostHealthError, check_host_health,
    current_executable_sha256, host_identity_sha256, start_host_guard,
};
use reap_venue::{VenueAdapter, VenueError, okx::OkxAdapter};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use url::Url;

pub use analysis::*;
pub use verification::*;

pub const CAPTURE_RUN_REPORT_FORMAT_VERSION: u16 = 5;
const MAX_CONNECTION_ATTEMPT_INTERVAL_MS: u64 = 60_000;
const MAX_REPORTED_UNKNOWN_FIELDS: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureConfig {
    #[serde(default)]
    pub venue: CaptureVenueConfig,
    #[serde(default)]
    pub runtime: CaptureRuntimeConfig,
    #[serde(default)]
    pub output: CaptureOutputConfig,
    #[serde(default)]
    pub host_guard: HostGuardConfig,
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
    pub connection_attempt_interval_ms: u64,
    pub connection_attempt_pacer_path: Option<PathBuf>,
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
            connection_attempt_interval_ms: 400,
            connection_attempt_pacer_path: Some(PathBuf::from(
                DEFAULT_OKX_CONNECTION_ATTEMPT_PACER_PATH,
            )),
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
    pub config_source: Option<CaptureConfigFileEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureStopReason {
    DurationElapsed,
    OperatorSignal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureBookHealth {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureConfigFileEvidence {
    pub source_path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureRunReport {
    pub format_version: u16,
    pub reap_version: String,
    pub java_reference_revision: String,
    pub executable_sha256: String,
    pub host_identity_sha256: Option<String>,
    pub host_preflight: Option<HostHealthSnapshot>,
    pub host_periodic_checks: u64,
    pub host_last_snapshot: Option<HostHealthSnapshot>,
    pub session_started_at_ms: u64,
    pub session_completed_at_ms: u64,
    pub capture_session_id: String,
    pub config_fingerprint: String,
    #[serde(default)]
    pub config_source: Option<CaptureConfigFileEvidence>,
    pub stop_reason: CaptureStopReason,
    pub elapsed_ms: u64,
    pub raw_path: PathBuf,
    pub normalized_path: Option<PathBuf>,
    pub raw_records: u64,
    pub normalized_records: u64,
    pub raw_bytes: u64,
    pub normalized_bytes: u64,
    pub raw_sha256: String,
    pub normalized_sha256: Option<String>,
    pub max_raw_queue_depth: usize,
    pub max_normalized_queue_depth: usize,
    pub parsed_events: u64,
    pub accepted_events: u64,
    pub duplicates: u64,
    pub gaps: u64,
    pub recoveries: u64,
    pub recovery_failures: u64,
    pub sequence_resets: u64,
    pub same_sequence_updates: u64,
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
    #[error("capture configuration contains unknown fields: {0}")]
    UnknownFields(String),
    #[error("capture configuration is invalid: {0}")]
    InvalidConfig(String),
    #[error("failed to partition capture subscriptions: {0}")]
    Partition(#[from] reap_feed::PartitionError),
    #[error("venue adapter failed: {0}")]
    Venue(#[from] VenueError),
    #[error("capture IO failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to create new {name} capture output {path}: {source}")]
    OpenOutput {
        name: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("capture serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("{0} capture writer closed unexpectedly")]
    WriterClosed(&'static str),
    #[error("capture writer task failed: {0}")]
    WriterJoin(#[from] tokio::task::JoinError),
    #[error("capture feed channel closed unexpectedly")]
    FeedClosed,
    #[error("capture raw record sequence exhausted")]
    RawRecordSequenceExhausted,
    #[error("capture connection pacer failed: {0}")]
    ConnectionPacer(#[from] reap_feed::ConnectionAttemptPacerError),
    #[error("capture connection pacer failed during runtime: {0}")]
    ConnectionPacerRuntime(String),
    #[error("capture provenance failed: {0}")]
    Provenance(String),
    #[error(transparent)]
    Host(#[from] HostHealthError),
    #[error("capture host guard closed unexpectedly")]
    HostGuardClosed,
    #[error("capture host guard task failed: {0}")]
    HostGuardJoin(tokio::task::JoinError),
}

impl CaptureConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, CaptureError> {
        Self::load_with_evidence(path).map(|(config, _)| config)
    }

    pub fn load_with_evidence(
        path: impl AsRef<Path>,
    ) -> Result<(Self, CaptureConfigFileEvidence), CaptureError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| CaptureError::ReadConfig {
            path: path.to_path_buf(),
            source,
        })?;
        let text = std::str::from_utf8(&bytes).map_err(|error| {
            CaptureError::InvalidConfig(format!("capture config is not UTF-8: {error}"))
        })?;
        let config = Self::from_toml(text)?;
        let evidence = CaptureConfigFileEvidence {
            source_path: path.to_path_buf(),
            bytes: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
        };
        Ok((config, evidence))
    }

    pub fn from_toml(text: &str) -> Result<Self, CaptureError> {
        let mut ignored_count = 0_u64;
        let mut ignored_paths = Vec::new();
        let deserializer = toml::Deserializer::parse(text)?;
        let config: Self = serde_ignored::deserialize(deserializer, |path| {
            ignored_count = ignored_count.saturating_add(1);
            if ignored_paths.len() < MAX_REPORTED_UNKNOWN_FIELDS {
                ignored_paths.push(path.to_string());
            }
        })?;
        if ignored_count > 0 {
            ignored_paths.sort();
            ignored_paths.dedup();
            let omitted = ignored_count.saturating_sub(ignored_paths.len() as u64);
            let mut message = ignored_paths.join(", ");
            if omitted > 0 {
                message.push_str(&format!(", and {omitted} additional field(s)"));
            }
            return Err(CaptureError::UnknownFields(message));
        }
        config.ensure_valid()?;
        Ok(config)
    }

    pub fn fingerprint(&self) -> Result<String, CaptureError> {
        Ok(sha256_hex(&serde_json::to_vec(self)?))
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
        let loopback = validate_ws_url(&self.venue.public_ws_url, &mut errors);
        errors.extend(self.host_guard.validation_errors("host_guard"));
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
        if !loopback
            && self.runtime.connection_attempt_interval_ms < OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS
        {
            errors.push(format!(
                "runtime.connection_attempt_interval_ms must be at least {OKX_MIN_CONNECTION_ATTEMPT_INTERVAL_MS} for non-loopback OKX endpoints"
            ));
        }
        if self.runtime.connection_attempt_interval_ms > MAX_CONNECTION_ATTEMPT_INTERVAL_MS {
            errors.push(format!(
                "runtime.connection_attempt_interval_ms must not exceed {MAX_CONNECTION_ATTEMPT_INTERVAL_MS}"
            ));
        }
        match self.runtime.connection_attempt_pacer_path.as_ref() {
            Some(path) if path.as_os_str().is_empty() => errors.push(
                "runtime.connection_attempt_pacer_path must not be empty when set".to_string(),
            ),
            None if !loopback => errors.push(
                "runtime.connection_attempt_pacer_path is required for official OKX endpoints"
                    .to_string(),
            ),
            _ => {}
        }
        if let Some(path) = self.runtime.connection_attempt_pacer_path.as_ref() {
            if path == &self.output.raw_path {
                errors.push(
                    "runtime.connection_attempt_pacer_path must differ from output.raw_path"
                        .to_string(),
                );
            }
            if self.output.normalized_path.as_ref() == Some(path) {
                errors.push(
                    "runtime.connection_attempt_pacer_path must differ from output.normalized_path"
                        .to_string(),
                );
            }
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

fn validate_ws_url(value: &str, errors: &mut Vec<String>) -> bool {
    match Url::parse(value) {
        Ok(url) => {
            let loopback = url.host_str().is_some_and(is_loopback_host);
            let loopback_ws = url.scheme() == "ws" && loopback;
            if url.scheme() != "wss" && !loopback_ws {
                errors.push(
                    "venue.public_ws_url must use wss (loopback ws is test-only)".to_string(),
                );
            }
            if !url.username().is_empty() || url.password().is_some() {
                errors.push("venue.public_ws_url must not contain user information".to_string());
            }
            loopback
        }
        Err(error) => {
            errors.push(format!("venue.public_ws_url is invalid: {error}"));
            false
        }
    }
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

pub async fn run_capture_path(
    path: impl AsRef<Path>,
    mut options: CaptureRunOptions,
) -> Result<CaptureRunReport, CaptureError> {
    let (config, config_source) = CaptureConfig::load_with_evidence(path)?;
    options.config_source = Some(config_source);
    run_capture(config, options).await
}

pub async fn run_capture(
    config: CaptureConfig,
    options: CaptureRunOptions,
) -> Result<CaptureRunReport, CaptureError> {
    config.ensure_valid()?;
    let config_fingerprint = config.fingerprint()?;
    let CaptureRunOptions {
        run_duration,
        config_source,
    } = options;
    if run_duration.is_some_and(|duration| duration.is_zero()) {
        return Err(CaptureError::InvalidConfig(
            "capture duration must be positive".to_string(),
        ));
    }
    let session_started_at_ms = unix_time_ms();
    let executable_sha256 = current_executable_sha256().map_err(CaptureError::Provenance)?;
    let (host_identity_sha256, host_preflight) = if config.host_guard.enabled {
        if let Some(parent) = config
            .output
            .raw_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            tokio::fs::create_dir_all(parent).await?;
        }
        (
            Some(host_identity_sha256().map_err(CaptureError::Provenance)?),
            Some(check_host_health(
                &config.host_guard,
                &config.output.raw_path,
            )?),
        )
    } else {
        (None, None)
    };
    let connection_attempt_interval =
        Duration::from_millis(config.runtime.connection_attempt_interval_ms);
    let connection_attempt_pacer = match (
        &config.runtime.connection_attempt_pacer_path,
        connection_attempt_interval.is_zero(),
    ) {
        (Some(path), false) => {
            ConnectionAttemptPacer::process_shared(connection_attempt_interval, path)?
        }
        _ => ConnectionAttemptPacer::new(connection_attempt_interval),
    };

    let subscriptions = config.subscriptions();
    let plans =
        partition_subscriptions(&subscriptions, config.runtime.max_subscriptions_per_socket)?;
    let expected_connections = plans
        .iter()
        .map(|plan| plan.conn_id.clone())
        .collect::<HashSet<_>>();
    let raw_writer = JsonlWriter::start(
        "raw",
        config.output.raw_path.clone(),
        config.runtime.writer_channel_capacity,
        config.output.flush_every_records,
        config.output.fsync_every_records,
    )
    .await?;
    let normalized_writer = match &config.output.normalized_path {
        Some(path) => match JsonlWriter::start(
            "normalized",
            path.clone(),
            config.runtime.writer_channel_capacity,
            config.output.flush_every_records,
            config.output.fsync_every_records,
        )
        .await
        {
            Ok(writer) => Some(writer),
            Err(error) => {
                let _ = raw_writer.shutdown().await;
                return Err(error);
            }
        },
        None => None,
    };
    let mut host_guard = config
        .host_guard
        .enabled
        .then(|| start_host_guard(config.host_guard.clone(), config.output.raw_path.clone()));
    let mut host_failures = host_guard.as_mut().map(HostGuardRuntime::take_failures);
    let adapter: Arc<dyn VenueAdapter> = Arc::new(OkxAdapter::new(
        &config.venue.public_ws_url,
        &config.venue.public_ws_url,
    ));
    let mut feed = spawn_supervised_feed(
        Arc::clone(&adapter),
        plans,
        no_bootstrap(),
        config.runtime.feed_channel_capacity,
        connection_attempt_pacer,
        ReconnectPolicy::default(),
    );
    let mut raw_rx = feed.take_raw();
    let mut status_rx = feed.take_status();

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
        run_duration,
        &mut host_failures,
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
    let raw_stats_result = raw_writer.shutdown().await;
    let normalized_stats_result = match normalized_writer {
        Some(writer) => writer.shutdown().await,
        None => Ok(JsonlWriterStats::default()),
    };
    let host_stats_result = match host_guard {
        Some(host_guard) => host_guard
            .shutdown()
            .await
            .map_err(CaptureError::HostGuardJoin),
        None => Ok(HostGuardStats::default()),
    };
    let pending_host_failure = host_failures
        .as_mut()
        .and_then(|failures| failures.try_recv().ok());
    host_failures.take();
    drain_result?;
    let stop_reason = loop_result?;
    let raw_stats = raw_stats_result?;
    let normalized_stats = normalized_stats_result?;
    let host_stats = host_stats_result?;
    if let Some(error) = pending_host_failure {
        return Err(error.into());
    }
    let run_elapsed_ms = elapsed_ms(&started);
    let session_completed_at_ms = unix_time_ms();
    Ok(state.report(
        CaptureRunTiming {
            stop_reason,
            elapsed_ms: run_elapsed_ms,
            completed_at_ms: session_completed_at_ms,
        },
        &config,
        CaptureRunProvenance {
            config_fingerprint,
            config_source,
            executable_sha256,
            host_identity_sha256,
            host_preflight,
            session_started_at_ms,
        },
        CaptureHostEvidence {
            periodic_checks: host_stats.checks,
            last_snapshot: host_stats.last_snapshot,
        },
        CaptureWriterEvidence {
            raw: raw_stats,
            normalized: normalized_stats,
        },
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
    host_failures: &mut Option<mpsc::Receiver<HostHealthError>>,
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
            failure = receive_host_failure(host_failures) => {
                let failure = failure.ok_or(CaptureError::HostGuardClosed)?;
                return Err(failure.into());
            }
            status = status_rx.recv() => {
                let status = status.ok_or(CaptureError::FeedClosed)?;
                if status.kind == ConnectionStatusKind::Fatal {
                    return Err(CaptureError::ConnectionPacerRuntime(format!(
                        "{}: {}",
                        status.conn_id, status.reason
                    )));
                }
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
                            source_conn_id: None,
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

async fn receive_host_failure(
    failures: &mut Option<mpsc::Receiver<HostHealthError>>,
) -> Option<HostHealthError> {
    match failures {
        Some(failures) => failures.recv().await,
        None => std::future::pending().await,
    }
}

struct CaptureState {
    processor: FeedProcessor,
    capture_session_id: String,
    next_raw_record_seq: u64,
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

struct CaptureRunProvenance {
    config_fingerprint: String,
    config_source: Option<CaptureConfigFileEvidence>,
    executable_sha256: String,
    host_identity_sha256: Option<String>,
    host_preflight: Option<HostHealthSnapshot>,
    session_started_at_ms: u64,
}

struct CaptureHostEvidence {
    periodic_checks: u64,
    last_snapshot: Option<HostHealthSnapshot>,
}

struct CaptureRunTiming {
    stop_reason: CaptureStopReason,
    elapsed_ms: u64,
    completed_at_ms: u64,
}

struct CaptureWriterEvidence {
    raw: JsonlWriterStats,
    normalized: JsonlWriterStats,
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
            next_raw_record_seq: 1,
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
            ConnectionStatusKind::Disconnected | ConnectionStatusKind::Fatal => {
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
        let capture_record_seq = self.next_raw_record_seq;
        let next_raw_record_seq = capture_record_seq
            .checked_add(1)
            .ok_or(CaptureError::RawRecordSequenceExhausted)?;
        raw_writer
            .send(raw_capture(
                &envelope,
                &self.capture_session_id,
                capture_record_seq,
            ))
            .await?;
        self.next_raw_record_seq = next_raw_record_seq;
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
            for output in self.processor.process_from(&envelope.conn_id, parsed) {
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
        timing: CaptureRunTiming,
        config: &CaptureConfig,
        provenance: CaptureRunProvenance,
        host: CaptureHostEvidence,
        writers: CaptureWriterEvidence,
    ) -> CaptureRunReport {
        let raw = writers.raw;
        let normalized = writers.normalized;
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
                    sequence_resets: health.sequence_resets,
                    same_sequence_updates: health.same_sequence_updates,
                    best_bid: health.best_bid,
                    best_ask: health.best_ask,
                },
                None => CaptureBookHealth {
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
            })
            .collect::<Vec<_>>();
        let all_connections_ready = self.expected_connections.is_subset(&self.ready_connections);
        let clean_capture = timing.stop_reason == CaptureStopReason::DurationElapsed
            && self.reached_all_connections_ready
            && all_connections_ready
            && all_books_ready
            && raw.records > 0
            && raw.records == self.next_raw_record_seq.saturating_sub(1)
            && (config.output.normalized_path.is_none() || normalized.records > 0)
            && self.parse_errors == 0
            && self.stale_book_events == 0
            && self.recovery_requests == 0
            && self.missing_recovery_routes == 0
            && processor.gaps == 0
            && processor.recovery_failures == 0
            && capture_host_evidence_is_healthy(config, &provenance, &host, timing.completed_at_ms);

        CaptureRunReport {
            format_version: CAPTURE_RUN_REPORT_FORMAT_VERSION,
            reap_version: env!("CARGO_PKG_VERSION").to_string(),
            java_reference_revision: PINNED_JAVA_REVISION.to_string(),
            executable_sha256: provenance.executable_sha256,
            host_identity_sha256: provenance.host_identity_sha256,
            host_preflight: provenance.host_preflight,
            host_periodic_checks: host.periodic_checks,
            host_last_snapshot: host.last_snapshot,
            session_started_at_ms: provenance.session_started_at_ms,
            session_completed_at_ms: timing.completed_at_ms,
            capture_session_id: self.capture_session_id.clone(),
            config_fingerprint: provenance.config_fingerprint,
            config_source: provenance.config_source,
            stop_reason: timing.stop_reason,
            elapsed_ms: timing.elapsed_ms,
            raw_path: config.output.raw_path.clone(),
            normalized_path: config.output.normalized_path.clone(),
            raw_records: raw.records,
            normalized_records: normalized.records,
            raw_bytes: raw.bytes,
            normalized_bytes: normalized.bytes,
            raw_sha256: raw.sha256,
            normalized_sha256: config
                .output
                .normalized_path
                .as_ref()
                .map(|_| normalized.sha256),
            max_raw_queue_depth: raw.max_queue_depth,
            max_normalized_queue_depth: normalized.max_queue_depth,
            parsed_events: processor.parsed,
            accepted_events: processor.accepted,
            duplicates: processor.duplicates,
            gaps: processor.gaps,
            recoveries: processor.recoveries,
            recovery_failures: processor.recovery_failures,
            sequence_resets: processor.sequence_resets,
            same_sequence_updates: processor.same_sequence_updates,
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

fn capture_host_evidence_is_healthy(
    config: &CaptureConfig,
    provenance: &CaptureRunProvenance,
    host: &CaptureHostEvidence,
    session_completed_at_ms: u64,
) -> bool {
    if !is_lower_sha256(&provenance.executable_sha256)
        || provenance.session_started_at_ms == 0
        || provenance.session_started_at_ms > session_completed_at_ms
    {
        return false;
    }
    if !config.host_guard.enabled {
        return provenance.host_identity_sha256.is_none()
            && provenance.host_preflight.is_none()
            && host.periodic_checks == 0
            && host.last_snapshot.is_none();
    }

    let Some(identity) = provenance.host_identity_sha256.as_deref() else {
        return false;
    };
    let Some(preflight) = provenance.host_preflight.as_ref() else {
        return false;
    };
    if !is_lower_sha256(identity)
        || preflight.checked_at_ms < provenance.session_started_at_ms
        || preflight.checked_at_ms > session_completed_at_ms
        || !host_snapshot_is_healthy(preflight, &config.host_guard)
    {
        return false;
    }
    match (host.periodic_checks, host.last_snapshot.as_ref()) {
        (0, None) => true,
        (0, Some(_)) | (1.., None) => false,
        (_, Some(last)) => {
            last.checked_at_ms >= preflight.checked_at_ms
                && last.checked_at_ms <= session_completed_at_ms
                && host_snapshot_is_healthy(last, &config.host_guard)
        }
    }
}

fn host_snapshot_is_healthy(snapshot: &HostHealthSnapshot, config: &HostGuardConfig) -> bool {
    snapshot.checked_at_ms > 0
        && snapshot.disk_available_bytes >= config.min_disk_available_bytes
        && snapshot.memory_available_bytes >= config.min_memory_available_bytes
        && (!config.require_clock_synchronized || snapshot.clock_synchronized)
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn raw_capture(
    envelope: &RawEnvelope,
    capture_session_id: &str,
    capture_record_seq: u64,
) -> RawCapture {
    RawCapture {
        capture_session_id: Some(capture_session_id.to_string()),
        capture_record_seq: Some(capture_record_seq),
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

#[derive(Debug, Clone, Default)]
struct JsonlWriterStats {
    records: u64,
    bytes: u64,
    max_queue_depth: usize,
    sha256: String,
}

struct JsonlWriter<T> {
    name: &'static str,
    sender: Option<mpsc::Sender<T>>,
    task: JoinHandle<Result<String, CaptureError>>,
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
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options
            .open(&path)
            .await
            .map_err(|source| CaptureError::OpenOutput {
                name,
                path: path.clone(),
                source,
            })?;
        sync_parent_directory(&path).await?;
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
        let sha256 = self.task.await??;
        Ok(JsonlWriterStats {
            records: self.records.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            max_queue_depth: self.max_queue_depth.load(Ordering::Relaxed),
            sha256,
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
) -> Result<String, CaptureError>
where
    T: Serialize,
{
    let flush_every_records = flush_every_records.max(1);
    let mut writer = BufWriter::new(file);
    let mut hasher = Sha256::new();
    let mut since_flush = 0_usize;
    let mut since_sync = 0_usize;
    while let Some(value) = receiver.recv().await {
        let mut line = serde_json::to_vec(&value)?;
        line.push(b'\n');
        writer.write_all(&line).await?;
        hasher.update(&line);
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
    writer.get_ref().sync_data().await?;
    Ok(digest_hex(hasher.finalize()))
}

#[cfg(unix)]
async fn sync_parent_directory(path: &Path) -> Result<(), CaptureError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    tokio::fs::File::open(parent).await?.sync_all().await?;
    Ok(())
}

#[cfg(not(unix))]
async fn sync_parent_directory(_path: &Path) -> Result<(), CaptureError> {
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    digest_hex(hasher.finalize())
}

fn digest_hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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
    fn deployment_config_is_absolute_redundant_and_matches_example() {
        let mut deployment =
            CaptureConfig::from_toml(include_str!("../../../deploy/capture/okx-btc-public.toml"))
                .unwrap();
        let local =
            CaptureConfig::from_toml(include_str!("../../../examples/capture-okx-public.toml"))
                .unwrap();

        assert_eq!(
            deployment.runtime.connection_attempt_pacer_path.as_deref(),
            Some(Path::new("/var/lib/reap/connectivity/okx-global.pacer"))
        );
        assert!(deployment.output.raw_path.is_absolute());
        assert!(
            deployment
                .subscriptions()
                .iter()
                .all(|subscription| !subscription.channel.is_private())
        );
        assert!(
            deployment
                .subscriptions()
                .iter()
                .all(|subscription| subscription.connections == 2)
        );

        deployment.runtime.connection_attempt_pacer_path =
            local.runtime.connection_attempt_pacer_path.clone();
        deployment.output.raw_path = local.output.raw_path.clone();
        assert_eq!(
            deployment.fingerprint().unwrap(),
            local.fingerprint().unwrap()
        );
    }

    #[test]
    fn config_rejects_private_duplicate_and_missing_book_subscriptions() {
        let config = CaptureConfig {
            venue: CaptureVenueConfig::default(),
            runtime: CaptureRuntimeConfig::default(),
            output: CaptureOutputConfig::default(),
            host_guard: HostGuardConfig::default(),
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
            host_guard: HostGuardConfig::default(),
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
    fn config_requires_tls_except_for_loopback_tests() {
        let mut config =
            CaptureConfig::from_toml(include_str!("../../../examples/capture-okx-public.toml"))
                .unwrap();
        config.venue.public_ws_url = "ws://example.com/ws/v5/public".to_string();

        let validation = config.validate();

        assert!(!validation.valid);
        assert!(validation.errors.iter().any(|error| error.contains("wss")));
        config.venue.public_ws_url = "ws://127.0.0.1:8080/ws/v5/public".to_string();
        config.runtime.connection_attempt_interval_ms = 0;
        assert!(config.validate().valid);

        config.venue.public_ws_url = "wss://ws.okx.com:8443/ws/v5/public".to_string();
        let validation = config.validate();
        assert!(!validation.valid);
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("must be at least 334"))
        );

        config.runtime.connection_attempt_interval_ms = 400;
        config.runtime.connection_attempt_pacer_path = None;
        let validation = config.validate();
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("pacer_path is required"))
        );

        config.runtime.connection_attempt_pacer_path = Some(config.output.raw_path.clone());
        let validation = config.validate();
        assert!(
            validation
                .errors
                .iter()
                .any(|error| error.contains("must differ from output.raw_path"))
        );
    }

    #[test]
    fn guarded_capture_evidence_is_bound_to_session_time_and_thresholds() {
        let mut config =
            CaptureConfig::from_toml(include_str!("../../../examples/capture-okx-public.toml"))
                .unwrap();
        config.host_guard.min_disk_available_bytes = 10;
        config.host_guard.min_memory_available_bytes = 20;
        let provenance = CaptureRunProvenance {
            config_fingerprint: "f".repeat(64),
            config_source: None,
            executable_sha256: "e".repeat(64),
            host_identity_sha256: Some("9".repeat(64)),
            host_preflight: Some(HostHealthSnapshot {
                checked_at_ms: 10,
                disk_available_bytes: 10,
                memory_available_bytes: 20,
                clock_synchronized: true,
            }),
            session_started_at_ms: 9,
        };
        let healthy = CaptureHostEvidence {
            periodic_checks: 1,
            last_snapshot: Some(HostHealthSnapshot {
                checked_at_ms: 11,
                disk_available_bytes: 10,
                memory_available_bytes: 20,
                clock_synchronized: true,
            }),
        };

        assert!(capture_host_evidence_is_healthy(
            &config,
            &provenance,
            &healthy,
            12,
        ));

        let late = CaptureHostEvidence {
            periodic_checks: 1,
            last_snapshot: Some(HostHealthSnapshot {
                checked_at_ms: 13,
                disk_available_bytes: 10,
                memory_available_bytes: 20,
                clock_synchronized: true,
            }),
        };
        assert!(!capture_host_evidence_is_healthy(
            &config,
            &provenance,
            &late,
            12,
        ));
    }

    #[test]
    fn capture_parser_rejects_unknown_fields_at_every_config_layer() {
        let config =
            CaptureConfig::from_toml(include_str!("../../../examples/capture-okx-public.toml"))
                .unwrap();
        let mut document = toml::Value::try_from(config).unwrap();
        document
            .as_table_mut()
            .unwrap()
            .insert("top_level_typo".to_string(), toml::Value::Boolean(true));
        document["venue"]
            .as_table_mut()
            .unwrap()
            .insert("websocket_typo".to_string(), toml::Value::Boolean(true));
        document["runtime"]
            .as_table_mut()
            .unwrap()
            .insert("pacing_typo".to_string(), toml::Value::Boolean(true));
        document["output"]
            .as_table_mut()
            .unwrap()
            .insert("fsync_typo".to_string(), toml::Value::Boolean(true));
        document["host_guard"]
            .as_table_mut()
            .unwrap()
            .insert("guard_typo".to_string(), toml::Value::Boolean(true));
        document["subscriptions"].as_array_mut().unwrap()[0]
            .as_table_mut()
            .unwrap()
            .insert("channel_typo".to_string(), toml::Value::Boolean(true));

        let error = CaptureConfig::from_toml(&toml::to_string(&document).unwrap())
            .unwrap_err()
            .to_string();

        for field in [
            "top_level_typo",
            "websocket_typo",
            "pacing_typo",
            "fsync_typo",
            "guard_typo",
            "channel_typo",
        ] {
            assert!(error.contains(field), "missing {field:?} in {error}");
        }
    }

    #[test]
    fn clean_report_requires_every_configured_book_snapshot() {
        let config = CaptureConfig {
            venue: CaptureVenueConfig::default(),
            runtime: CaptureRuntimeConfig::default(),
            output: CaptureOutputConfig::default(),
            host_guard: HostGuardConfig::default(),
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
        let envelope = capture.into_envelope().unwrap();
        for parsed in adapter.parse(&envelope).unwrap() {
            let _ = state.processor.process_from(&envelope.conn_id, parsed);
        }

        let report = state.report(
            CaptureRunTiming {
                stop_reason: CaptureStopReason::DurationElapsed,
                elapsed_ms: 1,
                completed_at_ms: 2,
            },
            &config,
            CaptureRunProvenance {
                config_fingerprint: config.fingerprint().unwrap(),
                config_source: None,
                executable_sha256: "0".repeat(64),
                host_identity_sha256: None,
                host_preflight: None,
                session_started_at_ms: 1,
            },
            CaptureHostEvidence {
                periodic_checks: 0,
                last_snapshot: None,
            },
            CaptureWriterEvidence {
                raw: JsonlWriterStats {
                    records: 1,
                    bytes: 1,
                    max_queue_depth: 1,
                    sha256: "0".repeat(64),
                },
                normalized: JsonlWriterStats::default(),
            },
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let event = NormalizedEvent::from(MarketEvent::Depth(OrderBook {
            symbol: "BTC-USDT".to_string(),
            ts_ms: 1,
            bids: vec![Level::new(100.0, 1.0)],
            asks: vec![Level::new(101.0, 1.0)],
        }));
        writer.send(event).await.unwrap();
        let stats = writer.shutdown().await.unwrap();
        assert_eq!(stats.records, 1);
        assert_eq!(stats.sha256.len(), 64);

        let text = std::fs::read_to_string(path).unwrap();
        let decoded: NormalizedEvent = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(decoded.ts_ms(), 1);
        assert_eq!(stats.sha256, sha256_hex(text.as_bytes()));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn host_preflight_fails_before_capture_output_or_network_startup() {
        let directory = tempfile::tempdir().unwrap();
        let raw_path = directory.path().join("new").join("raw.jsonl");
        let mut config =
            CaptureConfig::from_toml(include_str!("../../../examples/capture-okx-public.toml"))
                .unwrap();
        config.output.raw_path = raw_path.clone();
        config.host_guard.min_disk_available_bytes = u64::MAX;
        config.host_guard.min_memory_available_bytes = u64::MAX;
        config.host_guard.require_clock_synchronized = false;

        let error = run_capture(
            config,
            CaptureRunOptions {
                run_duration: Some(Duration::from_millis(1)),
                config_source: None,
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(
            error,
            CaptureError::Host(HostHealthError::Unhealthy { code, .. })
                if code.contains("disk_low") && code.contains("memory_low")
        ));
        assert!(!raw_path.exists());
    }

    #[tokio::test]
    async fn capture_refuses_to_append_a_second_process_session() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("existing.jsonl");
        std::fs::write(&path, "existing-session\n").unwrap();
        let config = CaptureConfig {
            venue: CaptureVenueConfig::default(),
            runtime: CaptureRuntimeConfig {
                connection_attempt_pacer_path: Some(directory.path().join("connect.pacer")),
                ..CaptureRuntimeConfig::default()
            },
            output: CaptureOutputConfig {
                raw_path: path.clone(),
                ..CaptureOutputConfig::default()
            },
            host_guard: HostGuardConfig::default(),
            subscriptions: vec![CaptureSubscriptionConfig {
                channel: "books".to_string(),
                symbol: "BTC-USDT".to_string(),
                connections: 1,
                priority: CapturePriority::Critical,
            }],
        };

        let error = run_capture(
            config,
            CaptureRunOptions {
                run_duration: Some(Duration::from_millis(1)),
                config_source: None,
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(
            error,
            CaptureError::OpenOutput {
                name: "raw",
                source,
                ..
            } if source.kind() == std::io::ErrorKind::AlreadyExists
        ));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "existing-session\n");
    }

    #[tokio::test]
    async fn normalized_output_failure_shuts_down_the_initialized_raw_writer() {
        let directory = tempfile::tempdir().unwrap();
        let raw_path = directory.path().join("raw.jsonl");
        let normalized_path = directory.path().join("existing-normalized.jsonl");
        std::fs::write(&normalized_path, "existing-session\n").unwrap();
        let config = CaptureConfig {
            venue: CaptureVenueConfig::default(),
            runtime: CaptureRuntimeConfig {
                connection_attempt_pacer_path: Some(directory.path().join("connect.pacer")),
                ..CaptureRuntimeConfig::default()
            },
            output: CaptureOutputConfig {
                raw_path: raw_path.clone(),
                normalized_path: Some(normalized_path.clone()),
                ..CaptureOutputConfig::default()
            },
            host_guard: HostGuardConfig::default(),
            subscriptions: vec![CaptureSubscriptionConfig {
                channel: "books".to_string(),
                symbol: "BTC-USDT".to_string(),
                connections: 1,
                priority: CapturePriority::Critical,
            }],
        };

        let error = run_capture(
            config,
            CaptureRunOptions {
                run_duration: Some(Duration::from_millis(1)),
                config_source: None,
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(
            error,
            CaptureError::OpenOutput {
                name: "normalized",
                source,
                ..
            } if source.kind() == std::io::ErrorKind::AlreadyExists
        ));
        assert_eq!(std::fs::read_to_string(raw_path).unwrap(), "");
        assert_eq!(
            std::fs::read_to_string(normalized_path).unwrap(),
            "existing-session\n"
        );
    }

    #[tokio::test]
    async fn connection_pacer_preflight_fails_before_capture_output_or_network_startup() {
        let directory = tempfile::tempdir().unwrap();
        let raw_path = directory.path().join("raw.jsonl");
        let mut config =
            CaptureConfig::from_toml(include_str!("../../../examples/capture-okx-public.toml"))
                .unwrap();
        config.venue.public_ws_url = "ws://127.0.0.1:18081/ws/v5/public".to_string();
        config.output.raw_path = raw_path.clone();
        config.host_guard.enabled = false;
        config.runtime.connection_attempt_pacer_path =
            Some(directory.path().join("missing").join("connect.pacer"));

        let error = run_capture(
            config,
            CaptureRunOptions {
                run_duration: Some(Duration::from_millis(1)),
                config_source: None,
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(error, CaptureError::ConnectionPacer(_)));
        assert!(!raw_path.exists());
    }

    #[tokio::test]
    async fn shutdown_drain_persists_queued_feed_frames() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let writer = JsonlWriter::start("raw", path.clone(), 4, 1_000, 1_000)
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
        let persisted: RawCapture =
            serde_json::from_str(std::fs::read_to_string(path).unwrap().trim()).unwrap();
        assert_eq!(
            persisted.capture_session_id.as_deref(),
            Some("test-session")
        );
        assert_eq!(persisted.capture_record_seq, Some(1));
    }
}
