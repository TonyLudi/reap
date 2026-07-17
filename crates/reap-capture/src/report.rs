use std::path::PathBuf;

use reap_telemetry::HostHealthSnapshot;
use serde::{Deserialize, Serialize};

pub const CAPTURE_RUN_REPORT_FORMAT_VERSION: u16 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureStopReason {
    DurationElapsed,
    OperatorSignal,
    RuntimeFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureFailureEvidence {
    pub code: String,
    pub message: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<CaptureFailureEvidence>,
    pub clean_capture: bool,
}
