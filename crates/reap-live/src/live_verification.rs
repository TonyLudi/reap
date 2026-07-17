use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use reap_core::PINNED_JAVA_REVISION;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::safety_contracts::LiveCleanSoakInputs;
use crate::{
    LIVE_LATENCY_EVIDENCE_SCHEMA_VERSION, LIVE_LATENCY_RESERVOIR_CAPACITY,
    LIVE_RUN_REPORT_SCHEMA_VERSION, LiveConfig, LiveConfigError, LiveMode, LiveRunReport,
    LiveStopReason, MAX_LIVE_FAILURE_CODE_BYTES, MAX_LIVE_FAILURE_MESSAGE_BYTES,
    MAX_LIVE_LATENCY_SERIES, MAX_LIVE_LATENCY_US, StartupGate, load_live_config_with_evidence,
};

pub const LIVE_RUN_VERIFICATION_FORMAT_VERSION: u16 = 2;
pub const MAX_LIVE_RUN_REPORT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveRunFileEvidence {
    pub source_path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveRunConfigVerification {
    pub source_path: PathBuf,
    pub recorded_source_path: Option<PathBuf>,
    pub bytes: u64,
    pub sha256: String,
    pub config_fingerprint: String,
    pub reported_config_fingerprint: String,
    pub evidence_config_fingerprint: String,
    pub reported_evidence_config_fingerprint: String,
    pub file_matches_report: bool,
    pub effective_matches_report: bool,
    pub matches_report: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum LiveRunVerificationFailure {
    UnsupportedReportSchema {
        actual: u32,
        supported: u32,
    },
    ConfigSourceMissing,
    ConfigFileMismatch,
    ConfigFingerprintMismatch,
    EvidenceConfigFingerprintMismatch,
    JavaRevisionMismatch,
    InvalidRunProvenance {
        message: String,
    },
    ExpectedModeMismatch {
        expected: LiveMode,
        actual: LiveMode,
    },
    RunInvariant {
        message: String,
    },
    HostEvidenceMismatch,
    AccountIdentityMismatch,
    RuntimeFailureEvidenceMismatch,
    DisconnectCounterMismatch,
    CleanSoakMismatch {
        reported: bool,
        derived: bool,
    },
    LatencyEvidenceInvariant {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveRunVerificationReport {
    pub format_version: u16,
    pub run_report: LiveRunFileEvidence,
    pub config: LiveRunConfigVerification,
    pub report_schema_version: u32,
    pub reap_version: String,
    pub executable_sha256: String,
    pub host_identity_sha256: Option<String>,
    pub account_identity_sha256s: std::collections::BTreeMap<String, String>,
    pub session_id: Option<String>,
    pub session_started_at_ms: u64,
    pub elapsed_ms: u64,
    pub mode: LiveMode,
    pub stop_reason: LiveStopReason,
    pub expected_mode: Option<LiveMode>,
    pub reported_clean_soak: bool,
    pub derived_clean_soak: bool,
    pub failures: Vec<LiveRunVerificationFailure>,
    pub evidence_valid: bool,
    pub acceptance_passed: bool,
}

#[derive(Debug, Error)]
pub enum LiveRunVerificationError {
    #[error("invalid live run report path {path}: {message}")]
    InvalidReportPath { path: PathBuf, message: String },
    #[error("live run report {path} is {actual} bytes; limit is {limit}")]
    ReportTooLarge {
        path: PathBuf,
        actual: u64,
        limit: u64,
    },
    #[error("failed to read live run report {path}: {source}")]
    ReadReport {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse live run report {path}: {source}")]
    ParseReport {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Config(#[from] LiveConfigError),
}

pub fn verify_live_run_paths(
    config_path: impl AsRef<Path>,
    report_path: impl AsRef<Path>,
    expected_mode: Option<LiveMode>,
) -> Result<LiveRunVerificationReport, LiveRunVerificationError> {
    let (config, config_file) = load_live_config_with_evidence(config_path)?;
    let (report_source, report_bytes) = read_report(report_path.as_ref())?;
    let report: LiveRunReport = serde_json::from_slice(&report_bytes).map_err(|source| {
        LiveRunVerificationError::ParseReport {
            path: report_source.clone(),
            source,
        }
    })?;
    let run_report = LiveRunFileEvidence {
        source_path: report_source,
        bytes: report_bytes.len() as u64,
        sha256: sha256_hex(&report_bytes),
    };

    let config_fingerprint = config.fingerprint()?;
    let evidence_config_fingerprint = config.evidence_fingerprint()?;
    let config_file_matches = report.config_source.as_ref().is_some_and(|source| {
        source.bytes == config_file.bytes && source.sha256 == config_file.sha256
    });
    let effective_matches = report.config_fingerprint == config_fingerprint
        && report.evidence_config_fingerprint == evidence_config_fingerprint;
    let config_verification = LiveRunConfigVerification {
        source_path: config_file.source_path.clone(),
        recorded_source_path: report
            .config_source
            .as_ref()
            .map(|source| source.source_path.clone()),
        bytes: config_file.bytes,
        sha256: config_file.sha256.clone(),
        config_fingerprint: config_fingerprint.clone(),
        reported_config_fingerprint: report.config_fingerprint.clone(),
        evidence_config_fingerprint: evidence_config_fingerprint.clone(),
        reported_evidence_config_fingerprint: report.evidence_config_fingerprint.clone(),
        file_matches_report: config_file_matches,
        effective_matches_report: effective_matches,
        matches_report: config_file_matches && effective_matches,
    };

    let mut failures = Vec::new();
    if report.schema_version != LIVE_RUN_REPORT_SCHEMA_VERSION {
        failures.push(LiveRunVerificationFailure::UnsupportedReportSchema {
            actual: report.schema_version,
            supported: LIVE_RUN_REPORT_SCHEMA_VERSION,
        });
    }
    if report.config_source.is_none() {
        failures.push(LiveRunVerificationFailure::ConfigSourceMissing);
    } else if !config_file_matches {
        failures.push(LiveRunVerificationFailure::ConfigFileMismatch);
    }
    if report.config_fingerprint != config_fingerprint {
        failures.push(LiveRunVerificationFailure::ConfigFingerprintMismatch);
    }
    if report.evidence_config_fingerprint != evidence_config_fingerprint {
        failures.push(LiveRunVerificationFailure::EvidenceConfigFingerprintMismatch);
    }
    if report.java_reference_revision != PINNED_JAVA_REVISION {
        failures.push(LiveRunVerificationFailure::JavaRevisionMismatch);
    }
    if let Some(expected) = expected_mode
        && report.mode != expected
    {
        failures.push(LiveRunVerificationFailure::ExpectedModeMismatch {
            expected,
            actual: report.mode,
        });
    }
    validate_run_provenance(&report, &mut failures);
    validate_run_shape(&report, &config, &mut failures);
    validate_latency_evidence(&report, &mut failures);

    let derived_clean_soak = derive_clean_soak(&report);
    if report.clean_soak != derived_clean_soak {
        failures.push(LiveRunVerificationFailure::CleanSoakMismatch {
            reported: report.clean_soak,
            derived: derived_clean_soak,
        });
    }
    let evidence_valid = failures.is_empty();
    let acceptance_passed = evidence_valid && derived_clean_soak;
    Ok(LiveRunVerificationReport {
        format_version: LIVE_RUN_VERIFICATION_FORMAT_VERSION,
        run_report,
        config: config_verification,
        report_schema_version: report.schema_version,
        reap_version: report.reap_version,
        executable_sha256: report.executable_sha256,
        host_identity_sha256: report.host_identity_sha256,
        account_identity_sha256s: report.account_identity_sha256s,
        session_id: report.session_id,
        session_started_at_ms: report.session_started_at_ms,
        elapsed_ms: report.elapsed_ms,
        mode: report.mode,
        stop_reason: report.stop_reason,
        expected_mode,
        reported_clean_soak: report.clean_soak,
        derived_clean_soak,
        failures,
        evidence_valid,
        acceptance_passed,
    })
}

fn validate_run_provenance(report: &LiveRunReport, failures: &mut Vec<LiveRunVerificationFailure>) {
    if report.reap_version.trim().is_empty() {
        failures.push(LiveRunVerificationFailure::InvalidRunProvenance {
            message: "Reap version is empty".to_string(),
        });
    }
    if !is_lower_sha256(&report.executable_sha256) {
        failures.push(LiveRunVerificationFailure::InvalidRunProvenance {
            message: "executable SHA-256 is invalid".to_string(),
        });
    }
    if report.session_started_at_ms == 0 {
        failures.push(LiveRunVerificationFailure::InvalidRunProvenance {
            message: "session start timestamp is zero".to_string(),
        });
    }
    if report
        .config_source
        .as_ref()
        .is_some_and(|source| !source.source_path.is_absolute())
    {
        failures.push(LiveRunVerificationFailure::InvalidRunProvenance {
            message: "recorded config source path is not absolute".to_string(),
        });
    }
}

fn validate_run_shape(
    report: &LiveRunReport,
    config: &LiveConfig,
    failures: &mut Vec<LiveRunVerificationFailure>,
) {
    let startup_failure = is_pre_session_startup_failure(report);
    if report.reached_ready != report.time_to_ready_ms.is_some()
        || report
            .time_to_ready_ms
            .is_some_and(|time_to_ready_ms| time_to_ready_ms > report.elapsed_ms)
        || report.max_readiness_outage_ms > report.elapsed_ms
        || (report.readiness_loss_count > 0 && !report.reached_ready)
    {
        failures.push(LiveRunVerificationFailure::RunInvariant {
            message: "readiness timing evidence is inconsistent".to_string(),
        });
    }
    for (label, snapshot) in [
        ("readiness_at_stop", &report.readiness_at_stop),
        ("final readiness", &report.readiness),
    ] {
        if snapshot.is_ready()
            && (!snapshot.metadata_verified
                || !snapshot.storage_ready
                || !snapshot.public_connectivity_ready
                || !snapshot.missing_reconciliation.is_empty()
                || !snapshot.missing_account_snapshots.is_empty()
                || !snapshot.missing_books.is_empty()
                || !snapshot.missing_private_streams.is_empty()
                || !snapshot.missing_order_transports.is_empty()
                || !snapshot.missing_stablecoin_rates.is_empty()
                || !snapshot.missing_strategy_references.is_empty()
                || !snapshot.faults.is_empty())
        {
            failures.push(LiveRunVerificationFailure::RunInvariant {
                message: format!("{label} claims ready with unmet readiness inputs"),
            });
        }
    }
    match report.mode {
        LiveMode::Validate => {
            let expected_readiness = StartupGate::new(config).snapshot();
            if report.session_id.is_some()
                || report.stop_reason != LiveStopReason::Validation
                || report.failure.is_some()
                || report.elapsed_ms != 0
                || report.reached_ready
                || report.clean_soak
                || !report.account_identity_sha256s.is_empty()
                || report.readiness_at_stop != expected_readiness
                || report.readiness != expected_readiness
                || has_runtime_evidence(report)
            {
                failures.push(LiveRunVerificationFailure::RunInvariant {
                    message: "validation report has live-session state".to_string(),
                });
            }
        }
        LiveMode::Observe | LiveMode::Demo => {
            if report.mode == LiveMode::Demo && !config.venue.environment.is_demo() {
                failures.push(LiveRunVerificationFailure::RunInvariant {
                    message: "demo report is bound to a production exchange config".to_string(),
                });
            }
            if startup_failure {
                let expected_readiness = StartupGate::new(config).snapshot();
                if report.reached_ready
                    || report.clean_soak
                    || !report.account_identity_sha256s.is_empty()
                    || report.readiness_at_stop != expected_readiness
                    || report.readiness != expected_readiness
                    || has_runtime_evidence(report)
                {
                    failures.push(LiveRunVerificationFailure::RunInvariant {
                        message: "startup failure report has live-session state".to_string(),
                    });
                }
            } else {
                if report
                    .session_id
                    .as_deref()
                    .is_none_or(|session| session.trim().is_empty())
                    || report.stop_reason == LiveStopReason::Validation
                {
                    failures.push(LiveRunVerificationFailure::RunInvariant {
                        message: "live report has no session or uses validation stop reason"
                            .to_string(),
                    });
                }
                let expected_accounts = config
                    .accounts
                    .iter()
                    .map(|account| account.id.as_str())
                    .collect::<BTreeSet<_>>();
                let reported_accounts = report
                    .account_identity_sha256s
                    .keys()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>();
                if reported_accounts != expected_accounts
                    || report
                        .account_identity_sha256s
                        .values()
                        .any(|identity| !is_lower_sha256(identity))
                {
                    failures.push(LiveRunVerificationFailure::AccountIdentityMismatch);
                }
            }
        }
    }

    let host_evidence_matches = if config.host_guard.enabled {
        report
            .host_identity_sha256
            .as_deref()
            .is_some_and(is_lower_sha256)
            && if report.mode == LiveMode::Validate || startup_failure {
                report.host_preflight.is_none()
                    && report.host_checks == 0
                    && report.host_last_snapshot.is_none()
            } else if let (Some(preflight), Some(last)) =
                (&report.host_preflight, &report.host_last_snapshot)
            {
                report.host_checks > 0
                    && preflight.checked_at_ms >= report.session_started_at_ms
                    && last.checked_at_ms >= preflight.checked_at_ms
                    && host_snapshot_is_healthy(preflight, config)
                    && host_snapshot_is_healthy(last, config)
            } else {
                false
            }
    } else {
        report.host_identity_sha256.is_none()
            && report.host_preflight.is_none()
            && report.host_checks == 0
            && report.host_last_snapshot.is_none()
    };
    if !host_evidence_matches {
        failures.push(LiveRunVerificationFailure::HostEvidenceMismatch);
    }

    if report.connection_disconnect_events
        != report
            .public_connection_disconnect_events
            .saturating_add(report.private_connection_disconnect_events)
            .saturating_add(report.order_transport_disconnect_events)
    {
        failures.push(LiveRunVerificationFailure::DisconnectCounterMismatch);
    }
    let failure_matches = match (&report.failure, report.stop_reason) {
        (None, LiveStopReason::RuntimeFailure) => false,
        (Some(_), stop) if stop != LiveStopReason::RuntimeFailure => false,
        (Some(failure), LiveStopReason::RuntimeFailure) => {
            !failure.code.is_empty()
                && failure.code.len() <= MAX_LIVE_FAILURE_CODE_BYTES
                && failure
                    .code
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
                && !failure.message.is_empty()
                && failure.message.len() <= MAX_LIVE_FAILURE_MESSAGE_BYTES
        }
        (Some(_), _) => false,
        (None, _) => true,
    };
    if !failure_matches {
        failures.push(LiveRunVerificationFailure::RuntimeFailureEvidenceMismatch);
    }
}

fn is_pre_session_startup_failure(report: &LiveRunReport) -> bool {
    matches!(report.mode, LiveMode::Observe | LiveMode::Demo)
        && report.session_id.is_none()
        && report.stop_reason == LiveStopReason::RuntimeFailure
        && report.failure.is_some()
}

fn has_runtime_evidence(report: &LiveRunReport) -> bool {
    report.readiness_loss_count != 0
        || report.reconciliation_drift_events != 0
        || report.book_recovery_events != 0
        || report.stream_stale_events != 0
        || report.connection_disconnect_events != 0
        || report.public_connection_disconnect_events != 0
        || report.private_connection_disconnect_events != 0
        || report.order_transport_disconnect_events != 0
        || report.order_transport_stale_events != 0
        || report.ambiguous_submit_events != 0
        || report.ambiguous_cancel_events != 0
        || report.partial_fill_events != 0
        || report.fill_convergence_timeout_events != 0
        || report.order_convergence_timeout_events != 0
        || report.restored_safety_latches != 0
        || report.operator_commands != 0
        || report.operator_mutations != 0
        || report.max_storage_queue_depth != 0
        || report.alerts_delivered != 0
        || report.alert_delivery_failures != 0
        || report.alert_failure_notifications_dropped != 0
        || report.max_alert_queue_depth != 0
        || report.dropped_storage_records != 0
        || report.active_orders_after_shutdown != 0
        || report.latency_evidence != Default::default()
}

fn host_snapshot_is_healthy(snapshot: &crate::HostHealthSnapshot, config: &LiveConfig) -> bool {
    snapshot.checked_at_ms > 0
        && snapshot.disk_available_bytes >= config.host_guard.min_disk_available_bytes
        && snapshot.memory_available_bytes >= config.host_guard.min_memory_available_bytes
        && (!config.host_guard.require_clock_synchronized || snapshot.clock_synchronized)
}

fn validate_latency_evidence(
    report: &LiveRunReport,
    failures: &mut Vec<LiveRunVerificationFailure>,
) {
    let evidence = &report.latency_evidence;
    if evidence.schema_version != LIVE_LATENCY_EVIDENCE_SCHEMA_VERSION
        || evidence.reservoir_capacity_per_series != LIVE_LATENCY_RESERVOIR_CAPACITY
        || evidence.maximum_latency_us != MAX_LIVE_LATENCY_US
        || evidence.series.len() > MAX_LIVE_LATENCY_SERIES
    {
        failures.push(LiveRunVerificationFailure::LatencyEvidenceInvariant {
            message: "latency evidence schema or collector bounds are invalid".to_string(),
        });
    }
    let mut identities = HashSet::new();
    for series in &evidence.series {
        if !identities.insert((series.class, series.symbol.as_str(), series.semantics)) {
            failures.push(LiveRunVerificationFailure::LatencyEvidenceInvariant {
                message: format!(
                    "duplicate latency series {:?}/{}/ {:?}",
                    series.class, series.symbol, series.semantics
                ),
            });
        }
        let invalid_observations = series
            .negative_clock_observations
            .saturating_add(series.above_limit_observations);
        let bounds_valid = if series.valid_observations == 0 {
            series.total_latency_us == 0
                && series.minimum_latency_us.is_none()
                && series.maximum_latency_us.is_none()
                && series.mean_latency_us.is_none()
                && series.retained_samples_us.is_empty()
        } else if let (Some(minimum), Some(maximum), Some(mean)) = (
            series.minimum_latency_us,
            series.maximum_latency_us,
            series.mean_latency_us,
        ) {
            let expected_mean = series.total_latency_us as f64 / series.valid_observations as f64;
            minimum <= maximum
                && maximum <= MAX_LIVE_LATENCY_US
                && mean.is_finite()
                && mean.to_bits() == expected_mean.to_bits()
                && (series.total_latency_us as u128)
                    >= (minimum as u128) * (series.valid_observations as u128)
                && (series.total_latency_us as u128)
                    <= (maximum as u128) * (series.valid_observations as u128)
                && series
                    .retained_samples_us
                    .iter()
                    .all(|sample| (minimum..=maximum).contains(sample))
        } else {
            false
        };
        let expected_retained = usize::try_from(series.valid_observations)
            .unwrap_or(usize::MAX)
            .min(LIVE_LATENCY_RESERVOIR_CAPACITY);
        if series.observations
            != series
                .valid_observations
                .saturating_add(invalid_observations)
            || series.symbol.trim().is_empty()
            || series.retained_samples_us.len() != expected_retained
            || series
                .retained_samples_us
                .iter()
                .any(|sample| *sample > MAX_LIVE_LATENCY_US)
            || !series
                .retained_samples_us
                .windows(2)
                .all(|pair| pair[0] <= pair[1])
            || !bounds_valid
        {
            failures.push(LiveRunVerificationFailure::LatencyEvidenceInvariant {
                message: format!(
                    "latency series {:?}/{} has inconsistent counters, bounds, or samples",
                    series.class, series.symbol
                ),
            });
        }
    }
}

fn derive_clean_soak(report: &LiveRunReport) -> bool {
    LiveCleanSoakInputs {
        duration_elapsed: report.stop_reason == LiveStopReason::DurationElapsed,
        reached_ready: report.reached_ready,
        readiness_at_stop_ready: report.readiness_at_stop.is_ready(),
        reconciliation_drift_free: report.reconciliation_drift_events == 0,
        operator_mutation_free: report.operator_mutations == 0,
        storage_records_complete: report.dropped_storage_records == 0,
        no_active_orders_after_shutdown: report.active_orders_after_shutdown == 0,
        alert_delivery_failure_free: report.alert_delivery_failures == 0,
    }
    .qualifies_as_clean_soak()
}

fn read_report(path: &Path) -> Result<(PathBuf, Vec<u8>), LiveRunVerificationError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        LiveRunVerificationError::InvalidReportPath {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(LiveRunVerificationError::InvalidReportPath {
            path: path.to_path_buf(),
            message: "must be a regular file and not a symbolic link".to_string(),
        });
    }
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        LiveRunVerificationError::InvalidReportPath {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    if metadata.len() > MAX_LIVE_RUN_REPORT_BYTES {
        return Err(LiveRunVerificationError::ReportTooLarge {
            path: canonical,
            actual: metadata.len(),
            limit: MAX_LIVE_RUN_REPORT_BYTES,
        });
    }
    let bytes =
        std::fs::read(&canonical).map_err(|source| LiveRunVerificationError::ReadReport {
            path: canonical.clone(),
            source,
        })?;
    if bytes.len() as u64 > MAX_LIVE_RUN_REPORT_BYTES {
        return Err(LiveRunVerificationError::ReportTooLarge {
            path: canonical,
            actual: bytes.len() as u64,
            limit: MAX_LIVE_RUN_REPORT_BYTES,
        });
    }
    Ok((canonical, bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LiveRunOptions, run_live_path};

    struct Fixture {
        _directory: tempfile::TempDir,
        config_path: PathBuf,
        report_path: PathBuf,
        report: LiveRunReport,
    }

    async fn fixture() -> Fixture {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("live.toml");
        let report_path = directory.path().join("live-report.json");
        std::fs::write(
            &config_path,
            include_bytes!("../../../examples/live-okx-demo.toml"),
        )
        .unwrap();
        let report = run_live_path(
            &config_path,
            LiveRunOptions {
                mode: LiveMode::Validate,
                demo_confirmed: false,
                run_duration: None,
            },
        )
        .await
        .unwrap();
        write_report(&report_path, &report);
        Fixture {
            _directory: directory,
            config_path,
            report_path,
            report,
        }
    }

    fn write_report(path: &Path, report: &LiveRunReport) {
        let mut bytes = serde_json::to_vec_pretty(report).unwrap();
        bytes.push(b'\n');
        std::fs::write(path, bytes).unwrap();
    }

    #[tokio::test]
    async fn verifier_accepts_source_bound_validation_report() {
        let fixture = fixture().await;

        let verification = verify_live_run_paths(
            &fixture.config_path,
            &fixture.report_path,
            Some(LiveMode::Validate),
        )
        .unwrap();

        assert!(verification.evidence_valid, "{verification:#?}");
        assert!(!verification.acceptance_passed);
        assert!(verification.config.matches_report);
        assert!(fixture.report.config_source.is_some());
        assert_eq!(verification.reap_version, fixture.report.reap_version);
        assert_eq!(
            verification.executable_sha256,
            fixture.report.executable_sha256
        );
        assert_eq!(
            verification.host_identity_sha256,
            fixture.report.host_identity_sha256
        );
        assert_eq!(
            verification.account_identity_sha256s,
            fixture.report.account_identity_sha256s
        );

        let mismatch = verify_live_run_paths(
            &fixture.config_path,
            &fixture.report_path,
            Some(LiveMode::Demo),
        )
        .unwrap();
        assert!(mismatch.failures.iter().any(|failure| matches!(
            failure,
            LiveRunVerificationFailure::ExpectedModeMismatch {
                expected: LiveMode::Demo,
                actual: LiveMode::Validate,
            }
        )));
    }

    #[tokio::test]
    async fn verifier_rejects_formatting_only_config_tampering() {
        let fixture = fixture().await;
        let mut config = std::fs::read(&fixture.config_path).unwrap();
        config.extend_from_slice(b"\n# formatting-only tamper\n");
        std::fs::write(&fixture.config_path, config).unwrap();

        let verification =
            verify_live_run_paths(&fixture.config_path, &fixture.report_path, None).unwrap();

        assert!(!verification.evidence_valid);
        assert!(
            verification
                .failures
                .contains(&LiveRunVerificationFailure::ConfigFileMismatch)
        );
        assert!(verification.config.effective_matches_report);
    }

    #[tokio::test]
    async fn verifier_rejects_legacy_report_without_config_source() {
        let fixture = fixture().await;
        let mut report = fixture.report;
        report.schema_version = 6;
        report.config_source = None;
        write_report(&fixture.report_path, &report);

        let verification =
            verify_live_run_paths(&fixture.config_path, &fixture.report_path, None).unwrap();

        assert!(!verification.evidence_valid);
        assert!(
            verification
                .failures
                .contains(&LiveRunVerificationFailure::ConfigSourceMissing)
        );
        assert!(verification.failures.iter().any(|failure| matches!(
            failure,
            LiveRunVerificationFailure::UnsupportedReportSchema { actual: 6, .. }
        )));
    }

    #[tokio::test]
    async fn verifier_rejects_validation_report_with_runtime_evidence() {
        let fixture = fixture().await;
        let mut report = fixture.report;
        report.operator_commands = 1;
        write_report(&fixture.report_path, &report);

        let verification =
            verify_live_run_paths(&fixture.config_path, &fixture.report_path, None).unwrap();

        assert!(!verification.evidence_valid);
        assert!(verification.failures.iter().any(|failure| matches!(
            failure,
            LiveRunVerificationFailure::RunInvariant { message }
                if message == "validation report has live-session state"
        )));
    }

    #[tokio::test]
    async fn verifier_accepts_startup_failure_as_diagnostic_evidence_only() {
        let fixture = fixture().await;
        let mut report = fixture.report;
        report.mode = LiveMode::Observe;
        report.stop_reason = LiveStopReason::RuntimeFailure;
        report.failure = Some(crate::LiveFailureEvidence {
            code: "host_guard".to_string(),
            message: "startup failed before a reportable runtime session".to_string(),
        });
        report.elapsed_ms = 7;
        write_report(&fixture.report_path, &report);

        let verification = verify_live_run_paths(
            &fixture.config_path,
            &fixture.report_path,
            Some(LiveMode::Observe),
        )
        .unwrap();

        assert!(verification.evidence_valid, "{verification:#?}");
        assert!(!verification.acceptance_passed);
        assert!(!verification.derived_clean_soak);
    }

    #[tokio::test]
    async fn verifier_rejects_forged_startup_runtime_state_and_session() {
        let fixture = fixture().await;
        let mut report = fixture.report.clone();
        report.mode = LiveMode::Observe;
        report.stop_reason = LiveStopReason::RuntimeFailure;
        report.failure = Some(crate::LiveFailureEvidence {
            code: "host_guard".to_string(),
            message: "startup failed before a reportable runtime session".to_string(),
        });
        report.operator_commands = 1;
        write_report(&fixture.report_path, &report);

        let runtime_state =
            verify_live_run_paths(&fixture.config_path, &fixture.report_path, None).unwrap();
        assert!(!runtime_state.evidence_valid);
        assert!(runtime_state.failures.iter().any(|failure| matches!(
            failure,
            LiveRunVerificationFailure::RunInvariant { message }
                if message == "startup failure report has live-session state"
        )));

        report.operator_commands = 0;
        report.session_id = Some("forged-session".to_string());
        write_report(&fixture.report_path, &report);
        let session =
            verify_live_run_paths(&fixture.config_path, &fixture.report_path, None).unwrap();
        assert!(!session.evidence_valid);
        assert!(
            session
                .failures
                .contains(&LiveRunVerificationFailure::AccountIdentityMismatch)
        );
    }

    #[tokio::test]
    async fn verifier_rederives_clean_soak_instead_of_trusting_flag() {
        let fixture = fixture().await;
        let mut report = fixture.report;
        report.clean_soak = true;
        write_report(&fixture.report_path, &report);

        let verification =
            verify_live_run_paths(&fixture.config_path, &fixture.report_path, None).unwrap();

        assert!(!verification.evidence_valid);
        assert!(
            verification
                .failures
                .contains(&LiveRunVerificationFailure::CleanSoakMismatch {
                    reported: true,
                    derived: false,
                })
        );
    }
}
