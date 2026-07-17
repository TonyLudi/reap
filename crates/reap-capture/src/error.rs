use std::path::PathBuf;

use reap_telemetry::HostHealthError;
use reap_venue::VenueError;
use thiserror::Error;

use crate::report::CaptureRunReport;

pub(crate) const MAX_CAPTURE_FAILURE_MESSAGE_BYTES: usize = 2_048;

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("invalid capture config path {path}: {message}")]
    InvalidConfigPath { path: PathBuf, message: String },
    #[error("failed to read capture config {path}: {source}")]
    ReadConfig {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("capture config {path} is {actual} bytes; maximum is {limit}")]
    ConfigTooLarge {
        path: PathBuf,
        actual: u64,
        limit: u64,
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
    #[error("{name} capture writer queue remained full for {timeout_ms}ms")]
    WriterBackpressure {
        name: &'static str,
        timeout_ms: u128,
    },
    #[error("{name} capture writer did not shut down within {timeout_ms}ms")]
    WriterShutdownTimeout {
        name: &'static str,
        timeout_ms: u128,
    },
    #[error("{name} capture writer did not abort within {timeout_ms}ms")]
    WriterAbortTimeout {
        name: &'static str,
        timeout_ms: u128,
    },
    #[error("{name} capture writer evidence scan exceeded {timeout_ms}ms")]
    WriterEvidenceTimeout {
        name: &'static str,
        timeout_ms: u128,
    },
    #[error("capture feed channel closed unexpectedly")]
    FeedClosed,
    #[error("capture feed shutdown and drain exceeded {timeout_ms}ms")]
    FeedShutdownTimeout { timeout_ms: u128 },
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
    #[error("capture host guard shutdown exceeded {timeout_ms}ms")]
    HostGuardShutdownTimeout { timeout_ms: u128 },
    #[error("{primary}; additional capture lifecycle failures: {secondary}")]
    LifecycleFailure {
        #[source]
        primary: Box<CaptureError>,
        secondary: String,
    },
    #[error("{source}")]
    ReportedFailure {
        #[source]
        source: Box<CaptureError>,
        report: Box<CaptureRunReport>,
    },
}

impl CaptureError {
    pub fn stable_code(&self) -> &'static str {
        match self {
            Self::InvalidConfigPath { .. }
            | Self::ReadConfig { .. }
            | Self::ConfigTooLarge { .. }
            | Self::ParseConfig(_)
            | Self::UnknownFields(_)
            | Self::InvalidConfig(_) => "config",
            Self::Partition(_) => "subscription_partition",
            Self::Venue(_) => "venue_adapter",
            Self::Io(_) | Self::OpenOutput { .. } => "capture_io",
            Self::Serialization(_) => "serialization",
            Self::WriterClosed(_) => "writer_closed",
            Self::WriterJoin(_) => "writer_task",
            Self::WriterBackpressure { .. } => "writer_backpressure",
            Self::WriterShutdownTimeout { .. } => "writer_shutdown_timeout",
            Self::WriterAbortTimeout { .. } => "writer_abort_timeout",
            Self::WriterEvidenceTimeout { .. } => "writer_evidence_timeout",
            Self::FeedClosed => "feed_closed",
            Self::FeedShutdownTimeout { .. } => "feed_shutdown_timeout",
            Self::RawRecordSequenceExhausted => "raw_record_sequence",
            Self::ConnectionPacer(_) | Self::ConnectionPacerRuntime(_) => "connection_pacer",
            Self::Provenance(_) => "provenance",
            Self::Host(_)
            | Self::HostGuardClosed
            | Self::HostGuardJoin(_)
            | Self::HostGuardShutdownTimeout { .. } => "host_guard",
            Self::LifecycleFailure { primary, .. } => primary.stable_code(),
            Self::ReportedFailure { source, .. } => source.stable_code(),
        }
    }
}

pub(crate) fn combine_capture_failures(
    failures: Vec<(&'static str, CaptureError)>,
) -> CaptureError {
    let mut failures = failures.into_iter();
    let Some((_, primary)) = failures.next() else {
        return CaptureError::InvalidConfig(
            "capture lifecycle failure aggregation received no failures".to_string(),
        );
    };
    combine_capture_lifecycle_errors(primary, failures.collect())
}

pub(crate) fn combine_capture_lifecycle_errors(
    primary: CaptureError,
    additional: Vec<(&'static str, CaptureError)>,
) -> CaptureError {
    if additional.is_empty() {
        return primary;
    }
    let secondary = additional
        .into_iter()
        .map(|(label, error)| format!("{label}: {error}"))
        .collect::<Vec<_>>()
        .join("; ");
    CaptureError::LifecycleFailure {
        primary: Box::new(primary),
        secondary: truncate_utf8(&secondary, MAX_CAPTURE_FAILURE_MESSAGE_BYTES),
    }
}

pub(crate) fn truncate_utf8(value: &str, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value.to_string();
    }
    let mut boundary = maximum_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value[..boundary].to_string()
}
