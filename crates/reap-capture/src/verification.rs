use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use reap_core::{NormalizedEvent, PINNED_JAVA_REVISION};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    CAPTURE_RUN_REPORT_FORMAT_VERSION, CaptureAnalysisBookHealth, CaptureAnalysisReport,
    CaptureConfig, CaptureError, CaptureRunReport, CaptureStopReason, MAX_CAPTURE_CONFIG_BYTES,
    analyze_capture, digest_hex, is_book_channel, sha256_hex,
};

pub const CAPTURE_VERIFICATION_FORMAT_VERSION: u16 = 3;
pub const MAX_CAPTURE_RUN_REPORT_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureRunReportEvidence {
    pub source_path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureConfigEvidence {
    pub source_path: PathBuf,
    pub recorded_source_path: Option<PathBuf>,
    pub bytes: u64,
    pub sha256: String,
    pub effective_fingerprint: String,
    pub reported_effective_fingerprint: String,
    pub file_matches_report: bool,
    pub effective_matches_report: bool,
    pub matches_report: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureFileVerification {
    pub source_path: PathBuf,
    pub recorded_path: PathBuf,
    pub expected_records: u64,
    pub actual_records: u64,
    pub expected_bytes: u64,
    pub actual_bytes: u64,
    pub expected_sha256: Option<String>,
    pub actual_sha256: String,
    pub reconstructed_records: Option<u64>,
    pub reconstructed_bytes: Option<u64>,
    pub reconstructed_sha256: Option<String>,
    pub parse_clean: bool,
    pub canonical_jsonl: Option<bool>,
    pub stable_while_reading: bool,
    pub matches_report: bool,
    pub matches_reconstruction: Option<bool>,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum CaptureVerificationFailure {
    UnsupportedRunReportFormat {
        actual: u16,
        supported: u16,
    },
    ConfigFileEvidenceMissing,
    ConfigFileMismatch,
    EffectiveConfigFingerprintMismatch,
    RawArtifactMismatch,
    RawArtifactChangedWhileReading,
    RawRecordSequenceIncomplete {
        expected_records: u64,
        sequenced_records: u64,
        first_sequence: Option<u64>,
        last_sequence: Option<u64>,
        sequence_errors: u64,
    },
    NormalizedArtifactMissing,
    NormalizedArtifactUnexpected,
    NormalizedArtifactMismatch,
    NormalizedArtifactNotCanonical {
        invalid_records: u64,
        first_error: Option<String>,
    },
    NormalizedArtifactChangedWhileReading,
    CaptureSessionMismatch,
    CounterMismatch {
        field: String,
        reported: u64,
        replayed: u64,
    },
    ExpectedConnectionsMismatch {
        reported: usize,
        configured: usize,
    },
    BookHealthMismatch {
        symbol: String,
    },
    RunReportInvariant {
        message: String,
    },
    CleanFlagMismatch {
        reported: bool,
        derived: bool,
    },
    RunReportNotClean,
    AnalysisIntegrityUnhealthy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureVerificationReport {
    pub format_version: u16,
    pub reap_version: String,
    pub java_reference_revision: String,
    pub executable_sha256: String,
    pub host_identity_sha256: Option<String>,
    pub host_periodic_checks: u64,
    pub session_started_at_ms: u64,
    pub session_completed_at_ms: u64,
    pub capture_session_id: String,
    pub run_report: CaptureRunReportEvidence,
    pub config: CaptureConfigEvidence,
    pub raw: CaptureFileVerification,
    pub normalized: Option<CaptureFileVerification>,
    pub analysis: CaptureAnalysisReport,
    pub failures: Vec<CaptureVerificationFailure>,
    pub passed: bool,
}

#[derive(Debug, Error)]
pub enum CaptureVerificationError {
    #[error("invalid {label} path {path}: {message}")]
    InvalidInputPath {
        label: &'static str,
        path: PathBuf,
        message: String,
    },
    #[error("{label} {path} is too large: {actual} bytes exceeds {limit}")]
    InputTooLarge {
        label: &'static str,
        path: PathBuf,
        actual: u64,
        limit: u64,
    },
    #[error("failed to read {label} {path}: {source}")]
    ReadInput {
        label: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("capture config {path} is not UTF-8: {source}")]
    ConfigUtf8 {
        path: PathBuf,
        #[source]
        source: std::str::Utf8Error,
    },
    #[error("failed to parse capture run report {path}: {source}")]
    ParseRunReport {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Capture(#[from] CaptureError),
}

pub fn verify_capture_paths(
    config_path: impl AsRef<Path>,
    run_report_path: impl AsRef<Path>,
    raw_path: impl AsRef<Path>,
    normalized_path: Option<&Path>,
) -> Result<CaptureVerificationReport, CaptureVerificationError> {
    let (run_report_source, run_report_bytes) = read_bounded_regular_file(
        run_report_path.as_ref(),
        "capture run report",
        MAX_CAPTURE_RUN_REPORT_BYTES,
    )?;
    let run_report: CaptureRunReport =
        serde_json::from_slice(&run_report_bytes).map_err(|source| {
            CaptureVerificationError::ParseRunReport {
                path: run_report_source.clone(),
                source,
            }
        })?;
    let run_report_evidence = CaptureRunReportEvidence {
        source_path: run_report_source,
        bytes: run_report_bytes.len() as u64,
        sha256: sha256_hex(&run_report_bytes),
    };

    let (config_source, config_bytes) = read_bounded_regular_file(
        config_path.as_ref(),
        "capture config",
        MAX_CAPTURE_CONFIG_BYTES,
    )?;
    let config_text = std::str::from_utf8(&config_bytes).map_err(|source| {
        CaptureVerificationError::ConfigUtf8 {
            path: config_source.clone(),
            source,
        }
    })?;
    let mut effective_config = CaptureConfig::from_toml(config_text)?;
    effective_config.output.raw_path = run_report.raw_path.clone();
    effective_config.output.normalized_path = run_report.normalized_path.clone();
    effective_config.ensure_valid()?;
    let effective_fingerprint = effective_config.fingerprint()?;
    let effective_config_matches = effective_fingerprint == run_report.config_fingerprint;
    let config_sha256 = sha256_hex(&config_bytes);
    let config_file_matches = run_report.config_source.as_ref().is_some_and(|source| {
        source.bytes == config_bytes.len() as u64 && source.sha256 == config_sha256
    });
    let config_matches = effective_config_matches && config_file_matches;
    let config_evidence = CaptureConfigEvidence {
        source_path: config_source,
        recorded_source_path: run_report
            .config_source
            .as_ref()
            .map(|source| source.source_path.clone()),
        bytes: config_bytes.len() as u64,
        sha256: config_sha256,
        effective_fingerprint,
        reported_effective_fingerprint: run_report.config_fingerprint.clone(),
        file_matches_report: config_file_matches,
        effective_matches_report: effective_config_matches,
        matches_report: config_matches,
    };

    let expected_connections = reap_feed::partition_subscriptions(
        &effective_config.subscriptions(),
        effective_config.runtime.max_subscriptions_per_socket,
    )
    .map_err(CaptureError::from)?
    .len();

    let raw_source = canonical_regular_file(raw_path.as_ref(), "raw capture")?;
    let raw_file =
        File::open(&raw_source).map_err(|source| CaptureVerificationError::ReadInput {
            label: "raw capture",
            path: raw_source.clone(),
            source,
        })?;
    let raw_metadata = raw_file
        .try_clone()
        .and_then(|file| file.metadata())
        .map_err(|source| CaptureVerificationError::ReadInput {
            label: "raw capture metadata",
            path: raw_source.clone(),
            source,
        })?;
    let mut analysis = analyze_capture(raw_file, &effective_config)?;
    analysis.source_path = Some(raw_source.clone());
    let raw_stable = raw_metadata.len() == analysis.bytes;
    let raw_matches = run_report.raw_records == analysis.lines
        && run_report.raw_bytes == analysis.bytes
        && run_report.raw_sha256 == analysis.sha256;
    let raw_record_sequence_complete = analysis.capture_record_sequence_complete
        && analysis.sequenced_records == run_report.raw_records;
    let raw = CaptureFileVerification {
        source_path: raw_source,
        recorded_path: run_report.raw_path.clone(),
        expected_records: run_report.raw_records,
        actual_records: analysis.lines,
        expected_bytes: run_report.raw_bytes,
        actual_bytes: analysis.bytes,
        expected_sha256: Some(run_report.raw_sha256.clone()),
        actual_sha256: analysis.sha256.clone(),
        reconstructed_records: None,
        reconstructed_bytes: None,
        reconstructed_sha256: None,
        parse_clean: analysis.error_count == 0 && analysis.ignored_lines == 0,
        canonical_jsonl: None,
        stable_while_reading: raw_stable,
        matches_report: raw_matches,
        matches_reconstruction: None,
        passed: raw_matches
            && raw_stable
            && raw_record_sequence_complete
            && analysis.error_count == 0
            && analysis.ignored_lines == 0,
    };

    let mut failures = Vec::new();
    if run_report.format_version != CAPTURE_RUN_REPORT_FORMAT_VERSION {
        failures.push(CaptureVerificationFailure::UnsupportedRunReportFormat {
            actual: run_report.format_version,
            supported: CAPTURE_RUN_REPORT_FORMAT_VERSION,
        });
    }
    if run_report.config_source.is_none() {
        failures.push(CaptureVerificationFailure::ConfigFileEvidenceMissing);
    } else if !config_file_matches {
        failures.push(CaptureVerificationFailure::ConfigFileMismatch);
    }
    if !effective_config_matches {
        failures.push(CaptureVerificationFailure::EffectiveConfigFingerprintMismatch);
    }
    if !raw_matches {
        failures.push(CaptureVerificationFailure::RawArtifactMismatch);
    }
    if !raw_stable {
        failures.push(CaptureVerificationFailure::RawArtifactChangedWhileReading);
    }
    if !raw_record_sequence_complete {
        failures.push(CaptureVerificationFailure::RawRecordSequenceIncomplete {
            expected_records: run_report.raw_records,
            sequenced_records: analysis.sequenced_records,
            first_sequence: analysis.first_capture_record_seq,
            last_sequence: analysis.last_capture_record_seq,
            sequence_errors: analysis.capture_record_sequence_errors,
        });
    }

    let normalized = match (&run_report.normalized_path, normalized_path) {
        (Some(recorded_path), Some(source_path)) => {
            let scan = scan_normalized_jsonl(source_path)?;
            let expected_sha256 = run_report.normalized_sha256.clone();
            let matches_report = run_report.normalized_records == scan.records
                && run_report.normalized_bytes == scan.bytes
                && expected_sha256.as_deref() == Some(scan.sha256.as_str());
            let matches_reconstruction = analysis.reconstructed_normalized_records == scan.records
                && analysis.reconstructed_normalized_bytes == scan.bytes
                && analysis.reconstructed_normalized_sha256 == scan.sha256;
            let passed = matches_report
                && matches_reconstruction
                && scan.invalid_records == 0
                && scan.stable_while_reading;
            if !matches_report || !matches_reconstruction {
                failures.push(CaptureVerificationFailure::NormalizedArtifactMismatch);
            }
            if scan.invalid_records > 0 {
                failures.push(CaptureVerificationFailure::NormalizedArtifactNotCanonical {
                    invalid_records: scan.invalid_records,
                    first_error: scan.first_error.clone(),
                });
            }
            if !scan.stable_while_reading {
                failures.push(CaptureVerificationFailure::NormalizedArtifactChangedWhileReading);
            }
            Some(CaptureFileVerification {
                source_path: scan.source_path,
                recorded_path: recorded_path.clone(),
                expected_records: run_report.normalized_records,
                actual_records: scan.records,
                expected_bytes: run_report.normalized_bytes,
                actual_bytes: scan.bytes,
                expected_sha256,
                actual_sha256: scan.sha256,
                reconstructed_records: Some(analysis.reconstructed_normalized_records),
                reconstructed_bytes: Some(analysis.reconstructed_normalized_bytes),
                reconstructed_sha256: Some(analysis.reconstructed_normalized_sha256.clone()),
                parse_clean: scan.invalid_records == 0,
                canonical_jsonl: Some(scan.invalid_records == 0),
                stable_while_reading: scan.stable_while_reading,
                matches_report,
                matches_reconstruction: Some(matches_reconstruction),
                passed,
            })
        }
        (Some(_), None) => {
            failures.push(CaptureVerificationFailure::NormalizedArtifactMissing);
            None
        }
        (None, Some(_)) => {
            failures.push(CaptureVerificationFailure::NormalizedArtifactUnexpected);
            None
        }
        (None, None) => None,
    };

    compare_counter(
        "parsed_events",
        run_report.parsed_events,
        analysis.parsed_events,
        &mut failures,
    );
    compare_counter(
        "accepted_events",
        run_report.accepted_events,
        analysis.accepted_events,
        &mut failures,
    );
    compare_counter(
        "duplicates",
        run_report.duplicates,
        analysis.duplicate_events,
        &mut failures,
    );
    compare_counter("gaps", run_report.gaps, analysis.gaps, &mut failures);
    compare_counter(
        "recoveries",
        run_report.recoveries,
        analysis.recoveries,
        &mut failures,
    );
    compare_counter(
        "recovery_failures",
        run_report.recovery_failures,
        analysis.recovery_failures,
        &mut failures,
    );
    compare_counter(
        "sequence_resets",
        run_report.sequence_resets,
        analysis.sequence_resets,
        &mut failures,
    );
    compare_counter(
        "same_sequence_updates",
        run_report.same_sequence_updates,
        analysis.same_sequence_updates,
        &mut failures,
    );
    compare_counter(
        "parse_errors",
        run_report.parse_errors,
        analysis.error_count,
        &mut failures,
    );

    if analysis.capture_sessions.len() != 1
        || analysis.capture_sessions.first() != Some(&run_report.capture_session_id)
    {
        failures.push(CaptureVerificationFailure::CaptureSessionMismatch);
    }
    if run_report.expected_connections != expected_connections {
        failures.push(CaptureVerificationFailure::ExpectedConnectionsMismatch {
            reported: run_report.expected_connections,
            configured: expected_connections,
        });
    }
    for symbol in mismatched_books(&run_report, &analysis) {
        failures.push(CaptureVerificationFailure::BookHealthMismatch { symbol });
    }
    for message in run_report_invariant_failures(&run_report, &effective_config) {
        failures.push(CaptureVerificationFailure::RunReportInvariant { message });
    }
    let stream_coverage_complete = capture_stream_coverage_complete(&analysis);
    let derived_clean = derive_clean_capture(
        &run_report,
        &effective_config,
        expected_connections,
        stream_coverage_complete,
    );
    if run_report.clean_capture != derived_clean {
        failures.push(CaptureVerificationFailure::CleanFlagMismatch {
            reported: run_report.clean_capture,
            derived: derived_clean,
        });
    }
    if !run_report.clean_capture {
        failures.push(CaptureVerificationFailure::RunReportNotClean);
    }
    if !analysis.integrity_healthy {
        failures.push(CaptureVerificationFailure::AnalysisIntegrityUnhealthy);
    }

    let passed = failures.is_empty()
        && raw.passed
        && normalized.as_ref().is_none_or(|evidence| evidence.passed);
    Ok(CaptureVerificationReport {
        format_version: CAPTURE_VERIFICATION_FORMAT_VERSION,
        reap_version: run_report.reap_version.clone(),
        java_reference_revision: run_report.java_reference_revision.clone(),
        executable_sha256: run_report.executable_sha256.clone(),
        host_identity_sha256: run_report.host_identity_sha256.clone(),
        host_periodic_checks: run_report.host_periodic_checks,
        session_started_at_ms: run_report.session_started_at_ms,
        session_completed_at_ms: run_report.session_completed_at_ms,
        capture_session_id: run_report.capture_session_id,
        run_report: run_report_evidence,
        config: config_evidence,
        raw,
        normalized,
        analysis,
        failures,
        passed,
    })
}

fn capture_stream_coverage_complete(analysis: &CaptureAnalysisReport) -> bool {
    analysis.error_count == 0
        && analysis
            .expected_streams
            .iter()
            .all(|stream| stream.complete)
        && analysis.unexpected_data_streams.is_empty()
}

fn compare_counter(
    field: &str,
    reported: u64,
    replayed: u64,
    failures: &mut Vec<CaptureVerificationFailure>,
) {
    if reported != replayed {
        failures.push(CaptureVerificationFailure::CounterMismatch {
            field: field.to_string(),
            reported,
            replayed,
        });
    }
}

fn mismatched_books(
    run_report: &CaptureRunReport,
    analysis: &CaptureAnalysisReport,
) -> Vec<String> {
    let reported = run_report
        .books
        .iter()
        .map(|book| (book.symbol.as_str(), book))
        .collect::<BTreeMap<_, _>>();
    let replayed = analysis
        .books
        .iter()
        .map(|book| (book.symbol.as_str(), book))
        .collect::<BTreeMap<_, _>>();
    reported
        .keys()
        .chain(replayed.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(
            |symbol| match (reported.get(symbol), replayed.get(symbol)) {
                (Some(reported), Some(replayed)) => !book_health_matches(reported, replayed),
                _ => true,
            },
        )
        .map(str::to_string)
        .collect()
}

fn book_health_matches(
    reported: &crate::CaptureBookHealth,
    replayed: &CaptureAnalysisBookHealth,
) -> bool {
    reported.sequence_status == replayed.sequence_status
        && reported.book_status == replayed.book_status
        && reported.last_seq_id == replayed.last_seq_id
        && reported.buffered_updates == replayed.buffered_updates
        && reported.sequence_resets == replayed.sequence_resets
        && reported.same_sequence_updates == replayed.same_sequence_updates
        && reported.best_bid == replayed.best_bid
        && reported.best_ask == replayed.best_ask
}

fn run_report_invariant_failures(report: &CaptureRunReport, config: &CaptureConfig) -> Vec<String> {
    let mut failures = Vec::new();
    if report.reap_version != env!("CARGO_PKG_VERSION") {
        failures.push(format!(
            "reap_version {} does not match verifier version {}",
            report.reap_version,
            env!("CARGO_PKG_VERSION")
        ));
    }
    if report.java_reference_revision != PINNED_JAVA_REVISION {
        failures.push(format!(
            "java_reference_revision does not match pinned revision {PINNED_JAVA_REVISION}"
        ));
    }
    if !is_sha256(&report.executable_sha256) {
        failures.push("executable_sha256 is not lowercase SHA-256".to_string());
    }
    if !session_bounds_are_valid(report) {
        failures.push("capture session wall-clock bounds are invalid".to_string());
    }
    if !host_evidence_is_healthy(report, config) {
        failures.push("host evidence does not match the configured host guard".to_string());
    }
    if report.capture_session_id.trim().is_empty() {
        failures.push("capture_session_id is empty".to_string());
    }
    if !is_sha256(&report.config_fingerprint) {
        failures.push("config_fingerprint is not lowercase SHA-256".to_string());
    }
    if let Some(source) = &report.config_source {
        if source.source_path.as_os_str().is_empty() {
            failures.push("config source path is empty".to_string());
        }
        if !is_sha256(&source.sha256) {
            failures.push("config source SHA-256 is invalid".to_string());
        }
    }
    if !is_sha256(&report.raw_sha256) {
        failures.push("raw_sha256 is not lowercase SHA-256".to_string());
    }
    match (&report.normalized_path, &report.normalized_sha256) {
        (Some(_), Some(sha256)) if !is_sha256(sha256) => {
            failures.push("normalized_sha256 is not lowercase SHA-256".to_string());
        }
        (Some(_), None) => failures.push("normalized output has no SHA-256".to_string()),
        (None, Some(_)) => failures.push("normalized SHA-256 exists without an output".to_string()),
        (None, None)
            if report.normalized_records != 0
                || report.normalized_bytes != 0
                || report.max_normalized_queue_depth != 0 =>
        {
            failures.push("normalized counters exist without an output".to_string());
        }
        _ => {}
    }
    if report.parsed_events != report.accepted_events.saturating_add(report.duplicates) {
        failures.push("parsed_events does not equal accepted_events plus duplicates".to_string());
    }
    if report.ready_connections_at_stop > report.expected_connections {
        failures.push("ready_connections_at_stop exceeds expected_connections".to_string());
    }
    let expected_books = config
        .subscriptions
        .iter()
        .filter(|subscription| is_book_channel(subscription.channel.trim()))
        .map(|subscription| subscription.symbol.trim())
        .collect::<BTreeSet<_>>();
    let reported_books = report
        .books
        .iter()
        .map(|book| book.symbol.as_str())
        .collect::<BTreeSet<_>>();
    if report.books.len() != reported_books.len() || reported_books != expected_books {
        failures.push("reported book set does not exactly match capture config".to_string());
    }
    failures
}

fn derive_clean_capture(
    report: &CaptureRunReport,
    config: &CaptureConfig,
    expected_connections: usize,
    stream_coverage_complete: bool,
) -> bool {
    let expected_books = config
        .subscriptions
        .iter()
        .filter(|subscription| is_book_channel(subscription.channel.trim()))
        .map(|subscription| subscription.symbol.trim())
        .collect::<BTreeSet<_>>();
    let ready_books = report
        .books
        .iter()
        .filter(|book| book.sequence_status == "ready" && book.book_status == "ready")
        .map(|book| book.symbol.as_str())
        .collect::<BTreeSet<_>>();
    report.stop_reason == CaptureStopReason::DurationElapsed
        && report.reached_all_connections_ready
        && report.expected_connections == expected_connections
        && report.ready_connections_at_stop == expected_connections
        && !expected_books.is_empty()
        && report.books.len() == expected_books.len()
        && ready_books == expected_books
        && stream_coverage_complete
        && report.raw_records > 0
        && (report.normalized_path.is_none() || report.normalized_records > 0)
        && report.parse_errors == 0
        && report.stale_book_events == 0
        && report.recovery_requests == 0
        && report.missing_recovery_routes == 0
        && report.gaps == 0
        && report.recovery_failures == 0
        && session_bounds_are_valid(report)
        && host_evidence_is_healthy(report, config)
}

fn session_bounds_are_valid(report: &CaptureRunReport) -> bool {
    report.session_started_at_ms > 0
        && report.session_completed_at_ms >= report.session_started_at_ms
}

fn host_evidence_is_healthy(report: &CaptureRunReport, config: &CaptureConfig) -> bool {
    if !config.host_guard.enabled {
        return report.host_identity_sha256.is_none()
            && report.host_preflight.is_none()
            && report.host_periodic_checks == 0
            && report.host_last_snapshot.is_none();
    }
    let Some(identity) = report.host_identity_sha256.as_deref() else {
        return false;
    };
    let Some(preflight) = report.host_preflight.as_ref() else {
        return false;
    };
    if !is_sha256(identity)
        || preflight.checked_at_ms < report.session_started_at_ms
        || preflight.checked_at_ms > report.session_completed_at_ms
        || !host_snapshot_is_healthy(preflight, config)
    {
        return false;
    }
    match (
        report.host_periodic_checks,
        report.host_last_snapshot.as_ref(),
    ) {
        (0, None) => true,
        (0, Some(_)) | (1.., None) => false,
        (_, Some(last)) => {
            last.checked_at_ms >= preflight.checked_at_ms
                && last.checked_at_ms <= report.session_completed_at_ms
                && host_snapshot_is_healthy(last, config)
        }
    }
}

fn host_snapshot_is_healthy(
    snapshot: &reap_telemetry::HostHealthSnapshot,
    config: &CaptureConfig,
) -> bool {
    snapshot.checked_at_ms > 0
        && snapshot.disk_available_bytes >= config.host_guard.min_disk_available_bytes
        && snapshot.memory_available_bytes >= config.host_guard.min_memory_available_bytes
        && (!config.host_guard.require_clock_synchronized || snapshot.clock_synchronized)
}

struct NormalizedScan {
    source_path: PathBuf,
    records: u64,
    bytes: u64,
    sha256: String,
    invalid_records: u64,
    first_error: Option<String>,
    stable_while_reading: bool,
}

fn scan_normalized_jsonl(path: &Path) -> Result<NormalizedScan, CaptureVerificationError> {
    let source_path = canonical_regular_file(path, "normalized capture")?;
    let file = File::open(&source_path).map_err(|source| CaptureVerificationError::ReadInput {
        label: "normalized capture",
        path: source_path.clone(),
        source,
    })?;
    let initial_bytes = file
        .metadata()
        .map_err(|source| CaptureVerificationError::ReadInput {
            label: "normalized capture metadata",
            path: source_path.clone(),
            source,
        })?
        .len();
    let mut reader = BufReader::new(file);
    let mut buffer = Vec::new();
    let mut hasher = Sha256::new();
    let mut records = 0_u64;
    let mut bytes = 0_u64;
    let mut invalid_records = 0_u64;
    let mut first_error = None;

    loop {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer).map_err(|source| {
            CaptureVerificationError::ReadInput {
                label: "normalized capture",
                path: source_path.clone(),
                source,
            }
        })?;
        if read == 0 {
            break;
        }
        records = records.saturating_add(1);
        bytes = bytes.saturating_add(read as u64);
        hasher.update(&buffer);
        let canonical = serde_json::from_slice::<NormalizedEvent>(trim_newline(&buffer))
            .ok()
            .and_then(|event| {
                let mut encoded = serde_json::to_vec(&event).ok()?;
                encoded.push(b'\n');
                Some(encoded == buffer)
            })
            .unwrap_or(false);
        if !canonical {
            invalid_records = invalid_records.saturating_add(1);
            if first_error.is_none() {
                first_error = Some(format!("line {records} is not canonical normalized JSONL"));
            }
        }
    }
    Ok(NormalizedScan {
        source_path,
        records,
        bytes,
        sha256: digest_hex(hasher.finalize()),
        invalid_records,
        first_error,
        stable_while_reading: bytes == initial_bytes,
    })
}

fn trim_newline(bytes: &[u8]) -> &[u8] {
    bytes.strip_suffix(b"\n").unwrap_or(bytes)
}

fn read_bounded_regular_file(
    path: &Path,
    label: &'static str,
    limit: u64,
) -> Result<(PathBuf, Vec<u8>), CaptureVerificationError> {
    let canonical = canonical_regular_file(path, label)?;
    let metadata =
        std::fs::metadata(&canonical).map_err(|source| CaptureVerificationError::ReadInput {
            label,
            path: canonical.clone(),
            source,
        })?;
    if metadata.len() > limit {
        return Err(CaptureVerificationError::InputTooLarge {
            label,
            path: canonical,
            actual: metadata.len(),
            limit,
        });
    }
    let bytes =
        std::fs::read(&canonical).map_err(|source| CaptureVerificationError::ReadInput {
            label,
            path: canonical.clone(),
            source,
        })?;
    if bytes.len() as u64 > limit {
        return Err(CaptureVerificationError::InputTooLarge {
            label,
            path: canonical,
            actual: bytes.len() as u64,
            limit,
        });
    }
    Ok((canonical, bytes))
}

fn canonical_regular_file(
    path: &Path,
    label: &'static str,
) -> Result<PathBuf, CaptureVerificationError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        CaptureVerificationError::InvalidInputPath {
            label,
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CaptureVerificationError::InvalidInputPath {
            label,
            path: path.to_path_buf(),
            message: "must be a regular file and not a symbolic link".to_string(),
        });
    }
    std::fs::canonicalize(path).map_err(|error| CaptureVerificationError::InvalidInputPath {
        label,
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use reap_feed::{FeedOutput, FeedProcessor, RawCapture};
    use reap_venue::{VenueAdapter, okx::OkxAdapter};
    use tempfile::TempDir;

    use super::*;
    use crate::{
        CaptureBookHealth, CaptureOutputConfig, CapturePriority, CaptureRuntimeConfig,
        CaptureSubscriptionConfig, CaptureVenueConfig,
    };

    const RAW_FIXTURE: &[u8] = include_bytes!("../../../fixtures/raw/okx/depth-reset.jsonl");

    struct VerificationFixture {
        _directory: TempDir,
        config_path: PathBuf,
        report_path: PathBuf,
        raw_path: PathBuf,
        normalized_path: Option<PathBuf>,
    }

    fn capture_config() -> CaptureConfig {
        CaptureConfig {
            venue: CaptureVenueConfig::default(),
            runtime: CaptureRuntimeConfig::default(),
            output: CaptureOutputConfig::default(),
            host_guard: Default::default(),
            subscriptions: vec![CaptureSubscriptionConfig {
                channel: "books".to_string(),
                symbol: "BTC-USDT".to_string(),
                connections: 2,
                priority: CapturePriority::Critical,
            }],
        }
    }

    fn setup(with_normalized: bool) -> VerificationFixture {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("capture.toml");
        let report_path = directory.path().join("capture-report.json");
        let raw_path = directory.path().join("archived-raw.jsonl");
        let normalized_path =
            with_normalized.then(|| directory.path().join("archived-normalized.jsonl"));
        std::fs::write(&raw_path, RAW_FIXTURE).unwrap();

        let config = capture_config();
        let config_bytes = toml::to_string(&config).unwrap().into_bytes();
        std::fs::write(&config_path, &config_bytes).unwrap();
        let recorded_raw_path = PathBuf::from("collector/original-raw.jsonl");
        let recorded_normalized_path =
            with_normalized.then(|| PathBuf::from("collector/original-normalized.jsonl"));
        let mut effective = config.clone();
        effective.output.raw_path = recorded_raw_path.clone();
        effective.output.normalized_path = recorded_normalized_path.clone();
        let analysis = analyze_capture(RAW_FIXTURE, &effective).unwrap();

        let normalized_bytes =
            with_normalized.then(|| reconstruct_normalized(RAW_FIXTURE, &config));
        if let (Some(path), Some(bytes)) = (&normalized_path, &normalized_bytes) {
            std::fs::write(path, bytes).unwrap();
            assert_eq!(analysis.reconstructed_normalized_records, line_count(bytes));
            assert_eq!(analysis.reconstructed_normalized_bytes, bytes.len() as u64);
            assert_eq!(analysis.reconstructed_normalized_sha256, sha256_hex(bytes));
        }
        let expected_connections = reap_feed::partition_subscriptions(
            &effective.subscriptions(),
            effective.runtime.max_subscriptions_per_socket,
        )
        .unwrap()
        .len();
        let report = CaptureRunReport {
            format_version: CAPTURE_RUN_REPORT_FORMAT_VERSION,
            reap_version: env!("CARGO_PKG_VERSION").to_string(),
            java_reference_revision: PINNED_JAVA_REVISION.to_string(),
            executable_sha256: "e".repeat(64),
            host_identity_sha256: None,
            host_preflight: None,
            host_periodic_checks: 0,
            host_last_snapshot: None,
            session_started_at_ms: 1,
            session_completed_at_ms: 2,
            capture_session_id: analysis.capture_sessions[0].clone(),
            config_fingerprint: effective.fingerprint().unwrap(),
            config_source: Some(crate::CaptureConfigFileEvidence {
                source_path: PathBuf::from("collector/capture.toml"),
                bytes: config_bytes.len() as u64,
                sha256: sha256_hex(&config_bytes),
            }),
            stop_reason: CaptureStopReason::DurationElapsed,
            elapsed_ms: 1_000,
            raw_path: recorded_raw_path,
            normalized_path: recorded_normalized_path,
            raw_records: analysis.lines,
            normalized_records: normalized_bytes
                .as_ref()
                .map_or(0, |bytes| line_count(bytes)),
            raw_bytes: RAW_FIXTURE.len() as u64,
            normalized_bytes: normalized_bytes
                .as_ref()
                .map_or(0, |bytes| bytes.len() as u64),
            raw_sha256: sha256_hex(RAW_FIXTURE),
            normalized_sha256: normalized_bytes.as_ref().map(|bytes| sha256_hex(bytes)),
            max_raw_queue_depth: 1,
            max_normalized_queue_depth: usize::from(with_normalized),
            parsed_events: analysis.parsed_events,
            accepted_events: analysis.accepted_events,
            duplicates: analysis.duplicate_events,
            gaps: analysis.gaps,
            recoveries: analysis.recoveries,
            recovery_failures: analysis.recovery_failures,
            sequence_resets: analysis.sequence_resets,
            same_sequence_updates: analysis.same_sequence_updates,
            recovery_requests: 0,
            missing_recovery_routes: 0,
            parse_errors: analysis.error_count,
            stale_book_events: 0,
            connection_disconnects: 0,
            expected_connections,
            ready_connections_at_stop: expected_connections,
            reached_all_connections_ready: true,
            books: analysis
                .books
                .iter()
                .map(|book| CaptureBookHealth {
                    symbol: book.symbol.clone(),
                    sequence_status: book.sequence_status.clone(),
                    book_status: book.book_status.clone(),
                    last_seq_id: book.last_seq_id,
                    buffered_updates: book.buffered_updates,
                    sequence_resets: book.sequence_resets,
                    same_sequence_updates: book.same_sequence_updates,
                    best_bid: book.best_bid,
                    best_ask: book.best_ask,
                })
                .collect(),
            clean_capture: true,
        };
        write_report(&report_path, &report);

        VerificationFixture {
            _directory: directory,
            config_path,
            report_path,
            raw_path,
            normalized_path,
        }
    }

    fn reconstruct_normalized(raw: &[u8], config: &CaptureConfig) -> Vec<u8> {
        let adapter = OkxAdapter::default();
        let mut processor = FeedProcessor::new(
            config.runtime.dedup_capacity_per_stream,
            config.runtime.max_sequence_buffer,
        );
        let mut output_bytes = Vec::new();
        for line in raw
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            let capture: RawCapture = serde_json::from_slice(line).unwrap();
            let envelope = capture.into_envelope().unwrap();
            for parsed in adapter.parse(&envelope).unwrap() {
                for output in processor.process_from(&envelope.conn_id, parsed) {
                    let event = match output {
                        FeedOutput::Event(event) => Some(event),
                        FeedOutput::System(event) => Some(NormalizedEvent::System(event)),
                        FeedOutput::Duplicate(_)
                        | FeedOutput::RecoveryRequired(_)
                        | FeedOutput::PrivateOrder { .. }
                        | FeedOutput::PrivateFill { .. }
                        | FeedOutput::PrivateAccount { .. } => None,
                    };
                    if let Some(event) = event {
                        serde_json::to_writer(&mut output_bytes, &event).unwrap();
                        output_bytes.push(b'\n');
                    }
                }
            }
        }
        output_bytes
    }

    fn line_count(bytes: &[u8]) -> u64 {
        bytes.iter().filter(|byte| **byte == b'\n').count() as u64
    }

    fn write_report(path: &Path, report: &CaptureRunReport) {
        let mut bytes = serde_json::to_vec_pretty(report).unwrap();
        bytes.push(b'\n');
        std::fs::write(path, bytes).unwrap();
    }

    fn verify(fixture: &VerificationFixture) -> CaptureVerificationReport {
        verify_capture_paths(
            &fixture.config_path,
            &fixture.report_path,
            &fixture.raw_path,
            fixture.normalized_path.as_deref(),
        )
        .unwrap()
    }

    #[test]
    fn verifier_accepts_moved_artifacts_and_reconstructed_normalized_output() {
        let fixture = setup(true);

        let report = verify(&fixture);

        assert!(report.passed, "{report:#?}");
        assert_eq!(report.reap_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(report.java_reference_revision, PINNED_JAVA_REVISION);
        assert_eq!(report.executable_sha256, "e".repeat(64));
        assert!(report.host_identity_sha256.is_none());
        assert_eq!(report.session_started_at_ms, 1);
        assert_eq!(report.session_completed_at_ms, 2);
        assert_ne!(report.raw.source_path, report.raw.recorded_path);
        assert!(report.normalized.as_ref().unwrap().matches_reconstruction == Some(true));
    }

    #[test]
    fn verifier_rejects_build_java_and_host_identity_tampering() {
        let fixture = setup(false);
        let mut run_report: CaptureRunReport =
            serde_json::from_slice(&std::fs::read(&fixture.report_path).unwrap()).unwrap();
        run_report.reap_version = "tampered".to_string();
        run_report.java_reference_revision = "0".repeat(40);
        run_report.executable_sha256 = "E".repeat(64);
        run_report.host_identity_sha256 = Some("9".repeat(64));
        run_report.session_completed_at_ms = 0;
        write_report(&fixture.report_path, &run_report);

        let report = verify(&fixture);
        let invariant_messages = report
            .failures
            .iter()
            .filter_map(|failure| match failure {
                CaptureVerificationFailure::RunReportInvariant { message } => {
                    Some(message.as_str())
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(!report.passed);
        for expected in [
            "reap_version",
            "java_reference_revision",
            "executable_sha256",
            "wall-clock",
            "host evidence",
        ] {
            assert!(
                invariant_messages
                    .iter()
                    .any(|message| message.contains(expected)),
                "missing {expected:?} in {invariant_messages:?}"
            );
        }
    }

    #[test]
    fn verifier_rejects_enabled_guard_without_host_evidence() {
        let fixture = setup(false);
        let mut config = capture_config();
        config.host_guard.enabled = true;
        let config_bytes = toml::to_string(&config).unwrap().into_bytes();
        std::fs::write(&fixture.config_path, &config_bytes).unwrap();

        let mut run_report: CaptureRunReport =
            serde_json::from_slice(&std::fs::read(&fixture.report_path).unwrap()).unwrap();
        let mut effective = config;
        effective.output.raw_path = run_report.raw_path.clone();
        effective.output.normalized_path = run_report.normalized_path.clone();
        run_report.config_fingerprint = effective.fingerprint().unwrap();
        let config_source = run_report.config_source.as_mut().unwrap();
        config_source.bytes = config_bytes.len() as u64;
        config_source.sha256 = sha256_hex(&config_bytes);
        write_report(&fixture.report_path, &run_report);

        let report = verify(&fixture);

        assert!(!report.passed);
        assert!(report.failures.iter().any(|failure| matches!(
            failure,
            CaptureVerificationFailure::RunReportInvariant { message }
                if message.contains("host evidence")
        )));
        assert!(report.failures.iter().any(|failure| matches!(
            failure,
            CaptureVerificationFailure::CleanFlagMismatch {
                reported: true,
                derived: false,
            }
        )));
    }

    #[test]
    fn verifier_rejects_clean_claim_when_configured_stream_is_absent() {
        let fixture = setup(false);
        let mut config = capture_config();
        config.subscriptions.push(CaptureSubscriptionConfig {
            channel: "trades".to_string(),
            symbol: "BTC-USDT".to_string(),
            connections: 2,
            priority: CapturePriority::High,
        });
        let config_bytes = toml::to_string(&config).unwrap().into_bytes();
        std::fs::write(&fixture.config_path, &config_bytes).unwrap();

        let mut run_report: CaptureRunReport =
            serde_json::from_slice(&std::fs::read(&fixture.report_path).unwrap()).unwrap();
        let mut effective = config;
        effective.output.raw_path = run_report.raw_path.clone();
        effective.output.normalized_path = run_report.normalized_path.clone();
        run_report.config_fingerprint = effective.fingerprint().unwrap();
        let expected_connections = reap_feed::partition_subscriptions(
            &effective.subscriptions(),
            effective.runtime.max_subscriptions_per_socket,
        )
        .unwrap()
        .len();
        run_report.expected_connections = expected_connections;
        run_report.ready_connections_at_stop = expected_connections;
        let config_source = run_report.config_source.as_mut().unwrap();
        config_source.bytes = config_bytes.len() as u64;
        config_source.sha256 = sha256_hex(&config_bytes);
        write_report(&fixture.report_path, &run_report);

        let report = verify(&fixture);

        assert!(!report.passed);
        assert!(report.failures.iter().any(|failure| matches!(
            failure,
            CaptureVerificationFailure::CleanFlagMismatch {
                reported: true,
                derived: false,
            }
        )));
        assert!(
            report
                .failures
                .contains(&CaptureVerificationFailure::AnalysisIntegrityUnhealthy)
        );
        assert!(
            report
                .analysis
                .expected_streams
                .iter()
                .any(|stream| stream.channel == "trades" && !stream.complete)
        );
    }

    #[test]
    fn verifier_rejects_the_right_replica_count_on_the_wrong_socket_plan() {
        let fixture = setup(false);
        let raw = std::fs::read_to_string(&fixture.raw_path)
            .unwrap()
            .replace("okx-books-critical-r1-0", "okx-books-critical-r9-0");
        std::fs::write(&fixture.raw_path, raw.as_bytes()).unwrap();

        let mut run_report: CaptureRunReport =
            serde_json::from_slice(&std::fs::read(&fixture.report_path).unwrap()).unwrap();
        run_report.raw_bytes = raw.len() as u64;
        run_report.raw_sha256 = sha256_hex(raw.as_bytes());
        write_report(&fixture.report_path, &run_report);

        let report = verify(&fixture);
        let coverage = &report.analysis.expected_streams[0];

        assert!(!report.passed);
        assert_eq!(coverage.observed_connections, 2);
        assert_eq!(
            coverage.missing_source_connections,
            ["okx-books-critical-r1-0"]
        );
        assert_eq!(
            coverage.unexpected_source_connections,
            ["okx-books-critical-r9-0"]
        );
        assert!(report.failures.iter().any(|failure| matches!(
            failure,
            CaptureVerificationFailure::CleanFlagMismatch {
                reported: true,
                derived: false,
            }
        )));
        assert!(
            report
                .failures
                .contains(&CaptureVerificationFailure::AnalysisIntegrityUnhealthy)
        );
    }

    #[test]
    fn clean_coverage_rejects_an_unclassified_data_frame() {
        let fixture = std::str::from_utf8(RAW_FIXTURE).unwrap();
        let mut unclassified: RawCapture =
            serde_json::from_str(fixture.lines().next().unwrap()).unwrap();
        unclassified.capture_record_seq = Some(8);
        unclassified.symbol = None;
        unclassified.payload["arg"] = serde_json::json!({"channel": "books"});
        if let Some(data) = unclassified.payload["data"].as_array_mut() {
            for row in data {
                if let Some(row) = row.as_object_mut() {
                    row.remove("instId");
                }
            }
        }
        let input = format!(
            "{fixture}{}\n",
            serde_json::to_string(&unclassified).unwrap()
        );

        let analysis = analyze_capture(input.as_bytes(), &capture_config()).unwrap();

        assert!(analysis.expected_streams[0].complete);
        assert!(analysis.error_count > 0);
        assert!(!capture_stream_coverage_complete(&analysis));
    }

    #[test]
    fn verifier_rejects_raw_tampering() {
        let fixture = setup(false);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture.raw_path)
            .unwrap()
            .write_all(b"\n")
            .unwrap();

        let report = verify(&fixture);

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&CaptureVerificationFailure::RawArtifactMismatch)
        );
    }

    #[test]
    fn verifier_rejects_record_sequence_tampering_with_matching_hashes() {
        let fixture = setup(false);
        let mut records = std::fs::read_to_string(&fixture.raw_path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<RawCapture>(line).unwrap())
            .collect::<Vec<_>>();
        records[2].capture_record_seq = None;
        let mut raw_bytes = Vec::new();
        for record in records {
            serde_json::to_writer(&mut raw_bytes, &record).unwrap();
            raw_bytes.push(b'\n');
        }
        std::fs::write(&fixture.raw_path, &raw_bytes).unwrap();

        let mut run_report: CaptureRunReport =
            serde_json::from_slice(&std::fs::read(&fixture.report_path).unwrap()).unwrap();
        run_report.raw_bytes = raw_bytes.len() as u64;
        run_report.raw_sha256 = sha256_hex(&raw_bytes);
        write_report(&fixture.report_path, &run_report);

        let report = verify(&fixture);

        assert!(!report.passed);
        assert!(report.failures.iter().any(|failure| matches!(
            failure,
            CaptureVerificationFailure::RawRecordSequenceIncomplete {
                expected_records: 7,
                sequenced_records: 6,
                ..
            }
        )));
    }

    #[test]
    fn verifier_rejects_normalized_tampering() {
        let fixture = setup(true);
        std::fs::OpenOptions::new()
            .append(true)
            .open(fixture.normalized_path.as_ref().unwrap())
            .unwrap()
            .write_all(b"{}\n")
            .unwrap();

        let report = verify(&fixture);

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&CaptureVerificationFailure::NormalizedArtifactMismatch)
        );
        assert!(report.failures.iter().any(|failure| matches!(
            failure,
            CaptureVerificationFailure::NormalizedArtifactNotCanonical { .. }
        )));
    }

    #[test]
    fn verifier_rejects_config_and_counter_tampering() {
        let fixture = setup(false);
        let mut config = capture_config();
        config.runtime.health_interval_ms += 1;
        std::fs::write(&fixture.config_path, toml::to_string(&config).unwrap()).unwrap();
        let mut run_report: CaptureRunReport =
            serde_json::from_slice(&std::fs::read(&fixture.report_path).unwrap()).unwrap();
        run_report.parsed_events += 1;
        write_report(&fixture.report_path, &run_report);

        let report = verify(&fixture);

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&CaptureVerificationFailure::EffectiveConfigFingerprintMismatch)
        );
        assert!(report.failures.iter().any(|failure| matches!(
            failure,
            CaptureVerificationFailure::CounterMismatch {
                field,
                ..
            } if field == "parsed_events"
        )));
    }

    #[test]
    fn verifier_binds_exact_config_bytes_not_only_effective_values() {
        let fixture = setup(false);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&fixture.config_path)
            .unwrap()
            .write_all(b"\n# formatting-only change\n")
            .unwrap();

        let report = verify(&fixture);

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&CaptureVerificationFailure::ConfigFileMismatch)
        );
        assert!(
            !report
                .failures
                .contains(&CaptureVerificationFailure::EffectiveConfigFingerprintMismatch)
        );
    }

    #[test]
    fn verifier_rejects_legacy_report_without_exact_config_evidence() {
        let fixture = setup(false);
        let mut run_report: CaptureRunReport =
            serde_json::from_slice(&std::fs::read(&fixture.report_path).unwrap()).unwrap();
        run_report.format_version = 2;
        run_report.config_source = None;
        write_report(&fixture.report_path, &run_report);

        let report = verify(&fixture);

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&CaptureVerificationFailure::ConfigFileEvidenceMissing)
        );
        assert!(report.failures.iter().any(|failure| matches!(
            failure,
            CaptureVerificationFailure::UnsupportedRunReportFormat { actual: 2, .. }
        )));
    }

    #[test]
    fn verifier_requires_normalized_artifact_when_report_declares_it() {
        let mut fixture = setup(true);
        fixture.normalized_path = None;

        let report = verify(&fixture);

        assert!(!report.passed);
        assert!(
            report
                .failures
                .contains(&CaptureVerificationFailure::NormalizedArtifactMissing)
        );
    }

    #[test]
    fn strict_run_report_parser_rejects_unknown_fields() {
        let fixture = setup(false);
        let mut report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&fixture.report_path).unwrap()).unwrap();
        report["unexpected"] = serde_json::json!(true);
        std::fs::write(&fixture.report_path, serde_json::to_vec(&report).unwrap()).unwrap();

        let error = verify_capture_paths(
            &fixture.config_path,
            &fixture.report_path,
            &fixture.raw_path,
            None,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            CaptureVerificationError::ParseRunReport { .. }
        ));
    }
}
