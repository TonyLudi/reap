use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use reap_core::PINNED_JAVA_REVISION;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::config::{FaultProxyConfig, FaultProxyConfigEvidence};
use crate::protocol::{FaultProxyRunReport, RUN_REPORT_FORMAT_VERSION};

pub const FAULT_PROXY_RUN_VERIFICATION_FORMAT_VERSION: u16 = 1;
pub const MAX_FAULT_PROXY_RUN_REPORT_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_FAULT_PROXY_WALL_MONOTONIC_DRIFT_MS: u64 = 5_000;
const MAX_FAULT_PROXY_STOP_REASON_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaultProxyRunFileEvidence {
    pub source_path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum FaultProxyRunVerificationFailure {
    UnsupportedFormat {
        actual: u32,
        supported: u32,
    },
    ConfigMismatch,
    JavaRevisionMismatch,
    ReapVersionInvalid,
    ExecutableSha256Invalid,
    HostIdentitySha256Invalid,
    ProxySessionIdInvalid,
    StatusSessionMismatch,
    TimestampInvalid,
    WallMonotonicElapsedMismatch {
        wall_elapsed_ms: u64,
        monotonic_elapsed_ms: u64,
        maximum_difference_ms: u64,
    },
    StopReasonInvalid,
    ListenerTasksNotJoined,
    ControlSocketNotRemoved,
    PendingFaultsRemain {
        rest: usize,
        websocket: usize,
    },
    WebsocketConnectionsRemain {
        active: BTreeMap<String, u64>,
    },
    ProxyErrorsPresent {
        count: u64,
        retained: usize,
    },
    CleanShutdownMismatch {
        reported: bool,
        derived: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaultProxyRunVerificationReport {
    pub format_version: u16,
    pub verifier_reap_version: String,
    pub config: FaultProxyConfigEvidence,
    pub run_report: FaultProxyRunFileEvidence,
    pub report_format_version: u32,
    pub java_reference_revision: String,
    pub reap_version: String,
    pub executable_sha256: String,
    pub host_identity_sha256: String,
    pub proxy_session_id: String,
    pub started_at_ms: u64,
    pub stopped_at_ms: u64,
    pub elapsed_ms: u64,
    pub stop_reason: String,
    pub completed_faults: u64,
    pub listener_tasks_joined_cleanly: bool,
    pub control_socket_removed: bool,
    pub reported_clean_shutdown: bool,
    pub derived_clean_shutdown: bool,
    pub failures: Vec<FaultProxyRunVerificationFailure>,
    pub limitations: Vec<String>,
    pub evidence_valid: bool,
    pub acceptance_passed: bool,
}

#[derive(Debug, Error)]
pub enum FaultProxyRunVerificationError {
    #[error("invalid {label} path {path}: {message}")]
    InvalidPath {
        label: &'static str,
        path: PathBuf,
        message: String,
    },
    #[error("{label} {path} is {actual} bytes; maximum is {maximum}")]
    InputTooLarge {
        label: &'static str,
        path: PathBuf,
        actual: u64,
        maximum: u64,
    },
    #[error("failed to read {label} {path}: {source}")]
    ReadInput {
        label: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse strict fault-proxy run report {path}: {source}")]
    ParseReport {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("failed to load exact fault-proxy config: {0}")]
    Config(#[from] crate::config::FaultProxyConfigError),
    #[error("{0} changed while the fault-proxy run report was being verified")]
    InputChanged(&'static str),
}

pub fn verify_fault_proxy_run_paths(
    config_path: impl AsRef<Path>,
    report_path: impl AsRef<Path>,
) -> Result<FaultProxyRunVerificationReport, FaultProxyRunVerificationError> {
    let (config_path, _) = read_bounded_regular_file(
        config_path.as_ref(),
        "fault-proxy config",
        MAX_FAULT_PROXY_RUN_REPORT_BYTES,
    )?;
    let (_, config) = FaultProxyConfig::load(&config_path)?;
    let (report_path, report_bytes) = read_bounded_regular_file(
        report_path.as_ref(),
        "fault-proxy run report",
        MAX_FAULT_PROXY_RUN_REPORT_BYTES,
    )?;
    let report: FaultProxyRunReport = serde_json::from_slice(&report_bytes).map_err(|source| {
        FaultProxyRunVerificationError::ParseReport {
            path: report_path.clone(),
            source,
        }
    })?;
    let run_report = file_evidence(report_path.clone(), &report_bytes);
    let mut failures = Vec::new();

    if report.format_version != RUN_REPORT_FORMAT_VERSION {
        failures.push(FaultProxyRunVerificationFailure::UnsupportedFormat {
            actual: report.format_version,
            supported: RUN_REPORT_FORMAT_VERSION,
        });
    }
    if report.config.bytes != config.bytes
        || report.config.sha256 != config.sha256
        || report.config.effective_fingerprint != config.effective_fingerprint
    {
        failures.push(FaultProxyRunVerificationFailure::ConfigMismatch);
    }
    if report.java_reference_revision != PINNED_JAVA_REVISION {
        failures.push(FaultProxyRunVerificationFailure::JavaRevisionMismatch);
    }
    if report.reap_version.is_empty()
        || report.reap_version.trim() != report.reap_version
        || report.reap_version.len() > 128
    {
        failures.push(FaultProxyRunVerificationFailure::ReapVersionInvalid);
    }
    if !is_sha256(&report.executable_sha256) {
        failures.push(FaultProxyRunVerificationFailure::ExecutableSha256Invalid);
    }
    if !is_sha256(&report.host_identity_sha256) {
        failures.push(FaultProxyRunVerificationFailure::HostIdentitySha256Invalid);
    }
    if !valid_identifier(&report.proxy_session_id) {
        failures.push(FaultProxyRunVerificationFailure::ProxySessionIdInvalid);
    }
    let status_session_matches = report.status.proxy_session_id == report.proxy_session_id;
    if !status_session_matches {
        failures.push(FaultProxyRunVerificationFailure::StatusSessionMismatch);
    }

    let timestamps_valid = report.started_at_ms > 0
        && report.stopped_at_ms >= report.started_at_ms
        && report.elapsed_ms > 0;
    if !timestamps_valid {
        failures.push(FaultProxyRunVerificationFailure::TimestampInvalid);
    } else {
        let wall_elapsed_ms = report.stopped_at_ms - report.started_at_ms;
        if wall_elapsed_ms.abs_diff(report.elapsed_ms) > MAX_FAULT_PROXY_WALL_MONOTONIC_DRIFT_MS {
            failures.push(
                FaultProxyRunVerificationFailure::WallMonotonicElapsedMismatch {
                    wall_elapsed_ms,
                    monotonic_elapsed_ms: report.elapsed_ms,
                    maximum_difference_ms: MAX_FAULT_PROXY_WALL_MONOTONIC_DRIFT_MS,
                },
            );
        }
    }
    if report.stop_reason.is_empty()
        || report.stop_reason.len() > MAX_FAULT_PROXY_STOP_REASON_BYTES
        || report.stop_reason.chars().any(char::is_control)
    {
        failures.push(FaultProxyRunVerificationFailure::StopReasonInvalid);
    }
    if !report.listener_tasks_joined_cleanly {
        failures.push(FaultProxyRunVerificationFailure::ListenerTasksNotJoined);
    }
    if !report.control_socket_removed {
        failures.push(FaultProxyRunVerificationFailure::ControlSocketNotRemoved);
    }
    if report.status.pending_rest_faults != 0 || report.status.pending_websocket_faults != 0 {
        failures.push(FaultProxyRunVerificationFailure::PendingFaultsRemain {
            rest: report.status.pending_rest_faults,
            websocket: report.status.pending_websocket_faults,
        });
    }
    let expected_connection_keys = [
        ("order".to_string(), 0),
        ("private".to_string(), 0),
        ("public".to_string(), 0),
    ]
    .into_iter()
    .collect::<BTreeMap<_, _>>();
    if report.status.websocket_connections_active != expected_connection_keys {
        failures.push(
            FaultProxyRunVerificationFailure::WebsocketConnectionsRemain {
                active: report.status.websocket_connections_active.clone(),
            },
        );
    }
    if report.status.proxy_errors != 0 || !report.status.recent_errors.is_empty() {
        failures.push(FaultProxyRunVerificationFailure::ProxyErrorsPresent {
            count: report.status.proxy_errors,
            retained: report.status.recent_errors.len(),
        });
    }
    let derived_clean_shutdown = report.listener_tasks_joined_cleanly
        && report.control_socket_removed
        && status_session_matches
        && report.status.pending_rest_faults == 0
        && report.status.pending_websocket_faults == 0
        && report.status.websocket_connections_active == expected_connection_keys
        && report.status.proxy_errors == 0
        && report.status.recent_errors.is_empty();
    if report.clean_shutdown != derived_clean_shutdown {
        failures.push(FaultProxyRunVerificationFailure::CleanShutdownMismatch {
            reported: report.clean_shutdown,
            derived: derived_clean_shutdown,
        });
    }

    let (_, config_final) = FaultProxyConfig::load(&config_path)?;
    if config_final != config {
        return Err(FaultProxyRunVerificationError::InputChanged(
            "fault-proxy config",
        ));
    }
    let (_, report_final) = read_bounded_regular_file(
        &report_path,
        "fault-proxy run report",
        MAX_FAULT_PROXY_RUN_REPORT_BYTES,
    )?;
    if report_final != report_bytes {
        return Err(FaultProxyRunVerificationError::InputChanged(
            "fault-proxy run report",
        ));
    }

    let evidence_valid = failures.is_empty();
    let acceptance_passed = evidence_valid && derived_clean_shutdown && report.clean_shutdown;
    Ok(FaultProxyRunVerificationReport {
        format_version: FAULT_PROXY_RUN_VERIFICATION_FORMAT_VERSION,
        verifier_reap_version: env!("CARGO_PKG_VERSION").to_string(),
        config,
        run_report,
        report_format_version: report.format_version,
        java_reference_revision: report.java_reference_revision,
        reap_version: report.reap_version,
        executable_sha256: report.executable_sha256,
        host_identity_sha256: report.host_identity_sha256,
        proxy_session_id: report.proxy_session_id,
        started_at_ms: report.started_at_ms,
        stopped_at_ms: report.stopped_at_ms,
        elapsed_ms: report.elapsed_ms,
        stop_reason: report.stop_reason,
        completed_faults: report.status.completed_faults,
        listener_tasks_joined_cleanly: report.listener_tasks_joined_cleanly,
        control_socket_removed: report.control_socket_removed,
        reported_clean_shutdown: report.clean_shutdown,
        derived_clean_shutdown,
        failures,
        limitations: vec![
            "the run report records in-process proxy state; build and host hashes are provenance identifiers, not remote attestation"
                .to_string(),
            "clean shutdown proves recorded listener, socket, pending-fault, connection, and error invariants; an external supervisor must still prove process lifecycle and timing"
                .to_string(),
            "this verifier does not bind the run to a live fault scenario or injector evidence; the production bundle performs those cross-artifact checks"
                .to_string(),
        ],
        evidence_valid,
        acceptance_passed,
    })
}

fn read_bounded_regular_file(
    path: &Path,
    label: &'static str,
    maximum: u64,
) -> Result<(PathBuf, Vec<u8>), FaultProxyRunVerificationError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        FaultProxyRunVerificationError::InvalidPath {
            label,
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(FaultProxyRunVerificationError::InvalidPath {
            label,
            path: path.to_path_buf(),
            message: "must be a regular file and not a symbolic link".to_string(),
        });
    }
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        FaultProxyRunVerificationError::InvalidPath {
            label,
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    if metadata.len() > maximum {
        return Err(FaultProxyRunVerificationError::InputTooLarge {
            label,
            path: canonical,
            actual: metadata.len(),
            maximum,
        });
    }
    let bytes =
        std::fs::read(&canonical).map_err(|source| FaultProxyRunVerificationError::ReadInput {
            label,
            path: canonical.clone(),
            source,
        })?;
    if bytes.len() as u64 > maximum {
        return Err(FaultProxyRunVerificationError::InputTooLarge {
            label,
            path: canonical,
            actual: bytes.len() as u64,
            maximum,
        });
    }
    Ok((canonical, bytes))
}

fn file_evidence(path: PathBuf, bytes: &[u8]) -> FaultProxyRunFileEvidence {
    FaultProxyRunFileEvidence {
        source_path: path,
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::FaultProxyStatus;

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, FaultProxyRunReport) {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("proxy.toml");
        std::fs::write(
            &config_path,
            include_bytes!("../../../examples/okx-demo-fault-proxy.toml"),
        )
        .unwrap();
        let (_, config) = FaultProxyConfig::load(&config_path).unwrap();
        let report_path = directory.path().join("run.json");
        let report = FaultProxyRunReport {
            format_version: RUN_REPORT_FORMAT_VERSION,
            proxy_session_id: "proxy-session".to_string(),
            config,
            java_reference_revision: PINNED_JAVA_REVISION.to_string(),
            reap_version: "0.1.0".to_string(),
            executable_sha256: "1".repeat(64),
            host_identity_sha256: "2".repeat(64),
            started_at_ms: 1_000,
            stopped_at_ms: 2_000,
            elapsed_ms: 1_000,
            stop_reason: "duration_elapsed".to_string(),
            status: FaultProxyStatus {
                proxy_session_id: "proxy-session".to_string(),
                websocket_connections_active: BTreeMap::from([
                    ("order".to_string(), 0),
                    ("private".to_string(), 0),
                    ("public".to_string(), 0),
                ]),
                completed_faults: 1,
                ..FaultProxyStatus::default()
            },
            listener_tasks_joined_cleanly: true,
            control_socket_removed: true,
            clean_shutdown: true,
        };
        std::fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();
        (directory, config_path, report_path, report)
    }

    #[test]
    fn exact_clean_run_passes_and_tampering_fails() {
        let (_directory, config_path, report_path, mut report) = fixture();
        let verified = verify_fault_proxy_run_paths(&config_path, &report_path).unwrap();
        assert!(verified.acceptance_passed, "{verified:#?}");
        assert_eq!(verified.completed_faults, 1);

        report.status.pending_rest_faults = 1;
        std::fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();
        let rejected = verify_fault_proxy_run_paths(&config_path, &report_path).unwrap();
        assert!(!rejected.acceptance_passed);
        assert!(rejected.failures.iter().any(|failure| matches!(
            failure,
            FaultProxyRunVerificationFailure::PendingFaultsRemain { .. }
        )));
        assert!(rejected.failures.iter().any(|failure| matches!(
            failure,
            FaultProxyRunVerificationFailure::CleanShutdownMismatch { .. }
        )));
    }

    #[test]
    fn config_provenance_session_and_timing_tampering_are_rejected() {
        let (_directory, config_path, report_path, mut report) = fixture();
        report.config.sha256 = "3".repeat(64);
        report.executable_sha256 = "invalid".to_string();
        report.status.proxy_session_id = "different-session".to_string();
        report.stopped_at_ms = 20_000;
        std::fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();

        let rejected = verify_fault_proxy_run_paths(&config_path, &report_path).unwrap();
        assert!(!rejected.acceptance_passed);
        assert!(
            rejected
                .failures
                .iter()
                .any(|failure| matches!(failure, FaultProxyRunVerificationFailure::ConfigMismatch))
        );
        assert!(rejected.failures.iter().any(|failure| matches!(
            failure,
            FaultProxyRunVerificationFailure::ExecutableSha256Invalid
        )));
        assert!(rejected.failures.iter().any(|failure| matches!(
            failure,
            FaultProxyRunVerificationFailure::StatusSessionMismatch
        )));
        assert!(rejected.failures.iter().any(|failure| matches!(
            failure,
            FaultProxyRunVerificationFailure::WallMonotonicElapsedMismatch { .. }
        )));
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_link_inputs_are_rejected() {
        use std::os::unix::fs::symlink;

        let (directory, config_path, report_path, _) = fixture();
        let linked = directory.path().join("linked.json");
        symlink(&report_path, &linked).unwrap();
        assert!(verify_fault_proxy_run_paths(&config_path, &linked).is_err());
    }

    #[test]
    fn unknown_report_fields_are_rejected() {
        let (_directory, config_path, report_path, report) = fixture();
        let mut value = serde_json::to_value(report).unwrap();
        value["unexpected"] = serde_json::json!(true);
        std::fs::write(&report_path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(verify_fault_proxy_run_paths(&config_path, &report_path).is_err());
    }
}
